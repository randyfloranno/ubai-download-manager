//! Shared helpers: repository root, cargo invocation, file walking.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Repository root (parent of the `xtask` crate).
pub fn root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().map(Path::to_path_buf).unwrap_or(manifest)
}

/// A `cargo` command (honours `$CARGO`) running in the repository root.
pub fn cargo() -> Command {
    let bin = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut cmd = Command::new(bin);
    cmd.current_dir(root());
    cmd
}

/// Human readable form of a command line.
pub fn display_cmd(cmd: &Command) -> String {
    let prog = Path::new(cmd.get_program())
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut s = prog;
    for a in cmd.get_args() {
        s.push(' ');
        s.push_str(&a.to_string_lossy());
    }
    s
}

/// Run a command with inherited stdio; error if it cannot start or exits non-zero.
pub fn run_status(mut cmd: Command) -> Result<(), String> {
    let line = display_cmd(&cmd);
    println!("> {line}");
    let status = cmd
        .status()
        .map_err(|e| format!("could not start `{line}`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{line}` failed ({status})"))
    }
}

/// Run a command and capture stdout; error on non-zero exit (stderr included).
pub fn run_capture(mut cmd: Command) -> Result<String, String> {
    let line = display_cmd(&cmd);
    let out = cmd
        .output()
        .map_err(|e| format!("could not start `{line}`: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "`{line}` failed ({}):\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("`{line}` printed non-UTF-8 output: {e}"))
}

/// Directory names never descended into.
const SKIP_DIRS: &[&str] = &["target", ".git", "node_modules", "dist", ".udm-dev"];

/// All files under `dir` (recursive, sorted) whose extension is `ext` (or any if `None`).
pub fn walk_files(dir: &Path, ext: Option<&str>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_dir() {
                let name = entry.file_name();
                if !SKIP_DIRS.iter().any(|s| name == *s) {
                    stack.push(path);
                }
            } else if ft.is_file() {
                let matches = match ext {
                    Some(e) => path.extension().is_some_and(|x| x.eq_ignore_ascii_case(e)),
                    None => true,
                };
                if matches {
                    out.push(path);
                }
            }
        }
    }
    out.sort();
    out
}

/// `path` relative to the repository root, with forward slashes.
pub fn rel(path: &Path) -> String {
    let r = root();
    let p = path.strip_prefix(&r).unwrap_or(path);
    p.to_string_lossy().replace('\\', "/")
}

/// Read a file as UTF-8 text with a readable error.
pub fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", rel(path)))
}

/// Format a list of problems for an error message.
pub fn bullet_list(items: &[String]) -> String {
    let mut s = String::new();
    for i in items {
        s.push_str("  - ");
        s.push_str(i);
        s.push('\n');
    }
    s
}
