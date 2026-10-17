//! Minimal client for the control protocol (CLI, native host, tests).

use std::net::{Ipv4Addr, SocketAddr};

use serde_json::Value;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use udm_core::{Message, MsgArg, limits};

use crate::protocol::Request;
use crate::server::{Line, read_line_bounded};

/// A connection to a control server.
#[derive(Debug)]
pub struct ControlClient {
    rd: BufReader<OwnedReadHalf>,
    wr: OwnedWriteHalf,
    next_id: u64,
}

impl ControlClient {
    /// Connects to `127.0.0.1:port`.
    pub async fn connect(port: u16) -> Result<Self, Message> {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let s = TcpStream::connect(addr).await.map_err(|_| {
            Message::new("error-control-connect").with_uint("port", u64::from(port))
        })?;
        let (rd, wr) = s.into_split();
        Ok(Self {
            rd: BufReader::new(rd),
            wr,
            next_id: 1,
        })
    }

    /// Sends a raw line and returns the raw response line.
    pub async fn raw(&mut self, line: &str) -> Result<String, Message> {
        let io = |_| Message::new("error-control-io");
        self.wr.write_all(line.as_bytes()).await.map_err(io)?;
        self.wr.write_all(b"\n").await.map_err(io)?;
        self.wr.flush().await.map_err(io)?;
        match read_line_bounded(&mut self.rd, limits::MAX_CONTROL_MESSAGE_BYTES).await {
            Ok(Line::Data(b)) => Ok(String::from_utf8_lossy(&b).into_owned()),
            Ok(Line::TooLong) => Err(crate::protocol::too_large()),
            Ok(Line::Eof) | Err(_) => Err(Message::new("error-control-io")),
        }
    }

    /// Calls a method; an error response becomes `Err` with its message.
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value, Message> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let line = self
            .raw(&Request::new(id, method, params).to_line())
            .await?;
        let v: Value =
            serde_json::from_str(&line).map_err(|_| Message::new("error-control-bad-json"))?;
        if v.get("ok") == Some(&Value::Bool(true)) {
            return Ok(v.get("result").cloned().unwrap_or(Value::Null));
        }
        Err(v.get("error").map_or_else(
            || Message::new("error-control-bad-request"),
            message_from_json,
        ))
    }
}

/// Inverse of [`crate::message_to_json`] (numbers come back as `UInt`/`Int`).
pub fn message_from_json(v: &Value) -> Message {
    let key = v
        .get("key")
        .and_then(Value::as_str)
        .unwrap_or("error-control-bad-request");
    let mut m = Message::new(key);
    if let Some(args) = v.get("args").and_then(Value::as_object) {
        for (k, a) in args {
            let arg = match a {
                Value::String(s) => MsgArg::Str(s.clone()),
                Value::Number(n) => match (n.as_u64(), n.as_i64()) {
                    (Some(u), _) => MsgArg::UInt(u),
                    (None, Some(i)) => MsgArg::Int(i),
                    _ => MsgArg::Str(n.to_string()),
                },
                other => MsgArg::Str(other.to_string()),
            };
            m.args.push((k.clone(), arg));
        }
    }
    m
}
