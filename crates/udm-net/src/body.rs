//! Bounded, timed response body streams.

use std::time::Duration;

use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;

use crate::error::{NetError, TimeoutPhase};
use crate::ftp::FtpData;
use crate::http::KeepAlive;

enum Source {
    Http {
        body: Incoming,
        // Keeps the HTTP/2 connection handle alive while the stream is read.
        _keep: KeepAlive,
    },
    Ftp(Box<FtpData>),
    Done,
}

/// What happens when more bytes arrive than `limit` allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Overflow {
    /// The server sent more than asked: yield up to the limit, then fail.
    Error,
    /// Reading stops at the limit (FTP `REST`+`RETR` partial read).
    Stop,
}

/// A response body. Every chunk read is guarded by the idle timeout; bytes past
/// the requested length are never yielded.
pub struct BodyStream {
    source: Source,
    limit: Option<u64>,
    expected: Option<u64>,
    overflow: Overflow,
    received: u64,
    idle: Duration,
    pending: Option<NetError>,
    finished: bool,
}

impl std::fmt::Debug for BodyStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BodyStream")
            .field("expected", &self.expected)
            .field("received", &self.received)
            .field("finished", &self.finished)
            .finish()
    }
}

impl BodyStream {
    pub(crate) fn http(
        body: Incoming,
        keep: KeepAlive,
        expected: Option<u64>,
        idle: Duration,
    ) -> Self {
        Self {
            source: Source::Http { body, _keep: keep },
            limit: expected,
            expected,
            overflow: Overflow::Error,
            received: 0,
            idle,
            pending: None,
            finished: false,
        }
    }

    pub(crate) fn ftp(
        data: FtpData,
        expected: Option<u64>,
        overflow: Overflow,
        idle: Duration,
    ) -> Self {
        Self {
            source: Source::Ftp(Box::new(data)),
            limit: expected,
            expected,
            overflow,
            received: 0,
            idle,
            pending: None,
            finished: false,
        }
    }

    /// Number of bytes this stream is expected to deliver, if known.
    pub fn expected_len(&self) -> Option<u64> {
        self.expected
    }

    /// Bytes yielded so far.
    pub fn received(&self) -> u64 {
        self.received
    }

    async fn next_raw(&mut self) -> Result<Option<Bytes>, NetError> {
        let expected = self.expected.unwrap_or(0);
        let received = self.received;
        match &mut self.source {
            Source::Done => Ok(None),
            Source::Ftp(d) => d.read_chunk().await,
            Source::Http { body, .. } => loop {
                match body.frame().await {
                    None => return Ok(None),
                    Some(Err(e)) => {
                        return Err(if e.is_incomplete_message() || is_unexpected_eof(&e) {
                            NetError::ShortBody { expected, received }
                        } else if e.is_timeout() {
                            NetError::Timeout {
                                phase: TimeoutPhase::Idle,
                                ms: 0,
                            }
                        } else {
                            // HTTP/2 RST_STREAM, connection reset, chunked framing errors.
                            NetError::Stream
                        });
                    }
                    Some(Ok(frame)) => {
                        if let Ok(data) = frame.into_data() {
                            return Ok(Some(data));
                        }
                        // Trailers are ignored.
                    }
                }
            },
        }
    }

    fn finish(&mut self) {
        self.finished = true;
        // Dropping the source closes the connection(s).
        self.source = Source::Done;
    }

    /// Reads the next chunk. `Ok(None)` at a complete end of body.
    ///
    /// Errors: [`NetError::Timeout`] after `idle_ms` of silence,
    /// [`NetError::ShortBody`] if the body ends early, [`NetError::ExtraBytes`]
    /// (after the in-range bytes were yielded) if the server sends more than asked.
    pub async fn next_chunk(&mut self) -> Result<Option<Bytes>, NetError> {
        if let Some(e) = self.pending.take() {
            self.finish();
            return Err(e);
        }
        if self.finished {
            return Ok(None);
        }
        loop {
            let raw = match tokio::time::timeout(self.idle, self.next_raw()).await {
                Ok(r) => r,
                Err(_) => Err(NetError::Timeout {
                    phase: TimeoutPhase::Idle,
                    ms: u64::try_from(self.idle.as_millis()).unwrap_or(u64::MAX),
                }),
            };
            let chunk = match raw {
                Ok(c) => c,
                Err(e) => {
                    self.finish();
                    return Err(e);
                }
            };
            let Some(mut bytes) = chunk else {
                // End of body.
                if let Some(exp) = self.expected
                    && self.received < exp
                {
                    let e = NetError::ShortBody {
                        expected: exp,
                        received: self.received,
                    };
                    self.finish();
                    return Err(e);
                }
                let done = match &mut self.source {
                    Source::Ftp(d) => d.complete().await,
                    _ => Ok(()),
                };
                self.finish();
                return done.map(|()| None);
            };
            if bytes.is_empty() {
                continue;
            }
            if let Some(limit) = self.limit {
                let room = limit.saturating_sub(self.received);
                if room == 0 {
                    self.finish();
                    return match self.overflow {
                        Overflow::Error => Err(NetError::ExtraBytes { expected: limit }),
                        Overflow::Stop => Ok(None),
                    };
                }
                let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                if len > room {
                    bytes.truncate(usize::try_from(room).unwrap_or(usize::MAX));
                    if self.overflow == Overflow::Error {
                        self.pending = Some(NetError::ExtraBytes { expected: limit });
                    }
                }
            }
            let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            self.received = self.received.saturating_add(len);
            if self.overflow == Overflow::Stop && self.limit == Some(self.received) {
                // Partial FTP read done: close data and control connections.
                self.finish();
            }
            return Ok(Some(bytes));
        }
    }
}

/// True when the error chain contains an I/O `UnexpectedEof` (peer closed mid-body).
fn is_unexpected_eof(e: &hyper::Error) -> bool {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    let mut depth = 0u8;
    while let Some(err) = cur {
        if let Some(io) = err.downcast_ref::<std::io::Error>() {
            return io.kind() == std::io::ErrorKind::UnexpectedEof;
        }
        depth = depth.saturating_add(1);
        if depth > 8 {
            break;
        }
        cur = err.source();
    }
    false
}
