//! AGENTS §4.4.2: known secret fixtures (site password, proxy password, browser
//! cookie) must never appear in any file UDM writes (state, queue, history,
//! settings, logs) nor in query results.

mod common;

use std::path::Path;

use common::*;
use serde_json::json;
use udm_testsrv::{AUTH_PASSWORD, AUTH_USER, TestServer};

const PROXY_PASSWORD: &str = "proxy_secret_fixture_778";
const COOKIE: &str = "sid=cookie_secret_fixture_991";

fn scan(dir: &Path, needles: &[&str]) {
    let mut stack = vec![dir.to_path_buf()];
    let mut files = 0;
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            files += 1;
            let bytes = std::fs::read(&p).unwrap();
            let text = String::from_utf8_lossy(&bytes);
            for n in needles {
                assert!(
                    !text.contains(n),
                    "secret {n:?} leaked into {}",
                    p.display()
                );
            }
        }
    }
    assert!(files > 0, "nothing was written to {}", dir.display());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_secret_in_logs_or_state() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    let host = url::Url::parse(&srv.url("/")).unwrap();
    let host = host.host_str().unwrap().to_owned();
    let out = h.out().display().to_string();

    exec(
        &h.engine,
        "settings.set",
        json!({"name": "download_dir", "value": out}),
    )
    .await
    .unwrap();
    exec(
        &h.engine,
        "auth.set_site",
        json!({"host": host, "user": AUTH_USER, "password": AUTH_PASSWORD}),
    )
    .await
    .unwrap();
    exec(
        &h.engine,
        "proxy.set",
        json!({"kind": "none", "user": "proxy-user", "password": PROXY_PASSWORD}),
    )
    .await
    .unwrap();

    let authed = add(&h, &srv.url("/auth/basic/files/small.bin"), 2).await;
    let v = wait_status(&h.engine, authed, &["completed", "failed"]).await;
    assert_eq!(v.status, "completed", "{v:?}");

    let cap = exec(
        &h.engine,
        "capture.browser",
        json!({"url": srv.url("/files/medium.bin"), "cookie": COOKIE, "file_name": "medium.bin"}),
    )
    .await
    .unwrap();
    let cid = cap["download_id"].as_str().unwrap().parse().unwrap();
    let v = wait_status(&h.engine, cid, &["completed", "failed"]).await;
    assert_eq!(v.status, "completed", "{v:?}");

    let needles = [AUTH_PASSWORD, PROXY_PASSWORD, COOKIE];
    let views = format!(
        "{:?} {:?} {:?}",
        h.engine.downloads(),
        h.engine.settings(),
        h.engine.history(None)
    );
    for n in needles {
        assert!(
            !views.contains(n),
            "secret {n:?} leaked into a query result"
        );
    }
    // The secrets went to the keychain stand-in instead.
    let store = format!("{:?}", h.secrets);
    assert!(!store.contains("entries: 0"), "{store}");
    for n in needles {
        assert!(!store.contains(n));
    }

    h.engine.shutdown().await;
    scan(&h.data(), &needles);
}
