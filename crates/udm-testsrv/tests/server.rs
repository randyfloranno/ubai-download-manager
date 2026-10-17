//! Integration tests for udm-testsrv (all local, small fixtures).

use base64::Engine as _;
use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use rustls_pki_types::{CertificateDer, ServerName};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use udm_testsrv::*;

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Resp {
    fn h(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

async fn raw_exchange<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S, req: &[u8]) -> Resp {
    s.write_all(req).await.unwrap();
    let mut buf = Vec::new();
    let _ = s.read_to_end(&mut buf).await; // drops may surface as errors
    let pos = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("no head");
    let head = String::from_utf8_lossy(&buf[..pos]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|l| {
            l.split_once(':')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        })
        .collect();
    Resp {
        status,
        headers,
        body: buf[pos + 4..].to_vec(),
    }
}

fn request(target: &str, extra: &[(&str, &str)]) -> Vec<u8> {
    let mut r = format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n");
    for (k, v) in extra {
        r.push_str(&format!("{k}: {v}\r\n"));
    }
    r.push_str("\r\n");
    r.into_bytes()
}

async fn get(port: u16, target: &str, extra: &[(&str, &str)]) -> Resp {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    raw_exchange(&mut s, &request(target, extra)).await
}

#[tokio::test]
async fn full_get_matches_fixture_sha() {
    let srv = TestServer::start().await.unwrap();
    let r = get(srv.port(), "/files/small.bin", &[]).await;
    assert_eq!(r.status, 200);
    let fx = fixture("small.bin").unwrap();
    assert_eq!(r.h("Content-Length"), Some("65536"));
    assert_eq!(r.h("Accept-Ranges"), Some("bytes"));
    assert_eq!(r.body, fx.to_vec());
    use sha2::Digest;
    let hex: String = sha2::Sha256::digest(&r.body)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(hex, fx.sha256_hex());
    assert!(r.h("ETag").unwrap().starts_with('"'));
    assert!(r.h("Last-Modified").unwrap().ends_with("GMT"));
    assert_eq!(srv.request_count(), 1);
}

#[tokio::test]
async fn range_206_with_exact_content_range() {
    let srv = TestServer::start().await.unwrap();
    let fx = fixture("small.bin").unwrap();
    let r = get(
        srv.port(),
        "/files/small.bin",
        &[("Range", "bytes=100-199")],
    )
    .await;
    assert_eq!(r.status, 206);
    assert_eq!(r.h("Content-Range"), Some("bytes 100-199/65536"));
    assert_eq!(r.h("Content-Length"), Some("100"));
    assert_eq!(r.body, fx.range(100, 100));
    let r = get(srv.port(), "/files/small.bin", &[("Range", "bytes=-10")]).await;
    assert_eq!(r.h("Content-Range"), Some("bytes 65526-65535/65536"));
    let r = get(srv.port(), "/files/small.bin", &[("Range", "bytes=70000-")]).await;
    assert_eq!(r.status, 416);
    assert_eq!(r.h("Content-Range"), Some("bytes */65536"));
    // If-Range mismatch -> full 200.
    let r = get(
        srv.port(),
        "/files/small.bin",
        &[("Range", "bytes=0-9"), ("If-Range", "\"nope\"")],
    )
    .await;
    assert_eq!(r.status, 200);
    let etag = fx.etag();
    let r = get(
        srv.port(),
        "/files/small.bin",
        &[("Range", "bytes=0-9"), ("If-Range", &etag)],
    )
    .await;
    assert_eq!(r.status, 206);
}

#[tokio::test]
async fn range_faults() {
    let srv = TestServer::start().await.unwrap();
    let fx = fixture("small.bin").unwrap();
    let p = srv.port();
    let r = get(
        p,
        "/files/small.bin?ignore_range=1",
        &[("Range", "bytes=10-19")],
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body.len(), 65536);
    let r = get(
        p,
        "/files/small.bin?bad_content_range=1",
        &[("Range", "bytes=10-19")],
    )
    .await;
    assert_eq!(r.status, 206);
    assert_eq!(r.h("Content-Range"), Some("bytes 11-19/65536"));
    let r = get(
        p,
        "/files/small.bin?extra_bytes=7",
        &[("Range", "bytes=10-19")],
    )
    .await;
    assert_eq!(r.h("Content-Length"), Some("10"));
    assert_eq!(r.body.len(), 17);
    assert_eq!(&r.body[..10], &fx.range(10, 10)[..]);
    let r = get(
        p,
        "/files/small.bin?no_range=1",
        &[("Range", "bytes=10-19")],
    )
    .await;
    assert_eq!(r.status, 200);
    assert!(r.h("Accept-Ranges").is_none());
    let r = get(p, "/files/tiny.txt?short_length=1", &[]).await;
    assert!(r.h("Content-Length").unwrap().parse::<usize>().unwrap() > r.body.len());
    let r = get(p, "/files/tiny.txt?long_length=1", &[]).await;
    assert!(r.h("Content-Length").unwrap().parse::<usize>().unwrap() < r.body.len());
    let r = get(p, "/files/tiny.txt?no_length=1", &[]).await;
    assert!(r.h("Content-Length").is_none());
    assert_eq!(r.body, TINY_CONTENT);
    let r = get(p, "/files/tiny.txt?weak_etag=1", &[]).await;
    assert!(r.h("ETag").unwrap().starts_with("W/\""));
    let r = get(p, "/files/tiny.txt?no_validators=1", &[]).await;
    assert!(r.h("ETag").is_none() && r.h("Last-Modified").is_none());
    let r = get(p, "/files/tiny.txt?gzip=1", &[]).await;
    assert_eq!(r.h("Content-Encoding"), Some("gzip"));
    assert_eq!(r.body.len() as u64, gzip_len(13));
    assert_eq!(&r.body[..2], &[0x1f, 0x8b]);
    let r = get(p, "/files/tiny.txt?huge_header=1", &[]).await;
    assert_eq!(r.h("X-Huge").unwrap().len(), HUGE_HEADER_BYTES);
}

#[tokio::test]
async fn drop_after_closes_early_then_recovers() {
    let srv = TestServer::start().await.unwrap();
    let q = "/files/small.bin?drop_after=1000&drop_times=2";
    for _ in 0..2 {
        let r = get(srv.port(), q, &[]).await;
        assert_eq!(r.h("Content-Length"), Some("65536"));
        assert_eq!(r.body.len(), 1000);
    }
    let r = get(srv.port(), q, &[]).await;
    assert_eq!(r.body.len(), 65536);
}

#[tokio::test]
async fn slow_throttles() {
    let srv = TestServer::start().await.unwrap();
    let t = std::time::Instant::now();
    let r = get(
        srv.port(),
        "/files/small.bin?slow=200000",
        &[("Range", "bytes=0-39999")],
    )
    .await;
    assert_eq!(r.body.len(), 40000);
    assert!(t.elapsed() >= Duration::from_millis(150));
}

#[tokio::test]
async fn stall_and_endless_headers() {
    let srv = TestServer::start().await.unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", srv.port())).await.unwrap();
    s.write_all(&request("/files/tiny.txt?stall=1", &[]))
        .await
        .unwrap();
    let mut b = [0u8; 16];
    assert!(
        tokio::time::timeout(Duration::from_millis(300), s.read(&mut b))
            .await
            .is_err()
    );

    let mut s = TcpStream::connect(("127.0.0.1", srv.port())).await.unwrap();
    s.write_all(&request("/files/tiny.txt?endless_headers=1", &[]))
        .await
        .unwrap();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0;
    while total < 256 * 1024 {
        let n = s.read(&mut buf).await.unwrap();
        assert!(n > 0);
        assert!(!buf[..n].windows(4).any(|w| w == b"\r\n\r\n"));
        total += n;
    }
}

#[tokio::test]
async fn version_change_changes_etag_and_content() {
    let srv = TestServer::start().await.unwrap();
    let a = get(srv.port(), "/files/small.bin", &[("Range", "bytes=0-63")]).await;
    srv.set_version("small.bin", 2);
    let b = get(srv.port(), "/files/small.bin", &[("Range", "bytes=0-63")]).await;
    assert_ne!(a.h("ETag"), b.h("ETag"));
    assert_ne!(a.body, b.body);
    assert_eq!(
        b.body,
        fixture_version("small.bin", 2).unwrap().range(0, 64)
    );
    assert_eq!(srv.fixture("small.bin").unwrap().version, 2);
}

#[tokio::test]
async fn redirect_chain_and_loop() {
    let srv = TestServer::start().await.unwrap();
    let mut target = "/redirect/3/files/tiny.txt?x=1".to_string();
    let mut hops = 0;
    loop {
        let r = get(srv.port(), &target, &[]).await;
        if r.status == 302 {
            target = r.h("Location").unwrap().to_string();
            hops += 1;
            continue;
        }
        assert_eq!(r.status, 200);
        assert_eq!(r.body, TINY_CONTENT);
        break;
    }
    assert_eq!(hops, 3);
    assert_eq!(target, "/files/tiny.txt?x=1");
    let r = get(srv.port(), "/redirect-loop", &[]).await;
    assert_eq!(r.h("Location"), Some("/redirect-loop"));
    let r = get(
        srv.port(),
        "/redirect-abs?to=http%3A%2F%2F127.0.0.1%3A1%2Fx",
        &[],
    )
    .await;
    assert_eq!(r.h("Location"), Some("http://127.0.0.1:1/x"));
}

#[tokio::test]
async fn disposition_site_status() {
    let srv = TestServer::start().await.unwrap();
    let r = get(
        srv.port(),
        "/disposition?v=attachment%3B%20filename%3D%22..%2Fevil.exe%22&file=tiny.txt",
        &[],
    )
    .await;
    assert_eq!(
        r.h("Content-Disposition"),
        Some("attachment; filename=\"../evil.exe\"")
    );
    assert_eq!(r.body, TINY_CONTENT);
    let r = get(srv.port(), "/site/index.html", &[]).await;
    let html = String::from_utf8(r.body).unwrap();
    assert!(html.contains("p1.html") && html.contains("/files/tiny.txt"));
    let r = get(srv.port(), "/site/p3.html", &[]).await;
    assert!(
        String::from_utf8(r.body)
            .unwrap()
            .contains("http://example.invalid/")
    );
    assert_eq!(get(srv.port(), "/status/503", &[]).await.status, 503);
    assert_eq!(get(srv.port(), "/nope", &[]).await.status, 404);
}

#[tokio::test]
async fn basic_auth() {
    let srv = TestServer::start().await.unwrap();
    let r = get(srv.port(), "/auth/basic/files/tiny.txt", &[]).await;
    assert_eq!(r.status, 401);
    assert_eq!(r.h("WWW-Authenticate"), Some("Basic realm=\"udm\""));
    let cred =
        base64::engine::general_purpose::STANDARD.encode(format!("{AUTH_USER}:{AUTH_PASSWORD}"));
    let r = get(
        srv.port(),
        "/auth/basic/files/tiny.txt",
        &[("Authorization", &format!("Basic {cred}"))],
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body, TINY_CONTENT);
}

#[tokio::test]
async fn digest_auth_accepts_correct_credentials() {
    let srv = TestServer::start().await.unwrap();
    let uri = "/auth/digest/files/tiny.txt";
    let r = get(srv.port(), uri, &[]).await;
    assert_eq!(r.status, 401);
    let ch = r.h("WWW-Authenticate").unwrap().to_string();
    assert!(
        ch.starts_with("Digest ")
            && ch.contains("algorithm=SHA-256")
            && ch.contains("qop=\"auth\"")
    );
    let nonce = ch
        .split("nonce=\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let (nc, cnonce) = ("00000001", "0a4f113b");
    // Independent computation with sha2.
    use sha2::Digest;
    let h = |s: String| -> String {
        sha2::Sha256::digest(s.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    let ha1 = h(format!("{AUTH_USER}:{AUTH_REALM}:{AUTH_PASSWORD}"));
    let ha2 = h(format!("GET:{uri}"));
    let resp = h(format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}"));
    assert_eq!(
        resp,
        digest_sha256_response(
            AUTH_USER,
            AUTH_REALM,
            AUTH_PASSWORD,
            "GET",
            uri,
            nonce,
            nc,
            cnonce
        )
    );
    let auth = format!(
        "Digest username=\"{AUTH_USER}\", realm=\"{AUTH_REALM}\", nonce=\"{nonce}\", uri=\"{uri}\", algorithm=SHA-256, qop=auth, nc={nc}, cnonce=\"{cnonce}\", response=\"{resp}\", opaque=\"udm-opaque\""
    );
    let r = get(srv.port(), uri, &[("Authorization", &auth)]).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body, TINY_CONTENT);
    let bad = auth.replace(&resp, &"0".repeat(64));
    assert_eq!(
        get(srv.port(), uri, &[("Authorization", &bad)])
            .await
            .status,
        401
    );
}

#[tokio::test]
async fn restart_keeps_port_and_serves() {
    let srv = TestServer::start().await.unwrap();
    let port = srv.port();
    // A long slow download that restart must cut.
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(&request("/files/small.bin?slow=1000", &[]))
        .await
        .unwrap();
    let mut b = [0u8; 256];
    assert!(s.read(&mut b).await.unwrap() > 0);
    srv.restart().await;
    let mut rest = Vec::new();
    let r = tokio::time::timeout(Duration::from_secs(2), s.read_to_end(&mut rest)).await;
    assert!(r.is_ok(), "connection not cut");
    assert_eq!(srv.port(), port);
    let r = get(port, "/files/tiny.txt", &[]).await;
    assert_eq!(r.body, TINY_CONTENT);
}

fn client_tls(cert: Vec<u8>, alpn: &[&[u8]]) -> tokio_rustls::TlsConnector {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(cert)).unwrap();
    let mut cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    cfg.alpn_protocols = alpn.iter().map(|a| a.to_vec()).collect();
    tokio_rustls::TlsConnector::from(Arc::new(cfg))
}

#[tokio::test]
async fn tls_h2_negotiation_ranges_and_reset() {
    let srv = TestServer::start_tls().await.unwrap();
    assert!(srv.base_url().starts_with("https://"));
    let conn = client_tls(srv.cert_der().unwrap(), &[b"h2", b"http/1.1"]);
    let tcp = TcpStream::connect(("127.0.0.1", srv.port())).await.unwrap();
    let tls = conn
        .connect(ServerName::try_from("127.0.0.1").unwrap(), tcp)
        .await
        .unwrap();
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(b"h2".as_slice()));
    let (mut send, conn_fut) = hyper::client::conn::http2::handshake(
        hyper_util::rt::TokioExecutor::new(),
        hyper_util::rt::TokioIo::new(tls),
    )
    .await
    .unwrap();
    tokio::spawn(conn_fut);
    let fx = fixture("small.bin").unwrap();
    let req = http::Request::builder()
        .uri(srv.url("/files/small.bin"))
        .header("range", "bytes=1000-2999")
        .body(Empty::<Bytes>::new())
        .unwrap();
    let resp = send.send_request(req).await.unwrap();
    assert_eq!(resp.status(), 206);
    assert_eq!(resp.headers()["content-range"], "bytes 1000-2999/65536");
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], &fx.range(1000, 2000)[..]);

    let req = http::Request::builder()
        .uri(srv.url("/files/small.bin?rst_after=5000"))
        .body(Empty::<Bytes>::new())
        .unwrap();
    let resp = send.send_request(req).await.unwrap();
    assert_eq!(resp.status(), 200);
    let mut body = resp.into_body();
    let mut got = 0usize;
    let mut errored = false;
    while let Some(f) = body.frame().await {
        match f {
            Ok(f) => got += f.data_ref().map(|d| d.len()).unwrap_or(0),
            Err(_) => {
                errored = true;
                break;
            }
        }
    }
    assert!(errored, "stream was not reset");
    assert!(got <= 5000);
    assert_eq!(srv.request_count(), 2);
}

