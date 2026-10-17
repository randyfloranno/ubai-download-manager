//! Minimal FTP / FTPS test server (passive mode only, binary RETR).
//!
//! Paths: `/files/<name>` (fixtures), `/drop/<N>/<name>` (data connection is
//! closed after N bytes, reply 426), `/longline` (reply line longer than
//! [`FTP_MAX_LINE`]). Commands `LONGLINE` and `FLOOD` trigger an over-long reply
//! line and an endless multi-line reply.

use crate::fixture::{Fixture, fixture};
use crate::server::{Tracker, bind_loopback};
use crate::tls;
use crate::util::{Io, mdtm_time};
use crate::{AUTH_PASSWORD, AUTH_USER};
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;

/// Maximum control line length the server accepts (and the length the
/// `LONGLINE` fault exceeds).
pub const FTP_MAX_LINE: usize = 4096;

/// Fixed `MDTM` answer for every fixture.
pub const FTP_MDTM: &str = "20151021072800";

const DATA_ACCEPT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Plain,
    Explicit,
    Implicit,
}

struct FtpShared {
    mode: Mode,
    acceptor: Option<TlsAcceptor>,
    retr: AtomicUsize,
    tracker: Arc<Tracker>,
}

/// FTP test server on 127.0.0.1.
pub struct FtpTestServer {
    port: u16,
    shared: Arc<FtpShared>,
    cert_der: Option<Vec<u8>>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl FtpTestServer {
    /// Plain FTP (no TLS).
    pub async fn start() -> io::Result<Self> {
        Self::start_mode(Mode::Plain).await
    }

    /// FTP with explicit FTPS (`AUTH TLS`, `PBSZ 0`, `PROT P`) available.
    pub async fn start_explicit_tls() -> io::Result<Self> {
        Self::start_mode(Mode::Explicit).await
    }

    /// Implicit FTPS: TLS from the first byte; data channels are protected.
    pub async fn start_implicit_tls() -> io::Result<Self> {
        Self::start_mode(Mode::Implicit).await
    }

    async fn start_mode(mode: Mode) -> io::Result<Self> {
        let listener = bind_loopback(0).await?;
        let port = listener.local_addr()?.port();
        let (acceptor, cert_der) = if mode == Mode::Plain {
            (None, None)
        } else {
            (
                Some(TlsAcceptor::from(tls::server_config(&[])?)),
                Some(tls::test_cert_der()?),
            )
        };
        let shared = Arc::new(FtpShared {
            mode,
            acceptor,
            retr: AtomicUsize::new(0),
            tracker: Arc::new(Tracker::default()),
        });
        let sh = shared.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    continue;
                };
                let sh2 = sh.clone();
                let h = tokio::spawn(async move {
                    let ctrl: Box<dyn Io> = if sh2.mode == Mode::Implicit {
                        let Some(acc) = sh2.acceptor.clone() else {
                            return;
                        };
                        match acc.accept(tcp).await {
                            Ok(t) => Box::new(t),
                            Err(_) => return,
                        }
                    } else {
                        Box::new(tcp)
                    };
                    let mut s = Session::new(sh2, ctrl);
                    let _ = s.run().await;
                });
                sh.tracker.track(h.abort_handle());
            }
        });
        Ok(FtpTestServer {
            port,
            shared,
            cert_der,
            task: Mutex::new(Some(task)),
        })
    }

    /// The control port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// `ftp://127.0.0.1:<port>` (`ftps://` for implicit TLS).
    pub fn base_url(&self) -> String {
        let scheme = if self.shared.mode == Mode::Implicit {
            "ftps"
        } else {
            "ftp"
        };
        format!("{scheme}://127.0.0.1:{}", self.port)
    }

    /// DER of the self-signed certificate (TLS modes only).
    pub fn cert_der(&self) -> Option<Vec<u8>> {
        self.cert_der.clone()
    }

    /// Number of RETR commands that started a transfer.
    pub fn retr_count(&self) -> usize {
        self.shared.retr.load(Ordering::Relaxed)
    }

    /// Stops the server and all sessions.
    pub fn shutdown(self) {
        drop(self);
    }
}

