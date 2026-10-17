//! HTTP/1.1 behaviour against the local fault-injection server.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::*;
use udm_core::Secret;
use udm_net::{Client, FetchBody, NetConfig, NetError, Protocol, TimeoutPhase, Timeouts};
use udm_proto::EntityTag;
use udm_testsrv::{AUTH_PASSWORD, AUTH_USER, TestServer, fixture};

const MEDIUM: &str = "/files/medium.bin";

#[tokio::test]
async fn probe_medium_reports_size_ranges_and_etag() {
    let srv = TestServer::start().await.unwrap();
    let fx = fixture("medium.bin").unwrap();
    let p = client().probe(&req(&srv.url(MEDIUM))).await.unwrap();
    assert_eq!(p.size, Some(fx.len));
    assert!(p.accepts_ranges);
    let etag = p.etag.unwrap();
    assert!(!etag.weak);
    assert_eq!(etag.to_header(), fx.etag());
    assert_eq!(
        p.last_modified.as_deref(),
        Some(fx.last_modified().as_str())
    );
    assert_eq!(p.protocol, Protocol::Http1);
    assert_eq!(p.final_url.as_str(), srv.url(MEDIUM));
}

#[tokio::test]
async fn probe_no_range_reports_size_without_ranges() {
    let srv = TestServer::start().await.unwrap();
    let p = client()
        .probe(&req(&srv.url("/files/small.bin?no_range=1")))
        .await
        .unwrap();
    assert_eq!(p.size, Some(65_536));
    assert!(!p.accepts_ranges);
}

#[tokio::test]
async fn probe_reads_content_disposition_raw() {
    let srv = TestServer::start().await.unwrap();
    let p = client()
        .probe(&req(&srv.url(
            "/disposition?v=attachment%3B%20filename%3D%22report.pdf%22&file=tiny.txt",
        )))
        .await
        .unwrap();
    assert_eq!(p.filename.as_deref(), Some("report.pdf"));
}

#[tokio::test]
async fn ranged_fetches_reassemble_to_fixture_sha256() {
    let srv = TestServer::start().await.unwrap();
    let fx = fixture("medium.bin").unwrap();
    let c = client();
    let etag = EntityTag::parse(&fx.etag()).unwrap();
    let half = fx.len / 2;
    let mut all = Vec::new();
    for r in [range(0, half), range(half, fx.len)] {
        let body = c
            .fetch(&req(&srv.url(MEDIUM)), Some(r), Some(fx.len), Some(&etag))
            .await
            .unwrap();
        let (partial, mut s) = unwrap_body(body);
        assert!(partial);
        assert_eq!(s.expected_len(), Some(r.len()));
        let (data, res) = read_all(&mut s).await;
        res.unwrap();
        assert_eq!(data.len() as u64, r.len());
        all.extend_from_slice(&data);
    }
    assert_eq!(sha256_hex(&all), fx.sha256_hex());
}

#[tokio::test]
async fn full_fetch_matches_fixture() {
    let srv = TestServer::start().await.unwrap();
    let fx = fixture("medium.bin").unwrap();
    let body = client()
        .fetch(&req(&srv.url(MEDIUM)), None, Some(fx.len), None)
        .await
        .unwrap();
    let (partial, mut s) = unwrap_body(body);
    assert!(!partial);
    let (data, res) = read_all(&mut s).await;
    res.unwrap();
    assert_eq!(sha256_hex(&data), fx.sha256_hex());
}

#[tokio::test]
async fn ignore_range_yields_full() {
    let srv = TestServer::start().await.unwrap();
    let fx = fixture("small.bin").unwrap();
    let body = client()
        .fetch(
            &req(&srv.url("/files/small.bin?ignore_range=1")),
            Some(range(1000, 2000)),
            Some(fx.len),
            None,
        )
        .await
        .unwrap();
    let (partial, mut s) = unwrap_body(body);
    assert!(!partial, "a 200 answer to a Range request must be Full");
    let (data, res) = read_all(&mut s).await;
    res.unwrap();
    assert_eq!(sha256_hex(&data), fx.sha256_hex());
}

