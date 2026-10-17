//! `cargo xtask docs`: bilingual documentation check (AGENTS.md §5.5).

use std::collections::BTreeSet;
use std::path::Path;

use crate::util;

fn rel_set(dir: &Path) -> BTreeSet<String> {
    util::walk_files(dir, None)
        .iter()
        .filter_map(|p| p.strip_prefix(dir).ok())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect()
}

/// Pure comparison: problems for files present in one language only.
pub fn compare(en: &BTreeSet<String>, id: &BTreeSet<String>) -> Vec<String> {
    let mut v = Vec::new();
    for f in en.difference(id) {
        v.push(format!("docs/en/{f} has no docs/id/{f}"));
    }
    for f in id.difference(en) {
        v.push(format!("docs/id/{f} has no docs/en/{f}"));
    }
    v
}

pub fn run() -> Result<(), String> {
    let root = util::root();
    let mut problems = Vec::new();
    for d in ["docs/en", "docs/id"] {
        if !root.join(d).is_dir() {
            problems.push(format!("directory {d} is missing"));
        }
    }
    let en = rel_set(&root.join("docs/en"));
    let id = rel_set(&root.join("docs/id"));
    problems.extend(compare(&en, &id));
    if !root.join("ROADMAP.md").is_file() {
        problems.push("ROADMAP.md is missing at the repository root".to_string());
    }
    if problems.is_empty() {
        println!(
            "docs: OK ({} files per language, ROADMAP.md present)",
            en.len()
        );
        Ok(())
    } else {
        Err(format!(
            "{} documentation problem(s):\n{}",
            problems.len(),
            util::bullet_list(&problems)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_both_sides() {
        let en: BTreeSet<String> = ["a.md", "x/b.md"].iter().map(|s| s.to_string()).collect();
        let id: BTreeSet<String> = ["a.md", "c.md"].iter().map(|s| s.to_string()).collect();
        let v = compare(&en, &id);
        assert_eq!(v.len(), 2);
        assert!(v[0].contains("x/b.md"));
        assert!(v[1].contains("c.md"));
    }
}
