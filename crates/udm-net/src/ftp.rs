//! Minimal in-house FTP / FTPS client (ADR 0001): passive mode, binary RETR,
//! `REST` for ranges, bounded control lines.
//!
//! `ftp://` is plain FTP. `ftps://` requires TLS: implicit TLS is tried first and,
//! when the server answers in plain text, the client reconnects and upgrades with
//! `AUTH TLS` (explicit FTPS). Certificate verification is the same as for HTTPS.

use std::net::IpAddr;

use bytes::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use udm_core::{ByteRange, limits};
use udm_proto::{FtpReply, FtpReplyParser};

use crate::body::{BodyStream, Overflow};
use crate::client::Client;
use crate::config::{ProbeInfo, Protocol, Request};
use crate::connect::{self, BoxIo, Target, with_timeout};
use crate::error::{NetError, TimeoutPhase, bounded_text};

const READ_CHUNK: usize = 65_536;
const FTP_ERROR_TEXT_CHARS: usize = 160;
const ANONYMOUS_USER: &str = "anonymous";
const ANONYMOUS_PASS: &str = "anonymous@";

/// The control connection.
pub(crate) struct FtpControl {
    io: BoxIo,
    buf: Vec<u8>,
    first_byte: std::time::Duration,
}

fn ftp_error(r: &FtpReply) -> NetError {
    NetError::Ftp {
        code: r.code,
        text: bounded_text(
            r.lines.first().map(String::as_str).unwrap_or(""),
            FTP_ERROR_TEXT_CHARS,
        ),
    }
}

fn expect(r: FtpReply, ok: &[u16]) -> Result<FtpReply, NetError> {
    if ok.contains(&r.code) {
        Ok(r)
    } else {
        Err(ftp_error(&r))
    }
}

impl FtpControl {
    fn new(io: BoxIo, first_byte: std::time::Duration) -> Self {
        Self {
            io,
            buf: Vec::new(),
            first_byte,
        }
    }

    /// Reads one line (bounded by `MAX_FTP_LINE_BYTES`), without CR/LF.
    async fn read_line(&mut self) -> Result<String, NetError> {
        let max = limits::MAX_FTP_LINE_BYTES.saturating_add(2);
        loop {
            if let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
                if pos > max {
                    return Err(NetError::Protocol);
                }
                let rest = self.buf.split_off(pos.saturating_add(1));
                let line = std::mem::replace(&mut self.buf, rest);
                let line = String::from_utf8_lossy(&line).into_owned();
                return Ok(line.trim_end_matches(['\r', '\n']).to_owned());
            }
            if self.buf.len() > max {
                return Err(NetError::Protocol);
            }
            let mut chunk = [0u8; 1024];
            let n = self.io.read(&mut chunk).await.map_err(|_| NetError::Io)?;
            if n == 0 {
                return Err(NetError::Io);
            }
            self.buf
                .extend_from_slice(chunk.get(..n).ok_or(NetError::Io)?);
        }
    }

    /// Reads one complete reply (bounded lines, first-byte timeout).
    async fn reply(&mut self) -> Result<FtpReply, NetError> {
        let t = self.first_byte;
        with_timeout(TimeoutPhase::FirstByte, t, async {
            let mut parser = FtpReplyParser::new();
            loop {
                let line = self.read_line().await?;
                match parser.push_line(&line) {
                    Ok(Some(r)) => return Ok(r),
                    Ok(None) => {}
                    Err(_) => return Err(NetError::Protocol),
                }
            }
        })
        .await
    }

    /// Final reply, skipping preliminary `1xx` replies.
    async fn final_reply(&mut self) -> Result<FtpReply, NetError> {
        loop {
            let r = self.reply().await?;
            if !(100..200).contains(&r.code) {
                return Ok(r);
            }
        }
    }

    async fn send(&mut self, line: &str) -> Result<(), NetError> {
        if line.contains(['\r', '\n', '\0']) {
            return Err(NetError::InvalidRequest);
        }
        let mut out = String::with_capacity(line.len().saturating_add(2));
        out.push_str(line);
        out.push_str("\r\n");
        self.io
            .write_all(out.as_bytes())
            .await
            .map_err(|_| NetError::Io)?;
        self.io.flush().await.map_err(|_| NetError::Io)
    }

    /// Sends a command and returns its final reply.
    async fn cmd(&mut self, line: &str) -> Result<FtpReply, NetError> {
        self.send(line).await?;
        self.final_reply().await
    }
}

/// Data connection plus the control connection that must stay open while it is read.
pub(crate) struct FtpData {
    data: BoxIo,
    control: FtpControl,
    buf: Vec<u8>,
}

impl FtpData {
    /// Reads raw bytes; `None` at end of data.
    pub(crate) async fn read_chunk(&mut self) -> Result<Option<Bytes>, NetError> {
        if self.buf.len() < READ_CHUNK {
            self.buf.resize(READ_CHUNK, 0);
        }
        match self.data.read(&mut self.buf).await {
            Ok(0) => Ok(None),
            Ok(n) => Ok(Some(Bytes::copy_from_slice(
                self.buf.get(..n).ok_or(NetError::Io)?,
            ))),
            // TLS peers that close without close_notify: treat as end of data;
            // the caller's length check detects truncation.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(None),
            Err(_) => Err(NetError::Io),
        }
    }

