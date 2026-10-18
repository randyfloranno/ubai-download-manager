//! The Downloads folder as the desktop configures it (Windows known folder,
//! XDG user dirs). Everything read here is treated as untrusted text.

use std::io::Read as _;
use std::path::{Component, Path, PathBuf};

use udm_core::limits;

/// The user's Downloads folder as the OS shell knows it, if it can be read,
/// is absolute and exists. Callers fall back to `<home>/Downloads`.
pub fn downloads_dir() -> Option<PathBuf> {
    os_downloads_dir().filter(|d| usable(d) && d.is_dir())
}

/// Absolute, bounded and free of `..`.
fn usable(dir: &Path) -> bool {
    dir.is_absolute()
        && dir.as_os_str().len() <= limits::MAX_PATH_BYTES
        && !dir.components().any(|c| c == Component::ParentDir)
}

/// Reads at most `limits::MAX_DESKTOP_CONFIG_BYTES` of UTF-8; longer or
/// non-UTF-8 input is refused, not truncated.
fn bounded_text(r: impl std::io::Read) -> Option<String> {
    let mut buf = Vec::new();
    r.take(limits::MAX_DESKTOP_CONFIG_BYTES.saturating_add(1))
        .read_to_end(&mut buf)
        .ok()?;
    if u64::try_from(buf.len()).ok()? > limits::MAX_DESKTOP_CONFIG_BYTES {
        return None;
    }
    String::from_utf8(buf).ok()
}

#[cfg(windows)]
const SHELL_FOLDERS_KEY: &str =
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders";
/// `FOLDERID_Downloads`.
#[cfg(windows)]
const DOWNLOADS_GUID: &str = "{374DE290-123F-4565-9164-39C4925E467B}";

#[cfg(windows)]
fn os_downloads_dir() -> Option<PathBuf> {
    use std::os::windows::process::CommandExt as _;
    /// `CREATE_NO_WINDOW`: no console flashes up from the GUI.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("reg.exe")
        .args(["query", SHELL_FOLDERS_KEY, "/v", DOWNLOADS_GUID])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    // Non-ASCII paths come out in the OEM code page and fail UTF-8 decoding;
    // the caller then falls back to `<home>\Downloads`.
    let text = bounded_text(out.stdout.as_slice())?;
    let raw = parse_reg_value(&text, DOWNLOADS_GUID)?;
    expand_env(&raw, |k| std::env::var(k).ok()).map(PathBuf::from)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn os_downloads_dir() -> Option<PathBuf> {
    let home = std::env::home_dir().filter(|h| h.is_absolute())?;
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|c| c.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let file = std::fs::File::open(config.join("user-dirs.dirs")).ok()?;
    parse_user_dirs(&bounded_text(file)?, &home)
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
fn os_downloads_dir() -> Option<PathBuf> {
    None
}

/// The data of value `name` in `reg query` output
/// (`    <name>    REG_EXPAND_SZ    <data>`).
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_reg_value(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix(name)?;
        let (kind, data) = rest.trim_start().split_once(char::is_whitespace)?;
        let data = data.trim();
        (matches!(kind, "REG_SZ" | "REG_EXPAND_SZ") && !data.is_empty()).then(|| data.to_owned())
    })
}

/// Expands `%NAME%` references; an unknown or unterminated one gives `None`.
#[cfg_attr(not(windows), allow(dead_code))]
fn expand_env(raw: &str, lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
    let mut out = String::new();
    let mut rest = raw;
    while let Some((before, after)) = rest.split_once('%') {
        let (name, tail) = after.split_once('%')?;
        out.push_str(before);
        out.push_str(&lookup(name)?);
        if out.len() > limits::MAX_PATH_BYTES {
            return None;
        }
        rest = tail;
    }
    out.push_str(rest);
    Some(out)
}

