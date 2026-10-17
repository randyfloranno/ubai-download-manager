//! `cargo xtask deny`: licence, ban and source policy (AGENTS.md §5.10).
//!
//! Uses `cargo deny check licenses bans sources` when cargo-deny is installed; otherwise a
//! built-in checker over `cargo metadata` that reads the same tables from `deny.toml`.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{metadata, util};

#[derive(Debug, Deserialize, Default)]
pub struct DenyToml {
    #[serde(default)]
    pub licenses: Licenses,
    #[serde(default)]
    pub bans: Bans,
    #[serde(default)]
    pub sources: Sources,
}

#[derive(Debug, Deserialize, Default)]
pub struct Licenses {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub exceptions: Vec<LicenceException>,
    #[serde(default)]
    pub clarify: Vec<Clarify>,
}

#[derive(Debug, Deserialize)]
pub struct LicenceException {
    #[serde(rename = "crate")]
    pub krate: String,
    #[serde(default)]
    pub allow: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Clarify {
    #[serde(rename = "crate")]
    pub krate: String,
    pub expression: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct Bans {
    #[serde(default)]
    pub deny: Vec<BanEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum BanEntry {
    Name(String),
    Spec {
        #[serde(rename = "crate")]
        krate: String,
    },
}

impl BanEntry {
    fn name(&self) -> &str {
        match self {
            BanEntry::Name(n) => n.split('@').next().unwrap_or(n),
            BanEntry::Spec { krate } => krate.split('@').next().unwrap_or(krate),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct Sources {
    #[serde(default, rename = "allow-registry")]
    pub allow_registry: Vec<String>,
    #[serde(default, rename = "allow-git")]
    pub allow_git: Vec<String>,
}

// ---------------------------------------------------------------- SPDX expressions

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Open,
    Close,
    And,
    Or,
    With,
    Id(String),
}

fn tokenize(expr: &str) -> Vec<Tok> {
    let spaced = expr
        .replace('(', " ( ")
        .replace(')', " ) ")
        .replace('/', " OR ");
    spaced
        .split_whitespace()
        .map(|w| match w {
            "(" => Tok::Open,
            ")" => Tok::Close,
            "AND" | "and" => Tok::And,
            "OR" | "or" => Tok::Or,
            "WITH" | "with" => Tok::With,
            other => Tok::Id(other.to_string()),
        })
        .collect()
}

struct Parser<'a> {
    toks: &'a [Tok],
    pos: usize,
    allowed: &'a dyn Fn(&str) -> bool,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn or_expr(&mut self) -> Result<bool, String> {
        let mut v = self.and_expr()?;
        while self.peek() == Some(&Tok::Or) {
            self.pos += 1;
            let r = self.and_expr()?;
            v = v || r;
        }
        Ok(v)
    }

    fn and_expr(&mut self) -> Result<bool, String> {
        let mut v = self.atom()?;
        while self.peek() == Some(&Tok::And) {
            self.pos += 1;
            let r = self.atom()?;
            v = v && r;
        }
        Ok(v)
    }

    fn atom(&mut self) -> Result<bool, String> {
        match self.toks.get(self.pos).cloned() {
            Some(Tok::Open) => {
                self.depth += 1;
                if self.depth > 32 {
                    return Err("licence expression nested too deeply".into());
                }
                self.pos += 1;
                let v = self.or_expr()?;
                if self.peek() != Some(&Tok::Close) {
                    return Err("missing `)`".into());
                }
                self.pos += 1;
                self.depth -= 1;
                Ok(v)
            }
            Some(Tok::Id(id)) => {
                self.pos += 1;
                let base = id.strip_suffix('+').unwrap_or(&id).to_string();
                if self.peek() == Some(&Tok::With) {
                    self.pos += 1;
                    let Some(Tok::Id(exc)) = self.toks.get(self.pos).cloned() else {
                        return Err("expected exception after WITH".into());
                    };
                    self.pos += 1;
                    Ok((self.allowed)(&format!("{base} WITH {exc}")))
                } else {
                    Ok((self.allowed)(&base))
                }
            }
            other => Err(format!("unexpected token {other:?}")),
        }
    }
}

/// Evaluate an SPDX expression: `OR` needs one allowed side, `AND` needs both.
pub fn expr_allowed(expr: &str, allowed: &dyn Fn(&str) -> bool) -> Result<bool, String> {
    let toks = tokenize(expr);
    if toks.is_empty() {
        return Err("empty licence expression".into());
    }
    let mut p = Parser {
        toks: &toks,
        pos: 0,
        allowed,
        depth: 0,
    };
    let v = p.or_expr()?;
    if p.pos != toks.len() {
        return Err(format!("trailing tokens in `{expr}`"));
    }
    Ok(v)
}

// ---------------------------------------------------------------- built-in checker

pub struct PkgInfo<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub license: Option<&'a str>,
    pub source: Option<&'a str>,
}

/// Pure built-in policy check over third-party packages.
pub fn builtin_check(cfg: &DenyToml, pkgs: &[PkgInfo<'_>]) -> Vec<String> {
    let clarify: BTreeMap<&str, &str> = cfg
        .licenses
        .clarify
        .iter()
        .map(|c| (c.krate.as_str(), c.expression.as_str()))
        .collect();
    let mut v = Vec::new();
    for p in pkgs {
        let id = format!("{} {}", p.name, p.version);
        // bans
        if cfg.bans.deny.iter().any(|b| b.name() == p.name) {
            v.push(format!("{id}: crate is banned in deny.toml [bans]"));
        }
        // sources
        if let Some(src) = p.source {
            if let Some(reg) = src.strip_prefix("registry+") {
                if !cfg.sources.allow_registry.iter().any(|r| r == reg) {
                    v.push(format!("{id}: unknown registry {reg}"));
                }
            } else if src.starts_with("sparse+") {
                let ok = src == "sparse+https://index.crates.io/"
                    || cfg
                        .sources
                        .allow_registry
                        .iter()
                        .any(|r| src.ends_with(r.as_str()));
                if !ok {
                    v.push(format!("{id}: unknown registry {src}"));
                }
            } else if let Some(git) = src.strip_prefix("git+")
                && !cfg
                    .sources
                    .allow_git
                    .iter()
                    .any(|g| git.starts_with(g.as_str()))
            {
                v.push(format!("{id}: unknown git source {git}"));
            }
        }
        // licences
        let expr = clarify.get(p.name).copied().or(p.license);
        let Some(expr) = expr else {
            v.push(format!(
                "{id}: no `license` field (add a [[licenses.clarify]] entry to deny.toml)"
            ));
            continue;
        };
        let extra: Vec<&String> = cfg
            .licenses
            .exceptions
            .iter()
            .filter(|e| e.krate == p.name)
            .flat_map(|e| e.allow.iter())
            .collect();
        let allowed = |l: &str| {
            cfg.licenses
                .allow
                .iter()
                .chain(extra.iter().copied())
                .any(|a| a == l)
        };
        match expr_allowed(expr, &allowed) {
            Ok(true) => {}
            Ok(false) => v.push(format!("{id}: licence `{expr}` is not allowed")),
            Err(e) => v.push(format!("{id}: cannot parse licence `{expr}`: {e}")),
        }
    }
    v
}

pub fn load_config() -> Result<DenyToml, String> {
    let path = util::root().join("deny.toml");
    let text = util::read(&path)?;
    toml::from_str(&text).map_err(|e| format!("cannot parse deny.toml: {e}"))
}

fn cargo_deny_available() -> bool {
    // `UDM_XTASK_BUILTIN_DENY=1` forces the built-in checker (useful to compare both).
    if std::env::var_os("UDM_XTASK_BUILTIN_DENY").is_some_and(|v| v == "1") {
        return false;
    }
    let mut cmd = util::cargo();
    cmd.args(["deny", "--version"]);
    cmd.output().is_ok_and(|o| o.status.success())
}

pub fn run() -> Result<(), String> {
    if cargo_deny_available() {
        let mut cmd = util::cargo();
        cmd.args([
            "deny",
            "check",
            "--hide-inclusion-graph",
            "licenses",
            "bans",
            "sources",
        ]);
        util::run_status(cmd)?;
        println!("deny: OK (cargo-deny)");
        return Ok(());
    }
    println!(
        "deny: cargo-deny not available (or UDM_XTASK_BUILTIN_DENY=1), using the built-in checker"
    );
    let cfg = load_config()?;
    let meta = metadata::load(false)?;
    let pkgs: Vec<PkgInfo<'_>> = meta
        .packages
        .iter()
        .filter(|p| !meta.workspace_members.contains(&p.id))
        .map(|p| PkgInfo {
            name: &p.name,
            version: &p.version,
            license: p.license.as_deref(),
            source: p.source.as_deref(),
        })
        .collect();
    let v = builtin_check(&cfg, &pkgs);
    if v.is_empty() {
        println!("deny: OK ({} third-party packages checked)", pkgs.len());
        Ok(())
    } else {
        Err(format!(
            "{} policy violation(s):\n{}",
            v.len(),
            util::bullet_list(&v)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> DenyToml {
        toml::from_str(include_str!("../../deny.toml")).unwrap()
    }

    fn ok(expr: &str) -> bool {
        let c = cfg();
        expr_allowed(expr, &|l| c.licenses.allow.iter().any(|a| a == l)).unwrap()
    }

    #[test]
    fn expressions() {
        assert!(ok("MIT"));
        assert!(ok("MIT OR Apache-2.0"));
        assert!(ok("MIT/Apache-2.0"));
        assert!(ok("Apache-2.0 AND ISC"));
        assert!(ok("(MIT OR Apache-2.0) AND Unicode-3.0"));
        assert!(ok("Apache-2.0 WITH LLVM-exception OR MIT"));
        assert!(ok("MIT OR Apache-2.0 OR LGPL-2.1-or-later"));
        assert!(!ok("MPL-2.0"));
        assert!(!ok("GPL-3.0"));
        assert!(!ok("MIT AND GPL-2.0+"));
        assert!(!ok("OpenSSL"));
        assert!(!ok("ISC AND OpenSSL"));
        assert!(!ok("GPL-2.0 WITH LLVM-exception"));
        assert!(expr_allowed("(MIT", &|_| true).is_err());
        assert!(expr_allowed("MIT Apache-2.0", &|_| true).is_err());
        assert!(expr_allowed("", &|_| true).is_err());
    }

    #[test]
    fn builtin_policy() {
        let c = cfg();
        let crates_io = "registry+https://github.com/rust-lang/crates.io-index";
        let pkgs = [
            PkgInfo {
                name: "serde",
                version: "1.0.0",
                license: Some("MIT OR Apache-2.0"),
                source: Some(crates_io),
            },
            PkgInfo {
                name: "ring",
                version: "0.17.14",
                license: Some("Apache-2.0 AND ISC"),
                source: Some(crates_io),
            },
            PkgInfo {
                name: "epaint_default_fonts",
                version: "0.32.0",
                license: Some("(MIT OR Apache-2.0) AND OFL-1.1 AND Ubuntu-font-1.0"),
                source: Some(crates_io),
            },
            PkgInfo {
                name: "other_fonts",
                version: "1.0.0",
                license: Some("OFL-1.1"),
                source: Some(crates_io),
            },
            PkgInfo {
                name: "openssl",
                version: "0.10.0",
                license: Some("Apache-2.0"),
                source: Some(crates_io),
            },
            PkgInfo {
                name: "nolic",
                version: "1.0.0",
                license: None,
                source: Some(crates_io),
            },
            PkgInfo {
                name: "gitdep",
                version: "1.0.0",
                license: Some("MIT"),
                source: Some("git+https://example.com/x#abc"),
            },
            PkgInfo {
                name: "option-ext",
                version: "0.2.0",
                license: Some("MPL-2.0"),
                source: Some(crates_io),
            },
        ];
        let v = builtin_check(&c, &pkgs);
        let joined = v.join("\n");
        assert_eq!(v.len(), 5, "{joined}");
        for bad in ["other_fonts", "openssl", "nolic", "gitdep", "option-ext"] {
            assert!(joined.contains(bad), "{bad} missing in {joined}");
        }
    }
}