#[tokio::test]
async fn tls_http1_and_downgrade() {
    let plain = TestServer::start().await.unwrap();
    let srv = TestServer::start_tls().await.unwrap();
    srv.set_downgrade_target(&plain.base_url());
    let conn = client_tls(srv.cert_der().unwrap(), &[b"http/1.1"]);
    let tcp = TcpStream::connect(("127.0.0.1", srv.port())).await.unwrap();
    let mut tls = conn
        .connect(ServerName::try_from("localhost").unwrap(), tcp)
        .await
        .unwrap();
    let r = raw_exchange(&mut tls, &request("/downgrade/files/small.bin", &[])).await;
    assert_eq!(r.status, 302);
    assert_eq!(
        r.h("Location").unwrap(),
        format!("{}/files/small.bin", plain.base_url())
    );
}

async fn ftp_line<S: AsyncRead + Unpin>(s: &mut S) -> String {
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    loop {
        s.read_exact(&mut b).await.unwrap();
        line.push(b[0]);
        if b[0] == b'\n' {
            return String::from_utf8(line).unwrap();
        }
    }
}

async fn ftp_cmd<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S, cmd: &str) -> String {
    s.write_all(format!("{cmd}\r\n").as_bytes()).await.unwrap();
    // Skip multi-line continuation lines ("ddd-").
    loop {
        let l = ftp_line(s).await;
        if l.as_bytes().get(3) == Some(&b' ') {
            return l;
        }
    }
}

