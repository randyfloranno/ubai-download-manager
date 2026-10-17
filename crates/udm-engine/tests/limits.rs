//! `hostile_*` tests for engine-level limits from `udm_core::limits`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use common::*;
use udm_core::limits;
use udm_engine::grabber::{GrabOptions, crawl};
use udm_net::{Client, NetConfig};
use udm_testsrv::TestServer;

fn grab(srv: &TestServer, path: &str, depth: u32) -> GrabOptions {
    GrabOptions {
        url: url::Url::parse(&srv.url(path)).unwrap(),
        depth,
        extensions: Vec::new(),
        same_host: true,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hostile_grabber_depth_is_capped() {
    // MAX_GRABBER_DEPTH: an endless binary tree of pages, depth asked = 999.
    let srv = TestServer::start().await.unwrap();
    let client = Arc::new(Client::new(NetConfig::default()).unwrap());
    let r = crawl(client, grab(&srv, "/maze/0.html?fan=2", 999)).await;
    assert!(r.error.is_none(), "{:?}", r.error);
    let expected = (1u64 << (limits::MAX_GRABBER_DEPTH + 1)) - 1;
    assert_eq!(r.pages, expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hostile_grabber_page_count_is_capped() {
    // MAX_GRABBER_PAGES: a wide endless site stops at the page cap.
    let srv = TestServer::start().await.unwrap();
    let client = Arc::new(Client::new(NetConfig::default()).unwrap());
    let r = crawl(
        client,
        grab(&srv, "/maze/0.html?fan=20", limits::MAX_GRABBER_DEPTH),
    )
    .await;
    assert_eq!(r.pages, u64::try_from(limits::MAX_GRABBER_PAGES).unwrap());
    assert!(r.files.len() <= limits::MAX_BATCH_SIZE);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hostile_too_many_queues_refused() {
    // MAX_QUEUES: the main queue counts.
    let h = start();
    for i in 1..limits::MAX_QUEUES {
        exec(
            &h.engine,
            "queue.create",
            serde_json::json!({ "queue": format!("q{i}") }),
        )
        .await
        .unwrap();
    }
    let err = exec(
        &h.engine,
        "queue.create",
        serde_json::json!({ "queue": "one-more" }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.key, "error-engine-too-many-queues");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hostile_queue_size_is_bounded() {
    // MAX_QUEUE_SIZE: adding beyond the bound fails with a typed error.
    let h = start();
    let url = "http://127.0.0.1:9/files/tiny.txt";
    for _ in 0..limits::MAX_QUEUE_SIZE {
        exec(
            &h.engine,
            "download.add",
            serde_json::json!({ "url": url, "start": false }),
        )
        .await
        .unwrap();
    }
    let err = exec(
        &h.engine,
        "download.add",
        serde_json::json!({ "url": url, "start": false }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.key, "error-engine-queue-full");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hostile_autosave_is_debounced_but_prompt() {
    // AUTOSAVE_DEBOUNCE_MS: a change is not written on every call, but lands
    // on disk well within 2 s (AGENTS.md §3.8).
    let h = start();
    let queue_file = udm_store::DataDir::new(h.data()).queue_file();
    let before = std::fs::read_to_string(&queue_file).unwrap_or_default();
    let v = exec(
        &h.engine,
        "download.add",
        serde_json::json!({ "url": "http://127.0.0.1:9/files/tiny.txt", "start": false }),
    )
    .await
    .unwrap();
    let id = v["download_id"].as_str().unwrap().to_owned();
    let now = std::fs::read_to_string(&queue_file).unwrap_or_default();
    assert_eq!(now, before, "saved synchronously, not debounced");
    let t = Instant::now();
    loop {
        let text = std::fs::read_to_string(&queue_file).unwrap_or_default();
        if text.contains(&id) {
            break;
        }
        assert!(
            t.elapsed() < Duration::from_millis(2_000),
            "autosave too slow"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
