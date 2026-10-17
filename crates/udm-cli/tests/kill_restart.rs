//! Process-level golden test: `udm-cli` is killed mid-download (no shutdown,
//! no final checkpoint), restarted on the same data dir, and must finish the
//! same file with the fixture's SHA-256 and no leftover part files.

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use udm_testsrv::TestServer;

fn cli(data: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_udm-cli"));
    c.arg("--data-dir")
        .arg(data)
        .arg("--locale")
        .arg("en")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c
}

async fn wait_exit(child: &mut Child, limit: Duration) -> std::process::ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(st) = child.try_wait().unwrap() {
            return st;
        }
        assert!(start.elapsed() < limit, "udm-cli did not exit in time");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn sha256_hex(p: &Path) -> String {
    let bytes = std::fs::read(p).unwrap();
    Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn golden_app_kill_restart() {
    let srv = TestServer::start().await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();

    let mut first = cli(&data)
        .arg("add")
        .arg(srv.url("/files/big.bin?slow=2000000"))
        .arg("--dir")
        .arg(&out)
        .arg("--connections")
        .arg("4")
        .arg("--wait")
        .spawn()
        .unwrap();
    // Long enough for checkpoints and the queue autosave, short of completion.
    tokio::time::sleep(Duration::from_millis(3000)).await;
    assert!(
        first.try_wait().unwrap().is_none(),
        "download finished before the kill; slow the fixture down"
    );
    first.kill().unwrap();
    let _ = first.wait();

    let parts: Vec<_> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(parts, vec!["big.bin.udmpart".to_owned()], "{parts:?}");

    let mut second = cli(&data).arg("wait").arg("d-000001").spawn().unwrap();
    let st = wait_exit(&mut second, Duration::from_secs(120)).await;
    let o = second.wait_with_output().unwrap();
    assert!(
        st.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    let stdout = String::from_utf8_lossy(&o.stdout);
    assert!(stdout.contains("\"completed\""), "{stdout}");

    let left: Vec<_> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, vec!["big.bin".to_owned()], "{left:?}");
    assert_eq!(
        sha256_hex(&out.join("big.bin")),
        udm_testsrv::fixture("big.bin").unwrap().sha256_hex()
    );
}