fn epsv_port(reply: &str) -> u16 {
    reply
        .split("|||")
        .nth(1)
        .unwrap()
        .split('|')
        .next()
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test]
async fn ftp_retr_with_rest_and_drop() {
    let srv = FtpTestServer::start().await.unwrap();
    let mut c = TcpStream::connect(("127.0.0.1", srv.port())).await.unwrap();
    assert!(ftp_line(&mut c).await.starts_with("220"));
    assert!(
        ftp_cmd(&mut c, "SIZE /files/small.bin")
            .await
            .starts_with("530")
    );
    assert!(
        ftp_cmd(&mut c, &format!("USER {AUTH_USER}"))
            .await
            .starts_with("331")
    );
    assert!(
        ftp_cmd(&mut c, &format!("PASS {AUTH_PASSWORD}"))
            .await
            .starts_with("230")
    );
    assert!(ftp_cmd(&mut c, "FEAT").await.starts_with("211"));
    assert!(ftp_cmd(&mut c, "TYPE I").await.starts_with("200"));
    assert_eq!(
        ftp_cmd(&mut c, "SIZE /files/small.bin").await.trim(),
        "213 65536"
    );
    assert_eq!(
        ftp_cmd(&mut c, "MDTM /files/small.bin").await.trim(),
        format!("213 {FTP_MDTM}")
    );
    assert!(ftp_cmd(&mut c, "CWD /files").await.starts_with("250"));
    let port = epsv_port(&ftp_cmd(&mut c, "EPSV").await);
    assert!(ftp_cmd(&mut c, "REST 1000").await.starts_with("350"));
    let mut data = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    assert!(ftp_cmd(&mut c, "RETR small.bin").await.starts_with("150"));
    let mut got = Vec::new();
    data.read_to_end(&mut got).await.unwrap();
    assert!(ftp_line(&mut c).await.starts_with("226"));
    assert_eq!(got, fixture("small.bin").unwrap().range(1000, 65536));

    // Drop fault.
    let reply = ftp_cmd(&mut c, "PASV").await;
    let nums: Vec<u16> = reply
        .split('(')
        .nth(1)
        .unwrap()
        .split(')')
        .next()
        .unwrap()
        .split(',')
        .map(|n| n.trim().parse().unwrap())
        .collect();
    let mut data = TcpStream::connect(("127.0.0.1", nums[4] * 256 + nums[5]))
        .await
        .unwrap();
    assert!(
        ftp_cmd(&mut c, "RETR /drop/500/small.bin")
            .await
            .starts_with("150")
    );
    let mut got = Vec::new();
    let _ = data.read_to_end(&mut got).await;
    assert_eq!(got.len(), 500);
    assert!(ftp_line(&mut c).await.starts_with("426"));
    let long = ftp_cmd(&mut c, "LONGLINE").await;
    assert!(long.len() > FTP_MAX_LINE);
    assert!(ftp_cmd(&mut c, "QUIT").await.starts_with("221"));
    assert_eq!(srv.retr_count(), 2);
}

