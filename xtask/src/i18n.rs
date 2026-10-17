//! `cargo xtask i18n`: Fluent catalogue consistency (ADR 0002, AGENTS.md §5.4).
//!
//! Checks:
//! 1. both catalogues parse without errors;
//! 2. every message/term (and every attribute) exists in both locales;
//! 3. every key-like string literal in `crates/**/*.rs` whose first segment is a
//!    known catalogue prefix is a catalogue key;
//! 4. every catalogue message is referenced (code literal or FTL message reference),
//!    unless allowed by a directive comment in a catalogue.
//!
//! Directive comments (any number of leading `#`), space separated:
//! - `## dynamic-prefixes: category- power- cmd-` keys starting with these may be built at runtime;
//! - `## keep: some-key other-key`                 exact keys kept although unreferenced;
//! - `## not-keys: content-length udm-core`        literals in code that look like keys but are not.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use fluent_syntax::ast;

use crate::util;

/// Parsed catalogue: message id -> attribute names; term ids; message ids referenced inside FTL.
#[derive(Debug, Default)]
pub struct Catalogue {
    pub messages: BTreeMap<String, BTreeSet<String>>,
    pub terms: BTreeMap<String, BTreeSet<String>>,
    pub ftl_refs: BTreeSet<String>,
    pub dynamic_prefixes: Vec<String>,
    pub keep: BTreeSet<String>,
    pub not_keys: BTreeSet<String>,
}

pub fn parse_catalogue(name: &str, src: &str) -> Result<Catalogue, String> {
    let res = match fluent_syntax::parser::parse(src) {
        Ok(r) => r,
        Err((_, errs)) => {
            let list: Vec<String> = errs
                .iter()
                .map(|e| {
                    let line = src
                        .get(..e.pos.start)
                        .map_or(0, |s| s.matches('\n').count())
                        + 1;
                    format!("{name}:{line}: {:?}", e.kind)
                })
                .collect();
            return Err(format!(
                "Fluent parse errors:\n{}",
                util::bullet_list(&list)
            ));
        }
    };
    let mut cat = Catalogue::default();
    for entry in &res.body {
        match entry {
            ast::Entry::Message(m) => {
                let attrs = m.attributes.iter().map(|a| a.id.name.to_string()).collect();
                if cat.messages.insert(m.id.name.to_string(), attrs).is_some() {
                    return Err(format!("{name}: duplicate message `{}`", m.id.name));
                }
                if let Some(p) = &m.value {
                    collect_pattern(p, &mut cat.ftl_refs);
                }
                for a in &m.attributes {
                    collect_pattern(&a.value, &mut cat.ftl_refs);
                }
            }
            ast::Entry::Term(t) => {
                let attrs = t.attributes.iter().map(|a| a.id.name.to_string()).collect();
                if cat.terms.insert(format!("-{}", t.id.name), attrs).is_some() {
                    return Err(format!("{name}: duplicate term `-{}`", t.id.name));
                }
                collect_pattern(&t.value, &mut cat.ftl_refs);
                for a in &t.attributes {
                    collect_pattern(&a.value, &mut cat.ftl_refs);
                }
            }
            ast::Entry::Junk { content } => {
                return Err(format!("{name}: unparsable content: {content}"));
            }
            _ => {}
        }
    }
    for line in src.lines() {
        let t = line.trim_start();
        if !t.starts_with('#') {
            continue;
        }
        let body = t.trim_start_matches('#').trim();
        let take = |tag: &str| -> Option<Vec<String>> {
            body.strip_prefix(tag)
                .map(|rest| rest.split_whitespace().map(str::to_string).collect())
        };
        if let Some(v) = take("dynamic-prefixes:") {
            cat.dynamic_prefixes.extend(v);
        } else if let Some(v) = take("keep:") {
            cat.keep.extend(v);
        } else if let Some(v) = take("not-keys:") {
            cat.not_keys.extend(v);
        }
    }
    Ok(cat)
}

fn collect_pattern(p: &ast::Pattern<&str>, out: &mut BTreeSet<String>) {
    for el in &p.elements {
        if let ast::PatternElement::Placeable { expression } = el {
            collect_expr(expression, out);
        }
    }
}

fn collect_expr(e: &ast::Expression<&str>, out: &mut BTreeSet<String>) {
    match e {
        ast::Expression::Select { selector, variants } => {
            collect_inline(selector, out);
            for v in variants {
                collect_pattern(&v.value, out);
            }
        }
        ast::Expression::Inline(i) => collect_inline(i, out),
    }
}

