//! UDM command-line client.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::unreachable,
    clippy::indexing_slicing,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects,
    rust_2018_idioms,
    unused_must_use
)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )
)]

mod out;
mod register;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use udm_control::protocol::{err_line, ok_line, parse_request};
use udm_control::{ControlClient, EngineHandler, Handler, IpcInfo, Request};
use udm_core::{Message, limits};
use udm_engine::{Engine, EngineConfig, StoredMessage, SystemPlatform};
use udm_i18n::{Locale, Localizer};
use udm_secrets::{KeyringStore, SecretStore};
use udm_store::DataDir;

/// Poll interval while waiting for a download.
const WAIT_POLL_MS: u64 = 250;

#[derive(Parser, Debug)]
#[command(name = "udm-cli", version, about = "UDM command-line client")]
struct Cli {
    /// Data dir (settings, queue, state). Default: the OS data dir.
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    /// Talk to a running UDM on this control port instead of a local engine.
    #[arg(long, global = true)]
    control: Option<u16>,
    /// Message language (`id` or `en`).
    #[arg(long, global = true)]
    locale: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Adds a download.
    Add {
        url: String,
        #[arg(long)]
        dir: Option<PathBuf>,
        #[arg(long)]
        connections: Option<u32>,
        #[arg(long)]
        file_name: Option<String>,
        #[arg(long)]
        queue: Option<String>,
        /// Waits until the download completes or fails.
        #[arg(long)]
        wait: bool,
    },
    /// Verifies a file against a checksum.
    Verify {
        file: PathBuf,
        #[arg(long, conflicts_with = "md5")]
        sha256: Option<String>,
        #[arg(long)]
        md5: Option<String>,
    },
    /// Runs a script: one control request (JSON) per line.
    Run {
        #[arg(long)]
        script: PathBuf,
    },
    /// Runs one command in the command-line language (`pause d-000001`).
    Exec { text: Vec<String> },
    /// Lists downloads.
    List,
    /// Waits for an existing download (e.g. one resumed after a restart).
    Wait { download_id: String },
    /// Registers or removes the browser native messaging host.
    NativeHost {
        #[command(subcommand)]
        action: HostCmd,
    },
}

#[derive(Subcommand, Debug)]
enum HostCmd {
    /// Writes the host manifests and registers them for the current user.
    Install {
        /// Chrome/Edge extension id (repeatable).
        #[arg(long = "extension-id")]
        extension_ids: Vec<String>,
        /// Path of `udm-native-host`. Default: next to `udm-cli`.
        #[arg(long)]
        host: Option<PathBuf>,
    },
    /// Removes the registration.
    Uninstall,
}

/// Where requests go.
enum Backend {
    Local {
        handler: EngineHandler,
        engine: Engine,
    },
    Remote(ControlClient),
}

impl Backend {
    async fn call(&mut self, method: &str, params: Value) -> Result<Value, Message> {
        match self {
            Self::Local { handler, .. } => handler.handle(Request::new(0, method, params)).await,
            Self::Remote(c) => c.call(method, params).await,
        }
    }

    /// One raw request line, answered with one response line.
    async fn line(&mut self, line: &str) -> String {
        match self {
            Self::Local { handler, .. } => match parse_request(line.as_bytes()) {
                Ok(req) => {
                    let id = req.id.clone();
                    match handler.handle(req).await {
                        Ok(v) => ok_line(&id, v),
                        Err(m) => err_line(&id, &m),
                    }
                }
                Err((id, m)) => err_line(&id, &m),
            },
            Self::Remote(c) => match c.raw(line).await {
                Ok(l) => l,
                Err(m) => err_line(&Value::Null, &m),
            },
        }
    }

    async fn close(self) {
        if let Self::Local { engine, .. } = self {
            engine.shutdown().await;
        }
    }
}

fn default_data_dir() -> PathBuf {
    udm_store::DataDir::os_default()
}

fn data_dir(cli: &Cli) -> PathBuf {
    let root = cli.data_dir.clone().unwrap_or_else(default_data_dir);
    std::path::absolute(&root).unwrap_or(root)
}

/// True when a UDM app is running on this data dir (its capture endpoint answers).
async fn app_running(dd: &DataDir) -> bool {
    let Ok(Some(info)) = IpcInfo::load(&dd.ipc_file()) else {
        return false;
    };
    match ControlClient::connect(info.port).await {
        Ok(mut c) => c
            .call("app.ping", json!({ "token": info.token }))
            .await
            .is_ok(),
        Err(_) => false,
    }
}

