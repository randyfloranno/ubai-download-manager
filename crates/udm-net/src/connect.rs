//! Transport: TCP connect with timeouts, HTTP CONNECT and SOCKS5 tunnels, TLS.

use std::net::IpAddr;
use std::sync::Arc;

use rustls::ClientConfig;
use rustls_pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use udm_core::Secret;
use udm_core::limits;

use crate::config::{ProxyConfig, Timeouts};
use crate::error::{NetError, TimeoutPhase};
use crate::tls::map_handshake_error;

/// Any bidirectional async byte stream.
pub(crate) trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// Boxed transport stream.
pub(crate) type BoxIo = Box<dyn Io>;

/// Host (without IPv6 brackets) and port of a connection target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    pub(crate) host: String,
    pub(crate) port: u16,
}

impl Target {
    /// Target from a URL, using `default_port` when the URL has none.
    pub(crate) fn from_url(u: &url::Url, default_port: u16) -> Result<Self, NetError> {
        let host = match u.host() {
            Some(url::Host::Domain(d)) => d.to_owned(),
            Some(url::Host::Ipv4(ip)) => ip.to_string(),
            Some(url::Host::Ipv6(ip)) => ip.to_string(),
            None => return Err(NetError::InvalidRequest),
        };
        Ok(Self {
            host,
            port: u.port_or_known_default().unwrap_or(default_port),
        })
    }

    /// `host:port` in authority form (IPv6 in brackets).
    pub(crate) fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

fn timeout_err(phase: TimeoutPhase, d: std::time::Duration) -> NetError {
    NetError::Timeout {
        phase,
        ms: u64::try_from(d.as_millis()).unwrap_or(u64::MAX),
    }
}

/// Runs `fut` with a timeout, mapping expiry to a [`NetError::Timeout`].
pub(crate) async fn with_timeout<T, F>(
    phase: TimeoutPhase,
    d: std::time::Duration,
    fut: F,
) -> Result<T, NetError>
where
    F: std::future::Future<Output = Result<T, NetError>>,
{
    match tokio::time::timeout(d, fut).await {
        Ok(r) => r,
        Err(_) => Err(timeout_err(phase, d)),
    }
}

/// Opens a TCP stream to `target`, through the proxy when one is configured.
/// Returns the stream and, for direct connections, the peer IP.
pub(crate) async fn open(
    proxy: Option<&ProxyConfig>,
    timeouts: &Timeouts,
    target: &Target,
) -> Result<(BoxIo, Option<IpAddr>), NetError> {
    with_timeout(TimeoutPhase::Connect, timeouts.connect(), async {
        match proxy {
            None => {
                let s = tcp(&target.host, target.port).await?;
                let ip = s.peer_addr().ok().map(|a| a.ip());
                Ok((Box::new(s) as BoxIo, ip))
            }
            Some(ProxyConfig::Http { host, port, auth }) => {
                let mut s = tcp(host, *port).await?;
                http_connect(&mut s, target, auth.as_ref()).await?;
                Ok((Box::new(s) as BoxIo, None))
            }
            Some(ProxyConfig::Socks5 { host, port, auth }) => {
                let mut s = tcp(host, *port).await?;
                socks5_connect(&mut s, target, auth.as_ref()).await?;
                Ok((Box::new(s) as BoxIo, None))
            }
        }
    })
    .await
}

/// Opens a direct TCP connection to an explicit IP and port (FTP passive data).
pub(crate) async fn open_ip(timeouts: &Timeouts, ip: IpAddr, port: u16) -> Result<BoxIo, NetError> {
    with_timeout(TimeoutPhase::Connect, timeouts.connect(), async {
        let s = TcpStream::connect((ip, port))
            .await
            .map_err(|_| NetError::Connect {
                host: ip.to_string(),
            })?;
        let _ = s.set_nodelay(true);
        Ok(Box::new(s) as BoxIo)
    })
    .await
}

async fn tcp(host: &str, port: u16) -> Result<TcpStream, NetError> {
    let s = TcpStream::connect((host, port))
        .await
        .map_err(|_| NetError::Connect {
            host: host.to_owned(),
        })?;
    // Nagle only adds latency for request/response traffic; failure is harmless.
    let _ = s.set_nodelay(true);
    Ok(s)
}

/// Wraps `io` in TLS. Returns the stream and the negotiated ALPN protocol is `h2`.
pub(crate) async fn tls(
    cfg: &Arc<ClientConfig>,
    timeouts: &Timeouts,
    io: BoxIo,
    host: &str,
) -> Result<(BoxIo, bool), NetError> {
    let name = ServerName::try_from(host.to_owned()).map_err(|_| NetError::InvalidRequest)?;
    let connector = tokio_rustls::TlsConnector::from(cfg.clone());
    with_timeout(TimeoutPhase::Tls, timeouts.tls(), async {
        let s = connector
            .connect(name, io)
            .await
            .map_err(|e| map_handshake_error(&e, host))?;
        let h2 = s.get_ref().1.alpn_protocol() == Some(b"h2".as_slice());
        Ok((Box::new(s) as BoxIo, h2))
    })
    .await
}

/// Sends an HTTP `CONNECT` and reads the proxy's response head (bounded).
async fn http_connect<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
    target: &Target,
    auth: Option<&(String, Secret)>,
) -> Result<(), NetError> {
    let authority = target.authority();
    let mut req = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n");
    if let Some((user, pass)) = auth {
        if !udm_proto::header_value_ok(user) || !udm_proto::header_value_ok(pass.expose()) {
            return Err(NetError::InvalidRequest);
        }
        req.push_str("Proxy-Authorization: ");
        req.push_str(&udm_proto::basic_authorization(user, pass.expose()));
        req.push_str("\r\n");
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes())
        .await
        .map_err(|_| NetError::Proxy { code: 0 })?;
    s.flush().await.map_err(|_| NetError::Proxy { code: 0 })?;

    // Read byte by byte so nothing after the head (tunnel data) is consumed.
    let mut head: Vec<u8> = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    loop {
        let n = s
            .read(&mut byte)
            .await
            .map_err(|_| NetError::Proxy { code: 0 })?;
        if n == 0 {
            return Err(NetError::Proxy { code: 0 });
        }
        head.extend_from_slice(&byte);
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
        if head.len() > limits::MAX_HEADER_BYTES {
            return Err(NetError::HeaderTooLarge {
                max: u64::try_from(limits::MAX_HEADER_BYTES).unwrap_or(u64::MAX),
            });
        }
    }
    let text = String::from_utf8_lossy(&head);
    let status_line = text.lines().next().unwrap_or("");
    let mut parts = status_line.split_whitespace();
    let version = parts.next().unwrap_or("");
    let code: u16 = parts.next().and_then(|c| c.parse().ok()).unwrap_or(0);
    if !version.starts_with("HTTP/1.") {
        return Err(NetError::Proxy { code: 0 });
    }
    match code {
        200..=299 => Ok(()),
        407 => Err(NetError::ProxyAuth),
        c => Err(NetError::Proxy { code: c }),
    }
}

