//! `cargo xtask glue`: WebExtension JS glue stays tiny (AGENTS.md §5.12).

use crate::util;

pub const MAX_GLUE_LINES: usize = 300;

pub fn run() -> Result<(), String> {
    let dir = util::root().join("extension/glue");
    let mut total = 0usize;
    for f in util::walk_files(&dir, Some("js")) {
        let n = util::read(&f)?.lines().count();
        println!("  {:>5}  {}", n, util::rel(&f));
        total += n;
    }
    if total <= MAX_GLUE_LINES {
        println!("glue: OK ({total} / {MAX_GLUE_LINES} lines)");
        Ok(())
    } else {
        Err(format!(
            "extension glue has {total} lines, limit is {MAX_GLUE_LINES} (move logic into Rust/wasm)"
        ))
    }
}