    /// Reads the transfer-complete reply after the data connection ended.
    pub(crate) async fn complete(&mut self) -> Result<(), NetError> {
        let r = self.control.final_reply().await?;
        expect(r, &[226, 250]).map(|_| ())
    }
}

/// Decoded, validated path for FTP commands.
fn ftp_path(u: &url::Url) -> Result<String, NetError> {
    let raw = u.path();
    let decoded = percent_decode(raw);
    if decoded.is_empty()
        || decoded == "/"
        || decoded.contains(['\r', '\n', '\0'])
        || decoded.len() > limits::MAX_URL_BYTES
    {
        return Err(NetError::InvalidRequest);
    }
    Ok(decoded)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut it = bytes.iter().copied();
    while let Some(b) = it.next() {
        if b == b'%' {
            let mut look = it.clone();
            let hi = look.next().and_then(|c| char::from(c).to_digit(16));
            let lo = look.next().and_then(|c| char::from(c).to_digit(16));
            if let (Some(h), Some(l)) = (hi, lo) {
                let v = h.saturating_mul(16).saturating_add(l);
                out.push(u8::try_from(v).unwrap_or(b'?'));
                it = look;
                continue;
            }
        }
        out.push(b);
    }
    String::from_utf8_lossy(&out).into_owned()
}

struct Session {
    control: FtpControl,
    target: Target,
    peer_ip: Option<IpAddr>,
    tls: bool,
}

impl Client {
    async fn ftp_connect_plain(
        &self,
        target: &Target,
    ) -> Result<(FtpControl, Option<IpAddr>), NetError> {
        let t = &self.cfg.timeouts;
        let (io, ip) = connect::open(self.cfg.proxy.as_ref(), t, target).await?;
        let mut c = FtpControl::new(io, t.first_byte());
        let greet = c.final_reply().await?;
        expect(greet, &[220])?;
        Ok((c, ip))
    }

    async fn ftp_open(&self, url: &url::Url) -> Result<Session, NetError> {
        let t = &self.cfg.timeouts;
        match url.scheme() {
            "ftp" => {
                let target = Target::from_url(url, 21)?;
                let (control, peer_ip) = self.ftp_connect_plain(&target).await?;
                Ok(Session {
                    control,
                    target,
                    peer_ip,
                    tls: false,
                })
            }
            "ftps" => {
                let target = Target::from_url(url, 990)?;
                // Implicit TLS first.
                let (io, ip) = connect::open(self.cfg.proxy.as_ref(), t, &target).await?;
                match connect::tls(&self.tls.ftp, t, io, &target.host).await {
                    Ok((io, _)) => {
                        let mut c = FtpControl::new(io, t.first_byte());
                        expect(c.final_reply().await?, &[220])?;
                        return Ok(Session {
                            control: c,
                            target,
                            peer_ip: ip,
                            tls: true,
                        });
                    }
                    Err(e @ NetError::TlsUntrusted { .. }) => return Err(e),
                    Err(e) => {
                        tracing::debug!(error = %e, "implicit FTPS failed; trying AUTH TLS");
                    }
                }
                // Explicit FTPS: plain greeting, then AUTH TLS.
                let (mut c, ip) = self.ftp_connect_plain(&target).await?;
                expect(c.cmd("AUTH TLS").await?, &[234])?;
                if !c.buf.is_empty() {
                    // Plain-text bytes after 234 would be injected into the TLS session.
                    return Err(NetError::Protocol);
                }
                let (io, _) = connect::tls(&self.tls.ftp, t, c.io, &target.host).await?;
                Ok(Session {
                    control: FtpControl::new(io, t.first_byte()),
                    target,
                    peer_ip: ip,
                    tls: true,
                })
            }
            s => Err(NetError::UnsupportedScheme {
                scheme: bounded_text(s, 16),
            }),
        }
    }

    /// Connects, logs in and selects binary mode.
    async fn ftp_login(&self, req: &Request) -> Result<(Session, url::Url), NetError> {
        let (url, url_creds) = udm_proto::split_credentials(&req.url);
        let mut s = self.ftp_open(&url).await?;
        let explicit = req
            .credentials
            .as_ref()
            .map(|(u, p)| (u.clone(), p.expose().to_owned()))
            .or(url_creds);
        let has_creds = explicit.is_some();
        let (user, pass) =
            explicit.unwrap_or_else(|| (ANONYMOUS_USER.to_owned(), ANONYMOUS_PASS.to_owned()));
        let r = s.control.cmd(&format!("USER {user}")).await?;
        let r = if r.code == 331 {
            s.control.cmd(&format!("PASS {pass}")).await?
        } else {
            r
        };
        match r.code {
            230 | 202 => {}
            530 if has_creds => return Err(NetError::AuthFailed),
            530 | 331 | 332 => return Err(NetError::AuthRequired),
            _ => return Err(ftp_error(&r)),
        }
        if s.tls {
            expect(s.control.cmd("PBSZ 0").await?, &[200])?;
            expect(s.control.cmd("PROT P").await?, &[200])?;
        }
        expect(s.control.cmd("TYPE I").await?, &[200])?;
        Ok((s, url))
    }

