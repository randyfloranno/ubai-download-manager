//! TCP server on 127.0.0.1. One JSON request per line, answered in order.
//! Lines and connections are bounded; each request runs in its own task so a
//! panic in a handler becomes an `error-engine-internal` response.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, watch};
use udm_core::{Message, limits};

use crate::handler::Handler;
use crate::protocol::{err_line, ok_line, parse_request, too_large};

/// Control server factory.
#[derive(Debug)]
pub struct ControlServer;

/// A running server; dropping it does not stop it, call [`ServerHandle::stop`].
#[derive(Debug)]
pub struct ServerHandle {
    addr: SocketAddr,
    stop: watch::Sender<bool>,
}

impl ServerHandle {
    /// Bound address (port 0 resolves to the real port).
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Stops accepting connections and closes open ones.
    pub fn stop(&self) {
        // No receiver left means the server already ended.
        let _ = self.stop.send(true);
    }
}

impl ControlServer {
    /// Binds `127.0.0.1:port` and serves requests with `handler`.
    pub async fn bind(port: u16, handler: Arc<dyn Handler>) -> std::io::Result<ServerHandle> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
        let addr = listener.local_addr()?;
        let (stop, stop_rx) = watch::channel(false);
        tokio::spawn(accept_loop(listener, handler, stop_rx));
        Ok(ServerHandle { addr, stop })
    }
}

async fn accept_loop(
    listener: TcpListener,
    handler: Arc<dyn Handler>,
    mut stop: watch::Receiver<bool>,
) {
    let slots = Arc::new(Semaphore::new(limits::MAX_CONTROL_CONNECTIONS));
    loop {
        let accepted = tokio::select! {
            _ = stop.changed() => return,
            a = listener.accept() => a,
        };
        let (mut stream, peer) = match accepted {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!("control accept failed: {e}");
                continue;
            }
        };
        if !peer.ip().is_loopback() {
            continue;
        }
        let Ok(permit) = Arc::clone(&slots).try_acquire_owned() else {
            let line = err_line(&Value::Null, &Message::new("error-control-busy"));
            let _ = write_line(&mut stream, &line).await;
            continue;
        };
        let h = Arc::clone(&handler);
        let stop_rx = stop.clone();
        tokio::spawn(async move {
            serve_connection(stream, h, stop_rx).await;
            drop(permit);
        });
    }
}

async fn write_line(stream: &mut (impl AsyncWriteExt + Unpin), line: &str) -> std::io::Result<()> {
    stream.write_all(line.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await
}

/// Outcome of reading one bounded line.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Line {
    /// A complete line (without the newline; a final line without newline also counts).
    Data(Vec<u8>),
    /// The line exceeded the limit; the rest was not read.
    TooLong,
    /// End of stream.
    Eof,
}

/// Reads one line of at most `max` bytes.
pub(crate) async fn read_line_bounded(
    r: &mut (impl AsyncBufRead + Unpin),
    max: usize,
) -> std::io::Result<Line> {
    let mut out = Vec::new();
    loop {
        let buf = r.fill_buf().await?;
        if buf.is_empty() {
            return Ok(if out.is_empty() {
                Line::Eof
            } else {
                Line::Data(out)
            });
        }
        let (chunk, found) = match buf.iter().position(|b| *b == b'\n') {
            Some(i) => (buf.get(..i).unwrap_or(&[]), Some(i)),
            None => (buf, None),
        };
        if out.len().saturating_add(chunk.len()) > max {
            return Ok(Line::TooLong);
        }
        out.extend_from_slice(chunk);
        let used = found.map_or(buf.len(), |i| i.saturating_add(1));
        r.consume(used);
        if found.is_some() {
            if out.last() == Some(&b'\r') {
                out.pop();
            }
            return Ok(Line::Data(out));
        }
    }
}