async fn backend(cli: &Cli) -> Result<Backend, Message> {
    if let Some(port) = cli.control {
        return ControlClient::connect(port).await.map(Backend::Remote);
    }
    let dd = DataDir::new(data_dir(cli));
    // Two engines on one data dir would overwrite each other's state.
    if app_running(&dd).await {
        return Err(Message::new("error-cli-app-running"));
    }
    let secrets: Arc<dyn SecretStore> = Arc::new(KeyringStore::new());
    let (engine, notices) = Engine::start(EngineConfig {
        data_dir: dd,
        platform: Arc::new(SystemPlatform),
        secrets,
        extra_root_der: Vec::new(),
        timeouts: None,
    })
    .map_err(|e| e.to_message())?;
    for n in &notices {
        out::notice(n);
    }
    Ok(Backend::Local {
        handler: EngineHandler::new(engine.clone()),
        engine,
    })
}

fn script_too_large() -> Message {
    Message::new("error-cli-script-too-large").with_uint("max", limits::MAX_SCRIPT_BYTES)
}

/// Reads a script, bounded by `limits::MAX_SCRIPT_BYTES`.
fn read_script(path: &Path) -> Result<String, Message> {
    let fail =
        || Message::new("error-cli-script-read").with_str("path", path.display().to_string());
    let meta = std::fs::metadata(path).map_err(|_| fail())?;
    if meta.len() > limits::MAX_SCRIPT_BYTES {
        return Err(script_too_large());
    }
    let bytes = std::fs::read(path).map_err(|_| fail())?;
    // The file may have grown between the two calls.
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limits::MAX_SCRIPT_BYTES {
        return Err(script_too_large());
    }
    String::from_utf8(bytes).map_err(|_| fail())
}

fn verify(file: &Path, sha256: Option<&str>, md5: Option<&str>) -> Result<Value, Message> {
    let raw = match (sha256, md5) {
        (Some(h), _) => format!("sha256:{h}"),
        (None, Some(h)) => format!("md5:{h}"),
        (None, None) => return Err(Message::new("error-cli-checksum-missing")),
    };
    let spec = udm_proto::ChecksumSpec::parse(&raw).map_err(|e| e.to_message())?;
    let digest = udm_engine::checksum::hash_file(file, spec.algo).map_err(|e| e.to_message())?;
    let name = file.display().to_string();
    if spec.matches(&digest) {
        Ok(json!({ "verified": true, "file": name }))
    } else {
        Err(Message::new("error-cli-checksum-mismatch").with_str("name", name))
    }
}

async fn wait(b: &mut Backend, id: &str) -> Result<Value, Message> {
    loop {
        let v = b
            .call(
                "engine.query",
                json!({ "what": "download", "download_id": id }),
            )
            .await?;
        match v.get("status").and_then(Value::as_str) {
            Some("completed") => return Ok(v),
            Some("failed") => {
                let m = v
                    .get("message")
                    .cloned()
                    .and_then(|m| serde_json::from_value::<StoredMessage>(m).ok())
                    .map_or_else(
                        || Message::new("error-cli-download-failed"),
                        |s| s.to_message(),
                    );
                return Err(m);
            }
            _ => tokio::time::sleep(Duration::from_millis(WAIT_POLL_MS)).await,
        }
    }
}

fn add_args(
    url: &str,
    dir: Option<&Path>,
    connections: Option<u32>,
    file_name: Option<&str>,
    queue: Option<&str>,
) -> Value {
    let mut o = serde_json::Map::new();
    o.insert("url".into(), Value::from(url));
    if let Some(d) = dir {
        o.insert("dir".into(), Value::from(d.display().to_string()));
    }
    if let Some(c) = connections {
        o.insert("connections".into(), Value::from(c));
    }
    if let Some(n) = file_name {
        o.insert("file_name".into(), Value::from(n));
    }
    if let Some(q) = queue {
        o.insert("queue".into(), Value::from(q));
    }
    Value::Object(o)
}

async fn dispatch(cli: &Cli, b: &mut Backend, script: Option<&str>) -> Result<Value, Message> {
    match &cli.cmd {
        Cmd::Add {
            url,
            dir,
            connections,
            file_name,
            queue,
            wait: w,
        } => {
            let args = add_args(
                url,
                dir.as_deref(),
                *connections,
                file_name.as_deref(),
                queue.as_deref(),
            );
            let v = b
                .call(
                    "engine.execute",
                    json!({ "command": "download.add", "args": args }),
                )
                .await?;
            if !*w {
                return Ok(v);
            }
            let id = v
                .get("download_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            wait(b, &id).await
        }
        Cmd::Exec { text } => {
            b.call("cmdline.input", json!({ "text": text.join(" ") }))
                .await
        }
        Cmd::List => b.call("engine.query", json!({ "what": "downloads" })).await,
        Cmd::Wait { download_id } => wait(b, download_id).await,
        Cmd::Run { .. } => {
            let mut failed = false;
            for line in script.unwrap_or_default().lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let resp = b.line(line).await;
                let ok = serde_json::from_str::<Value>(&resp)
                    .ok()
                    .and_then(|v| v.get("ok").and_then(Value::as_bool))
                    .unwrap_or(false);
                failed |= !ok;
                out::line(&resp);
            }
            if failed {
                Err(Message::new("error-cli-script-failed"))
            } else {
                Ok(Value::Null)
            }
        }
        Cmd::Verify { .. } | Cmd::NativeHost { .. } => Ok(Value::Null),
    }
}