#[tokio::test]
async fn hostile_bad_content_range_is_rejected() {
    let srv = TestServer::start().await.unwrap();
    let err = client()
        .fetch(
            &req(&srv.url("/files/small.bin?bad_content_range=1")),
            Some(range(1000, 2000)),
            Some(65_536),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::BadContentRange { .. }), "{err:?}");
    assert!(err.is_retryable());
}

#[tokio::test]
async fn hostile_content_range_total_mismatch_is_rejected() {
    let srv = TestServer::start().await.unwrap();
    let err = client()
        .fetch(
            &req(&srv.url("/files/small.bin")),
            Some(range(0, 100)),
            Some(65_537),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::BadContentRange { .. }), "{err:?}");
}

#[tokio::test]
async fn hostile_short_length_is_rejected() {
    // A 206 whose Content-Length claims more than its Content-Range.
    let srv = TestServer::start().await.unwrap();
    let err = client()
        .fetch(
            &req(&srv.url("/files/small.bin?short_length=1")),
            Some(range(0, 4096)),
            Some(65_536),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::BadContentRange { .. }), "{err:?}");
    assert!(err.is_retryable());
}

#[tokio::test]
async fn hostile_long_length_is_rejected() {
    // A 206 whose Content-Length claims less than its Content-Range.
    let srv = TestServer::start().await.unwrap();
    let err = client()
        .fetch(
            &req(&srv.url("/files/small.bin?long_length=1")),
            Some(range(0, 4096)),
            Some(65_536),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::BadContentRange { .. }), "{err:?}");
}

#[tokio::test]
async fn hostile_full_body_shorter_than_expected_total_is_short_body() {
    let srv = TestServer::start().await.unwrap();
    let body = client()
        .fetch(
            &req(&srv.url("/files/small.bin?long_length=1")),
            None,
            Some(65_536),
            None,
        )
        .await
        .unwrap();
    let (_, mut s) = unwrap_body(body);
    let (_, res) = read_all(&mut s).await;
    assert!(matches!(res, Err(NetError::ShortBody { .. })), "{res:?}");
}

#[tokio::test]
async fn hostile_extra_bytes_never_exceed_range() {
    let fx = fixture("small.bin").unwrap();
    let srv = TestServer::start().await.unwrap();
    // With a correct Content-Length the extra bytes are cut by framing.
    for q in ["extra_bytes=1000", "extra_bytes=1000&no_length=1"] {
        let body = client()
            .fetch(
                &req(&srv.url(&format!("/files/small.bin?{q}"))),
                Some(range(0, 4096)),
                Some(fx.len),
                None,
            )
            .await
            .unwrap();
        let (_, mut s) = unwrap_body(body);
        let (data, res) = read_all(&mut s).await;
        assert!(data.len() <= 4096, "{q}: yielded {} bytes", data.len());
        assert_eq!(data, fx.range(0, data.len() as u64), "{q}");
        match res {
            Ok(()) => assert_eq!(data.len(), 4096, "{q}"),
            Err(NetError::ExtraBytes { expected }) => assert_eq!(expected, 4096, "{q}"),
            Err(e) => panic!("{q}: unexpected {e:?}"),
        }
    }
    // Without Content-Length the server streams past the range: must be ExtraBytes.
    let body = client()
        .fetch(
            &req(&srv.url("/files/small.bin?extra_bytes=1000&no_length=1")),
            Some(range(0, 4096)),
            Some(fx.len),
            None,
        )
        .await
        .unwrap();
    let (_, mut s) = unwrap_body(body);
    let (data, res) = read_all(&mut s).await;
    assert_eq!(data.len(), 4096);
    assert!(matches!(res, Err(NetError::ExtraBytes { .. })), "{res:?}");
}