fn collect_inline(i: &ast::InlineExpression<&str>, out: &mut BTreeSet<String>) {
    match i {
        ast::InlineExpression::MessageReference { id, .. } => {
            out.insert(id.name.to_string());
        }
        ast::InlineExpression::TermReference { id, arguments, .. } => {
            out.insert(format!("-{}", id.name));
            if let Some(a) = arguments {
                collect_args(a, out);
            }
        }
        ast::InlineExpression::FunctionReference { arguments, .. } => collect_args(arguments, out),
        ast::InlineExpression::Placeable { expression } => collect_expr(expression, out),
        _ => {}
    }
}

fn collect_args(a: &ast::CallArguments<&str>, out: &mut BTreeSet<String>) {
    for p in &a.positional {
        collect_inline(p, out);
    }
    for n in &a.named {
        collect_inline(&n.value, out);
    }
}

/// Extract string literal contents with their 1-based line numbers from Rust source.
/// Handles comments (nested block comments), char literals vs lifetimes, byte and raw strings.
pub fn string_literals(src: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    let at = |k: usize| chars.get(k).copied();
    while let Some(c) = at(i) {
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            '/' if at(i + 1) == Some('/') => {
                while let Some(c) = at(i) {
                    if c == '\n' {
                        break;
                    }
                    i += 1;
                }
            }
            '/' if at(i + 1) == Some('*') => {
                let mut depth = 0usize;
                while let Some(c) = at(i) {
                    if c == '/' && at(i + 1) == Some('*') {
                        depth += 1;
                        i += 2;
                    } else if c == '*' && at(i + 1) == Some('/') {
                        depth = depth.saturating_sub(1);
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        if c == '\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                }
            }
            '\'' => {
                // char literal or lifetime/label
                if at(i + 1) == Some('\\') {
                    i += 2;
                    while let Some(c) = at(i) {
                        i += 1;
                        if c == '\'' {
                            break;
                        }
                    }
                } else if at(i + 2) == Some('\'') {
                    if at(i + 1) == Some('\n') {
                        line += 1;
                    }
                    i += 3;
                } else {
                    i += 1;
                }
            }
            'r' | 'b' | 'c' if !prev_is_ident(&chars, i) => {
                // possible raw string r"..", r#".."#, br"..", b"..", c"..", cr".."
                let mut j = i;
                if matches!(at(j), Some('b') | Some('c')) {
                    j += 1;
                }
                let raw = at(j) == Some('r');
                if raw {
                    j += 1;
                }
                let mut hashes = 0usize;
                if raw {
                    while at(j) == Some('#') {
                        hashes += 1;
                        j += 1;
                    }
                }
                if at(j) == Some('"') && (raw || j > i) {
                    let start_line = line;
                    j += 1;
                    let mut s = String::new();
                    while let Some(c) = at(j) {
                        if raw {
                            if c == '"' && (1..=hashes).all(|h| at(j + h) == Some('#')) {
                                j += 1 + hashes;
                                break;
                            }
                        } else if c == '\\' {
                            if let Some(n) = at(j + 1) {
                                if n == '\n' {
                                    line += 1;
                                }
                                s.push(c);
                                s.push(n);
                            }
                            j += 2;
                            continue;
                        } else if c == '"' {
                            j += 1;
                            break;
                        }
                        if c == '\n' {
                            line += 1;
                        }
                        s.push(c);
                        j += 1;
                    }
                    out.push((start_line, s));
                    i = j;
                } else {
                    // identifier starting with r/b/c: skip the identifier
                    i += 1;
                    while at(i).is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        i += 1;
                    }
                }
            }
            '"' => {
                let start_line = line;
                let mut j = i + 1;
                let mut s = String::new();
                while let Some(c) = at(j) {
                    if c == '\\' {
                        if let Some(n) = at(j + 1) {
                            if n == '\n' {
                                line += 1;
                            }
                            s.push(c);
                            s.push(n);
                        }
                        j += 2;
                        continue;
                    }
                    if c == '"' {
                        j += 1;
                        break;
                    }
                    if c == '\n' {
                        line += 1;
                    }
                    s.push(c);
                    j += 1;
                }
                out.push((start_line, s));
                i = j;
            }
            c if c.is_alphanumeric() || c == '_' => {
                // skip whole identifiers so `r`/`b` inside them are not prefixes
                while at(i).is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    out
}

fn prev_is_ident(chars: &[char], i: usize) -> bool {
    i.checked_sub(1)
        .and_then(|p| chars.get(p))
        .is_some_and(|c| c.is_alphanumeric() || *c == '_')
}

/// `^[a-z][a-z0-9]*(-[a-z0-9]+)+$`
pub fn is_key_like(s: &str) -> bool {
    let mut parts = s.split('-');
    let Some(first) = parts.next() else {
        return false;
    };
    let mut fc = first.chars();
    if !fc.next().is_some_and(|c| c.is_ascii_lowercase()) {
        return false;
    }
    if !fc.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        return false;
    }
    let mut n = 0;
    for p in parts {
        if p.is_empty()
            || !p
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            return false;
        }
        n += 1;
    }
    n > 0
}

/// A string literal found in code.
pub struct Literal {
    pub file: String,
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Default)]
pub struct Report {
    pub errors: Vec<String>,
    pub referenced: usize,
    pub dynamic: usize,
    pub kept: usize,
}

