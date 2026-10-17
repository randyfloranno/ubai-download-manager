//! TLS client configuration and certificate verification.
//!
//! Verification is never disabled. A certificate is accepted when
//! 1. the platform verifier (OS trust store) accepts it, or
//! 2. a verifier built from `NetConfig::extra_root_der` accepts it, or
//! 3. the user pinned exactly this leaf certificate (SHA-256 of its DER) for this host.
//!
//! Otherwise the handshake fails with an error that carries the host and the
//! fingerprint so the UI can show them and offer an explicit exception.

use std::fmt;
use std::sync::Arc;

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error as TlsError, OtherError,
    RootCertStore, SignatureScheme,
};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};

use crate::config::TlsException;
use crate::error::NetError;

/// Lower-case hex SHA-256 of a DER certificate.
pub fn cert_fingerprint(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push(hex_digit(b >> 4));
        out.push(hex_digit(b & 0x0f));
    }
    out
}

fn hex_digit(n: u8) -> char {
    char::from_digit(u32::from(n), 16).unwrap_or('0')
}

/// Normalises a fingerprint for comparison: lower-case hex digits only.
fn normalize_fingerprint(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_hexdigit)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Normalises a host for comparison: lower-case, no IPv6 brackets, no trailing dot.
pub(crate) fn normalize_host(s: &str) -> String {
    s.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn server_name_host(name: &ServerName<'_>) -> String {
    match name {
        ServerName::DnsName(d) => normalize_host(d.as_ref()),
        ServerName::IpAddress(ip) => std::net::IpAddr::from(*ip).to_string(),
        _ => String::new(),
    }
}

/// Carried inside a rustls error so the caller can recover host and fingerprint.
#[derive(Debug)]
pub(crate) struct Untrusted {
    pub(crate) host: String,
    pub(crate) fingerprint: String,
}

impl fmt::Display for Untrusted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "untrusted certificate for {}", self.host)
    }
}

impl std::error::Error for Untrusted {}

struct UdmVerifier {
    platform: Option<Arc<dyn ServerCertVerifier>>,
    extra: Option<Arc<WebPkiServerVerifier>>,
    pins: Vec<(String, String)>,
    provider: Arc<CryptoProvider>,
}

impl fmt::Debug for UdmVerifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UdmVerifier")
            .field("platform", &self.platform.is_some())
            .field("extra", &self.extra.is_some())
            .field("pins", &self.pins.len())
            .finish()
    }
}

impl ServerCertVerifier for UdmVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        if let Some(p) = &self.platform
            && let Ok(v) =
                p.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
        {
            return Ok(v);
        }
        if let Some(x) = &self.extra
            && let Ok(v) =
                x.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
        {
            return Ok(v);
        }
        let host = server_name_host(server_name);
        let fingerprint = cert_fingerprint(end_entity.as_ref());
        if self
            .pins
            .iter()
            .any(|(h, fp)| *h == host && *fp == fingerprint)
        {
            tracing::warn!(%host, %fingerprint, "accepting certificate through pinned user exception");
            return Ok(ServerCertVerified::assertion());
        }
        Err(TlsError::InvalidCertificate(CertificateError::Other(
            OtherError(Arc::new(Untrusted { host, fingerprint })),
        )))
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The two TLS client configurations a [`crate::Client`] uses.
pub(crate) struct TlsConfigs {
    /// ALPN `h2`, `http/1.1` (HTTPS).
    pub(crate) http: Arc<ClientConfig>,
    /// No ALPN (FTPS control and data channels; shares nothing but the verifier).
    pub(crate) ftp: Arc<ClientConfig>,
}

/// Builds the verifier and the client configurations.
pub(crate) fn build(
    extra_root_der: &[Vec<u8>],
    exceptions: &[TlsException],
) -> Result<TlsConfigs, NetError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let platform: Option<Arc<dyn ServerCertVerifier>> =
        match rustls_platform_verifier::Verifier::new(provider.clone()) {
            Ok(v) => Some(Arc::new(v)),
            Err(e) => {
                tracing::warn!(error = %e, "platform certificate verifier unavailable");
                None
            }
        };
    let extra = if extra_root_der.is_empty() {
        None
    } else {
        let mut roots = RootCertStore::empty();
        for der in extra_root_der {
            roots
                .add(CertificateDer::from(der.clone()))
                .map_err(|_| NetError::Tls {
                    host: String::new(),
                })?;
        }
        let v = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
            .build()
            .map_err(|_| NetError::Tls {
                host: String::new(),
            })?;
        Some(v)
    };
    let pins = exceptions
        .iter()
        .map(|e| {
            (
                normalize_host(&e.host),
                normalize_fingerprint(&e.sha256_fingerprint),
            )
        })
        .filter(|(h, fp)| !h.is_empty() && fp.len() == 64)
        .collect();
    let verifier = Arc::new(UdmVerifier {
        platform,
        extra,
        pins,
        provider: provider.clone(),
    });
    let make = |alpn: Vec<Vec<u8>>| -> Result<Arc<ClientConfig>, NetError> {
        let mut cfg = ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|_| NetError::Tls {
                host: String::new(),
            })?
            .dangerous()
            .with_custom_certificate_verifier(verifier.clone())
            .with_no_client_auth();
        cfg.alpn_protocols = alpn;
        Ok(Arc::new(cfg))
    };
    Ok(TlsConfigs {
        http: make(vec![b"h2".to_vec(), b"http/1.1".to_vec()])?,
        ftp: make(Vec::new())?,
    })
}

/// Maps a TLS handshake I/O error to a [`NetError`].
pub(crate) fn map_handshake_error(e: &std::io::Error, host: &str) -> NetError {
    let tls = e
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<TlsError>());
    if let Some(TlsError::InvalidCertificate(CertificateError::Other(OtherError(inner)))) = tls
        && let Some(u) = inner.downcast_ref::<Untrusted>()
    {
        return NetError::TlsUntrusted {
            host: u.host.clone(),
            fingerprint: u.fingerprint.clone(),
        };
    }
    if tls.is_some() {
        NetError::Tls {
            host: host.to_owned(),
        }
    } else {
        NetError::Io
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_lower_hex() {
        let fp = cert_fingerprint(b"abc");
        assert_eq!(
            fp,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn normalizers() {
        assert_eq!(normalize_host("[::1]"), "::1");
        assert_eq!(normalize_host("Example.COM."), "example.com");
        assert_eq!(normalize_fingerprint("AB:cd ef"), "abcdef");
    }
}