#[tokio::test]
async fn slow_server_completes() {
    let srv = TestServer::start().await.unwrap();
    let fx = fixture("tiny.txt").unwrap();
    let body = client()
        .fetch(
            &req(&srv.url("/files/tiny.txt?slow=65536")),
            None,
            Some(fx.len),
            None,
        )
        .await
        .unwrap();
    let (_, mut s) = unwrap_body(body);
    let (data, res) = read_all(&mut s).await;
    res.unwrap();
    assert_eq!(sha256_hex(&data), fx.sha256_hex());
}

fn short_timeouts() -> Client {
    Client::new(NetConfig {
        timeouts: Timeouts {
            connect_ms: 2_000,
            tls_ms: 2_000,
            first_byte_ms: 400,
            idle_ms: 300,
        },
        ..NetConfig::default()
    })
    .unwrap()
}

#[tokio::test]
async fn hostile_stall_times_out() {
    let srv = TestServer::start().await.unwrap();
    let err = short_timeouts()
        .fetch(&req(&srv.url("/files/small.bin?stall=1")), None, None, None)
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            NetError::Timeout {
                phase: TimeoutPhase::FirstByte,
                ..
            }
        ),
        "{err:?}"
    );
    assert!(err.is_retryable());
}

#[tokio::test]
async fn hostile_trickle_hits_idle_timeout() {
    let srv = TestServer::start().await.unwrap();
    let body = short_timeouts()
        .fetch(&req(&srv.url("/files/small.bin?slow=1")), None, None, None)
        .await
        .unwrap();
    let (_, mut s) = unwrap_body(body);
    let (_, res) = read_all(&mut s).await;
    assert!(
        matches!(
            res,
            Err(NetError::Timeout {
                phase: TimeoutPhase::Idle,
                ..
            })
        ),
        "{res:?}"
    );
}

#[tokio::test]
async fn drop_mid_body_is_retryable() {
    let srv = TestServer::start().await.unwrap();
    let body = client()
        .fetch(
            &req(&srv.url("/files/medium.bin?drop_after=10000")),
            Some(range(0, 100_000)),
            Some(4_194_304),
            None,
        )
        .await
        .unwrap();
    let (_, mut s) = unwrap_body(body);
    let (data, res) = read_all(&mut s).await;
    assert!(data.len() < 100_000);
    let err = res.unwrap_err();
    assert!(err.is_retryable(), "{err:?}");
}

#[tokio::test]
async fn redirects_up_to_limit_are_followed() {
    let srv = TestServer::start().await.unwrap();
    let p = client()
        .probe(&req(&srv.url("/redirect/10/files/tiny.txt")))
        .await
        .unwrap();
    assert_eq!(p.size, Some(13));
    assert_eq!(p.final_url.as_str(), srv.url("/files/tiny.txt"));
}

#[tokio::test]
async fn hostile_too_many_redirects() {
    let srv = TestServer::start().await.unwrap();
    let err = client()
        .probe(&req(&srv.url("/redirect/11/files/tiny.txt")))
        .await
        .unwrap_err();
    assert_eq!(err, NetError::TooManyRedirects { max: 10 });
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn hostile_redirect_loop() {
    let srv = TestServer::start().await.unwrap();
    let err = client()
        .probe(&req(&srv.url("/redirect-loop")))
        .await
        .unwrap_err();
    assert_eq!(err, NetError::RedirectLoop);
}

#[tokio::test]
async fn hostile_gzip_content_encoding_rejected() {
    let srv = TestServer::start().await.unwrap();
    let c = client();
    let err = c
        .fetch(&req(&srv.url("/files/small.bin?gzip=1")), None, None, None)
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::ContentEncoding { .. }), "{err:?}");
    assert!(!err.is_retryable());
    let err = c
        .probe(&req(&srv.url("/files/small.bin?gzip=1")))
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::ContentEncoding { .. }), "{err:?}");
}

