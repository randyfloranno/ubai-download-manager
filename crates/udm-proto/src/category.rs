//! Automatic categories by file extension.
//!
//! The default extension table below was written for UDM (no third-party source).

use crate::filename::extension;

/// One category with the extensions (lower-case, without dot) that select it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoryRule {
    /// Category id, e.g. `documents`.
    pub id: String,
    /// Extensions without the dot.
    pub extensions: Vec<String>,
}

const DEFAULTS: [(&str, &[&str]); 5] = [
    (
        "documents",
        &[
            "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "txt",
            "csv", "epub", "md", "tex", "xps",
        ],
    ),
    (
        "video",
        &[
            "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "3gp", "ogv",
            "vob", "m2ts",
        ],
    ),
    (
        "music",
        &[
            "mp3", "wav", "flac", "aac", "ogg", "oga", "m4a", "wma", "opus", "aiff", "aif", "mid",
            "midi", "ape",
        ],
    ),
    (
        "programs",
        &[
            "exe", "msi", "msix", "appx", "dmg", "pkg", "deb", "rpm", "apk", "appimage", "flatpak",
            "snap", "jar", "bat", "cmd", "sh",
        ],
    ),
    (
        "archives",
        &[
            "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "tbz2", "xz", "txz", "zst", "lz",
            "lzma", "cab", "iso", "img", "arj",
        ],
    ),
];

/// Built-in rules: `documents`, `video`, `music`, `programs`, `archives`.
pub fn default_category_rules() -> Vec<CategoryRule> {
    DEFAULTS
        .iter()
        .map(|(id, exts)| CategoryRule {
            id: (*id).to_owned(),
            extensions: exts.iter().map(|e| (*e).to_owned()).collect(),
        })
        .collect()
}

/// Category id for a file name, first matching rule wins (extension compared
/// case-insensitively). `None` means the caller's `general` category.
pub fn classify(filename: &str, rules: &[CategoryRule]) -> Option<String> {
    let ext = extension(filename)?;
    rules
        .iter()
        .find(|r| {
            r.extensions
                .iter()
                .any(|e| e.trim_start_matches('.').eq_ignore_ascii_case(&ext))
        })
        .map(|r| r.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_defaults() {
        let rules = default_category_rules();
        let ids: Vec<_> = rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["documents", "video", "music", "programs", "archives"]);
        assert_eq!(classify("a.PDF", &rules).as_deref(), Some("documents"));
        assert_eq!(classify("movie.mkv", &rules).as_deref(), Some("video"));
        assert_eq!(classify("song.flac", &rules).as_deref(), Some("music"));
        assert_eq!(classify("setup.exe", &rules).as_deref(), Some("programs"));
        assert_eq!(classify("src.tar.gz", &rules).as_deref(), Some("archives"));
        assert_eq!(classify("unknown.xyz", &rules), None);
        assert_eq!(classify("noext", &rules), None);
        assert_eq!(classify(".zip", &rules), None);
    }

    #[test]
    fn custom_rules_first_match_wins() {
        let rules = vec![
            CategoryRule {
                id: "mine".into(),
                extensions: vec![".ISO".into()],
            },
            CategoryRule {
                id: "archives".into(),
                extensions: vec!["iso".into()],
            },
        ];
        assert_eq!(classify("x.iso", &rules).as_deref(), Some("mine"));
    }

    #[test]
    fn hostile_category_table_within_limits() {
        let rules = default_category_rules();
        assert!(rules.len() <= udm_core::limits::MAX_CATEGORIES);
        assert!(
            rules
                .iter()
                .all(|r| r.extensions.len() <= udm_core::limits::MAX_CATEGORY_EXTENSIONS)
        );
    }
}
