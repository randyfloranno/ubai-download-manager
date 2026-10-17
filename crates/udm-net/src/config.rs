//! Client configuration and request/response description types.

use std::time::Duration;

use udm_core::Secret;
use udm_core::limits;
use udm_proto::EntityTag;

/// Timeouts for every network phase, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    /// TCP connect (and proxy handshake).
    pub connect_ms: u64,
    /// TLS handshake.
    pub tls_ms: u64,
    /// From sending a request (or FTP command) to the response head (or reply).
    pub first_byte_ms: u64,
    /// Maximum silence while reading a body.
    pub idle_ms: u64,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect_ms: limits::CONNECT_TIMEOUT_MS,
            tls_ms: limits::TLS_HANDSHAKE_TIMEOUT_MS,
            first_byte_ms: limits::FIRST_BYTE_TIMEOUT_MS,
            idle_ms: limits::IDLE_READ_TIMEOUT_MS,
        }
    }
}

impl Timeouts {
    pub(crate) fn connect(&self) -> Duration {
        Duration::from_millis(self.connect_ms)
    }
    pub(crate) fn tls(&self) -> Duration {
        Duration::from_millis(self.tls_ms)
    }
    pub(crate) fn first_byte(&self) -> Duration {
        Duration::from_millis(self.first_byte_ms)
    }
    pub(crate) fn idle(&self) -> Duration {
        Duration::from_millis(self.idle_ms)
    }
}

/// Outgoing proxy. All traffic (HTTP, HTTPS, FTP control and data) is tunnelled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProxyConfig {
    /// HTTP proxy, used through `CONNECT` tunnels for every target.
    Http {
        /// Proxy host name or IP.
        host: String,
        /// Proxy port.
        port: u16,
        /// Optional `(user, password)` for `Proxy-Authorization: Basic`.
        auth: Option<(String, Secret)>,
    },
    /// SOCKS5 proxy (RFC 1928), optional username/password (RFC 1929).
    Socks5 {
        /// Proxy host name or IP.
        host: String,
        /// Proxy port.
        port: u16,
        /// Optional `(user, password)`.
        auth: Option<(String, Secret)>,
    },
}

/// A per-host TLS exception granted by an explicit user action: the server
/// certificate whose leaf DER has this SHA-256 fingerprint is accepted for `host`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TlsException {
    /// Host name or IP literal (case-insensitive; IPv6 with or without brackets).
    pub host: String,
    /// Lower-case hex SHA-256 of the leaf certificate DER (separators are ignored).
    pub sha256_fingerprint: String,
}

/// Client configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetConfig {
    /// `User-Agent` header value.
    pub user_agent: String,
    /// Optional proxy.
    pub proxy: Option<ProxyConfig>,
    /// Pinned per-host certificate exceptions.
    pub tls_exceptions: Vec<TlsException>,
    /// Extra trusted root certificates (DER), in addition to the platform store.
    pub extra_root_der: Vec<Vec<u8>>,
    /// Timeouts.
    pub timeouts: Timeouts,
    /// Allow HTTPS→HTTP / FTPS→FTP redirects for every request.
    pub allow_downgrade: bool,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            user_agent: concat!("UDM/", env!("CARGO_PKG_VERSION")).to_owned(),
            proxy: None,
            tls_exceptions: Vec::new(),
            extra_root_der: Vec::new(),
            timeouts: Timeouts::default(),
            allow_downgrade: false,
        }
    }
}

/// One request description. Credentials embedded in the URL are stripped and
/// used as `credentials` when none are given explicitly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// Target URL (http, https, ftp, ftps).
    pub url: url::Url,
    /// Optional `Referer`.
    pub referer: Option<String>,
    /// Optional `Cookie` header value (from the browser extension).
    pub cookie: Option<Secret>,
    /// Optional `(user, password)` for HTTP Basic/Digest or FTP login.
    pub credentials: Option<(String, Secret)>,
    /// Allow a secure→insecure redirect for this request.
    pub allow_downgrade: bool,
}

impl Request {
    /// A plain GET request for `url`.
    pub fn new(url: url::Url) -> Self {
        Self {
            url,
            referer: None,
            cookie: None,
            credentials: None,
            allow_downgrade: false,
        }
    }
}

/// Wire protocol used for a response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// HTTP/1.1 (plain or TLS).
    Http1,
    /// HTTP/2 over TLS (ALPN `h2`).
    Http2,
    /// Plain FTP.
    Ftp,
    /// Explicit FTPS (`AUTH TLS`).
    Ftps,
}

/// Metadata learned from a probe request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeInfo {
    /// URL after redirects (credentials stripped).
    pub final_url: url::Url,
    /// Total size, if known.
    pub size: Option<u64>,
    /// True when the server answered a range request correctly.
    pub accepts_ranges: bool,
    /// Entity tag, if any (may be weak).
    pub etag: Option<EntityTag>,
    /// Raw `Last-Modified` (HTTP date; FTP `MDTM` reformatted as an HTTP date).
    pub last_modified: Option<String>,
    /// Raw, unsanitised file name from `Content-Disposition`.
    pub filename: Option<String>,
    /// `Content-Type`, if any.
    pub content_type: Option<String>,
    /// Protocol of the final response.
    pub protocol: Protocol,
}
