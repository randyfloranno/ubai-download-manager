//! `--sample`: an in-process `udm-testsrv` and a list covering every state.
//! Never contacts the internet.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use udm_core::Message;
use udm_engine::{Engine, args_from_json};
use udm_testsrv::TestServer;

/// Keeps the test server alive for the life of the app.
pub struct Sample {
    _server: TestServer,
}

async fn exec(engine: &Engine, cmd: &str, args: Value) -> Result<Value, Message> {
    let args = args_from_json(&args)?;
    engine.execute(cmd, args).await.map_err(|e| e.to_message())
}

async fn add(engine: &Engine, url: String, extra: Value) -> Result<String, Message> {
    let mut args = json!({ "url": url });
    if let (Some(a), Some(e)) = (args.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            a.insert(k.clone(), v.clone());
        }
    }
    let r = exec(engine, "download.add", args).await?;
    r.get("download_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Message::new("error-engine-internal"))
}

async fn wait_for(engine: &Engine, id: &str, status: &[&str], max: Duration) {
    let t = Instant::now();
    while t.elapsed() < max {
        let done = engine
            .downloads()
            .iter()
            .any(|d| d.id == id && status.contains(&d.status.as_str()));
        if done {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Starts the server and seeds the list. Previous sample entries are removed
/// first, because they point at a server port that no longer exists.
pub async fn seed(engine: &Engine, root: &Path) -> Result<Sample, Message> {
    let srv = TestServer::start()
        .await
        .map_err(|_| Message::new("error-app-sample-server"))?;
    for d in engine.downloads() {
        exec(
            engine,
            "download.remove",
            json!({ "download_id": d.id, "delete_file": true, "confirm": true }),
        )
        .await?;
    }
    // Running entries stop asynchronously; wait so their names are free again.
    let t = Instant::now();
    while !engine.downloads().is_empty() && t.elapsed() < Duration::from_secs(10) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let out = root.join("downloads");
    exec(
        engine,
        "settings.set",
        json!({ "name": "download_dir", "value": out.display().to_string() }),
    )
    .await?;

    // Completed.
    let tiny = add(engine, srv.url("/files/tiny.txt"), json!({})).await?;
    let small = add(
        engine,
        srv.url("/files/small.bin"),
        json!({ "connections": 4 }),
    )
    .await?;
    let done = ["completed", "failed"];
    wait_for(engine, &tiny, &done, Duration::from_secs(10)).await;
    wait_for(engine, &small, &done, Duration::from_secs(10)).await;

    // Running, throttled so it stays visible.
    add(
        engine,
        srv.url("/files/huge.bin"),
        json!({ "connections": 8, "speed_limit": 393_216 }),
    )
    .await?;

    // Paused part-way.
    let big = add(
        engine,
        srv.url("/files/big.bin"),
        json!({ "connections": 4, "speed_limit": 2_097_152 }),
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    exec(engine, "download.pause", json!({ "download_id": big })).await?;

    // Restarted because the remote file changed between attempts.
    let medium = add(
        engine,
        srv.url("/files/medium.bin"),
        json!({ "connections": 2, "speed_limit": 32_768 }),
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    exec(engine, "download.pause", json!({ "download_id": medium })).await?;
    wait_for(engine, &medium, &["paused"], Duration::from_secs(5)).await;
    srv.set_version("medium.bin", 2);
    exec(engine, "download.resume", json!({ "download_id": medium })).await?;

    // Failed (404).
    add(engine, srv.url("/files/missing.bin"), json!({})).await?;

    // Dangerous double extension, waiting in the main queue.
    add(
        engine,
        srv.url("/files/tiny.txt"),
        json!({ "file_name": "invoice.pdf.exe", "start": false }),
    )
    .await?;

    // Scheduled queue (left over from a previous sample run: recreate it).
    if engine.queues().iter().any(|q| q.id == "night") {
        exec(
            engine,
            "queue.delete",
            json!({ "queue": "night", "confirm": true }),
        )
        .await?;
    }
    exec(
        engine,
        "queue.create",
        json!({ "queue": "night", "name": "night", "concurrency": 2 }),
    )
    .await?;
    add(
        engine,
        srv.url("/files/medium.bin"),
        json!({ "file_name": "nightly.bin", "queue": "night", "start": false }),
    )
    .await?;
    exec(
        engine,
        "schedule.set",
        json!({ "queue": "night", "start_at": "23:00", "stop_at": "05:00", "after_done": "sleep" }),
    )
    .await?;

    Ok(Sample { _server: srv })
}