/// SOCKS5 handshake (RFC 1928) with optional username/password (RFC 1929).
async fn socks5_connect<S: AsyncRead + AsyncWrite + Unpin>(
    s: &mut S,
    target: &Target,
    auth: Option<&(String, Secret)>,
) -> Result<(), NetError> {
    let perr = |_| NetError::Proxy { code: 0 };
    let greeting: &[u8] = if auth.is_some() {
        &[5, 2, 0x00, 0x02]
    } else {
        &[5, 1, 0x00]
    };
    s.write_all(greeting).await.map_err(perr)?;
    let mut reply = [0u8; 2];
    s.read_exact(&mut reply).await.map_err(perr)?;
    let [ver, method] = reply;
    if ver != 5 {
        return Err(NetError::Proxy { code: 0 });
    }
    match (method, auth) {
        (0x00, _) => {}
        (0x02, Some((user, pass))) => {
            let u = user.as_bytes();
            let p = pass.expose().as_bytes();
            let ul = u8::try_from(u.len()).map_err(|_| NetError::InvalidRequest)?;
            let pl = u8::try_from(p.len()).map_err(|_| NetError::InvalidRequest)?;
            if ul == 0 {
                return Err(NetError::InvalidRequest);
            }
            let mut msg = Vec::with_capacity(u.len().saturating_add(p.len()).saturating_add(3));
            msg.push(1);
            msg.push(ul);
            msg.extend_from_slice(u);
            msg.push(pl);
            msg.extend_from_slice(p);
            s.write_all(&msg).await.map_err(perr)?;
            let mut r = [0u8; 2];
            s.read_exact(&mut r).await.map_err(perr)?;
            if r != [1, 0] {
                return Err(NetError::ProxyAuth);
            }
        }
        (0xff, _) | (0x02, None) => return Err(NetError::ProxyAuth),
        _ => return Err(NetError::Proxy { code: 0 }),
    }

    let mut req: Vec<u8> = vec![5, 1, 0];
    match target.host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            req.push(1);
            req.extend_from_slice(&ip.octets());
        }
        Ok(IpAddr::V6(ip)) => {
            req.push(4);
            req.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            let h = target.host.as_bytes();
            let hl = u8::try_from(h.len()).map_err(|_| NetError::InvalidRequest)?;
            req.push(3);
            req.push(hl);
            req.extend_from_slice(h);
        }
    }
    req.extend_from_slice(&target.port.to_be_bytes());
    s.write_all(&req).await.map_err(perr)?;

    let mut head = [0u8; 4];
    s.read_exact(&mut head).await.map_err(perr)?;
    let [ver, rep, _rsv, atyp] = head;
    if ver != 5 {
        return Err(NetError::Proxy { code: 0 });
    }
    if rep != 0 {
        return Err(NetError::Proxy {
            code: u16::from(rep),
        });
    }
    // Bound address length by the address type; never trust a length beyond 255.
    let addr_len: usize = match atyp {
        1 => 4,
        4 => 16,
        3 => {
            let mut l = [0u8; 1];
            s.read_exact(&mut l).await.map_err(perr)?;
            let [len] = l;
            usize::from(len)
        }
        _ => return Err(NetError::Proxy { code: 0 }),
    };
    let mut rest = vec![0u8; addr_len.saturating_add(2)];
    s.read_exact(&mut rest).await.map_err(perr)?;
    s.flush().await.map_err(perr)?;
    Ok(())
}
