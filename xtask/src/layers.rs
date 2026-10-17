//! `cargo xtask layers`: enforce the crate layering of AGENTS.md §5.8.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::{metadata, util};

#[derive(Debug, Deserialize)]
pub struct Config {
    pub layers: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub ext: ExtConfig,
    #[serde(default)]
    pub no_dependencies: NoDeps,
    #[serde(default)]
    pub exception: Vec<Exception>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ExtConfig {
    #[serde(default)]
    pub allowed: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct NoDeps {
    #[serde(default)]
    pub crates: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Exception {
    #[serde(rename = "crate")]
    pub krate: String,
    pub dep: String,
    /// If true, the exception only applies when the dependency is optional.
    #[serde(default)]
    pub optional: bool,
    #[allow(dead_code)]
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Num(u8),
    Ext,
    Dev,
}

impl std::fmt::Display for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Layer::Num(n) => write!(f, "L{n}"),
            Layer::Ext => write!(f, "ext"),
            Layer::Dev => write!(f, "dev"),
        }
    }
}

/// One dependency edge of a workspace package, as needed by the checker.
#[derive(Debug, Clone)]
pub struct Edge {
    pub dep: String,
    /// `None` normal, `Some("dev")`, `Some("build")`.
    pub kind: Option<String>,
    pub optional: bool,
}

#[derive(Debug, Clone)]
pub struct Pkg {
    pub name: String,
    pub deps: Vec<Edge>,
}

fn kind_label(kind: Option<&str>) -> &'static str {
    match kind {
        Some("dev") => "dev-dependency",
        Some("build") => "build-dependency",
        _ => "dependency",
    }
}

/// Pure layer checker: returns every violation found.
pub fn check(config: &Config, pkgs: &[Pkg]) -> Result<Vec<String>, String> {
    let mut layer_of: BTreeMap<&str, Layer> = BTreeMap::new();
    for (key, crates) in &config.layers {
        let layer = match key.as_str() {
            "ext" => Layer::Ext,
            "dev" => Layer::Dev,
            k => {
                let n = k
                    .strip_prefix('L')
                    .and_then(|n| n.parse::<u8>().ok())
                    .ok_or_else(|| {
                        format!("layers.toml: unknown layer `{k}` (use L0..L9, ext, dev)")
                    })?;
                Layer::Num(n)
            }
        };
        for c in crates {
            if layer_of.insert(c.as_str(), layer).is_some() {
                return Err(format!(
                    "layers.toml: crate `{c}` is listed in more than one layer"
                ));
            }
        }
    }

    let workspace: BTreeSet<&str> = pkgs.iter().map(|p| p.name.as_str()).collect();
    let mut v = Vec::new();

    for name in layer_of.keys() {
        if !workspace.contains(name) {
            v.push(format!(
                "`{name}` is listed in layers.toml but is not a workspace member"
            ));
        }
    }

    for pkg in pkgs {
        let Some(&from) = layer_of.get(pkg.name.as_str()) else {
            v.push(format!(
                "`{}` is a workspace member but has no layer in xtask/layers.toml",
                pkg.name
            ));
            continue;
        };
        let no_deps = config.no_dependencies.crates.contains(&pkg.name);
        for e in &pkg.deps {
            let is_dev_kind = e.kind.as_deref() == Some("dev");
            if no_deps && !is_dev_kind {
                v.push(format!(
                    "`{}` must have no dependencies but has {} `{}`",
                    pkg.name,
                    kind_label(e.kind.as_deref()),
                    e.dep
                ));
                continue;
            }
            let Some(&to) = layer_of.get(e.dep.as_str()) else {
                continue; // not a workspace crate
            };
            if is_dev_kind {
                continue; // dev-dependencies may point anywhere
            }
            let what = kind_label(e.kind.as_deref());
            if from == Layer::Dev {
                continue; // dev tooling may use any crate
            }
            let excepted = config
                .exception
                .iter()
                .any(|x| x.krate == pkg.name && x.dep == e.dep && (!x.optional || e.optional));
            if excepted {
                continue;
            }
            let ok = match (from, to) {
                (_, Layer::Dev) => false,
                (_, Layer::Ext) => false,
                (Layer::Ext, _) => config.ext.allowed.contains(&e.dep),
                (Layer::Num(a), Layer::Num(b)) => b < a,
                (Layer::Dev, _) => true,
            };
            if !ok {
                let why = match (from, to) {
                    (_, Layer::Dev) => "dev crates may only be dev-dependencies".to_string(),
                    (_, Layer::Ext) => "nothing may depend on the extension crate".to_string(),
                    (Layer::Ext, _) => format!(
                        "ext crates may only depend on {}",
                        config.ext.allowed.join(", ")
                    ),
                    _ => "dependencies must point to a strictly lower layer".to_string(),
                };
                v.push(format!(
                    "`{}` ({from}) has {what} on `{}` ({to}): {why}",
                    pkg.name, e.dep
                ));
            }
        }
    }
    Ok(v)
}

