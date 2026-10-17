//! SOCKS5 and HTTP CONNECT proxies (local test proxies only).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::*;
use udm_core::Secret;
use udm_net::{Client, NetConfig, NetError, ProxyConfig};
use udm_testsrv::{HttpConnectProxy, Socks5Proxy, TestServer, fixture};

fn via(proxy: ProxyConfig) -> Client {
    Client::new(NetConfig {
        proxy: Some(proxy),
        ..cfg()
    })
    .unwrap()
}

fn creds() -> Option<(String, String)> {
    Some(("proxy-user".into(), "proxy-pass".into()))
}

async fn fetch_small(c: &Client, srv: &TestServer) -> Result<Vec<u8>, NetError> {
    let b = c
        .fetch(&req(&srv.url("/files/small.bin")), None, Some(65_536), None)
        .await?;
    let (_, mut s) = unwrap_body(b);
    let (data, res) = read_all(&mut s).await;
    res.map(|()| data)
}

#[tokio::test]
async fn socks5_without_auth() {
    let srv = TestServer::start().await.unwrap();
    let px = Socks5Proxy::start(None).await.unwrap();
    let c = via(ProxyConfig::Socks5 {
        host: "127.0.0.1".into(),
        port: px.port(),
        auth: None,
    });
    let data = fetch_small(&c, &srv).await.unwrap();
    assert_eq!(
        sha256_hex(&data),
        fixture("small.bin").unwrap().sha256_hex()
    );
    assert!(px.tunnel_count() >= 1);
}

#[tokio::test]
async fn socks5_with_auth_and_wrong_auth() {
    let srv = TestServer::start().await.unwrap();
    let px = Socks5Proxy::start(creds()).await.unwrap();
    let good = via(ProxyConfig::Socks5 {
        host: "127.0.0.1".into(),
        port: px.port(),
        auth: Some(("proxy-user".into(), Secret::new("proxy-pass"))),
    });
    fetch_small(&good, &srv).await.unwrap();
    assert!(px.tunnel_count() >= 1);
    for auth in [None, Some(("proxy-user".into(), Secret::new("nope")))] {
        let bad = via(ProxyConfig::Socks5 {
            host: "127.0.0.1".into(),
            port: px.port(),
            auth,
        });
        let e = fetch_small(&bad, &srv).await.unwrap_err();
        assert!(
            matches!(e, NetError::ProxyAuth | NetError::Proxy { .. }),
            "{e:?}"
        );
    }
}

#[tokio::test]
async fn http_connect_without_auth() {
    let srv = TestServer::start().await.unwrap();
    let px = HttpConnectProxy::start(None).await.unwrap();
    let c = via(ProxyConfig::Http {
        host: "127.0.0.1".into(),
        port: px.port(),
        auth: None,
    });
    let data = fetch_small(&c, &srv).await.unwrap();
    assert_eq!(data.len(), 65_536);
    assert!(px.tunnel_count() >= 1);
}

#[tokio::test]
async fn http_connect_with_auth_and_wrong_auth() {
    let srv = TestServer::start().await.unwrap();
    let px = HttpConnectProxy::start(creds()).await.unwrap();
    let good = via(ProxyConfig::Http {
        host: "127.0.0.1".into(),
        port: px.port(),
        auth: Some(("proxy-user".into(), Secret::new("proxy-pass"))),
    });
    fetch_small(&good, &srv).await.unwrap();
    assert!(px.tunnel_count() >= 1);
    let bad = via(ProxyConfig::Http {
        host: "127.0.0.1".into(),
        port: px.port(),
        auth: None,
    });
    let e = fetch_small(&bad, &srv).await.unwrap_err();
    assert_eq!(e, NetError::ProxyAuth);
    assert!(!e.is_retryable());
}

#[tokio::test]
async fn https_through_http_connect() {
    let srv = TestServer::start_tls().await.unwrap();
    let px = HttpConnectProxy::start(None).await.unwrap();
    let c = Client::new(NetConfig {
        proxy: Some(ProxyConfig::Http {
            host: "127.0.0.1".into(),
            port: px.port(),
            auth: None,
        }),
        extra_root_der: vec![srv.cert_der().unwrap()],
        ..cfg()
    })
    .unwrap();
    let p = c.probe(&req(&srv.url("/files/tiny.txt"))).await.unwrap();
    assert_eq!(p.size, Some(13));
    assert!(px.tunnel_count() >= 1);
}

#[tokio::test]
async fn proxy_unreachable_is_connect_error() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let srv = TestServer::start().await.unwrap();
    let c = via(ProxyConfig::Socks5 {
        host: "127.0.0.1".into(),
        port,
        auth: None,
    });
    let e = c
        .probe(&req(&srv.url("/files/tiny.txt")))
        .await
        .unwrap_err();
    assert!(matches!(e, NetError::Connect { .. }), "{e:?}");
    assert_eq!(srv.request_count(), 0);
}
