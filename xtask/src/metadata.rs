//! Minimal `cargo metadata --format-version 1` model (parsed with serde_json).

use serde::Deserialize;

use crate::util;

#[derive(Debug, Deserialize)]
pub struct Metadata {
    pub packages: Vec<Package>,
    pub workspace_members: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub id: String,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    pub manifest_path: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Dependency {
    pub name: String,
    /// `None` = normal, `Some("dev")`, `Some("build")`.
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub optional: bool,
}

/// Run `cargo metadata`; `no_deps` limits it to workspace packages.
pub fn load(no_deps: bool) -> Result<Metadata, String> {
    let mut cmd = util::cargo();
    cmd.args(["metadata", "--format-version", "1"]);
    if no_deps {
        cmd.arg("--no-deps");
    }
    let out = util::run_capture(cmd)?;
    serde_json::from_str(&out).map_err(|e| format!("cannot parse cargo metadata output: {e}"))
}

impl Metadata {
    /// Packages that are workspace members.
    pub fn workspace_packages(&self) -> Vec<&Package> {
        self.packages
            .iter()
            .filter(|p| self.workspace_members.contains(&p.id))
            .collect()
    }
}