impl Drop for FtpTestServer {
    fn drop(&mut self) {
        if let Some(h) = self.task.lock().ok().and_then(|mut t| t.take()) {
            h.abort();
        }
        self.shared.tracker.abort_all();
    }
}

enum Target {
    File {
        fx: Fixture,
        drop_after: Option<u64>,
    },
    LongLine,
}

struct Session {
    sh: Arc<FtpShared>,
    ctrl: Option<Box<dyn Io>>,
    buf: Vec<u8>,
    user: Option<String>,
    logged_in: bool,
    cwd: String,
    rest: u64,
    pasv: Option<TcpListener>,
    tls_active: bool,
    prot_p: bool,
}

fn normalize(cwd: &str, arg: &str) -> Vec<String> {
    let full = if arg.starts_with('/') {
        arg.to_string()
    } else {
        format!("{cwd}/{arg}")
    };
    let mut out: Vec<String> = Vec::new();
    for seg in full.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s.to_string()),
        }
    }
    out
}

fn resolve(cwd: &str, arg: &str) -> Option<Target> {
    let p = normalize(cwd, arg);
    let parts: Vec<&str> = p.iter().map(String::as_str).collect();
    match parts.as_slice() {
        ["files", name] => Some(Target::File {
            fx: fixture(name)?,
            drop_after: None,
        }),
        ["drop", n, name] => Some(Target::File {
            fx: fixture(name)?,
            drop_after: Some(n.parse().ok()?),
        }),
        ["longline"] => Some(Target::LongLine),
        _ => None,
    }
}

fn is_dir(cwd: &str, arg: &str) -> Option<String> {
    let p = normalize(cwd, arg);
    let parts: Vec<&str> = p.iter().map(String::as_str).collect();
    let ok = match parts.as_slice() {
        [] | ["files"] | ["drop"] => true,
        ["drop", n] => n.parse::<u64>().is_ok(),
        _ => false,
    };
    ok.then(|| format!("/{}", p.join("/")))
}

impl Session {
    fn new(sh: Arc<FtpShared>, ctrl: Box<dyn Io>) -> Self {
        let implicit = sh.mode == Mode::Implicit;
        Session {
            sh,
            ctrl: Some(ctrl),
            buf: Vec::new(),
            user: None,
            logged_in: false,
            cwd: "/".to_string(),
            rest: 0,
            pasv: None,
            tls_active: implicit,
            prot_p: implicit,
        }
    }

    async fn send(&mut self, line: &str) -> io::Result<()> {
        let c = self
            .ctrl
            .as_mut()
            .ok_or_else(|| io::Error::other("no control"))?;
        c.write_all(line.as_bytes()).await?;
        c.write_all(b"\r\n").await?;
        c.flush().await
    }