#[tokio::test]
async fn hostile_endless_headers_bounded() {
    let srv = TestServer::start().await.unwrap();
    let err = client()
        .probe(&req(&srv.url("/files/small.bin?endless_headers=1")))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            NetError::HeaderTooLarge { .. } | NetError::TooManyHeaders { .. }
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn hostile_huge_header_bounded() {
    let srv = TestServer::start().await.unwrap();
    let err = client()
        .probe(&req(&srv.url("/files/small.bin?huge_header=1")))
        .await
        .unwrap_err();
    assert!(matches!(err, NetError::HeaderTooLarge { .. }), "{err:?}");
}

#[tokio::test]
async fn remote_change_detected_by_strong_etag() {
    let srv = TestServer::start().await.unwrap();
    let stale = EntityTag::parse("\"definitely-not-the-current-etag\"").unwrap();
    let err = client()
        .fetch(
            &req(&srv.url(MEDIUM)),
            Some(range(0, 100)),
            Some(4_194_304),
            Some(&stale),
        )
        .await
        .unwrap_err();
    assert_eq!(err, NetError::RemoteChanged);
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn http_status_errors() {
    let srv = TestServer::start().await.unwrap();
    let c = client();
    let e = c
        .probe(&req(&srv.url("/files/nope.bin")))
        .await
        .unwrap_err();
    assert_eq!(e, NetError::HttpStatus { status: 404 });
    assert!(!e.is_retryable());
    let e = c.probe(&req(&srv.url("/status/503"))).await.unwrap_err();
    assert_eq!(e, NetError::HttpStatus { status: 503 });
    assert!(e.is_retryable());
}

#[tokio::test]
async fn basic_auth_with_credentials() {
    let srv = TestServer::start().await.unwrap();
    let mut r = req(&srv.url("/auth/basic/files/tiny.txt"));
    r.credentials = Some((AUTH_USER.into(), Secret::new(AUTH_PASSWORD)));
    let p = client().probe(&r).await.unwrap();
    assert_eq!(p.size, Some(13));
}

#[tokio::test]
async fn basic_auth_from_url_userinfo() {
    let srv = TestServer::start().await.unwrap();
    let url = format!(
        "http://{AUTH_USER}:{AUTH_PASSWORD}@127.0.0.1:{}/auth/basic/files/tiny.txt",
        srv.port()
    );
    let p = client().probe(&req(&url)).await.unwrap();
    assert_eq!(p.size, Some(13));
    assert!(
        p.final_url.password().is_none(),
        "credentials must be stripped"
    );
    assert_eq!(p.final_url.username(), "");
}

#[tokio::test]
async fn digest_auth_with_credentials_fetches_data() {
    let srv = TestServer::start().await.unwrap();
    let fx = fixture("small.bin").unwrap();
    let mut r = req(&srv.url("/auth/digest/files/small.bin"));
    r.credentials = Some((AUTH_USER.into(), Secret::new(AUTH_PASSWORD)));
    let body = client().fetch(&r, None, Some(fx.len), None).await.unwrap();
    let (_, mut s) = unwrap_body(body);
    let (data, res) = read_all(&mut s).await;
    res.unwrap();
    assert_eq!(sha256_hex(&data), fx.sha256_hex());
}

#[tokio::test]
async fn auth_required_and_failed() {
    let srv = TestServer::start().await.unwrap();
    let c = client();
    for path in ["/auth/basic/files/tiny.txt", "/auth/digest/files/tiny.txt"] {
        let e = c.probe(&req(&srv.url(path))).await.unwrap_err();
        assert_eq!(e, NetError::AuthRequired, "{path}");
        let mut r = req(&srv.url(path));
        r.credentials = Some((AUTH_USER.into(), Secret::new("wrong-password")));
        let e = c.probe(&r).await.unwrap_err();
        assert_eq!(e, NetError::AuthFailed, "{path}");
        assert!(!e.is_retryable());
    }
}

#[tokio::test]
async fn fetch_page_returns_html_and_truncates() {
    let srv = TestServer::start().await.unwrap();
    let c = client();
    let (url, html) = c
        .fetch_page(&req(&srv.url("/site/index.html")), 1 << 20)
        .await
        .unwrap();
    assert_eq!(url.as_str(), srv.url("/site/index.html"));
    assert!(html.contains("href"), "{html}");
    let (_, cut) = c
        .fetch_page(&req(&srv.url("/site/index.html")), 10)
        .await
        .unwrap();
    assert!(cut.len() <= 10);
    let e = c
        .fetch_page(&req(&srv.url("/files/tiny.txt")), 1000)
        .await
        .unwrap_err();
    assert!(matches!(e, NetError::NotHtml { .. }), "{e:?}");
}

#[tokio::test]
async fn hostile_cookie_with_crlf_rejected() {
    let srv = TestServer::start().await.unwrap();
    let mut r = req(&srv.url("/files/tiny.txt"));
    r.cookie = Some(Secret::new("a=b\r\nX-Injected: 1"));
    let e = client().probe(&r).await.unwrap_err();
    assert_eq!(e, NetError::InvalidRequest);
}

#[tokio::test]
async fn hostile_empty_range_rejected() {
    let srv = TestServer::start().await.unwrap();
    let e = client()
        .fetch(&req(&srv.url(MEDIUM)), Some(range(5, 5)), None, None)
        .await
        .unwrap_err();
    assert_eq!(e, NetError::InvalidRequest);
}

#[tokio::test]
async fn connect_refused_is_retryable() {
    // Bind and drop to get a port that is (very likely) closed.
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let e = client()
        .probe(&req(&format!("http://127.0.0.1:{port}/x")))
        .await
        .unwrap_err();
    assert!(matches!(e, NetError::Connect { .. }), "{e:?}");
    assert!(e.is_retryable());
}

#[tokio::test]
async fn partial_and_full_variants_are_distinct() {
    let srv = TestServer::start().await.unwrap();
    let b = client()
        .fetch(
            &req(&srv.url("/files/tiny.txt")),
            Some(range(0, 5)),
            Some(13),
            None,
        )
        .await
        .unwrap();
    assert!(matches!(b, FetchBody::Partial(_)));
}

fn timeouts_ms(connect_ms: u64, tls_ms: u64) -> Client {
    Client::new(NetConfig {
        timeouts: Timeouts {
            connect_ms,
            tls_ms,
            first_byte_ms: 400,
            idle_ms: 300,
        },
        ..NetConfig::default()
    })
    .unwrap()
}

#[test]
fn hostile_default_timeouts_come_from_limits() {
    // CONNECT_TIMEOUT_MS, TLS_HANDSHAKE_TIMEOUT_MS, FIRST_BYTE_TIMEOUT_MS,
    // IDLE_READ_TIMEOUT_MS: every phase has a finite, non-zero default.
    let t = Timeouts::default();
    assert_eq!(t.connect_ms, udm_core::limits::CONNECT_TIMEOUT_MS);
    assert_eq!(t.tls_ms, udm_core::limits::TLS_HANDSHAKE_TIMEOUT_MS);
    assert_eq!(t.first_byte_ms, udm_core::limits::FIRST_BYTE_TIMEOUT_MS);
    assert_eq!(t.idle_ms, udm_core::limits::IDLE_READ_TIMEOUT_MS);
    for ms in [t.connect_ms, t.tls_ms, t.first_byte_ms, t.idle_ms] {
        assert!(ms > 0 && ms <= 120_000, "{ms}");
    }
}

#[tokio::test]
async fn hostile_tls_handshake_stall_times_out() {
    // A TCP peer that accepts and then never speaks TLS.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let hold = tokio::spawn(async move {
        let mut keep = Vec::new();
        while let Ok((s, _)) = listener.accept().await {
            keep.push(s);
        }
    });
    let started = std::time::Instant::now();
    let err = timeouts_ms(2_000, 300)
        .probe(&req(&format!("https://127.0.0.1:{port}/x.bin")))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            NetError::Timeout {
                phase: TimeoutPhase::Tls,
                ..
            }
        ),
        "{err:?}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    hold.abort();
}
