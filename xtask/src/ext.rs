//! `cargo xtask ext`: build the browser extension into `extension/dist/{chrome,firefox}`.
//!
//! Steps: release build of `udm-ext` for wasm32, `wasm-bindgen --target web`, copy
//! `extension/glue/*`, generate a Manifest V3 `manifest.json` per browser.

use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use crate::util;

pub const BROWSERS: &[&str] = &["chrome", "firefox"];

/// Generate the Manifest V3 for one browser.
pub fn manifest(browser: &str, version: &str) -> Value {
    let mut m = json!({
        "manifest_version": 3,
        "name": "UDM",
        "version": version,
        "description": "UDM (Ubai Download Manager) browser integration",
        "permissions": ["contextMenus", "cookies", "downloads", "nativeMessaging", "storage"],
        "host_permissions": ["<all_urls>"],
        "icons": icon_map(),
        "action": { "default_title": "UDM", "default_icon": icon_map() },
        "content_security_policy": {
            "extension_pages": "script-src 'self' 'wasm-unsafe-eval'; object-src 'self'"
        },
    });
    let background = if browser == "firefox" {
        json!({ "scripts": ["background.js"], "type": "module" })
    } else {
        json!({ "service_worker": "background.js", "type": "module" })
    };
    if let Some(obj) = m.as_object_mut() {
        obj.insert("background".into(), background);
        if browser == "firefox" {
            obj.insert(
                "browser_specific_settings".into(),
                json!({ "gecko": { "id": "udm@randyfloranno.github.io", "strict_min_version": "128.0" } }),
            );
        }
    }
    m
}

/// Icon edges shipped with the extension.
pub const ICON_SIZES: &[u16] = &[16, 32, 48, 128];

fn icon_map() -> Value {
    let mut m = serde_json::Map::new();
    for s in ICON_SIZES {
        m.insert(s.to_string(), json!(format!("icons/icon-{s}.png")));
    }
    Value::Object(m)
}

/// Render the UDM logo (`udm_core::logo`) as `icon-<size>.png` files in `dir`.
pub fn write_logo_pngs(dir: &Path, sizes: &[u16]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", util::rel(dir)))?;
    for &s in sizes {
        let px = udm_core::logo::rgba(s).ok_or_else(|| format!("cannot render logo at {s}px"))?;
        let path = dir.join(format!("icon-{s}.png"));
        let mut bytes = Vec::new();
        let mut enc = png::Encoder::new(&mut bytes, u32::from(s), u32::from(s));
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()
            .and_then(|mut w| w.write_image_data(&px))
            .map_err(|e| format!("cannot encode logo at {s}px: {e}"))?;
        std::fs::write(&path, bytes)
            .map_err(|e| format!("cannot write {}: {e}", util::rel(&path)))?;
    }
    Ok(())
}

pub fn workspace_version() -> Result<String, String> {
    let text = util::read(&util::root().join("Cargo.toml"))?;
    let v: toml::Value =
        toml::from_str(&text).map_err(|e| format!("cannot parse Cargo.toml: {e}"))?;
    v.get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .and_then(toml::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "Cargo.toml has no [workspace.package] version".to_string())
}

pub fn copy_dir(from: &Path, to: &Path) -> Result<usize, String> {
    let mut n = 0;
    for f in util::walk_files(from, None) {
        let Ok(rel) = f.strip_prefix(from) else {
            continue;
        };
        let dest = to.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        std::fs::copy(&f, &dest).map_err(|e| format!("cannot copy {}: {e}", util::rel(&f)))?;
        n += 1;
    }
    Ok(n)
}

pub fn run() -> Result<(), String> {
    let root = util::root();
    let glue = root.join("extension/glue");
    if !glue.is_dir() {
        return Err(
            "extension/glue/ is missing (the WebExtension glue must exist before packaging)".into(),
        );
    }
    let has_bindgen = Command::new("wasm-bindgen")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !has_bindgen {
        return Err("wasm-bindgen not found on PATH. Install the version pinned in ADR 0001:\n  cargo install wasm-bindgen-cli --version 0.2.129 --locked".into());
    }
    let version = workspace_version()?;

    let mut build = util::cargo();
    build.args([
        "build",
        "-p",
        "udm-ext",
        "--target",
        "wasm32-unknown-unknown",
        "--release",
    ]);
    util::run_status(build)?;

    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .map(|p| if p.is_absolute() { p } else { root.join(p) })
        .unwrap_or_else(|| root.join("target"));
    let wasm = target_dir.join("wasm32-unknown-unknown/release/udm_ext.wasm");
    if !wasm.is_file() {
        return Err(format!(
            "{} not found after build (udm-ext needs `[lib] crate-type = [\"cdylib\", \"rlib\"]`)",
            wasm.display()
        ));
    }

    for browser in BROWSERS {
        let out = root.join("extension/dist").join(browser);
        if out.exists() {
            std::fs::remove_dir_all(&out)
                .map_err(|e| format!("cannot clean {}: {e}", util::rel(&out)))?;
        }
        std::fs::create_dir_all(&out)
            .map_err(|e| format!("cannot create {}: {e}", util::rel(&out)))?;
        let mut bg = Command::new("wasm-bindgen");
        bg.current_dir(&root)
            .args(["--target", "web", "--no-typescript", "--out-dir"])
            .arg(&out)
            .arg(&wasm);
        util::run_status(bg)?;
        let copied = copy_dir(&glue, &out)?;
        write_logo_pngs(&out.join("icons"), ICON_SIZES)?;
        let text = serde_json::to_string_pretty(&manifest(browser, &version))
            .map_err(|e| e.to_string())?;
        std::fs::write(out.join("manifest.json"), format!("{text}\n"))
            .map_err(|e| format!("cannot write manifest for {browser}: {e}"))?;
        println!(
            "ext: {browser}: {} ({copied} glue files, version {version})",
            util::rel(&out)
        );
    }
    println!("ext: OK");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests() {
        let c = manifest("chrome", "0.1.0");
        assert_eq!(c["manifest_version"], 3);
        assert_eq!(c["background"]["service_worker"], "background.js");
        assert!(c.get("browser_specific_settings").is_none());
        let f = manifest("firefox", "0.1.0");
        assert_eq!(f["background"]["scripts"][0], "background.js");
        assert_eq!(f["background"]["type"], "module");
        assert_eq!(
            f["browser_specific_settings"]["gecko"]["id"],
            "udm@randyfloranno.github.io"
        );
        assert_eq!(f["permissions"].as_array().unwrap().len(), 5);
        assert!(
            c["content_security_policy"]["extension_pages"]
                .as_str()
                .unwrap()
                .contains("wasm-unsafe-eval")
        );
        assert_eq!(f["version"], "0.1.0");
    }
}
