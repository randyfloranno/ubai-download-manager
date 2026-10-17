//! `cargo xtask dist`: release binaries, browser extension, licences and docs
//! collected into `target/dist/udm-<version>-<os>-<arch>/`.
//!
//! The release build disables the `dev-sample` feature, so `--sample` and the
//! in-process test server are not shipped.

use std::path::{Path, PathBuf};

use crate::{ext, licences, util};

const BINARIES: &[&str] = &["udm", "udm-cli", "udm-native-host"];

fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|p| if p.is_absolute() { p } else { root.join(p) })
        .unwrap_or_else(|| root.join("target"))
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::copy(from, to)
        .map(|_| ())
        .map_err(|e| format!("cannot copy {}: {e}", from.display()))
}

pub fn run() -> Result<(), String> {
    let root = util::root();
    let version = ext::workspace_version()?;

    let mut build = util::cargo();
    build.args([
        "build",
        "--release",
        "--locked",
        "-p",
        "udm-app",
        "-p",
        "udm-cli",
        "-p",
        "udm-native-host",
        "--no-default-features",
    ]);
    util::run_status(build)?;
    ext::run()?;
    licences::run()?;

    let target = target_dir(&root);
    let name = format!(
        "udm-{version}-{}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let out = target.join("dist").join(&name);
    if out.exists() {
        std::fs::remove_dir_all(&out)
            .map_err(|e| format!("cannot clean {}: {e}", out.display()))?;
    }

    for bin in BINARIES {
        let file = exe(bin);
        copy(&target.join("release").join(&file), &out.join(&file))?;
    }
    for browser in ["chrome", "firefox"] {
        ext::copy_dir(
            &root.join("extension/dist").join(browser),
            &out.join("extension").join(browser),
        )?;
    }
    ext::copy_dir(&root.join("docs"), &out.join("docs"))?;
    for f in [
        "README.md",
        "ROADMAP.md",
        "ATTRIBUTION.md",
        "LICENSE-MIT",
        "LICENSE-APACHE",
    ] {
        let src = root.join(f);
        if src.is_file() {
            copy(&src, &out.join(f))?;
        }
    }
    copy(
        &root.join("target/licences/THIRD-PARTY.md"),
        &out.join("THIRD-PARTY.md"),
    )?;

    println!("dist: {}", out.display());
    println!(
        "next: `{} native-host install --extension-id <chrome-id>` registers the browser host",
        exe("udm-cli")
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exe_suffix_matches_platform() {
        assert_eq!(exe("udm").ends_with(".exe"), cfg!(windows));
    }
}