    async fn read_line(&mut self) -> io::Result<Option<String>> {
        let mut chunk = [0u8; 1024];
        loop {
            if let Some(pos) = self.buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.buf.drain(..=pos).collect();
                let s = String::from_utf8_lossy(&line)
                    .trim_end_matches(['\r', '\n'])
                    .to_string();
                return Ok(Some(s));
            }
            if self.buf.len() > FTP_MAX_LINE {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
            }
            let c = self
                .ctrl
                .as_mut()
                .ok_or_else(|| io::Error::other("no control"))?;
            let n = c.read(&mut chunk).await?;
            if n == 0 {
                return Ok(None);
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    async fn run(&mut self) -> io::Result<()> {
        self.send("220 udm-testsrv FTP ready").await?;
        loop {
            let line = match self.read_line().await {
                Ok(Some(l)) => l,
                Ok(None) => return Ok(()),
                Err(e) => {
                    let _ = self.send("500 Line too long").await;
                    return Err(e);
                }
            };
            let (cmd, arg) = match line.split_once(' ') {
                Some((c, a)) => (c.to_ascii_uppercase(), a.trim().to_string()),
                None => (line.trim().to_ascii_uppercase(), String::new()),
            };
            if !self.handle(&cmd, &arg).await? {
                return Ok(());
            }
        }
    }

    /// Returns `Ok(false)` to end the session.
    async fn handle(&mut self, cmd: &str, arg: &str) -> io::Result<bool> {
        let pre_login = matches!(
            cmd,
            "USER"
                | "PASS"
                | "AUTH"
                | "PBSZ"
                | "PROT"
                | "FEAT"
                | "SYST"
                | "QUIT"
                | "NOOP"
                | "LONGLINE"
                | "FLOOD"
                | "OPTS"
        );
        if !pre_login && !self.logged_in {
            self.send("530 Please login with USER and PASS").await?;
            return Ok(true);
        }
        match cmd {
            "USER" => {
                self.user = Some(arg.to_string());
                self.logged_in = false;
                self.send("331 Password required").await?;
            }
            "PASS" => {
                let ok = match self.user.as_deref() {
                    Some("anonymous") | Some("ftp") => true,
                    Some(u) => u == AUTH_USER && arg == AUTH_PASSWORD,
                    None => false,
                };
                if ok {
                    self.logged_in = true;
                    self.send("230 Login successful").await?;
                } else {
                    self.send("530 Login incorrect").await?;
                }
            }
            "SYST" => self.send("215 UNIX Type: L8").await?,
            "FEAT" => {
                let mut lines = vec![
                    "211-Features:",
                    " SIZE",
                    " MDTM",
                    " REST STREAM",
                    " EPSV",
                    " PASV",
                ];
                if self.sh.mode != Mode::Plain {
                    lines.extend([" AUTH TLS", " PBSZ", " PROT"]);
                }
                lines.push("211 End");
                self.send(&lines.join("\r\n")).await?;
            }
            "OPTS" | "NOOP" | "MODE" | "STRU" => self.send("200 OK").await?,
            "PWD" | "XPWD" => {
                let cwd = self.cwd.clone();
                self.send(&format!("257 \"{cwd}\" is the current directory"))
                    .await?;
            }
            "CWD" | "CDUP" => {
                let a = if cmd == "CDUP" { ".." } else { arg };
                match is_dir(&self.cwd, a) {
                    Some(d) => {
                        self.cwd = d;
                        self.send("250 Directory changed").await?;
                    }
                    None => self.send("550 No such directory").await?,
                }
            }
            "TYPE" => self.send("200 Type set").await?,
            "SIZE" => match resolve(&self.cwd, arg) {
                Some(Target::File { fx, .. }) => self.send(&format!("213 {}", fx.len)).await?,
                Some(Target::LongLine) => self.long_line().await?,
                None => self.send("550 No such file").await?,
            },
            "MDTM" => match resolve(&self.cwd, arg) {
                Some(Target::File { fx, .. }) => {
                    self.send(&format!("213 {}", mdtm_time(fx.mtime_unix())))
                        .await?
                }
                Some(Target::LongLine) => self.long_line().await?,
                None => self.send("550 No such file").await?,
            },
            "REST" => match arg.parse::<u64>() {
                Ok(n) => {
                    self.rest = n;
                    self.send(&format!("350 Restarting at {n}")).await?;
                }
                Err(_) => self.send("501 Bad REST argument").await?,
            },
            "PASV" => {
                let l = bind_loopback(0).await?;
                let p = l.local_addr()?.port();
                self.pasv = Some(l);
                self.send(&format!(
                    "227 Entering Passive Mode (127,0,0,1,{},{})",
                    p >> 8,
                    p & 0xff
                ))
                .await?;
            }
            "EPSV" => {
                let l = bind_loopback(0).await?;
                let p = l.local_addr()?.port();
                self.pasv = Some(l);
                self.send(&format!("229 Entering Extended Passive Mode (|||{p}|)"))
                    .await?;
            }
            "AUTH" => {
                let acc = self.sh.acceptor.clone();
                match acc {
                    Some(acc)
                        if !self.tls_active
                            && (arg.eq_ignore_ascii_case("TLS")
                                || arg.eq_ignore_ascii_case("SSL")) =>
                    {
                        self.send("234 Proceed with negotiation").await?;
                        let plain = self
                            .ctrl
                            .take()
                            .ok_or_else(|| io::Error::other("no control"))?;
                        let t = acc.accept(plain).await?;
                        self.ctrl = Some(Box::new(t));
                        self.buf.clear();
                        self.tls_active = true;
                    }
                    _ => self.send("502 AUTH not available").await?,
                }
            }
            "PBSZ" => self.send("200 PBSZ=0").await?,
            "PROT" => match arg.to_ascii_uppercase().as_str() {
                "P" if self.tls_active => {
                    self.prot_p = true;
                    self.send("200 Protection level set to Private").await?;
                }
                "C" => {
                    self.prot_p = false;
                    self.send("200 Protection level set to Clear").await?;
                }
                _ => self.send("503 PROT not available").await?,
            },
            "RETR" => self.retr(arg).await?,
            "LONGLINE" => self.long_line().await?,
            "FLOOD" => {
                let mut i: u64 = 0;
                loop {
                    self.send(&format!("211-flood line {i}")).await?;
                    i = i.wrapping_add(1);
                }
            }
            "QUIT" => {
                self.send("221 Goodbye").await?;
                return Ok(false);
            }
            _ => self.send("502 Command not implemented").await?,
        }
        Ok(true)
    }

    async fn long_line(&mut self) -> io::Result<()> {
        let line = format!("550 {}", "x".repeat(FTP_MAX_LINE + 1024));
        self.send(&line).await
    }

    async fn retr(&mut self, arg: &str) -> io::Result<()> {
        let (fx, drop_after) = match resolve(&self.cwd, arg) {
            Some(Target::File { fx, drop_after }) => (fx, drop_after),
            Some(Target::LongLine) => return self.long_line().await,
            None => return self.send("550 No such file").await,
        };
        let rest = std::mem::take(&mut self.rest);
        if rest > fx.len {
            return self.send("554 REST beyond end of file").await;
        }
        let Some(listener) = self.pasv.take() else {
            return self.send("425 Use PASV or EPSV first").await;
        };
        self.sh.retr.fetch_add(1, Ordering::Relaxed);
        self.send(&format!(
            "150 Opening BINARY mode data connection for {} ({} bytes)",
            fx.name,
            fx.len - rest
        ))
        .await?;
        let tcp = match tokio::time::timeout(DATA_ACCEPT_TIMEOUT, listener.accept()).await {
            Ok(Ok((tcp, _))) => tcp,
            _ => return self.send("425 Can't open data connection").await,
        };
        let mut data: Box<dyn Io> = if self.prot_p {
            let Some(acc) = self.sh.acceptor.clone() else {
                return self.send("425 TLS not available").await;
            };
            match acc.accept(tcp).await {
                Ok(t) => Box::new(t),
                Err(_) => {
                    return self
                        .send("425 TLS negotiation failed on data connection")
                        .await;
                }
            }
        } else {
            Box::new(tcp)
        };
        let end = match drop_after {
            Some(n) => rest.saturating_add(n).min(fx.len),
            None => fx.len,
        };
        let mut off = rest;
        let mut buf = vec![0u8; 64 * 1024];
        while off < end {
            let want = ((end - off) as usize).min(buf.len());
            let n = fx.fill(off, &mut buf[..want]);
            if n == 0 {
                break;
            }
            if data.write_all(&buf[..n]).await.is_err() {
                return self.send("426 Connection closed; transfer aborted").await;
            }
            off += n as u64;
        }
        if drop_after.is_some() && end < fx.len {
            drop(data);
            return self.send("426 Connection closed; transfer aborted").await;
        }
        let _ = data.flush().await;
        let _ = data.shutdown().await;
        drop(data);
        self.send("226 Transfer complete").await
    }
}