pub fn load_config() -> Result<Config, String> {
    let path = util::root().join("xtask").join("layers.toml");
    let text = util::read(&path)?;
    toml::from_str(&text).map_err(|e| format!("cannot parse xtask/layers.toml: {e}"))
}

pub fn run() -> Result<(), String> {
    let config = load_config()?;
    let meta = metadata::load(true)?;
    let pkgs: Vec<Pkg> = meta
        .workspace_packages()
        .into_iter()
        .map(|p| Pkg {
            name: p.name.clone(),
            deps: p
                .dependencies
                .iter()
                .map(|d| Edge {
                    dep: d.name.clone(),
                    kind: d.kind.clone(),
                    optional: d.optional,
                })
                .collect(),
        })
        .collect();
    let violations = check(&config, &pkgs)?;
    if violations.is_empty() {
        println!("layers: OK ({} workspace crates checked)", pkgs.len());
        Ok(())
    } else {
        Err(format!(
            "{} layering violation(s):\n{}",
            violations.len(),
            util::bullet_list(&violations)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        toml::from_str(include_str!("../layers.toml")).unwrap()
    }

    fn edge(dep: &str, kind: Option<&str>, optional: bool) -> Edge {
        Edge {
            dep: dep.into(),
            kind: kind.map(str::to_string),
            optional,
        }
    }

    fn all_pkgs(extra: &[(&str, Vec<Edge>)]) -> Vec<Pkg> {
        let names = [
            "udm-core",
            "udm-i18n",
            "udm-proto",
            "udm-store",
            "udm-net",
            "udm-secrets",
            "udm-sys",
            "udm-engine",
            "udm-control",
            "udm-app",
            "udm-cli",
            "udm-native-host",
            "udm-ext",
            "udm-testsrv",
            "xtask",
        ];
        names
            .iter()
            .map(|n| Pkg {
                name: n.to_string(),
                deps: extra
                    .iter()
                    .find(|(m, _)| m == n)
                    .map(|(_, d)| d.clone())
                    .unwrap_or_default(),
            })
            .collect()
    }

    #[test]
    fn clean_graph_passes() {
        let pkgs = all_pkgs(&[
            ("udm-i18n", vec![edge("udm-core", None, false)]),
            (
                "udm-engine",
                vec![
                    edge("udm-net", None, false),
                    edge("udm-testsrv", Some("dev"), false),
                ],
            ),
            (
                "udm-app",
                vec![
                    edge("udm-control", None, false),
                    edge("udm-testsrv", None, true),
                ],
            ),
            (
                "udm-ext",
                vec![edge("udm-proto", None, false), edge("serde", None, false)],
            ),
            ("xtask", vec![edge("udm-core", None, false)]),
            ("udm-core", vec![edge("proptest", Some("dev"), false)]),
        ]);
        assert_eq!(check(&config(), &pkgs).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn upward_and_same_layer_deps_fail() {
        let pkgs = all_pkgs(&[(
            "udm-net",
            vec![
                edge("udm-engine", None, false),
                edge("udm-sys", Some("build"), false),
            ],
        )]);
        let v = check(&config(), &pkgs).unwrap();
        assert_eq!(v.len(), 2, "{v:?}");
    }

    #[test]
    fn dev_crate_as_normal_dep_fails_without_exception() {
        let pkgs = all_pkgs(&[
            ("udm-engine", vec![edge("udm-testsrv", None, false)]),
            // exception requires optional = true
            ("udm-app", vec![edge("udm-testsrv", None, false)]),
        ]);
        let v = check(&config(), &pkgs).unwrap();
        assert_eq!(v.len(), 2, "{v:?}");
    }

    #[test]
    fn core_must_have_no_dependencies() {
        let pkgs = all_pkgs(&[("udm-core", vec![edge("serde", None, false)])]);
        let v = check(&config(), &pkgs).unwrap();
        assert_eq!(v.len(), 1);
        assert!(v[0].contains("no dependencies"));
    }

    #[test]
    fn ext_rules() {
        let pkgs = all_pkgs(&[
            ("udm-ext", vec![edge("udm-store", None, false)]),
            ("udm-app", vec![edge("udm-ext", None, false)]),
        ]);
        let v = check(&config(), &pkgs).unwrap();
        assert_eq!(v.len(), 2, "{v:?}");
    }

    #[test]
    fn unknown_member_reported() {
        let mut pkgs = all_pkgs(&[]);
        pkgs.push(Pkg {
            name: "udm-new".into(),
            deps: vec![],
        });
        pkgs.retain(|p| p.name != "udm-cli");
        let v = check(&config(), &pkgs).unwrap();
        assert_eq!(v.len(), 2, "{v:?}");
    }
}
