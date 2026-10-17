//! Self-signed certificate (rcgen) and rustls server configs (ring provider).

use rustls::ServerConfig;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::io;
use std::sync::{Arc, OnceLock};

struct CertPair {
    cert: Vec<u8>,
    key: Vec<u8>,
}

fn cert_pair() -> io::Result<&'static CertPair> {
    static PAIR: OnceLock<Result<CertPair, String>> = OnceLock::new();
    let r = PAIR.get_or_init(|| {
        let ck = rcgen::generate_simple_self_signed(vec![
            "localhost".to_string(),
            "127.0.0.1".to_string(),
        ])
        .map_err(|e| e.to_string())?;
        Ok(CertPair {
            cert: ck.cert.der().to_vec(),
            key: ck.signing_key.serialize_der(),
        })
    });
    r.as_ref().map_err(|e| io::Error::other(e.clone()))
}

/// DER of the process-wide self-signed test certificate (SANs `localhost`,
/// `127.0.0.1`). Clients add it to their root store in tests.
pub fn test_cert_der() -> io::Result<Vec<u8>> {
    Ok(cert_pair()?.cert.clone())
}

/// A rustls server config using the ring provider and the test certificate.
pub(crate) fn server_config(alpn: &[&[u8]]) -> io::Result<Arc<ServerConfig>> {
    let pair = cert_pair()?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(pair.cert.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(pair.key.clone())),
        )
        .map_err(io::Error::other)?;
    cfg.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
    Ok(Arc::new(cfg))
}
