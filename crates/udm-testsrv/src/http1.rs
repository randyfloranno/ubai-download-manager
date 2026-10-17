//! Raw HTTP/1.1 transport (one request per connection, `Connection: close`),
//! so faults can lie about lengths, stream endless headers, etc.

use crate::plan::{BodyGen, HttpState, Next, Plan, reason, route};
use crate::util::{MAX_REQUEST_HEAD_BYTES, parse_request, read_head};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(crate) async fn serve<S: AsyncRead + AsyncWrite + Unpin + Send>(
    state: Arc<HttpState>,
    mut s: S,
) {
    let head = match read_head(&mut s, MAX_REQUEST_HEAD_BYTES).await {
        Ok(Some((head, _left))) => head,
        Ok(None) => return,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::InvalidData {
                let _ = s
                    .write_all(b"HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await;
                let _ = s.shutdown().await;
            }
            return;
        }
    };
    let Some(req) = parse_request(&head) else {
        let _ = s
            .write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await;
        let _ = s.shutdown().await;
        return;
    };
    state.requests.fetch_add(1, Ordering::Relaxed);
    let resp = match route(&state, &req) {
        Plan::Stall => {
            // Hold the connection open, never answer, until the peer leaves.
            let mut sink = [0u8; 1024];
            while let Ok(n) = s.read(&mut sink).await {
                if n == 0 {
                    break;
                }
            }
            return;
        }
        Plan::Respond(r) => r,
    };

    let mut head = format!("HTTP/1.1 {} {}\r\n", resp.status, reason(resp.status)).into_bytes();
    for (k, v) in &resp.headers {
        head.extend_from_slice(k.as_bytes());
        head.extend_from_slice(b": ");
        head.extend_from_slice(v);
        head.extend_from_slice(b"\r\n");
    }
    head.extend_from_slice(b"Connection: close\r\n");
    if resp.endless_headers {
        if s.write_all(&head).await.is_err() {
            return;
        }
        let mut i: u64 = 0;
        loop {
            let line = format!("X-Filler-{i}: {}\r\n", "f".repeat(48));
            if s.write_all(line.as_bytes()).await.is_err() {
                return;
            }
            i = i.wrapping_add(1);
            if i & 0xff == 0 {
                tokio::task::yield_now().await;
            }
        }
    }
    head.extend_from_slice(b"\r\n");
    if s.write_all(&head).await.is_err() {
        return;
    }
    if !resp.head_only {
        let mut body_gen = BodyGen::new(resp.body, resp.cut, resp.slow);
        loop {
            match body_gen.next().await {
                Next::Data(b) => {
                    if s.write_all(&b).await.is_err() {
                        return;
                    }
                }
                Next::Done => break,
                Next::Cut => {
                    // Abrupt close mid-body.
                    let _ = s.flush().await;
                    return;
                }
            }
        }
    }
    let _ = s.flush().await;
    let _ = s.shutdown().await;
}