    async fn ftp_size(s: &mut Session, path: &str) -> Result<Option<u64>, NetError> {
        let r = s.control.cmd(&format!("SIZE {path}")).await?;
        match r.code {
            213 => Ok(r.lines.first().and_then(|l| l.trim().parse::<u64>().ok())),
            550 => Err(ftp_error(&r)),
            _ => Ok(None),
        }
    }

    /// FTP probe: `SIZE`, `MDTM`, `REST 0`.
    pub(crate) async fn ftp_probe(&self, req: &Request) -> Result<ProbeInfo, NetError> {
        let (mut s, url) = self.ftp_login(req).await?;
        let path = ftp_path(&url)?;
        let size = Self::ftp_size(&mut s, &path).await?;
        let r = s.control.cmd(&format!("MDTM {path}")).await?;
        let last_modified = if r.code == 213 {
            udm_proto::parse_mdtm(&r.text()).map(udm_proto::format_http_date)
        } else {
            None
        };
        let rest = s.control.cmd("REST 0").await?;
        let accepts_ranges = rest.code == 350 && size.is_some();
        // Best effort; the connection is dropped right after.
        let _ = s.control.send("QUIT").await;
        Ok(ProbeInfo {
            final_url: url,
            size,
            accepts_ranges,
            etag: None,
            last_modified,
            filename: None,
            content_type: None,
            protocol: if s.tls { Protocol::Ftps } else { Protocol::Ftp },
        })
    }

    /// Opens a passive data connection (EPSV, falling back to PASV). The address in
    /// a PASV reply is ignored (anti-bounce): data always goes to the control peer.
    async fn ftp_passive(&self, s: &mut Session) -> Result<BoxIo, NetError> {
        let r = s.control.cmd("EPSV").await?;
        let port = if r.code == 229 {
            udm_proto::parse_epsv(&r.text()).ok_or(NetError::Protocol)?
        } else {
            let r = expect(s.control.cmd("PASV").await?, &[227])?;
            udm_proto::parse_pasv(&r.text())
                .ok_or(NetError::Protocol)?
                .1
        };
        let t = &self.cfg.timeouts;
        match (s.peer_ip, self.cfg.proxy.as_ref()) {
            (Some(ip), None) => connect::open_ip(t, ip, port).await,
            _ => {
                let target = Target {
                    host: s.target.host.clone(),
                    port,
                };
                connect::open(self.cfg.proxy.as_ref(), t, &target)
                    .await
                    .map(|(io, _)| io)
            }
        }
    }

    /// FTP fetch: `REST` + `RETR`. A range yields `Partial` (stopped after the
    /// range length), no range yields `Full`.
    pub(crate) async fn ftp_fetch(
        &self,
        req: &Request,
        range: Option<ByteRange>,
        expected_total: Option<u64>,
    ) -> Result<(BodyStream, bool), NetError> {
        let (mut s, url) = self.ftp_login(req).await?;
        let path = ftp_path(&url)?;
        let size = Self::ftp_size(&mut s, &path).await?;
        if let (Some(a), Some(b)) = (size, expected_total)
            && a != b
        {
            return Err(NetError::RemoteChanged);
        }
        if let Some(r) = range {
            if r.is_empty() {
                return Err(NetError::InvalidRequest);
            }
            if r.start() > 0 {
                expect(s.control.cmd(&format!("REST {}", r.start())).await?, &[350])?;
            }
        }
        let data = self.ftp_passive(&mut s).await?;
        s.control.send(&format!("RETR {path}")).await?;
        let r = s.control.reply().await?;
        if !matches!(r.code, 125 | 150) {
            return Err(ftp_error(&r));
        }
        let data = if s.tls {
            connect::tls(&self.tls.ftp, &self.cfg.timeouts, data, &s.target.host)
                .await?
                .0
        } else {
            data
        };
        let ftp_data = FtpData {
            data,
            control: s.control,
            buf: Vec::new(),
        };
        let idle = self.cfg.timeouts.idle();
        Ok(match range {
            Some(r) => (
                BodyStream::ftp(ftp_data, Some(r.len()), Overflow::Stop, idle),
                true,
            ),
            None => (
                BodyStream::ftp(ftp_data, expected_total.or(size), Overflow::Error, idle),
                false,
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_path() {
        let u = url::Url::parse("ftp://h/files/a%20b.bin").unwrap();
        assert_eq!(ftp_path(&u).unwrap(), "/files/a b.bin");
    }

    #[test]
    fn hostile_ftp_path_crlf_injection_rejected() {
        let u = url::Url::parse("ftp://h/a%0d%0aDELE%20x").unwrap();
        assert_eq!(ftp_path(&u), Err(NetError::InvalidRequest));
    }

    #[test]
    fn hostile_ftp_path_root_rejected() {
        let u = url::Url::parse("ftp://h/").unwrap();
        assert!(ftp_path(&u).is_err());
    }
}
