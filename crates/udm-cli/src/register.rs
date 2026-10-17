//! Native messaging host registration (Chrome, Edge, Firefox).
//!
//! Manifests are written to `<data-dir>/native-host/`. On Windows the browsers
//! find them through `HKCU\Software\...\NativeMessagingHosts\<name>` (set with
//! `reg.exe`); on macOS and Linux the manifest is copied into each browser's
//! per-user `NativeMessagingHosts` folder.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use udm_core::{FIREFOX_EXTENSION_ID, Message, NATIVE_HOST_NAME};

/// Browser families with their own manifest flavour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavour {
    /// Chrome, Edge, Chromium: `allowed_origins`.
    Chromium,
    /// Firefox: `allowed_extensions`.
    Firefox,
}

/// A Chrome/Edge extension id: 32 letters `a`–`p`.
pub fn valid_extension_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

/// The host manifest for one flavour.
pub fn manifest_json(flavour: Flavour, host_path: &Path, extension_ids: &[String]) -> Value {
    let mut m = json!({
        "name": NATIVE_HOST_NAME,
        "description": "UDM (Ubai Download Manager)",
        "path": host_path.display().to_string(),
        "type": "stdio",
    });
    if let Some(o) = m.as_object_mut() {
        match flavour {
            Flavour::Chromium => {
                let origins: Vec<String> = extension_ids
                    .iter()
                    .map(|id| format!("chrome-extension://{id}/"))
                    .collect();
                o.insert("allowed_origins".into(), Value::from(origins));
            }
            Flavour::Firefox => {
                o.insert("allowed_extensions".into(), json!([FIREFOX_EXTENSION_ID]));
            }
        }
    }
    m
}

/// Default host binary: next to the running `udm-cli`.
pub fn default_host_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) {
        "udm-native-host.exe"
    } else {
        "udm-native-host"
    };
    Some(exe.parent()?.join(name))
}

fn write_err(path: &Path) -> Message {
    Message::new("error-cli-register-write").with_str("path", path.display().to_string())
}

fn write_manifest(path: &Path, v: &Value) -> Result<(), Message> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| write_err(path))?;
    }
    let text = serde_json::to_string_pretty(v).map_err(|_| write_err(path))?;
    udm_store::atomic_write(path, text.as_bytes()).map_err(|_| write_err(path))
}

/// Registry keys (Windows) that point browsers at a manifest.
#[cfg(windows)]
const REG_KEYS: &[(&str, Flavour)] = &[
    (
        r"HKCU\Software\Google\Chrome\NativeMessagingHosts",
        Flavour::Chromium,
    ),
    (
        r"HKCU\Software\Microsoft\Edge\NativeMessagingHosts",
        Flavour::Chromium,
    ),
    (
        r"HKCU\Software\Mozilla\NativeMessagingHosts",
        Flavour::Firefox,
    ),
];

#[cfg(windows)]
fn reg(args: &[&str]) -> Result<(), Message> {
    let st = std::process::Command::new("reg.exe")
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|_| Message::new("error-cli-register-reg"))?;
    if st.success() {
        Ok(())
    } else {
        Err(Message::new("error-cli-register-reg"))
    }
}

/// Per-user manifest folders (macOS, Linux).
#[cfg(not(windows))]
fn user_dirs() -> Vec<(PathBuf, Flavour)> {
    let Some(home) = std::env::home_dir().filter(|h| h.is_absolute()) else {
        return Vec::new();
    };
    if cfg!(target_os = "macos") {
        let s = home.join("Library/Application Support");
        vec![
            (
                s.join("Google/Chrome/NativeMessagingHosts"),
                Flavour::Chromium,
            ),
            (
                s.join("Microsoft Edge/NativeMessagingHosts"),
                Flavour::Chromium,
            ),
            (s.join("Chromium/NativeMessagingHosts"), Flavour::Chromium),
            (s.join("Mozilla/NativeMessagingHosts"), Flavour::Firefox),
        ]
    } else {
        vec![
            (
                home.join(".config/google-chrome/NativeMessagingHosts"),
                Flavour::Chromium,
            ),
            (
                home.join(".config/microsoft-edge/NativeMessagingHosts"),
                Flavour::Chromium,
            ),
            (
                home.join(".config/chromium/NativeMessagingHosts"),
                Flavour::Chromium,
            ),
            (
                home.join(".mozilla/native-messaging-hosts"),
                Flavour::Firefox,
            ),
        ]
    }
}

fn manifest_name(flavour: Flavour) -> String {
    match flavour {
        Flavour::Chromium => format!("{NATIVE_HOST_NAME}.json"),
        Flavour::Firefox => format!("{NATIVE_HOST_NAME}-firefox.json"),
    }
}

