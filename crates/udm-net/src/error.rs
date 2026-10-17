//! Typed network errors. Every variant maps to an i18n key `error-net-*`.

use core::fmt;

use udm_core::Message;
use udm_proto::ProtoError;

/// Phase of a request in which a timeout fired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeoutPhase {
    /// TCP connect (including proxy handshake).
    Connect,
    /// TLS handshake.
    Tls,
    /// Waiting for the response head / FTP reply.
    FirstByte,
    /// Silence while reading a body.
    Idle,
}

/// Errors produced by the network layer.
///
/// Variants never carry unbounded hostile input: strings taken from the network
/// are truncated and filtered to printable ASCII before they are stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetError {
    /// TCP connection to a host failed.
    Connect {
        /// Host we tried to reach.
        host: String,
    },
    /// A timeout fired.
    Timeout {
        /// Which phase timed out.
        phase: TimeoutPhase,
        /// The timeout in milliseconds.
        ms: u64,
    },
    /// TLS handshake failed for a reason other than an untrusted certificate.
    Tls {
        /// Host of the TLS peer.
        host: String,
    },
    /// The server certificate is not trusted and no matching pinned exception exists.
    TlsUntrusted {
        /// Host of the TLS peer.
        host: String,
        /// Lower-case hex SHA-256 of the leaf certificate DER.
        fingerprint: String,
    },
    /// Unexpected HTTP status.
    HttpStatus {
        /// The status code.
        status: u16,
    },
    /// More than `limits::MAX_REDIRECTS` redirects.
    TooManyRedirects {
        /// The limit.
        max: u32,
    },
    /// A redirect pointed back to a URL already visited.
    RedirectLoop,
    /// A redirect from a secure to an insecure scheme was refused.
    DowngradeRefused,
    /// A redirect `Location` was missing or invalid.
    RedirectInvalid,
    /// The response head is larger than `limits::MAX_HEADER_BYTES`.
    HeaderTooLarge {
        /// The limit.
        max: u64,
    },
    /// The response has more than `limits::MAX_HEADER_COUNT` headers.
    TooManyHeaders {
        /// The limit.
        max: u64,
    },
    /// File data arrived with a non-identity `Content-Encoding`.
    ContentEncoding {
        /// The (bounded, sanitised) encoding name.
        encoding: String,
    },
    /// The body ended before the expected number of bytes.
    ShortBody {
        /// Bytes expected.
        expected: u64,
        /// Bytes received.
        received: u64,
    },
    /// The server sent more bytes than requested.
    ExtraBytes {
        /// Bytes expected.
        expected: u64,
    },
    /// `Content-Range` missing or not matching the requested range and size.
    BadContentRange {
        /// Underlying protocol error.
        detail: ProtoError,
    },
    /// The remote file changed (strong ETag or size differs from the validator).
    RemoteChanged,
    /// The server requires authentication and no credentials were supplied.
    AuthRequired,
    /// The supplied credentials were rejected.
    AuthFailed,
    /// The proxy refused or failed the tunnel.
    Proxy {
        /// HTTP status or SOCKS reply code; 0 for a protocol error.
        code: u16,
    },
    /// The proxy requires (other) credentials.
    ProxyAuth,
    /// FTP server returned an unexpected reply.
    Ftp {
        /// Reply code (0 when the reply was malformed).
        code: u16,
        /// First reply line, bounded and sanitised.
        text: String,
    },
    /// URL scheme is not supported for this operation.
    UnsupportedScheme {
        /// The scheme.
        scheme: String,
    },
    /// The URL or a request field is invalid (bad header value, CR/LF in an FTP path, ...).
    InvalidRequest,
    /// I/O error on an established connection.
    Io,
    /// HTTP/2 stream reset or other transport-level stream failure.
    Stream,
    /// The peer violated the protocol (malformed response, bad SOCKS reply, ...).
    Protocol,
    /// A page requested for the site grabber is not HTML.
    NotHtml {
        /// The (bounded, sanitised) content type.
        content_type: String,
    },
}

