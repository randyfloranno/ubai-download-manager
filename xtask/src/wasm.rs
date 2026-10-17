//! `cargo xtask wasm`: wasm-safe crates must build for wasm32-unknown-unknown (AGENTS.md §5.11).

use crate::util;

pub const WASM_CRATES: &[&str] = &["udm-core", "udm-proto", "udm-i18n", "udm-ext"];

pub fn run() -> Result<(), String> {
    let mut cmd = util::cargo();
    cmd.args(["build", "--target", "wasm32-unknown-unknown"]);
    for c in WASM_CRATES {
        cmd.args(["-p", c]);
    }
    util::run_status(cmd)?;
    println!("wasm: OK");
    Ok(())
}