async fn run(cli: &Cli) -> Result<(), Message> {
    let script = match &cli.cmd {
        Cmd::Verify { file, sha256, md5 } => {
            out::json(&verify(file, sha256.as_deref(), md5.as_deref())?);
            return Ok(());
        }
        Cmd::Run { script } => Some(read_script(script)?),
        Cmd::NativeHost { action } => {
            let dir = data_dir(cli).join("native-host");
            let v = match action {
                HostCmd::Install {
                    extension_ids,
                    host,
                } => {
                    let host = host
                        .clone()
                        .or_else(register::default_host_path)
                        .ok_or_else(|| {
                            Message::new("error-cli-host-missing").with_str("path", "")
                        })?;
                    register::install(&dir, &host, extension_ids)?
                }
                HostCmd::Uninstall => register::uninstall(&dir),
            };
            out::json(&v);
            return Ok(());
        }
        _ => None,
    };
    let mut b = backend(cli).await?;
    let r = dispatch(cli, &mut b, script.as_deref()).await;
    b.close().await;
    match r? {
        Value::Null => Ok(()),
        v => {
            out::json(&v);
            Ok(())
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let locale = cli
        .locale
        .as_deref()
        .and_then(Locale::parse)
        .unwrap_or_default();
    let loc = Localizer::new(locale);
    let Ok(rt) = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    else {
        out::error(&loc, &Message::new("error-app-runtime"));
        return ExitCode::FAILURE;
    };
    let guarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rt.block_on(run(&cli))));
    match guarded {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Ok(Err(m)) => {
            out::error(&loc, &m);
            ExitCode::FAILURE
        }
        Err(_) => {
            out::error(&loc, &Message::new("error-engine-internal"));
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_cli_script_too_large() {
        // MAX_SCRIPT_BYTES: oversized scripts are refused before reading.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        let f = std::fs::File::create(&p).unwrap();
        f.set_len(limits::MAX_SCRIPT_BYTES + 1).unwrap();
        drop(f);
        assert_eq!(
            read_script(&p).unwrap_err().key,
            "error-cli-script-too-large"
        );
        std::fs::write(&p, b"{}\n").unwrap();
        assert_eq!(read_script(&p).unwrap(), "{}\n");
        std::fs::write(&p, [0xff, 0xfe]).unwrap();
        assert_eq!(read_script(&p).unwrap_err().key, "error-cli-script-read");
    }

    #[test]
    fn verify_matches_and_mismatches() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("e.bin");
        std::fs::write(&p, b"").unwrap();
        let empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(verify(&p, Some(empty), None).unwrap()["verified"], true);
        let wrong = "0".repeat(64);
        assert_eq!(
            verify(&p, Some(&wrong), None).unwrap_err().key,
            "error-cli-checksum-mismatch"
        );
        assert!(verify(&p, None, None).is_err());
        assert!(verify(&p, Some("zz"), None).is_err());
    }

    #[test]
    fn cli_parses() {
        let c = Cli::try_parse_from([
            "udm-cli",
            "add",
            "http://h/f",
            "--connections",
            "8",
            "--wait",
        ])
        .unwrap();
        assert!(matches!(
            c.cmd,
            Cmd::Add {
                wait: true,
                connections: Some(8),
                ..
            }
        ));
        let c = Cli::try_parse_from(["udm-cli", "--control", "7878", "exec", "pause", "d-000001"])
            .unwrap();
        assert_eq!(c.control, Some(7878));
    }

    #[tokio::test]
    async fn local_backend_runs_script_lines() {
        let dir = tempfile::tempdir().unwrap();
        let cli = Cli::try_parse_from([
            "udm-cli",
            "--data-dir",
            dir.path().to_str().unwrap(),
            "list",
        ])
        .unwrap();
        let mut b = backend(&cli).await.unwrap();
        let ok = b
            .line(r#"{"id":7,"method":"engine.query","params":{"what":"downloads"}}"#)
            .await;
        assert!(ok.contains("\"ok\":true"), "{ok}");
        let bad = b.line("not json").await;
        assert!(bad.contains("\"ok\":false"), "{bad}");
        let listed = dispatch(&cli, &mut b, None).await.unwrap();
        assert!(listed.is_array() || listed.is_object());
        b.close().await;
    }
}