/// Pure analysis over both catalogues and the code literals.
pub fn analyse(id: &Catalogue, en: &Catalogue, literals: &[Literal]) -> Report {
    let mut r = Report::default();
    compare_sets("message", &id.messages, &en.messages, &mut r.errors);
    compare_sets("term", &id.terms, &en.terms, &mut r.errors);

    let dynamic: Vec<&String> = id
        .dynamic_prefixes
        .iter()
        .chain(&en.dynamic_prefixes)
        .collect();
    let keep: BTreeSet<&String> = id.keep.iter().chain(&en.keep).collect();
    let not_keys: BTreeSet<&String> = id.not_keys.iter().chain(&en.not_keys).collect();

    let keys: BTreeSet<&str> = en
        .messages
        .keys()
        .chain(id.messages.keys())
        .map(String::as_str)
        .collect();
    let prefixes: BTreeSet<&str> = keys.iter().filter_map(|k| k.split('-').next()).collect();

    let mut used: BTreeSet<&str> = BTreeSet::new();
    for lit in literals {
        let t = lit.text.as_str();
        if keys.contains(t) {
            used.insert(t);
            continue;
        }
        if !is_key_like(t) || not_keys.contains(&lit.text) {
            continue;
        }
        let first = t.split('-').next().unwrap_or_default();
        if prefixes.contains(first) {
            r.errors.push(format!(
                "{}:{}: `{t}` looks like an i18n key but is not in the catalogue (add it, or list it under `## not-keys:`)",
                lit.file, lit.line
            ));
        }
    }
    let ftl_refs: BTreeSet<&str> = id
        .ftl_refs
        .iter()
        .chain(&en.ftl_refs)
        .map(String::as_str)
        .collect();
    for k in &keys {
        if used.contains(k) || ftl_refs.contains(k) {
            r.referenced += 1;
        } else if dynamic.iter().any(|p| k.starts_with(p.as_str())) {
            r.dynamic += 1;
        } else if keep.iter().any(|x| x.as_str() == *k) {
            r.kept += 1;
        } else {
            r.errors.push(format!(
                "catalogue key `{k}` is never referenced in code (remove it, or cover it with `## dynamic-prefixes:` / `## keep:`)"
            ));
        }
    }
    r
}

fn compare_sets(
    what: &str,
    id: &BTreeMap<String, BTreeSet<String>>,
    en: &BTreeMap<String, BTreeSet<String>>,
    errors: &mut Vec<String>,
) {
    for (k, attrs) in id {
        match en.get(k) {
            None => errors.push(format!("{what} `{k}` exists in id only")),
            Some(en_attrs) => {
                for a in attrs.symmetric_difference(en_attrs) {
                    let side = if attrs.contains(a) { "id" } else { "en" };
                    errors.push(format!(
                        "{what} `{k}`: attribute `.{a}` exists in {side} only"
                    ));
                }
            }
        }
    }
    for k in en.keys() {
        if !id.contains_key(k) {
            errors.push(format!("{what} `{k}` exists in en only"));
        }
    }
}

fn load(locale: &str) -> Result<Catalogue, String> {
    let path = util::root()
        .join("crates/udm-i18n/locales")
        .join(locale)
        .join("udm.ftl");
    let src = util::read(&path)?;
    parse_catalogue(&util::rel(&path), &src)
}

pub fn collect_literals(dir: &Path) -> Result<Vec<Literal>, String> {
    let mut out = Vec::new();
    for f in util::walk_files(dir, Some("rs")) {
        let src = util::read(&f)?;
        let file = util::rel(&f);
        for (line, text) in string_literals(&src) {
            out.push(Literal {
                file: file.clone(),
                line,
                text,
            });
        }
    }
    Ok(out)
}

