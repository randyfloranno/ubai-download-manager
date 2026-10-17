//! FTP and FTPS against the local FTP test server.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::*;
use udm_net::{Client, NetConfig, NetError, Protocol};
use udm_testsrv::{FtpTestServer, fixture};

fn trusted(srv: &FtpTestServer) -> Client {
    Client::new(NetConfig {
        extra_root_der: vec![srv.cert_der().unwrap()],
        ..cfg()
    })
    .unwrap()
}

/// Explicit FTPS: the server advertises `ftp://`; UDM asks for TLS via `ftps://`.
fn explicit_base(srv: &FtpTestServer) -> String {
    format!("ftps://127.0.0.1:{}", srv.port())
}

async fn check_server(c: &Client, base: &str, proto: Protocol) {
    let fx = fixture("medium.bin").unwrap();
    let url = format!("{base}/files/medium.bin");
    let p = c.probe(&req(&url)).await.unwrap();
    assert_eq!(p.size, Some(fx.len));
    assert!(p.accepts_ranges);
    assert_eq!(p.protocol, proto);
    assert!(p.last_modified.is_some());

    let half = fx.len / 2;
    let mut all = Vec::new();
    for r in [range(0, half), range(half, fx.len)] {
        let b = c
            .fetch(&req(&url), Some(r), Some(fx.len), None)
            .await
            .unwrap();
        let (partial, mut s) = unwrap_body(b);
        assert!(partial);
        let (data, res) = read_all(&mut s).await;
        res.unwrap();
        assert_eq!(data.len() as u64, r.len());
        all.extend_from_slice(&data);
    }
    assert_eq!(sha256_hex(&all), fx.sha256_hex());

    let b = c.fetch(&req(&url), None, Some(fx.len), None).await.unwrap();
    let (partial, mut s) = unwrap_body(b);
    assert!(!partial);
    let (data, res) = read_all(&mut s).await;
    res.unwrap();
    assert_eq!(sha256_hex(&data), fx.sha256_hex());
}

#[tokio::test]
async fn ftp_plain_probe_and_fetch() {
    let srv = FtpTestServer::start().await.unwrap();
    check_server(&client(), &srv.base_url(), Protocol::Ftp).await;
}

#[tokio::test]
async fn ftps_explicit_probe_and_fetch() {
    let srv = FtpTestServer::start_explicit_tls().await.unwrap();
    let base = explicit_base(&srv);
    check_server(&trusted(&srv), &base, Protocol::Ftps).await;
}

#[tokio::test]
async fn ftps_implicit_probe_and_fetch() {
    let srv = FtpTestServer::start_implicit_tls().await.unwrap();
    assert!(srv.base_url().starts_with("ftps://"));
    check_server(&trusted(&srv), &srv.base_url(), Protocol::Ftps).await;
}

#[tokio::test]
async fn hostile_ftps_untrusted_certificate() {
    let srv = FtpTestServer::start_implicit_tls().await.unwrap();
    let e = client()
        .probe(&req(&format!("{}/files/tiny.txt", srv.base_url())))
        .await
        .unwrap_err();
    assert!(matches!(e, NetError::TlsUntrusted { .. }), "{e:?}");
}

#[tokio::test]
async fn ftp_data_drop_is_retryable() {
    let srv = FtpTestServer::start().await.unwrap();
    let url = format!("{}/drop/10000/small.bin", srv.base_url());
    let b = client()
        .fetch(&req(&url), None, Some(65_536), None)
        .await
        .unwrap();
    let (_, mut s) = unwrap_body(b);
    let (data, res) = read_all(&mut s).await;
    assert!(data.len() <= 10_000);
    let e = res.unwrap_err();
    assert!(
        matches!(e, NetError::ShortBody { .. } | NetError::Ftp { .. }),
        "{e:?}"
    );
    assert!(e.is_retryable(), "{e:?}");
}

#[tokio::test]
async fn hostile_ftp_long_reply_line_bounded() {
    let srv = FtpTestServer::start().await.unwrap();
    let url = format!("{}/longline", srv.base_url());
    let e = client().probe(&req(&url)).await.unwrap_err();
    assert_eq!(e, NetError::Protocol);
}

#[tokio::test]
async fn ftp_missing_file_is_error() {
    let srv = FtpTestServer::start().await.unwrap();
    let url = format!("{}/files/nope.bin", srv.base_url());
    let e = client().probe(&req(&url)).await.unwrap_err();
    assert!(matches!(e, NetError::Ftp { code: 550, .. }), "{e:?}");
    assert!(!e.is_retryable());
}

#[tokio::test]
async fn hostile_ftp_crlf_in_path_rejected() {
    let srv = FtpTestServer::start().await.unwrap();
    let url = format!("{}/files/tiny.txt%0D%0ADELE%20x", srv.base_url());
    let e = client().probe(&req(&url)).await.unwrap_err();
    assert_eq!(e, NetError::InvalidRequest);
}
