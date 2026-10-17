//! UDM desktop application (`udm`): egui front-end over the engine.
//!
//! The UI only renders engine snapshots and dispatches commands; everything
//! else lives in the lower layers.
// Release builds are GUI-subsystem binaries: no console window behind the app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
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

mod bridge;
#[cfg(feature = "dev-sample")]
mod sample;
mod shot;
mod ui;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use udm_control::{CaptureHandler, ControlServer, EngineHandler, IpcInfo};
use udm_core::Message;
use udm_engine::{Engine, EngineConfig, SystemPlatform};
use udm_i18n::{Locale, Localizer};
use udm_secrets::{KeyringStore, SecretStore};
use udm_store::DataDir;

/// Command-line options.
#[derive(Debug, Parser)]
#[command(name = "udm", version, about = "UDM (Ubai Download Manager)")]
struct Args {
    /// Start an in-process test server and seed the list with sample downloads.
    #[arg(long)]
    sample: bool,
    /// Open the agent control port on 127.0.0.1:PORT.
    #[arg(long, value_name = "PORT")]
    control: Option<u16>,
    /// Data directory (settings, queue, state, history).
    #[arg(long, value_name = "DIR")]
    data_dir: Option<PathBuf>,
}

fn default_data_dir() -> PathBuf {
    udm_store::DataDir::os_default()
}

/// The control port is a development tool: release builds need an opt-in.
fn control_allowed() -> bool {
    cfg!(debug_assertions) || std::env::var("UDM_ALLOW_CONTROL").is_ok_and(|v| v == "1")
}

fn fail(loc: &Localizer, m: &Message) -> ExitCode {
    eprintln!("udm: {}", loc.message(m));
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .try_init();
    let args = Args::parse();
    let boot_loc = Localizer::new(Locale::Id);

    if args.control.is_some() && !control_allowed() {
        return fail(&boot_loc, &Message::new("error-app-control-refused"));
    }
    if args.sample && !cfg!(feature = "dev-sample") {
        return fail(&boot_loc, &Message::new("error-app-sample-unavailable"));
    }

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => return fail(&boot_loc, &Message::new("error-app-runtime")),
    };
    let data_dir = DataDir::new(args.data_dir.clone().unwrap_or_else(default_data_dir));
    let secrets: Arc<dyn SecretStore> = Arc::new(KeyringStore::new());
    let cfg = EngineConfig {
        data_dir: data_dir.clone(),
        platform: Arc::new(SystemPlatform),
        secrets,
        extra_root_der: Vec::new(),
        timeouts: None,
    };
    let started = {
        let _guard = rt.enter();
        Engine::start(cfg)
    };
    let (engine, startup_notices) = match started {
        Ok(v) => v,
        Err(e) => return fail(&boot_loc, &e.to_message()),
    };

    #[cfg(feature = "dev-sample")]
    let _sample = if args.sample {
        match rt.block_on(sample::seed(&engine, data_dir.root())) {
            Ok(s) => Some(s),
            Err(m) => return fail(&boot_loc, &m),
        }
    } else {
        None
    };

    let (bridge, ui_rx) = bridge::Bridge::new();
    let bridge = Arc::new(bridge);

    let control = match args.control {
        Some(port) => {
            let handler = EngineHandler::with_ui(engine.clone(), bridge.clone());
            match rt.block_on(ControlServer::bind(port, Arc::new(handler))) {
                Ok(h) => Some(h),
                Err(_) => {
                    let m =
                        Message::new("error-control-connect").with_uint("port", u64::from(port));
                    return fail(&boot_loc, &m);
                }
            }
        }
        None => None,
    };

    // Endpoint for the native messaging host, guarded by a per-run token.
    let capture = rt.block_on(async {
        let ipc = IpcInfo::generate(0).ok()?;
        let handler = CaptureHandler::new(engine.clone(), ipc.token.clone());
        let h = ControlServer::bind(0, Arc::new(handler)).await.ok()?;
        let ipc = IpcInfo {
            port: h.addr().port(),
            token: ipc.token,
        };
        if let Err(e) = ipc.save(&data_dir.ipc_file()) {
            tracing::warn!("cannot write ipc file: {e:?}");
        }
        Some(h)
    });

    let app = ui::UdmApp::new(
        engine.clone(),
        rt.handle().clone(),
        ui_rx,
        bridge.clone(),
        startup_notices,
    );
    let title = Localizer::new(app.locale()).tr("app-title");
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([1100.0, 680.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "udm",
        options,
        Box::new(move |cc| {
            bridge.set_context(cc.egui_ctx.clone());
            Ok(Box::new(app))
        }),
    );

    if let Some(c) = &control {
        c.stop();
    }
    if let Some(c) = &capture {
        c.stop();
        let _ = std::fs::remove_file(data_dir.ipc_file());
    }
    rt.block_on(engine.shutdown());
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("window error: {e}");
            fail(&boot_loc, &Message::new("error-app-window"))
        }
    }
}