pub fn run() -> Result<(), String> {
    let id = load("id")?;
    let en = load("en")?;
    let literals = collect_literals(&util::root().join("crates"))?;
    let r = analyse(&id, &en, &literals);
    println!(
        "i18n: {} messages, {} terms; {} referenced, {} dynamic-prefix, {} kept; {} string literals scanned",
        en.messages.len(),
        en.terms.len(),
        r.referenced,
        r.dynamic,
        r.kept,
        literals.len()
    );
    if r.errors.is_empty() {
        println!("i18n: OK");
        Ok(())
    } else {
        Err(format!(
            "{} i18n problem(s):\n{}",
            r.errors.len(),
            util::bullet_list(&r.errors)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_like() {
        assert!(is_key_like("menu-file"));
        assert!(is_key_like("error-net-timeout2"));
        assert!(!is_key_like("menu"));
        assert!(!is_key_like("Menu-file"));
        assert!(!is_key_like("menu--file"));
        assert!(!is_key_like("menu-file-"));
        assert!(!is_key_like("1menu-file"));
        assert!(!is_key_like("menu_file-x"));
        assert!(!is_key_like("menu-File"));
    }

    #[test]
    fn literals_basic() {
        let src = r##"
fn f<'a>(x: &'a str) -> char {
    // "comment-key" ignored
    /* "block-key" /* nested "x-y" */ still */
    let a = "menu-file";
    let b = r#"raw "quoted" str"#;
    let c = b"bytes-x";
    let q = '"';
    let e = '\'';
    let s = "esc \" quote";
    let br = br"raw-bytes";
    'outer: loop { break 'outer; }
    let multi = "line1
line2";
    let after = "status-paused";
    'x'
}
"##;
        let lits = string_literals(src);
        let texts: Vec<&str> = lits.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "menu-file",
                "raw \"quoted\" str",
                "bytes-x",
                "esc \\\" quote",
                "raw-bytes",
                "line1\nline2",
                "status-paused"
            ]
        );
        let line_of = |t: &str| lits.iter().find(|(_, s)| s == t).unwrap().0;
        assert_eq!(line_of("menu-file"), 5);
        assert_eq!(line_of("status-paused"), 15);
    }

    #[test]
    fn identifiers_ending_in_r_or_b_are_not_prefixes() {
        let lits = string_literals("let ptr = 1; foo(br, \"menu-file\"); let rb = r;");
        assert_eq!(lits, vec![(1, "menu-file".to_string())]);
    }

    fn cat(src: &str) -> Catalogue {
        parse_catalogue("t.ftl", src).unwrap()
    }

    fn lit(text: &str) -> Literal {
        Literal {
            file: "a.rs".into(),
            line: 1,
            text: text.into(),
        }
    }

    #[test]
    fn analyse_all_rules() {
        let en = cat(
            "## dynamic-prefixes: category-\n## not-keys: menu-not-a-key\n-brand = UDM\nmenu-file = File { -brand }\n    .tooltip = t\nmenu-edit = Edit\nmenu-unused = x\nmenu-ref = { menu-edit }\ncategory-video = Video\nen-only = x\n",
        );
        let id = cat(
            "-brand = UDM\nmenu-file = Berkas\nmenu-edit = Sunting\nmenu-unused = x\nmenu-ref = { menu-edit }\ncategory-video = Video\nid-only = x\n",
        );
        let lits = vec![
            lit("menu-file"),
            lit("menu-ref"),
            lit("menu-missing"),
            lit("menu-not-a-key"),
            lit("other-thing"),
        ];
        let r = analyse(&id, &en, &lits);
        let joined = r.errors.join("\n");
        assert!(joined.contains("`id-only` exists in id only"), "{joined}");
        assert!(joined.contains("`en-only` exists in en only"), "{joined}");
        assert!(
            joined.contains("attribute `.tooltip` exists in en only"),
            "{joined}"
        );
        assert!(joined.contains("`menu-missing` looks like"), "{joined}");
        assert!(
            joined.contains("`menu-unused` is never referenced"),
            "{joined}"
        );
        assert!(!joined.contains("menu-edit"), "{joined}");
        assert!(!joined.contains("menu-not-a-key"), "{joined}");
        assert!(!joined.contains("other-thing"), "{joined}");
        assert!(!joined.contains("category-video"), "{joined}");
        assert_eq!(r.dynamic, 1);
    }

    #[test]
    fn parse_errors_fail() {
        assert!(parse_catalogue("t.ftl", "menu-file = \n = broken {").is_err());
    }
}
