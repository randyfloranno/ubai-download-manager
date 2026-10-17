//! User settings (settings file). Secrets are never stored here, only
//! keychain references.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use udm_core::{Message, limits};
use udm_proto::{CategoryRule, default_category_rules};

/// Category configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryConfig {
    /// Machine id (`documents`, `video`, …, `general`).
    pub id: String,
    /// Target folder; `None` = default folder / `<default>/<Category>`.
    #[serde(default)]
    pub folder: Option<PathBuf>,
    /// Lower-case extensions without dot.
    #[serde(default)]
    pub extensions: Vec<String>,
}

/// Proxy kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyKind {
    /// Direct connection.
    #[default]
    None,
    /// HTTP CONNECT proxy.
    Http,
    /// SOCKS5 proxy.
    Socks5,
}

impl ProxyKind {
    /// Machine names.
    pub const NAMES: &'static [&'static str] = &["none", "http", "socks5"];

    /// Parses a machine name.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(ProxyKind::None),
            "http" => Some(ProxyKind::Http),
            "socks5" => Some(ProxyKind::Socks5),
            _ => None,
        }
    }
}

/// Proxy settings. The password lives in the keychain under [`PROXY_SECRET_REF`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxySettings {
    /// Kind.
    pub kind: ProxyKind,
    /// Host.
    #[serde(default)]
    pub host: String,
    /// Port.
    #[serde(default)]
    pub port: u16,
    /// User name, if authenticated.
    #[serde(default)]
    pub user: Option<String>,
}

/// Keychain reference of the proxy password.
pub const PROXY_SECRET_REF: &str = "proxy:main";

/// Credentials for one site; the password lives in the keychain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteAuth {
    /// Host name (lower case).
    pub host: String,
    /// User name.
    pub user: String,
}

impl SiteAuth {
    /// Keychain reference of the password.
    pub fn secret_ref(&self) -> String {
        format!("site:{}:{}", self.host, self.user)
    }
}

/// A per-host TLS exception granted explicitly by the user.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsExceptionSetting {
    /// Host.
    pub host: String,
    /// SHA-256 fingerprint (hex) of the accepted leaf certificate.
    pub sha256_fingerprint: String,
}

/// All settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// UI locale (`id` / `en`).
    pub locale: String,
    /// Default download folder.
    pub download_dir: PathBuf,
    /// Put files into per-category sub-folders of the default folder.
    pub category_subfolders: bool,
    /// Default connections per download.
    pub connections: u32,
    /// Global speed limit (0 = unlimited).
    pub global_speed_limit: u64,
    /// Maximum simultaneous downloads started outside queues.
    pub max_concurrent: u32,
    /// Categories.
    pub categories: Vec<CategoryConfig>,
    /// Watch the clipboard for links.
    pub clipboard_capture: bool,
    /// Show a notification when a download completes.
    pub notify_on_complete: bool,
    /// Browser capture: only take over files with these extensions (empty = all).
    pub capture_extensions: Vec<String>,
    /// Browser capture: minimum size to take over (0 = any).
    pub capture_min_size: u64,
    /// Proxy.
    pub proxy: ProxySettings,
    /// Site credentials.
    pub site_auth: Vec<SiteAuth>,
    /// TLS exceptions.
    pub tls_exceptions: Vec<TlsExceptionSetting>,
    /// User-Agent header.
    pub user_agent: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            locale: "id".to_owned(),
            download_dir: PathBuf::from("Downloads"),
            category_subfolders: true,
            connections: limits::DEFAULT_CONNECTIONS,
            global_speed_limit: 0,
            max_concurrent: 4,
            categories: default_categories(),
            clipboard_capture: false,
            notify_on_complete: true,
            capture_extensions: Vec::new(),
            capture_min_size: 0,
            proxy: ProxySettings::default(),
            site_auth: Vec::new(),
            tls_exceptions: Vec::new(),
            user_agent: concat!("UDM/", env!("CARGO_PKG_VERSION")).to_owned(),
        }
    }
}

/// The fallback category for files no rule matches.
pub const GENERAL_CATEGORY: &str = "general";

