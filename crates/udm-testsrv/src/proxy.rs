//! SOCKS5 (RFC 1928 / RFC 1929) and HTTP CONNECT test proxies.
//! Both only connect to loopback targets.

use crate::server::{Tracker, bind_loopback};
use crate::util::{MAX_REQUEST_HEAD_BYTES, find, read_head, resolve_loopback, split_host_port};
use base64::Engine as _;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

struct ProxyShared {
    auth: Option<(String, String)>,
    tunnels: AtomicUsize,
    forwards: AtomicUsize,
    tracker: Tracker,
}

struct ProxyCore {
    port: u16,
    shared: Arc<ProxyShared>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl ProxyCore {
    async fn start<F, Fut>(auth: Option<(String, String)>, handler: F) -> io::Result<Self>
    where
        F: Fn(Arc<ProxyShared>, TcpStream) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = io::Result<()>> + Send + 'static,
    {
        let listener: TcpListener = bind_loopback(0).await?;
        let port = listener.local_addr()?.port();
        let shared = Arc::new(ProxyShared {
            auth,
            tunnels: AtomicUsize::new(0),
            forwards: AtomicUsize::new(0),
            tracker: Tracker::default(),
        });
        let sh = shared.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    continue;
                };
                let h = tokio::spawn(handler(sh.clone(), tcp));
                sh.tracker.track(h.abort_handle());
            }
        });
        Ok(ProxyCore {
            port,
            shared,
            task: Mutex::new(Some(task)),
        })
    }
}

impl Drop for ProxyCore {
    fn drop(&mut self) {
        if let Some(h) = self.task.lock().ok().and_then(|mut t| t.take()) {
            h.abort();
        }
        self.shared.tracker.abort_all();
    }
}

/// SOCKS5 proxy (CONNECT only; IPv4 and domain targets; loopback only).
pub struct Socks5Proxy {
    core: ProxyCore,
}

impl Socks5Proxy {
    /// Starts on 127.0.0.1 with a free port. With `auth`, RFC 1929
    /// user/password is required; otherwise "no authentication".
    pub async fn start(auth: Option<(String, String)>) -> io::Result<Self> {
        Ok(Socks5Proxy {
            core: ProxyCore::start(auth, socks5_conn).await?,
        })
    }

    pub fn port(&self) -> u16 {
        self.core.port
    }

    /// Number of established tunnels.
    pub fn tunnel_count(&self) -> usize {
        self.core.shared.tunnels.load(Ordering::Relaxed)
    }

    pub fn shutdown(self) {
        drop(self);
    }
}

async fn socks5_conn(sh: Arc<ProxyShared>, mut c: TcpStream) -> io::Result<()> {
    let mut hdr = [0u8; 2];
    c.read_exact(&mut hdr).await?;
    if hdr[0] != 5 {
        return Ok(());
    }
    let mut methods = vec![0u8; usize::from(hdr[1])];
    c.read_exact(&mut methods).await?;
    let want = if sh.auth.is_some() { 2u8 } else { 0u8 };
    if !methods.contains(&want) {
        c.write_all(&[5, 0xFF]).await?;
        return Ok(());
    }
    c.write_all(&[5, want]).await?;
    if let Some((user, pass)) = &sh.auth {
        let mut v = [0u8; 2];
        c.read_exact(&mut v).await?;
        let mut u = vec![0u8; usize::from(v[1])];
        c.read_exact(&mut u).await?;
        let mut pl = [0u8; 1];
        c.read_exact(&mut pl).await?;
        let mut p = vec![0u8; usize::from(pl[0])];
        c.read_exact(&mut p).await?;
        if v[0] != 1 || u != user.as_bytes() || p != pass.as_bytes() {
            c.write_all(&[1, 1]).await?;
            return Ok(());
        }
        c.write_all(&[1, 0]).await?;
    }
    let mut req = [0u8; 4];
    c.read_exact(&mut req).await?;
    let reply = |code: u8| [5, code, 0, 1, 0, 0, 0, 0, 0, 0];
    if req[0] != 5 {
        return Ok(());
    }
    let host = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            c.read_exact(&mut a).await?;
            std::net::Ipv4Addr::from(a).to_string()
        }
        3 => {
            let mut l = [0u8; 1];
            c.read_exact(&mut l).await?;
            let mut d = vec![0u8; usize::from(l[0])];
            c.read_exact(&mut d).await?;
            String::from_utf8_lossy(&d).into_owned()
        }
        _ => {
            c.write_all(&reply(8)).await?;
            return Ok(());
        }
    };
    let mut pb = [0u8; 2];
    c.read_exact(&mut pb).await?;
    let port = u16::from_be_bytes(pb);
    if req[1] != 1 {
        c.write_all(&reply(7)).await?;
        return Ok(());
    }
    let Some(addr) = resolve_loopback(&host, port) else {
        c.write_all(&reply(2)).await?;
        return Ok(());
    };
    let mut up = match TcpStream::connect(addr).await {
        Ok(u) => u,
        Err(_) => {
            c.write_all(&reply(5)).await?;
            return Ok(());
        }
    };
    let local = up.local_addr()?;
    let mut ok = vec![5, 0, 0, 1];
    match local.ip() {
        std::net::IpAddr::V4(ip) => ok.extend_from_slice(&ip.octets()),
        std::net::IpAddr::V6(_) => ok.extend_from_slice(&[127, 0, 0, 1]),
    }
    ok.extend_from_slice(&local.port().to_be_bytes());
    c.write_all(&ok).await?;
    sh.tunnels.fetch_add(1, Ordering::Relaxed);
    let _ = tokio::io::copy_bidirectional(&mut c, &mut up).await;
    Ok(())
}