#[tokio::test]
async fn ftps_explicit_retr() {
    let srv = FtpTestServer::start_explicit_tls().await.unwrap();
    let conn = client_tls(srv.cert_der().unwrap(), &[]);
    let mut c = TcpStream::connect(("127.0.0.1", srv.port())).await.unwrap();
    assert!(ftp_line(&mut c).await.starts_with("220"));
    assert!(ftp_cmd(&mut c, "AUTH TLS").await.starts_with("234"));
    let mut c = conn
        .connect(ServerName::try_from("127.0.0.1").unwrap(), c)
        .await
        .unwrap();
    assert!(ftp_cmd(&mut c, "USER anonymous").await.starts_with("331"));
    assert!(ftp_cmd(&mut c, "PASS x@y").await.starts_with("230"));
    assert!(ftp_cmd(&mut c, "PBSZ 0").await.starts_with("200"));
    assert!(ftp_cmd(&mut c, "PROT P").await.starts_with("200"));
    let port = epsv_port(&ftp_cmd(&mut c, "EPSV").await);
    let tcp = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    assert!(
        ftp_cmd(&mut c, "RETR /files/tiny.txt")
            .await
            .starts_with("150")
    );
    let mut data = conn
        .connect(ServerName::try_from("127.0.0.1").unwrap(), tcp)
        .await
        .unwrap();
    let mut got = Vec::new();
    let _ = data.read_to_end(&mut got).await;
    assert_eq!(got, TINY_CONTENT);
    assert!(ftp_line(&mut c).await.starts_with("226"));
}

