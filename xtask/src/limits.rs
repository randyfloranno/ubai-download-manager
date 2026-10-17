//! `cargo xtask limits`: every constant in `udm-core/src/limits.rs` must be exercised
//! by a file that contains `hostile_*` tests (AGENTS.md §3.4).

use crate::util;

/// Names of `pub const NAME` items in the source.
pub fn const_names(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in src.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("pub const ") else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            out.push(name);
        }
    }
    out
}

/// True if `name` occurs in `src` as a whole identifier.
pub fn contains_ident(src: &str, name: &str) -> bool {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut start = 0;
    while let Some(pos) = src.get(start..).and_then(|s| s.find(name)) {
        let abs = start + pos;
        let before = src.get(..abs).and_then(|s| s.chars().next_back());
        let after = src.get(abs + name.len()..).and_then(|s| s.chars().next());
        if !before.is_some_and(is_ident) && !after.is_some_and(is_ident) {
            return true;
        }
        start = abs + name.len();
    }
    false
}

pub fn run() -> Result<(), String> {
    let root = util::root();
    let limits_path = root.join("crates/udm-core/src/limits.rs");
    let src = util::read(&limits_path)?;
    let names = const_names(&src);
    let mut hostile_files = Vec::new();
    for f in util::walk_files(&root.join("crates"), Some("rs")) {
        if f == limits_path {
            continue;
        }
        let text = util::read(&f)?;
        if text.contains("fn hostile_") {
            hostile_files.push(text);
        }
    }
    let missing: Vec<String> = names
        .iter()
        .filter(|n| !hostile_files.iter().any(|t| contains_ident(t, n)))
        .cloned()
        .collect();
    println!(
        "limits: {} constants, {} files with hostile_ tests, {} covered",
        names.len(),
        hostile_files.len(),
        names.len() - missing.len()
    );
    if missing.is_empty() {
        println!("limits: OK");
        Ok(())
    } else {
        Err(format!(
            "{} limit(s) without a hostile_ test (no file containing `fn hostile_` mentions them):\n{}",
            missing.len(),
            util::bullet_list(&missing)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_consts() {
        let src = "/// doc\npub const MAX_A: u32 = 1;\nconst PRIVATE: u8 = 2;\n    pub const B_2: u64 = 3;\n";
        assert_eq!(const_names(src), vec!["MAX_A", "B_2"]);
    }

    #[test]
    fn ident_boundaries() {
        assert!(contains_ident("limits::MAX_A)", "MAX_A"));
        assert!(!contains_ident("MAX_AB", "MAX_A"));
        assert!(!contains_ident("XMAX_A", "MAX_A"));
        assert!(contains_ident("XMAX_A MAX_A", "MAX_A"));
    }
}