/// Default category list: the proto rules plus `general`.
pub fn default_categories() -> Vec<CategoryConfig> {
    let mut out: Vec<CategoryConfig> = default_category_rules()
        .into_iter()
        .map(|r| CategoryConfig {
            id: r.id,
            folder: None,
            extensions: r.extensions,
        })
        .collect();
    out.push(CategoryConfig {
        id: GENERAL_CATEGORY.to_owned(),
        folder: None,
        extensions: Vec::new(),
    });
    out
}

/// Folder name used for a category sub-folder (stable, not translated, so
/// paths do not change with the UI language).
fn category_folder_name(id: &str) -> String {
    let mut chars = id.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::from("General"),
    }
}

impl Settings {
    /// Rules for [`udm_proto::classify`].
    pub fn category_rules(&self) -> Vec<CategoryRule> {
        self.categories
            .iter()
            .filter(|c| !c.extensions.is_empty())
            .map(|c| CategoryRule {
                id: c.id.clone(),
                extensions: c.extensions.clone(),
            })
            .collect()
    }

    /// Category id for a file name.
    pub fn classify(&self, file_name: &str) -> String {
        udm_proto::classify(file_name, &self.category_rules())
            .unwrap_or_else(|| GENERAL_CATEGORY.to_owned())
    }

    /// Target folder for a category.
    pub fn folder_for(&self, category: &str) -> PathBuf {
        let configured = self
            .categories
            .iter()
            .find(|c| c.id == category)
            .and_then(|c| c.folder.clone());
        match configured {
            Some(f) => f,
            None if self.category_subfolders && category != GENERAL_CATEGORY => {
                self.download_dir.join(category_folder_name(category))
            }
            None => self.download_dir.clone(),
        }
    }

    /// Credentials configured for a host.
    pub fn site_auth_for(&self, host: &str) -> Option<&SiteAuth> {
        self.site_auth
            .iter()
            .find(|s| s.host.eq_ignore_ascii_case(host))
    }

    /// Validates bounds after loading from disk or changing.
    pub fn validate(&self) -> Result<(), Message> {
        if self.connections == 0 || self.connections > limits::MAX_CONNECTIONS_PER_DOWNLOAD {
            return Err(Message::new("error-settings-bad-value").with_str("name", "connections"));
        }
        if self.global_speed_limit > limits::MAX_SPEED_LIMIT_BYTES_PER_SEC {
            return Err(
                Message::new("error-settings-bad-value").with_str("name", "global_speed_limit")
            );
        }
        if self.max_concurrent == 0 || self.max_concurrent > limits::MAX_CONCURRENT_DOWNLOADS {
            return Err(Message::new("error-settings-bad-value").with_str("name", "max_concurrent"));
        }
        if self.categories.len() > limits::MAX_CATEGORIES
            || self
                .categories
                .iter()
                .any(|c| c.extensions.len() > limits::MAX_CATEGORY_EXTENSIONS)
        {
            return Err(Message::new("error-settings-bad-value").with_str("name", "categories"));
        }
        if self.locale != "id" && self.locale != "en" {
            return Err(Message::new("error-settings-bad-value").with_str("name", "locale"));
        }
        if !udm_proto::header_value_ok(&self.user_agent) {
            return Err(Message::new("error-settings-bad-value").with_str("name", "user_agent"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_classify() {
        let s = Settings::default();
        s.validate().unwrap();
        assert_eq!(s.classify("movie.mp4"), "video");
        assert_eq!(s.classify("thing.unknownext"), GENERAL_CATEGORY);
        assert_eq!(
            s.folder_for("video"),
            PathBuf::from("Downloads").join("Video")
        );
        assert_eq!(s.folder_for(GENERAL_CATEGORY), PathBuf::from("Downloads"));
    }

    #[test]
    fn hostile_settings_out_of_bounds() {
        let s = Settings {
            connections: 10_000,
            ..Settings::default()
        };
        assert!(s.validate().is_err());
        let s = Settings {
            user_agent: "a\r\nInjected: 1".into(),
            ..Settings::default()
        };
        assert!(s.validate().is_err());
        let s: Settings = serde_json::from_str(r#"{"locale":"en","unknown":1}"#).unwrap();
        assert_eq!(s.connections, limits::DEFAULT_CONNECTIONS);
    }
}