impl NetError {
    /// The i18n key for this error.
    pub fn key(&self) -> &'static str {
        match self {
            Self::Connect { .. } => "error-net-connect",
            Self::Timeout { .. } => "error-net-timeout",
            Self::Tls { .. } => "error-net-tls",
            Self::TlsUntrusted { .. } => "error-net-tls-untrusted",
            Self::HttpStatus { .. } => "error-net-http-status",
            Self::TooManyRedirects { .. } => "error-net-too-many-redirects",
            Self::RedirectLoop => "error-net-redirect-loop",
            Self::DowngradeRefused => "error-net-downgrade-refused",
            Self::RedirectInvalid => "error-net-redirect-invalid",
            Self::HeaderTooLarge { .. } => "error-net-header-too-large",
            Self::TooManyHeaders { .. } => "error-net-too-many-headers",
            Self::ContentEncoding { .. } => "error-net-content-encoding",
            Self::ShortBody { .. } => "error-net-short-body",
            Self::ExtraBytes { .. } => "error-net-extra-bytes",
            Self::BadContentRange { .. } => "error-net-bad-content-range",
            Self::RemoteChanged => "error-net-remote-changed",
            Self::AuthRequired => "error-net-auth-required",
            Self::AuthFailed => "error-net-auth-failed",
            Self::Proxy { .. } => "error-net-proxy",
            Self::ProxyAuth => "error-net-proxy-auth",
            Self::Ftp { .. } => "error-net-ftp",
            Self::UnsupportedScheme { .. } => "error-net-unsupported-scheme",
            Self::InvalidRequest => "error-net-invalid-request",
            Self::Io => "error-net-io",
            Self::Stream => "error-net-stream",
            Self::Protocol => "error-net-protocol",
            Self::NotHtml { .. } => "error-net-not-html",
        }
    }

    /// Localisable message (key + arguments).
    pub fn to_message(&self) -> Message {
        let m = Message::new(self.key());
        match self {
            Self::Connect { host } | Self::Tls { host } => m.with_str("host", host.clone()),
            Self::Timeout { ms, .. } => m.with_uint("seconds", ms.div_ceil(1000)),
            Self::TlsUntrusted { host, fingerprint } => m
                .with_str("host", host.clone())
                .with_str("fingerprint", fingerprint.clone()),
            Self::HttpStatus { status } => m.with_uint("status", u64::from(*status)),
            Self::TooManyRedirects { max } => m.with_uint("max", u64::from(*max)),
            Self::HeaderTooLarge { max } => m.with_bytes("max", *max),
            Self::TooManyHeaders { max } => m.with_uint("max", *max),
            Self::ContentEncoding { encoding } => m.with_str("encoding", encoding.clone()),
            Self::ShortBody { expected, received } => m
                .with_bytes("expected", *expected)
                .with_bytes("received", *received),
            Self::ExtraBytes { expected } => m.with_bytes("expected", *expected),
            Self::Proxy { code } => m.with_uint("code", u64::from(*code)),
            Self::Ftp { code, text } => m
                .with_uint("code", u64::from(*code))
                .with_str("text", text.clone()),
            Self::UnsupportedScheme { scheme } => m.with_str("scheme", scheme.clone()),
            Self::NotHtml { content_type } => m.with_str("type", content_type.clone()),
            Self::BadContentRange { .. }
            | Self::RedirectLoop
            | Self::DowngradeRefused
            | Self::RedirectInvalid
            | Self::RemoteChanged
            | Self::AuthRequired
            | Self::AuthFailed
            | Self::ProxyAuth
            | Self::InvalidRequest
            | Self::Io
            | Self::Stream
            | Self::Protocol => m,
        }
    }

    /// True when retrying the same request later may succeed.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Connect { .. }
            | Self::Timeout { .. }
            | Self::Io
            | Self::ShortBody { .. }
            | Self::ExtraBytes { .. }
            | Self::BadContentRange { .. }
            | Self::Stream
            | Self::Protocol
            | Self::Proxy { .. } => true,
            Self::HttpStatus { status } => *status >= 500 || *status == 408 || *status == 429,
            // FTP 4xx replies are transient by definition (RFC 959 §4.2).
            Self::Ftp { code, .. } => (400..500).contains(code) || *code == 0,
            Self::Tls { .. }
            | Self::TlsUntrusted { .. }
            | Self::TooManyRedirects { .. }
            | Self::RedirectLoop
            | Self::DowngradeRefused
            | Self::RedirectInvalid
            | Self::HeaderTooLarge { .. }
            | Self::TooManyHeaders { .. }
            | Self::ContentEncoding { .. }
            | Self::RemoteChanged
            | Self::AuthRequired
            | Self::AuthFailed
            | Self::ProxyAuth
            | Self::UnsupportedScheme { .. }
            | Self::InvalidRequest
            | Self::NotHtml { .. } => false,
        }
    }
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.to_message(), f)
    }
}

impl std::error::Error for NetError {}

/// Bounds and sanitises a string that came from the network so it can be stored
/// in an error and shown to the user: printable ASCII only, at most `max` chars.
pub(crate) fn bounded_text(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(max)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_classification() {
        assert!(NetError::HttpStatus { status: 503 }.is_retryable());
        assert!(NetError::HttpStatus { status: 429 }.is_retryable());
        assert!(NetError::HttpStatus { status: 408 }.is_retryable());
        assert!(!NetError::HttpStatus { status: 404 }.is_retryable());
        assert!(!NetError::DowngradeRefused.is_retryable());
        assert!(!NetError::RedirectLoop.is_retryable());
        assert!(NetError::Stream.is_retryable());
        assert!(
            NetError::Ftp {
                code: 421,
                text: String::new()
            }
            .is_retryable()
        );
        assert!(
            !NetError::Ftp {
                code: 550,
                text: String::new()
            }
            .is_retryable()
        );
    }

    #[test]
    fn messages_carry_args() {
        let m = NetError::TlsUntrusted {
            host: "h".into(),
            fingerprint: "ab".into(),
        }
        .to_message();
        assert_eq!(m.key, "error-net-tls-untrusted");
        assert!(m.arg("fingerprint").is_some());
        let m = NetError::Timeout {
            phase: TimeoutPhase::Idle,
            ms: 1500,
        }
        .to_message();
        assert_eq!(m.arg("seconds"), Some(&udm_core::MsgArg::UInt(2)));
    }

    #[test]
    fn hostile_bounded_text_strips_control_and_truncates() {
        let s = format!("a\r\nb\u{202e}{}", "x".repeat(500));
        let t = bounded_text(&s, 10);
        assert_eq!(t, "abxxxxxxxx");
    }
}
