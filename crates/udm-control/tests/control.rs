#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use udm_control::{CaptureHandler, ControlClient, ControlServer, EngineHandler};
use udm_engine::{Engine, EngineConfig, SystemPlatform};
use udm_secrets::MemoryStore;
use udm_store::DataDir;

fn engine(dir: &std::path::Path) -> Engine {
    let cfg = EngineConfig {
        data_dir: DataDir::new(dir.join("data")),
        platform: Arc::new(SystemPlatform),
        secrets: Arc::new(MemoryStore::new()),
        extra_root_der: Vec::new(),
        timeouts: None,
    };
    Engine::start(cfg).unwrap().0
}

#[tokio::test(flavor = "multi_thread")]
async fn control_port_end_to_end() {
    let srv = udm_testsrv::TestServer::start().await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let e = engine(dir.path());
    let ctl = ControlServer::bind(0, Arc::new(EngineHandler::new(e.clone())))
        .await
        .unwrap();
    let mut c = ControlClient::connect(ctl.addr().port()).await.unwrap();

    assert_eq!(c.call("app.ping", Value::Null).await.unwrap()["pong"], true);
    let cmds = c.call("engine.list_commands", Value::Null).await.unwrap();
    assert!(cmds.as_array().unwrap().len() > 30);

    let out = dir.path().join("out");
    let text = format!(
        "add {} --dir '{}' --connections 4",
        srv.url("/files/big.bin"),
        out.display()
    );
    let r = c
        .call("cmdline.input", json!({ "text": text }))
        .await
        .unwrap();
    assert_eq!(r["command"], "download.add");
    let id = r["result"]["download_id"].as_str().unwrap().to_owned();

    let t = Instant::now();
    loop {
        let d = c
            .call(
                "engine.query",
                json!({ "what": "download", "download_id": id }),
            )
            .await
            .unwrap();
        if d["status"] == "completed" {
            break;
        }
        assert_ne!(d["status"], "failed", "{d}");
        assert!(t.elapsed() < Duration::from_secs(60));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let err = c
        .call(
            "engine.execute",
            json!({ "command": "download.remove", "args": { "download_id": id } }),
        )
        .await
        .unwrap_err();
    assert_eq!(err.key, "error-command-needs-confirmation");
    c.call(
        "engine.execute",
        json!({ "command": "download.remove", "args": { "download_id": id, "confirm": true } }),
    )
    .await
    .unwrap();
    assert_eq!(
        c.call("ui.screenshot", json!({})).await.unwrap_err().key,
        "error-control-no-ui"
    );
    assert_eq!(
        c.call("nope", Value::Null).await.unwrap_err().key,
        "error-control-unknown-method"
    );
    let h = c
        .call("engine.query", json!({ "what": "history" }))
        .await
        .unwrap();
    assert_eq!(h.as_array().unwrap().len(), 1);
    ctl.stop();
    e.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hostile_capture_endpoint_restricted() {
    let dir = tempfile::tempdir().unwrap();
    let e = engine(dir.path());
    let ctl = ControlServer::bind(0, Arc::new(CaptureHandler::new(e.clone(), "s3cret".into())))
        .await
        .unwrap();
    let mut c = ControlClient::connect(ctl.addr().port()).await.unwrap();
    assert_eq!(
        c.call("app.ping", json!({})).await.unwrap_err().key,
        "error-control-unauthorized"
    );
    assert_eq!(
        c.call("app.ping", json!({ "token": "s3cres" }))
            .await
            .unwrap_err()
            .key,
        "error-control-unauthorized"
    );
    c.call("app.ping", json!({ "token": "s3cret" }))
        .await
        .unwrap();
    let forbidden = [
        json!({ "token": "s3cret", "command": "download.remove_completed", "args": { "confirm": true } }),
        json!({ "token": "s3cret", "command": "settings.set", "args": { "name": "x", "value": "y" } }),
    ];
    for p in forbidden {
        assert_eq!(
            c.call("engine.execute", p).await.unwrap_err().key,
            "error-control-forbidden"
        );
    }
    assert_eq!(
        c.call(
            "engine.query",
            json!({ "token": "s3cret", "what": "settings" })
        )
        .await
        .unwrap_err()
        .key,
        "error-control-forbidden"
    );
    ctl.stop();
    e.shutdown().await;
}
