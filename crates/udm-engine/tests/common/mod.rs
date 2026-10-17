#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use udm_core::DownloadId;
use udm_engine::{Engine, EngineConfig, Platform, SystemPlatform, args_from_json};
use udm_secrets::MemoryStore;
use udm_store::DataDir;
use udm_sys::testing::{Fault, FaultyWriter};
use udm_sys::{FileWriter, PartFile, PowerAction, SysError};

/// Platform for tests: real files, no power actions, empty clipboard,
/// optional injected write fault.
#[derive(Default)]
pub struct TestPlatform {
    pub fail_after: Option<(u64, Fault)>,
    pub power_calls: AtomicU64,
    pub clipboard: std::sync::Mutex<Option<String>>,
}

impl Platform for TestPlatform {
    fn open_part(
        &self,
        path: &Path,
        size: Option<u64>,
        pre: bool,
    ) -> Result<Arc<dyn FileWriter>, SysError> {
        let f = PartFile::open(path, size, pre)?;
        Ok(match self.fail_after {
            Some((n, fault)) => Arc::new(FaultyWriter::new(f, n, fault)),
            None => Arc::new(f),
        })
    }
    fn ensure_space(&self, dir: &Path, remaining: u64) -> Result<(), SysError> {
        SystemPlatform.ensure_space(dir, remaining)
    }
    fn rename_no_overwrite(&self, from: &Path, to: &Path) -> Result<(), SysError> {
        SystemPlatform.rename_no_overwrite(from, to)
    }
    fn power(&self, _action: PowerAction) -> Result<(), SysError> {
        self.power_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn clipboard_text(&self) -> Result<Option<String>, SysError> {
        Ok(self.clipboard.lock().unwrap().clone())
    }
}

pub struct Harness {
    pub engine: Engine,
    pub dir: tempfile::TempDir,
    pub platform: Arc<TestPlatform>,
    pub secrets: Arc<MemoryStore>,
}

impl Harness {
    pub fn out(&self) -> PathBuf {
        self.dir.path().join("out")
    }
    pub fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }
}

pub fn config(
    data: &Path,
    platform: Arc<TestPlatform>,
    secrets: Arc<MemoryStore>,
    roots: Vec<Vec<u8>>,
) -> EngineConfig {
    EngineConfig {
        data_dir: DataDir::new(data),
        platform,
        secrets,
        extra_root_der: roots,
        timeouts: Some(udm_net::Timeouts {
            connect_ms: 2_000,
            tls_ms: 2_000,
            first_byte_ms: 3_000,
            idle_ms: 3_000,
        }),
    }
}

pub fn start_with(platform: TestPlatform, roots: Vec<Vec<u8>>) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let platform = Arc::new(platform);
    let secrets = Arc::new(MemoryStore::new());
    let (engine, notices) = Engine::start(config(
        &dir.path().join("data"),
        platform.clone(),
        secrets.clone(),
        roots,
    ))
    .unwrap();
    assert!(notices.is_empty(), "{notices:?}");
    Harness {
        engine,
        dir,
        platform,
        secrets,
    }
}

pub fn start() -> Harness {
    start_with(TestPlatform::default(), Vec::new())
}

pub async fn exec(e: &Engine, cmd: &str, args: Value) -> Result<Value, udm_core::Message> {
    let a = args_from_json(&args)?;
    e.execute(cmd, a).await.map_err(|x| x.to_message())
}

pub async fn add(h: &Harness, url: &str, connections: u32) -> DownloadId {
    let v = exec(
        &h.engine,
        "download.add",
        serde_json::json!({ "url": url, "dir": h.out().display().to_string(), "connections": connections }),
    )
    .await
    .unwrap();
    v["download_id"].as_str().unwrap().parse().unwrap()
}

/// Waits until the download reaches a status in `want` (or panics after 60 s).
pub async fn wait_status(e: &Engine, id: DownloadId, want: &[&str]) -> udm_engine::DownloadView {
    let t = Instant::now();
    loop {
        let v = e.download(id).expect("download exists");
        if want.contains(&v.status.as_str()) {
            return v;
        }
        assert!(
            t.elapsed() < Duration::from_secs(60),
            "timeout waiting for {want:?}, last {v:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub fn sha256_file(p: &Path) -> String {
    let bytes = std::fs::read(p).unwrap();
    let mut h = udm_proto::Hasher::new(udm_proto::ChecksumAlgo::Sha256);
    h.update(&bytes);
    udm_proto::to_hex(&h.finalize())
}

/// Downloads to completion and checks the golden SHA-256.
pub async fn golden(
    h: &Harness,
    url: &str,
    fixture: &str,
    connections: u32,
) -> udm_engine::DownloadView {
    let id = add(h, url, connections).await;
    let v = wait_status(&h.engine, id, &["completed", "failed"]).await;
    assert_eq!(v.status, "completed", "{v:?}");
    let path = Path::new(&v.dir).join(&v.file_name);
    assert_eq!(
        sha256_file(&path),
        udm_testsrv::fixture(fixture).unwrap().sha256_hex(),
        "{url}"
    );
    assert!(
        !Path::new(&v.dir)
            .join(format!("{}.udmpart", v.file_name))
            .exists()
    );
    v
}
