//! HTTPS / HTTP/2 behaviour against the local TLS test server.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::*;
use udm_net::{Client, NetConfig, NetError, Protocol, TlsException, cert_fingerprint};
use udm_testsrv::{TestServer, fixture};

fn trusted(srv: &TestServer) -> Client {
    Client::new(NetConfig {
        extra_root_der: vec![srv.cert_der().unwrap()],
        ..cfg()
    })
    .unwrap()
}

#[tokio::test]
async fn https_with_extra_root_uses_http2() {
    let srv = TestServer::start_tls().await.unwrap();
    let fx = fixture("medium.bin").unwrap();
    let c = trusted(&srv);
    let p = c.probe(&req(&srv.url("/files/medium.bin"))).await.unwrap();
    assert_eq!(p.protocol, Protocol::Http2);
    assert_eq!(p.size, Some(fx.len));
    assert!(p.accepts_ranges);
    let mut all = Vec::new();
    let half = fx.len / 2;
    for r in [range(0, half), range(half, fx.len)] {
        let b = c
            .fetch(
                &req(&srv.url("/files/medium.bin")),
                Some(r),
                Some(fx.len),
                None,
            )
            .await
            .unwrap();
        let (partial, mut s) = unwrap_body(b);
        assert!(partial);
        let (data, res) = read_all(&mut s).await;
        res.unwrap();
        all.extend_from_slice(&data);
    }
    assert_eq!(sha256_hex(&all), fx.sha256_hex());
}

#[tokio::test]
async fn hostile_untrusted_certificate_reports_host_and_fingerprint() {
    let srv = TestServer::start_tls().await.unwrap();
    let err = client()
        .probe(&req(&srv.url("/files/tiny.txt")))
        .await
        .unwrap_err();
    let expected_fp = cert_fingerprint(&srv.cert_der().unwrap());
    match &err {
        NetError::TlsUntrusted { host, fingerprint } => {
            assert_eq!(host, "127.0.0.1");
            assert_eq!(fingerprint, &expected_fp);
        }
        other => panic!("expected TlsUntrusted, got {other:?}"),
    }
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn pinned_exception_accepts_exact_certificate_only() {
    let srv = TestServer::start_tls().await.unwrap();
    let fp = cert_fingerprint(&srv.cert_der().unwrap());
    let ok = Client::new(NetConfig {
        tls_exceptions: vec![TlsException {
            host: "127.0.0.1".into(),
            sha256_fingerprint: fp.to_uppercase(),
        }],
        ..cfg()
    })
    .unwrap();
    let p = ok.probe(&req(&srv.url("/files/tiny.txt"))).await.unwrap();
    assert_eq!(p.size, Some(13));

    // Same fingerprint pinned for another host: still refused.
    let other_host = Client::new(NetConfig {
        tls_exceptions: vec![TlsException {
            host: "example.com".into(),
            sha256_fingerprint: fp,
        }],
        ..cfg()
    })
    .unwrap();
    let e = other_host
        .probe(&req(&srv.url("/files/tiny.txt")))
        .await
        .unwrap_err();
    assert!(matches!(e, NetError::TlsUntrusted { .. }), "{e:?}");

    // Wrong fingerprint for the right host: refused.
    let wrong = Client::new(NetConfig {
        tls_exceptions: vec![TlsException {
            host: "127.0.0.1".into(),
            sha256_fingerprint: "00".repeat(32),
        }],
        ..cfg()
    })
    .unwrap();
    let e = wrong
        .probe(&req(&srv.url("/files/tiny.txt")))
        .await
        .unwrap_err();
    assert!(matches!(e, NetError::TlsUntrusted { .. }), "{e:?}");
}

#[tokio::test]
async fn hostile_https_to_http_downgrade_refused() {
    let tls = TestServer::start_tls().await.unwrap();
    let plain = TestServer::start().await.unwrap();
    tls.set_downgrade_target(&plain.base_url());
    let c = trusted(&tls);
    let e = c
        .probe(&req(&tls.url("/downgrade/files/tiny.txt")))
        .await
        .unwrap_err();
    assert_eq!(e, NetError::DowngradeRefused);
    assert_eq!(plain.request_count(), 0, "must not contact the http target");

    let mut r = req(&tls.url("/downgrade/files/tiny.txt"));
    r.allow_downgrade = true;
    let p = c.probe(&r).await.unwrap();
    assert_eq!(p.final_url.scheme(), "http");
    assert_eq!(p.size, Some(13));
}

#[tokio::test]
async fn downgrade_allowed_globally() {
    let tls = TestServer::start_tls().await.unwrap();
    let plain = TestServer::start().await.unwrap();
    tls.set_downgrade_target(&plain.base_url());
    let c = Client::new(NetConfig {
        extra_root_der: vec![tls.cert_der().unwrap()],
        allow_downgrade: true,
        ..cfg()
    })
    .unwrap();
    let p = c
        .probe(&req(&tls.url("/downgrade/files/tiny.txt")))
        .await
        .unwrap();
    assert_eq!(p.final_url.scheme(), "http");
}

#[tokio::test]
async fn h2_stream_reset_is_retryable() {
    let srv = TestServer::start_tls().await.unwrap();
    let b = trusted(&srv)
        .fetch(
            &req(&srv.url("/files/medium.bin?rst_after=20000")),
            Some(range(0, 1_000_000)),
            Some(4_194_304),
            None,
        )
        .await
        .unwrap();
    let (_, mut s) = unwrap_body(b);
    let (data, res) = read_all(&mut s).await;
    assert!(data.len() < 1_000_000);
    let e = res.unwrap_err();
    assert!(e.is_retryable(), "{e:?}");
}

#[tokio::test]
async fn hostile_h2_huge_header_bounded() {
    let srv = TestServer::start_tls().await.unwrap();
    let e = trusted(&srv)
        .probe(&req(&srv.url("/files/small.bin?huge_header=1")))
        .await
        .unwrap_err();
    assert!(
        matches!(
            e,
            NetError::HeaderTooLarge { .. } | NetError::Protocol | NetError::Stream
        ),
        "{e:?}"
    );
}

#[tokio::test]
async fn hostile_h2_gzip_rejected() {
    let srv = TestServer::start_tls().await.unwrap();
    let e = trusted(&srv)
        .fetch(&req(&srv.url("/files/small.bin?gzip=1")), None, None, None)
        .await
        .unwrap_err();
    assert!(matches!(e, NetError::ContentEncoding { .. }), "{e:?}");
}