async fn serve_connection(
    stream: TcpStream,
    handler: Arc<dyn Handler>,
    mut stop: watch::Receiver<bool>,
) {
    let (rd, mut wr) = stream.into_split();
    let mut rd = BufReader::new(rd);
    loop {
        let line = tokio::select! {
            _ = stop.changed() => return,
            l = read_line_bounded(&mut rd, limits::MAX_CONTROL_MESSAGE_BYTES) => l,
        };
        let reply = match line {
            Err(_) | Ok(Line::Eof) => return,
            Ok(Line::TooLong) => {
                let _ = write_line(&mut wr, &err_line(&Value::Null, &too_large())).await;
                return;
            }
            Ok(Line::Data(bytes)) if bytes.iter().all(u8::is_ascii_whitespace) => continue,
            Ok(Line::Data(bytes)) => match parse_request(&bytes) {
                Err((id, m)) => err_line(&id, &m),
                Ok(req) => {
                    let id = req.id.clone();
                    let h = Arc::clone(&handler);
                    match tokio::spawn(async move { h.handle(req).await }).await {
                        Ok(Ok(v)) => ok_line(&id, v),
                        Ok(Err(m)) => err_line(&id, &m),
                        Err(_) => err_line(&id, &Message::new("error-engine-internal")),
                    }
                }
            },
        };
        if write_line(&mut wr, &reply).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handler::BoxFuture;
    use crate::protocol::Request;
    use tokio::io::AsyncReadExt;

    struct Echo;
    impl Handler for Echo {
        fn handle(&self, req: Request) -> BoxFuture<'_, Result<Value, Message>> {
            Box::pin(async move {
                match req.method.as_str() {
                    "boom" => panic!("handler panic"),
                    "fail" => Err(Message::new("error-x")),
                    _ => Ok(req.params),
                }
            })
        }
    }

    async fn roundtrip(addr: SocketAddr, lines: &[&str]) -> Vec<Value> {
        let mut s = TcpStream::connect(addr).await.unwrap();
        for l in lines {
            s.write_all(l.as_bytes()).await.unwrap();
            s.write_all(b"\n").await.unwrap();
        }
        s.shutdown().await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn serves_requests_and_survives_panics() {
        let srv = ControlServer::bind(0, Arc::new(Echo)).await.unwrap();
        assert!(srv.addr().ip().is_loopback());
        let r = roundtrip(
            srv.addr(),
            &[
                r#"{"id":1,"method":"echo","params":{"a":1}}"#,
                r#"{"id":2,"method":"boom"}"#,
                "garbage",
                r#"{"id":"x","method":"fail"}"#,
            ],
        )
        .await;
        assert_eq!(r.len(), 4);
        assert_eq!(r[0]["result"]["a"], 1);
        assert_eq!(r[1]["error"]["key"], "error-engine-internal");
        assert_eq!(r[2]["error"]["key"], "error-control-bad-json");
        assert_eq!(r[3]["id"], "x");
        srv.stop();
    }

    #[tokio::test]
    async fn hostile_control_line_too_long_closes() {
        let srv = ControlServer::bind(0, Arc::new(Echo)).await.unwrap();
        let mut s = TcpStream::connect(srv.addr()).await.unwrap();
        let big = vec![b'x'; limits::MAX_CONTROL_MESSAGE_BYTES + 10];
        // The server may close before everything is written.
        let _ = s.write_all(&big).await;
        let mut out = String::new();
        let _ = s.read_to_string(&mut out).await;
        let v: Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
        assert_eq!(v["error"]["key"], "error-control-message-too-large");
        srv.stop();
    }

    #[tokio::test]
    async fn hostile_control_connections_bounded() {
        let srv = ControlServer::bind(0, Arc::new(Echo)).await.unwrap();
        let mut held = Vec::new();
        for _ in 0..limits::MAX_CONTROL_CONNECTIONS {
            let mut s = TcpStream::connect(srv.addr()).await.unwrap();
            // A round trip proves the connection holds a slot.
            s.write_all(b"{\"id\":1,\"method\":\"echo\"}\n")
                .await
                .unwrap();
            let mut r = BufReader::new(s);
            let mut l = String::new();
            r.read_line(&mut l).await.unwrap();
            held.push(r);
        }
        let mut extra = TcpStream::connect(srv.addr()).await.unwrap();
        let mut out = String::new();
        extra.read_to_string(&mut out).await.unwrap();
        assert!(out.contains("error-control-busy"), "{out}");
        srv.stop();
    }

    #[tokio::test]
    async fn bounded_line_reader() {
        let mut r = BufReader::new(&b"ab\r\ncdef\nlast"[..]);
        assert_eq!(
            read_line_bounded(&mut r, 3).await.unwrap(),
            Line::Data(b"ab".to_vec())
        );
        assert_eq!(read_line_bounded(&mut r, 3).await.unwrap(), Line::TooLong);
        let mut r = BufReader::new(&b"last"[..]);
        assert_eq!(
            read_line_bounded(&mut r, 8).await.unwrap(),
            Line::Data(b"last".to_vec())
        );
        assert_eq!(read_line_bounded(&mut r, 8).await.unwrap(), Line::Eof);
    }
}
