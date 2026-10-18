//! Golden fault-injection tests (AGENTS.md §4.1.8): the SHA-256 of every
//! finished file must equal the fixture's SHA-256.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::Path;
use std::time::Duration;

use common::*;
use udm_testsrv::{TestServer, fixture_version};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_plain_multi_segment() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    let v = golden(&h, &srv.url("/files/big.bin"), "big.bin", 8).await;
    assert!(v.resumable);
    // History records the completion.
    assert_eq!(h.engine.history(None).len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn open_file_and_folder_go_through_the_platform() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    let v = golden(&h, &srv.url("/files/tiny.txt"), "tiny.txt", 1).await;
    let file = Path::new(&v.dir).join(&v.file_name);
    let id = serde_json::json!({ "download_id": v.id });
    exec(&h.engine, "download.open", id.clone()).await.unwrap();
    exec(&h.engine, "download.open_folder", id).await.unwrap();
    assert_eq!(
        *h.platform.opened.lock().unwrap(),
        vec![(file.clone(), false), (file.clone(), true)]
    );
    // A file deleted outside UDM is reported, not opened.
    std::fs::remove_file(&file).unwrap();
    let err = exec(
        &h.engine,
        "download.open",
        serde_json::json!({ "download_id": v.id }),
    )
    .await
    .unwrap_err();
    assert_eq!(err.key, "error-engine-file-missing");
    // The folder is still shown.
    exec(
        &h.engine,
        "download.open_folder",
        serde_json::json!({ "download_id": v.id }),
    )
    .await
    .unwrap();
    assert_eq!(h.platform.opened.lock().unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_tiny_and_single_connection() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    golden(&h, &srv.url("/files/tiny.txt"), "tiny.txt", 8).await;
    golden(&h, &srv.url("/files/medium.bin"), "medium.bin", 1).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_connection_drop_mid_segment() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    golden(
        &h,
        &srv.url("/files/big.bin?drop_after=1000000&drop_times=6"),
        "big.bin",
        4,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_range_ignored_falls_back_to_single() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    golden(
        &h,
        &srv.url("/files/medium.bin?ignore_range=1"),
        "medium.bin",
        8,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_no_range_support() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    let v = golden(
        &h,
        &srv.url("/files/medium.bin?no_range=1"),
        "medium.bin",
        8,
    )
    .await;
    assert!(!v.resumable);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_unknown_size() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    golden(
        &h,
        &srv.url("/files/medium.bin?no_length=1"),
        "medium.bin",
        8,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_wrong_content_range_never_delivers() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    let id = add(&h, &srv.url("/files/medium.bin?bad_content_range=1"), 4).await;
    let v = wait_status(&h.engine, id, &["completed", "failed"]).await;
    // Either the engine recovered on a single connection with a correct file
    // or it failed; it never delivers a corrupt file.
    if v.status == "completed" {
        let p = Path::new(&v.dir).join(&v.file_name);
        assert_eq!(
            sha256_file(&p),
            udm_testsrv::fixture("medium.bin").unwrap().sha256_hex()
        );
    } else {
        assert!(!Path::new(&v.dir).join(&v.file_name).exists() || v.file_name.is_empty());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_content_length_lies() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    for q in ["short_length=1", "long_length=1", "extra_bytes=5000"] {
        let id = add(&h, &srv.url(&format!("/files/medium.bin?{q}")), 4).await;
        let v = wait_status(&h.engine, id, &["completed", "failed"]).await;
        let p = Path::new(&v.dir).join(&v.file_name);
        if v.status == "completed" {
            assert_eq!(
                sha256_file(&p),
                udm_testsrv::fixture("medium.bin").unwrap().sha256_hex(),
                "{q}"
            );
        } else {
            assert!(
                !p.exists() || v.file_name.is_empty(),
                "{q}: corrupt file delivered"
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_slow_server() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    golden(&h, &srv.url("/files/small.bin?slow=65536"), "small.bin", 2).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_server_restart() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    let id = add(&h, &srv.url("/files/big.bin?slow=4000000"), 4).await;
    wait_status(&h.engine, id, &["downloading"]).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    srv.restart().await;
    let v = wait_status(&h.engine, id, &["completed", "failed"]).await;
    assert_eq!(v.status, "completed", "{v:?}");
    let p = Path::new(&v.dir).join(&v.file_name);
    assert_eq!(
        sha256_file(&p),
        udm_testsrv::fixture("big.bin").unwrap().sha256_hex()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_pause_resume() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    let id = add(&h, &srv.url("/files/big.bin?slow=4000000"), 4).await;
    wait_status(&h.engine, id, &["downloading"]).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    exec(
        &h.engine,
        "download.pause",
        serde_json::json!({ "download_id": id.to_string() }),
    )
    .await
    .unwrap();
    let v = wait_status(&h.engine, id, &["paused"]).await;
    assert!(v.downloaded > 0);
    assert!(
        Path::new(&v.dir)
            .join(format!("{}.udmpart", v.file_name))
            .exists()
    );
    exec(
        &h.engine,
        "download.resume",
        serde_json::json!({ "download_id": id.to_string() }),
    )
    .await
    .unwrap();
    let v = wait_status(&h.engine, id, &["completed", "failed"]).await;
    assert_eq!(v.status, "completed", "{v:?}");
    let p = Path::new(&v.dir).join(&v.file_name);
    assert_eq!(
        sha256_file(&p),
        udm_testsrv::fixture("big.bin").unwrap().sha256_hex()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_etag_change_between_attempts_restarts() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    let id = add(&h, &srv.url("/files/big.bin?slow=4000000"), 4).await;
    wait_status(&h.engine, id, &["downloading"]).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    exec(
        &h.engine,
        "download.pause",
        serde_json::json!({ "download_id": id.to_string() }),
    )
    .await
    .unwrap();
    wait_status(&h.engine, id, &["paused"]).await;
    srv.set_version("big.bin", 2);
    srv.reset_faults();
    exec(
        &h.engine,
        "download.resume",
        serde_json::json!({ "download_id": id.to_string() }),
    )
    .await
    .unwrap();
    let v = wait_status(&h.engine, id, &["completed", "failed"]).await;
    assert_eq!(v.status, "completed", "{v:?}");
    let p = Path::new(&v.dir).join(&v.file_name);
    assert_eq!(
        sha256_file(&p),
        fixture_version("big.bin", 2).unwrap().sha256_hex()
    );
    assert_eq!(
        v.message.as_ref().map(|m| m.key.as_str()),
        Some("download-restarted-remote-changed")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_app_restart_resumes() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    let id = add(&h, &srv.url("/files/big.bin?slow=4000000"), 4).await;
    wait_status(&h.engine, id, &["downloading"]).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    h.engine.shutdown().await;
    // A new engine on the same data dir picks the download up again.
    let (e2, notices) = udm_engine::Engine::start(config(
        &h.data(),
        h.platform.clone(),
        h.secrets.clone(),
        Vec::new(),
    ))
    .unwrap();
    assert!(notices.is_empty(), "{notices:?}");
    let v = wait_status(&e2, id, &["completed", "failed"]).await;
    assert_eq!(v.status, "completed", "{v:?}");
    let p = Path::new(&v.dir).join(&v.file_name);
    assert_eq!(
        sha256_file(&p),
        udm_testsrv::fixture("big.bin").unwrap().sha256_hex()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_http2_stream_reset() {
    let srv = TestServer::start_tls().await.unwrap();
    let h = start_with(TestPlatform::default(), vec![srv.cert_der().unwrap()]);
    golden(
        &h,
        &srv.url("/files/medium.bin?rst_after=500000&drop_times=3"),
        "medium.bin",
        4,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_checksum_verified() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    let sha = udm_testsrv::fixture("small.bin").unwrap().sha256_hex();
    let v = exec(
        &h.engine,
        "download.add",
        serde_json::json!({ "url": srv.url("/files/small.bin"), "dir": h.out().display().to_string(), "checksum": format!("sha256:{sha}") }),
    )
    .await
    .unwrap();
    let id: udm_core::DownloadId = v["download_id"].as_str().unwrap().parse().unwrap();
    let v = wait_status(&h.engine, id, &["completed", "failed"]).await;
    assert_eq!(v.status, "completed");
    let r = exec(
        &h.engine,
        "download.verify",
        serde_json::json!({ "download_id": id.to_string() }),
    )
    .await
    .unwrap();
    assert_eq!(r["matches"], serde_json::json!(true));

    // A wrong checksum fails and leaves no final file.
    let bad = "0".repeat(64);
    let v = exec(
        &h.engine,
        "download.add",
        serde_json::json!({ "url": srv.url("/files/tiny.txt"), "dir": h.out().display().to_string(), "checksum": format!("sha256:{bad}") }),
    )
    .await
    .unwrap();
    let id: udm_core::DownloadId = v["download_id"].as_str().unwrap().parse().unwrap();
    let v = wait_status(&h.engine, id, &["completed", "failed"]).await;
    assert_eq!(v.status, "failed");
    assert!(!h.out().join("tiny.txt").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_disk_full_pauses_with_state() {
    let srv = TestServer::start().await.unwrap();
    let h = start_with(
        TestPlatform {
            fail_after: Some((1_000_000, udm_sys::testing::Fault::DiskFull)),
            ..TestPlatform::default()
        },
        Vec::new(),
    );
    let id = add(&h, &srv.url("/files/medium.bin"), 2).await;
    let v = wait_status(&h.engine, id, &["paused", "failed", "completed"]).await;
    assert_eq!(v.status, "paused", "{v:?}");
    assert_eq!(
        v.message.as_ref().map(|m| m.key.as_str()),
        Some("error-disk-full")
    );
    assert!(
        Path::new(&v.dir)
            .join(format!("{}.udmpart", v.file_name))
            .exists()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_name_is_auto_renamed() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    golden(&h, &srv.url("/files/tiny.txt"), "tiny.txt", 1).await;
    let v = golden(&h, &srv.url("/files/tiny.txt"), "tiny.txt", 1).await;
    assert_eq!(v.file_name, "tiny (1).txt");
}

async fn wait_gone(e: &udm_engine::Engine, id: udm_core::DownloadId) {
    let t = std::time::Instant::now();
    while e.download(id).is_some() {
        assert!(
            t.elapsed() < Duration::from_secs(30),
            "download not removed"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remove_with_delete_file_removes_the_part_file() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    // Running.
    let run = add(&h, &srv.url("/files/big.bin?slow=2000000"), 4).await;
    let v = wait_status(&h.engine, run, &["downloading"]).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let run_part = Path::new(&v.dir).join(format!("{}.udmpart", v.file_name));
    assert!(run_part.exists());
    exec(
        &h.engine,
        "download.remove",
        serde_json::json!({ "download_id": run.to_string(), "delete_file": true, "confirm": true }),
    )
    .await
    .unwrap();
    wait_gone(&h.engine, run).await;
    assert!(!run_part.exists(), "running part file left behind");
    // Paused.
    let p = add(&h, &srv.url("/files/medium.bin?slow=500000"), 2).await;
    wait_status(&h.engine, p, &["downloading"]).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    exec(
        &h.engine,
        "download.pause",
        serde_json::json!({ "download_id": p.to_string() }),
    )
    .await
    .unwrap();
    let v = wait_status(&h.engine, p, &["paused"]).await;
    let p_part = Path::new(&v.dir).join(format!("{}.udmpart", v.file_name));
    assert!(p_part.exists());
    exec(
        &h.engine,
        "download.remove",
        serde_json::json!({ "download_id": p.to_string(), "delete_file": true, "confirm": true }),
    )
    .await
    .unwrap();
    wait_gone(&h.engine, p).await;
    assert!(!p_part.exists(), "paused part file left behind");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remove_right_after_add_leaves_no_part_file() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    for _ in 0..5 {
        let id = add(&h, &srv.url("/files/big.bin?slow=2000000"), 4).await;
        exec(&h.engine, "download.remove", serde_json::json!({ "download_id": id.to_string(), "delete_file": true, "confirm": true }))
            .await
            .unwrap();
        wait_gone(&h.engine, id).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let left: Vec<_> = std::fs::read_dir(h.out())
        .map(|r| r.filter_map(Result::ok).map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "left behind: {left:?}");
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            // The live engine renames `*.tmp` files away during atomic saves.
            match std::fs::copy(e.path(), target) {
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                r => {
                    r.unwrap();
                }
            }
        }
    }
}

/// App killed mid-download (no graceful shutdown): the data dir as it was on
/// disk at that moment must restart on the same part file, never on `name (1)`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn golden_crash_without_shutdown_resumes_same_part_file() {
    let srv = TestServer::start().await.unwrap();
    let h = start();
    exec(
        &h.engine,
        "settings.set",
        serde_json::json!({ "name": "download_dir", "value": h.out().display().to_string() }),
    )
    .await
    .unwrap();
    // No `dir`: the folder is decided by the task (category default).
    let v = exec(
        &h.engine,
        "download.add",
        serde_json::json!({ "url": srv.url("/files/big.bin?slow=4000000"), "connections": 4 }),
    )
    .await
    .unwrap();
    let id: udm_core::DownloadId = v["download_id"].as_str().unwrap().parse().unwrap();
    wait_status(&h.engine, id, &["downloading"]).await;
    // Longer than the autosave debounce and a checkpoint interval.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let crashed = h.dir.path().join("crashed");
    copy_dir(&h.data(), &crashed);
    h.engine.shutdown().await;

    let (e2, notices) = udm_engine::Engine::start(config(
        &crashed,
        h.platform.clone(),
        h.secrets.clone(),
        Vec::new(),
    ))
    .unwrap();
    assert!(notices.is_empty(), "{notices:?}");
    let before = e2.download(id).unwrap();
    assert_eq!(Path::new(&before.dir), h.out(), "{before:?}");
    let v = wait_status(&e2, id, &["completed", "failed"]).await;
    assert_eq!(v.status, "completed", "{v:?}");
    assert_eq!(v.file_name, "big.bin");
    let p = Path::new(&v.dir).join(&v.file_name);
    assert_eq!(
        sha256_file(&p),
        udm_testsrv::fixture("big.bin").unwrap().sha256_hex()
    );
    let left: Vec<_> = std::fs::read_dir(h.out())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name())
        .collect();
    assert_eq!(left, vec![std::ffi::OsString::from("big.bin")], "{left:?}");
    e2.shutdown().await;
}