/// HTTP proxy: `CONNECT host:port` tunnels and absolute-form `GET http://…`
/// forwarding, optional `Proxy-Authorization: Basic` (loopback targets only).
pub struct HttpConnectProxy {
    core: ProxyCore,
}

impl HttpConnectProxy {
    /// Starts on 127.0.0.1 with a free port. With `auth`, Basic proxy auth is
    /// required (407 otherwise).
    pub async fn start(auth: Option<(String, String)>) -> io::Result<Self> {
        Ok(HttpConnectProxy {
            core: ProxyCore::start(auth, http_proxy_conn).await?,
        })
    }

    pub fn port(&self) -> u16 {
        self.core.port
    }

    /// Number of established CONNECT tunnels.
    pub fn tunnel_count(&self) -> usize {
        self.core.shared.tunnels.load(Ordering::Relaxed)
    }

    /// Number of forwarded absolute-form requests.
    pub fn forward_count(&self) -> usize {
        self.core.shared.forwards.load(Ordering::Relaxed)
    }

    pub fn shutdown(self) {
        drop(self);
    }
}

async fn simple_reply(c: &mut TcpStream, status: &str, extra: &str) -> io::Result<()> {
    let r = format!("HTTP/1.1 {status}\r\n{extra}Content-Length: 0\r\nConnection: close\r\n\r\n");
    c.write_all(r.as_bytes()).await?;
    c.shutdown().await
}

async fn http_proxy_conn(sh: Arc<ProxyShared>, mut c: TcpStream) -> io::Result<()> {
    let Some((head, left)) = read_head(&mut c, MAX_REQUEST_HEAD_BYTES).await? else {
        return Ok(());
    };
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split(' ');
    let (method, target, version) = (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
        parts.next().unwrap_or("HTTP/1.1"),
    );
    let headers: Vec<(String, String)> = lines
        .filter(|l| !l.is_empty())
        .filter_map(|l| {
            l.split_once(':')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        })
        .collect();
    if let Some((user, pass)) = &sh.auth {
        let expected = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
        let ok = headers.iter().any(|(k, v)| {
            k.eq_ignore_ascii_case("proxy-authorization")
                && v.split_once(' ')
                    .is_some_and(|(s, b)| s.eq_ignore_ascii_case("basic") && b.trim() == expected)
        });
        if !ok {
            return simple_reply(
                &mut c,
                "407 Proxy Authentication Required",
                "Proxy-Authenticate: Basic realm=\"udm-proxy\"\r\n",
            )
            .await;
        }
    }
    if method.eq_ignore_ascii_case("CONNECT") {
        let Some(addr) = split_host_port(target).and_then(|(h, p)| resolve_loopback(&h, p)) else {
            return simple_reply(&mut c, "403 Forbidden", "").await;
        };
        let Ok(mut up) = TcpStream::connect(addr).await else {
            return simple_reply(&mut c, "502 Bad Gateway", "").await;
        };
        c.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
        if !left.is_empty() {
            up.write_all(&left).await?;
        }
        sh.tunnels.fetch_add(1, Ordering::Relaxed);
        let _ = tokio::io::copy_bidirectional(&mut c, &mut up).await;
        return Ok(());
    }
    // Absolute-form forwarding: GET http://host:port/path HTTP/1.1
    let Some(rest) = target.strip_prefix("http://") else {
        return simple_reply(&mut c, "400 Bad Request", "").await;
    };
    let (authority, path) = match find(rest.as_bytes(), b"/") {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = split_host_port(authority).unwrap_or((authority.to_string(), 80));
    let Some(addr) = resolve_loopback(&host, port) else {
        return simple_reply(&mut c, "403 Forbidden", "").await;
    };
    let Ok(mut up) = TcpStream::connect(addr).await else {
        return simple_reply(&mut c, "502 Bad Gateway", "").await;
    };
    let mut out = format!("{method} {path} {version}\r\n");
    for (k, v) in &headers {
        if k.eq_ignore_ascii_case("proxy-authorization")
            || k.eq_ignore_ascii_case("proxy-connection")
        {
            continue;
        }
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str("\r\n");
    up.write_all(out.as_bytes()).await?;
    if !left.is_empty() {
        up.write_all(&left).await?;
    }
    sh.forwards.fetch_add(1, Ordering::Relaxed);
    let _ = tokio::io::copy_bidirectional(&mut c, &mut up).await;
    Ok(())
}
