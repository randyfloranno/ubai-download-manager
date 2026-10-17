//! UDM native messaging host for browser extensions.
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

mod frame;
mod relay;

use std::io::{BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use udm_core::Message;
use udm_store::DataDir;

/// Data dir shared with the app: `UDM_DATA_DIR` or the OS default.
fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("UDM_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    udm_store::DataDir::os_default()
}

fn send<W: Write>(
    out: &mut W,
    r: Result<serde_json::Value, Message>,
) -> Result<(), frame::FrameError> {
    let body = serde_json::to_vec(&relay::reply(r))
        .map_err(|_| frame::FrameError::Io(std::io::ErrorKind::InvalidData))?;
    frame::write_frame(out, &body)
}

fn main() -> ExitCode {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return ExitCode::FAILURE;
    };
    let ipc_file = DataDir::new(data_dir()).ipc_file();
    let mut input = BufReader::new(std::io::stdin().lock());
    let mut out = BufWriter::new(std::io::stdout().lock());
    loop {
        let body = match frame::read_frame(&mut input) {
            Ok(Some(b)) => b,
            Ok(None) => return ExitCode::SUCCESS,
            Err(e) => {
                // The stream cannot be resynchronised: answer once and stop.
                let _ = send(&mut out, Err(Message::new(e.key())));
                return ExitCode::FAILURE;
            }
        };
        let guarded =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match relay::parse(&body) {
                Ok(msg) => rt.block_on(relay::forward(msg, &ipc_file)),
                Err(m) => Err(m),
            }));
        let r = guarded.unwrap_or_else(|_| Err(Message::new("error-native-internal")));
        if send(&mut out, r).is_err() {
            return ExitCode::FAILURE;
        }
    }
}