/// Writes both manifests into `dir` and registers them for the current user.
pub fn install(dir: &Path, host: &Path, extension_ids: &[String]) -> Result<Value, Message> {
    if let Some(bad) = extension_ids.iter().find(|id| !valid_extension_id(id)) {
        return Err(Message::new("error-cli-extension-id").with_str("id", bad.as_str()));
    }
    if !host.is_file() {
        return Err(
            Message::new("error-cli-host-missing").with_str("path", host.display().to_string())
        );
    }
    let mut written = Vec::new();
    for flavour in [Flavour::Chromium, Flavour::Firefox] {
        let p = dir.join(manifest_name(flavour));
        write_manifest(&p, &manifest_json(flavour, host, extension_ids))?;
        written.push(p.display().to_string());
    }
    #[cfg(windows)]
    for (key, flavour) in REG_KEYS {
        let manifest = dir.join(manifest_name(*flavour)).display().to_string();
        let full = format!(r"{key}\{NATIVE_HOST_NAME}");
        reg(&["add", &full, "/ve", "/t", "REG_SZ", "/d", &manifest, "/f"])?;
        written.push(full);
    }
    #[cfg(not(windows))]
    for (folder, flavour) in user_dirs() {
        let p = folder.join(format!("{NATIVE_HOST_NAME}.json"));
        write_manifest(&p, &manifest_json(flavour, host, extension_ids))?;
        written.push(p.display().to_string());
    }
    Ok(json!({ "registered": written }))
}

/// Removes the registration; missing entries are not an error.
pub fn uninstall(dir: &Path) -> Value {
    let mut removed = Vec::new();
    #[cfg(windows)]
    for (key, _) in REG_KEYS {
        let full = format!(r"{key}\{NATIVE_HOST_NAME}");
        if reg(&["delete", &full, "/f"]).is_ok() {
            removed.push(full);
        }
    }
    #[cfg(not(windows))]
    for (folder, _) in user_dirs() {
        let p = folder.join(format!("{NATIVE_HOST_NAME}.json"));
        if std::fs::remove_file(&p).is_ok() {
            removed.push(p.display().to_string());
        }
    }
    for flavour in [Flavour::Chromium, Flavour::Firefox] {
        let p = dir.join(manifest_name(flavour));
        if std::fs::remove_file(&p).is_ok() {
            removed.push(p.display().to_string());
        }
    }
    json!({ "removed": removed })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "abcdefghijklmnopabcdefghijklmnop";

    #[test]
    fn manifest_json_shape() {
        let host = Path::new("C:/UDM/udm-native-host.exe");
        let c = manifest_json(Flavour::Chromium, host, &[ID.to_owned()]);
        assert_eq!(c["name"], NATIVE_HOST_NAME);
        assert_eq!(c["type"], "stdio");
        assert_eq!(c["path"], "C:/UDM/udm-native-host.exe");
        assert_eq!(c["allowed_origins"][0], format!("chrome-extension://{ID}/"));
        assert!(c.get("allowed_extensions").is_none());
        let f = manifest_json(Flavour::Firefox, host, &[]);
        assert_eq!(f["allowed_extensions"][0], FIREFOX_EXTENSION_ID);
        assert!(f.get("allowed_origins").is_none());
        // Chrome requires lower-case letters, digits, dots and underscores.
        assert!(
            NATIVE_HOST_NAME
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_')
        );
    }

    #[test]
    fn hostile_extension_id_rejected() {
        assert!(valid_extension_id(ID));
        for bad in [
            "",
            "abc",
            "ABCDEFGHIJKLMNOPABCDEFGHIJKLMNOP",
            "qbcdefghijklmnopabcdefghijklmnop",
            "abcdefghijklmnopabcdefghijklmno/",
            "abcdefghijklmnopabcdefghijklmnopa",
            "abcdefghijklmnop\"bcdefghijklmnop",
        ] {
            assert!(!valid_extension_id(bad), "{bad}");
        }
        let dir = tempfile::tempdir().unwrap();
        let e = install(dir.path(), dir.path(), &["x\" /f".to_owned()]).unwrap_err();
        assert_eq!(e.key, "error-cli-extension-id");
        // Nothing was written or registered.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn missing_host_binary_refused() {
        let dir = tempfile::tempdir().unwrap();
        let e = install(dir.path(), &dir.path().join("nope.exe"), &[ID.to_owned()]).unwrap_err();
        assert_eq!(e.key, "error-cli-host-missing");
    }
}