#[tokio::test]
async fn socks5_tunnel_with_auth() {
    let srv = TestServer::start().await.unwrap();
    let proxy = Socks5Proxy::start(Some(("pu".into(), "pp".into())))
        .await
        .unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", proxy.port()))
        .await
        .unwrap();
    s.write_all(&[5, 1, 2]).await.unwrap();
    let mut b = [0u8; 2];
    s.read_exact(&mut b).await.unwrap();
    assert_eq!(b, [5, 2]);
    s.write_all(&[1, 2, b'p', b'u', 2, b'p', b'p'])
        .await
        .unwrap();
    s.read_exact(&mut b).await.unwrap();
    assert_eq!(b, [1, 0]);
    let mut req = vec![5, 1, 0, 3, 9];
    req.extend_from_slice(b"localhost");
    req.extend_from_slice(&srv.port().to_be_bytes());
    s.write_all(&req).await.unwrap();
    let mut rep = [0u8; 10];
    s.read_exact(&mut rep).await.unwrap();
    assert_eq!(rep[1], 0);
    let r = raw_exchange(&mut s, &request("/files/tiny.txt", &[])).await;
    assert_eq!(r.body, TINY_CONTENT);
    assert_eq!(proxy.tunnel_count(), 1);

    // Non-loopback target is refused (never reaches the internet).
    let open = Socks5Proxy::start(None).await.unwrap();
    let mut s = TcpStream::connect(("127.0.0.1", open.port()))
        .await
        .unwrap();
    s.write_all(&[5, 1, 0]).await.unwrap();
    s.read_exact(&mut b).await.unwrap();
    assert_eq!(b, [5, 0]);
    s.write_all(&[5, 1, 0, 1, 8, 8, 8, 8, 0, 80]).await.unwrap();
    s.read_exact(&mut rep).await.unwrap();
    assert_eq!(rep[1], 2);
    assert_eq!(open.tunnel_count(), 0);
}

#[tokio::test]
async fn http_connect_proxy_with_auth() {
    let srv = TestServer::start().await.unwrap();
    let proxy = HttpConnectProxy::start(Some(("pu".into(), "pp".into())))
        .await
        .unwrap();
    let target = format!("127.0.0.1:{}", srv.port());
    let mut s = TcpStream::connect(("127.0.0.1", proxy.port()))
        .await
        .unwrap();
    let r = raw_exchange(
        &mut s,
        format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes(),
    )
    .await;
    assert_eq!(r.status, 407);
    let cred = base64::engine::general_purpose::STANDARD.encode("pu:pp");
    let mut s = TcpStream::connect(("127.0.0.1", proxy.port()))
        .await
        .unwrap();
    s.write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nProxy-Authorization: Basic {cred}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut head = vec![0u8; "HTTP/1.1 200 Connection Established\r\n\r\n".len()];
    s.read_exact(&mut head).await.unwrap();
    assert!(head.starts_with(b"HTTP/1.1 200"));
    let r = raw_exchange(&mut s, &request("/files/tiny.txt", &[])).await;
    assert_eq!(r.body, TINY_CONTENT);
    assert_eq!(proxy.tunnel_count(), 1);
}
