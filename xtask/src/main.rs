//! UDM build tooling. Run `cargo xtask` for help.
//!
//! Every subcommand works from PowerShell on Windows; no shell scripts.

mod ci;
mod ctl;
mod deny;
mod dist;
mod docs;
mod ext;
mod glue;
mod i18n;
mod layers;
mod licences;
mod limits;
mod metadata;
mod util;
mod wasm;

use std::process::ExitCode;

const HELP: &str = "\
UDM xtask — build tooling (run from the repository, works in PowerShell)

USAGE:
    cargo xtask <COMMAND> [ARGS]

COMMANDS:
    ci [--fast]        Run every quality gate in order, stop at the first failure.
                       --fast skips the separate hostile_/golden_ test reruns.
    layers             Check crate layering against xtask/layers.toml.
    i18n               Check Fluent catalogues (id/en) against keys used in code.
    docs               Check docs/en and docs/id have identical file sets; ROADMAP.md exists.
    limits             Check every limits.rs constant is used by a file with hostile_ tests.
    deny               Licence/ban/source check (cargo-deny if installed, else built-in).
    wasm               Build wasm-safe crates for wasm32-unknown-unknown.
    glue               Check extension/glue/**/*.js is at most 300 lines in total.
    ctl <PORT> <JSON>  Send one control request to 127.0.0.1:PORT, print the response.
    licences           Write target/licences/THIRD-PARTY.md for all third-party crates.
    ext                Build the browser extension into extension/dist/{chrome,firefox}.
    dist               Release build + extension + licences into target/dist/udm-<version>-<os>-<arch>.
    help               Show this text.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    };
    let rest = args.get(1..).unwrap_or(&[]);
    let result = match cmd.as_str() {
        "ci" => ci::run(rest),
        "layers" => layers::run(),
        "i18n" => i18n::run(),
        "docs" => docs::run(),
        "limits" => limits::run(),
        "deny" => deny::run(),
        "wasm" => wasm::run(),
        "glue" => glue::run(),
        "ctl" => ctl::run(rest),
        "licences" | "licenses" => licences::run(),
        "ext" => ext::run(),
        "dist" => dist::run(),
        "help" | "-h" | "--help" => {
            print!("{HELP}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{HELP}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("xtask {cmd}: FAILED\n{msg}");
            ExitCode::FAILURE
        }
    }
}