/// `XDG_DOWNLOAD_DIR` from `user-dirs.dirs`: `"$HOME/<path>"` or `"/<path>"`.
/// Set to `$HOME` itself the folder is disabled, so `None`.
#[cfg_attr(any(windows, target_os = "macos"), allow(dead_code))]
fn parse_user_dirs(text: &str, home: &Path) -> Option<PathBuf> {
    let value = text.lines().rev().find_map(|line| {
        let v = line.trim().strip_prefix("XDG_DOWNLOAD_DIR=")?;
        v.strip_prefix('"')?.strip_suffix('"')
    })?;
    let plain = |s: &str| !s.is_empty() && !s.contains(['\\', '`', '$', '"']);
    let dir = match value.strip_prefix("$HOME/") {
        Some(rel) if plain(rel) => home.join(rel),
        None if value.starts_with('/') && plain(value) => PathBuf::from(value),
        _ => return None,
    };
    usable(&dir).then_some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUID: &str = "{374DE290-123F-4565-9164-39C4925E467B}";
    const REG: &str = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\User Shell Folders\r\n    {374DE290-123F-4565-9164-39C4925E467B}    REG_EXPAND_SZ    %USERPROFILE%\\My Downloads\r\n\r\n";

    fn env(k: &str) -> Option<String> {
        (k == "USERPROFILE").then(|| r"C:\Users\ana".to_owned())
    }

    #[test]
    fn reg_query_output_is_parsed_and_expanded() {
        let raw = parse_reg_value(REG, GUID).unwrap();
        assert_eq!(raw, r"%USERPROFILE%\My Downloads");
        assert_eq!(expand_env(&raw, env).unwrap(), r"C:\Users\ana\My Downloads");
        assert_eq!(
            parse_reg_value("    {x}    REG_SZ    D:\\Unduhan\r\n", "{x}").unwrap(),
            r"D:\Unduhan"
        );
    }

    #[test]
    fn hostile_reg_and_env_values_are_refused() {
        assert_eq!(parse_reg_value("    {x}    REG_DWORD    0x1", "{x}"), None);
        assert_eq!(parse_reg_value("    {x}    REG_SZ    ", "{x}"), None);
        assert_eq!(parse_reg_value("ERROR: not found", "{x}"), None);
        assert_eq!(expand_env("%NOPE%\\x", env), None);
        assert_eq!(expand_env("%USERPROFILE", env), None);
        // Expansion cannot grow past the path limit.
        let big = |_: &str| Some("a".repeat(limits::MAX_PATH_BYTES));
        assert_eq!(expand_env("%A%%A%", big), None);
        // Oversized or non-UTF-8 output is refused, not truncated.
        let long = vec![b'a'; usize::try_from(limits::MAX_DESKTOP_CONFIG_BYTES).unwrap() + 1];
        assert_eq!(bounded_text(long.as_slice()), None);
        assert_eq!(bounded_text(&[0xff, 0xfe][..]), None);
    }

    #[cfg(unix)]
    #[test]
    fn user_dirs_are_parsed() {
        let home = Path::new("/home/ana");
        let text =
            "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOWNLOAD_DIR=\"$HOME/Unduhan\"\n";
        assert_eq!(parse_user_dirs(text, home), Some(home.join("Unduhan")));
    }

    #[cfg(unix)]
    #[test]
    fn user_dirs_absolute_value() {
        let abs = "XDG_DOWNLOAD_DIR=\"/data/dl\"\n";
        assert_eq!(
            parse_user_dirs(abs, Path::new("/home/ana")),
            Some(PathBuf::from("/data/dl"))
        );
    }

    #[test]
    fn hostile_user_dirs_values_are_refused() {
        let home = Path::new("/home/ana");
        for v in [
            "XDG_DOWNLOAD_DIR=\"$HOME\"",
            "XDG_DOWNLOAD_DIR=\"$HOME/\"",
            "XDG_DOWNLOAD_DIR=\"$HOME/../../etc\"",
            "XDG_DOWNLOAD_DIR=\"relative/dl\"",
            "XDG_DOWNLOAD_DIR=\"$OTHER/dl\"",
            "XDG_DOWNLOAD_DIR=\"$HOME/$(rm)\"",
            "XDG_DOWNLOAD_DIR=$HOME/unquoted",
            "",
        ] {
            assert_eq!(parse_user_dirs(v, home), None, "{v}");
        }
    }

    #[test]
    fn downloads_dir_is_absolute_when_found() {
        assert!(downloads_dir().is_none_or(|d| d.is_absolute() && d.is_dir()));
    }
}
