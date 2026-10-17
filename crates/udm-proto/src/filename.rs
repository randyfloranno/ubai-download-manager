//! File-name sanitising, extensions, auto-rename and dangerous-name detection.
//!
//! [`sanitize_filename`] is the **single** function every file name from the network
//! (URL path, `Content-Disposition`, redirect target) passes through.

use ::url::Url;
use percent_encoding::percent_decode_str;
use udm_core::limits;

use crate::text::truncate_bytes;

/// Name used when nothing usable remains.
const FALLBACK_NAME: &str = "download";

/// Longest extension kept intact when a long name is truncated.
const MAX_KEPT_EXTENSION_BYTES: usize = 32;

/// Multi-part extensions kept together by [`auto_rename`].
const COMPOUND_EXTENSIONS: [&str; 8] = [
    ".tar.gz",
    ".tar.bz2",
    ".tar.xz",
    ".tar.zst",
    ".tar.lz",
    ".tar.lzma",
    ".tar.br",
    ".tar.z",
];

/// Executable/script extensions that make a disguised double extension dangerous.
const EXECUTABLE_EXTENSIONS: [&str; 24] = [
    "exe", "scr", "com", "bat", "cmd", "pif", "msi", "msp", "vbs", "vbe", "js", "jse", "wsf",
    "wsh", "ps1", "lnk", "hta", "cpl", "jar", "reg", "scf", "sh", "app", "command",
];

/// "Harmless-looking" extensions used to disguise an executable.
const DECOY_EXTENSIONS: [&str; 32] = [
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "rtf", "txt", "csv", "jpg", "jpeg",
    "png", "gif", "bmp", "webp", "mp3", "wav", "mp4", "avi", "mkv", "mov", "wmv", "zip", "rar",
    "7z", "htm", "html", "iso", "epub", "invoice",
];

/// Returns one safe, non-empty path component for the given raw name.
///
/// - keeps only the last component after `/` or `\` (so `..`, absolute paths and
///   drive prefixes cannot escape the target folder);
/// - removes control characters (C0, DEL, C1), Unicode bidi overrides/isolates and
///   invisible marks; replaces `< > : " | ? *` with `_`;
/// - trims surrounding whitespace and trailing dots/spaces;
/// - prefixes Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM0-9`,
///   `LPT0-9`, any case, with or without extension) with `_`;
/// - truncates to `limits::MAX_FILENAME_BYTES` on a char boundary, keeping the extension;
/// - falls back to `download` when nothing is left.
pub fn sanitize_filename(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .filter(|c| !is_removed_char(*c))
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let mut name = trim_name(&cleaned).to_owned();
    if is_windows_reserved(&name) {
        name.insert(0, '_');
    }
    let name = truncate_keep_extension(&name, limits::MAX_FILENAME_BYTES);
    let name = trim_name(&name);
    if name.is_empty() {
        FALLBACK_NAME.to_owned()
    } else {
        name.to_owned()
    }
}

fn is_removed_char(c: char) -> bool {
    c.is_control() // C0, DEL, C1
        || matches!(
            c,
            '\u{200B}'..='\u{200F}' // zero-width, LRM, RLM
                | '\u{202A}'..='\u{202E}' // bidi embeddings and overrides
                | '\u{2066}'..='\u{2069}' // bidi isolates
                | '\u{061C}' // Arabic letter mark
                | '\u{FEFF}' // BOM / zero-width no-break space
        )
}

/// Trims whitespace at both ends and dots/spaces at the end (Windows strips them silently).
fn trim_name(s: &str) -> &str {
    s.trim().trim_end_matches(['.', ' ']).trim_end()
}

fn is_windows_reserved(name: &str) -> bool {
    // Windows applies device names to the part before the first dot, ignoring trailing spaces.
    let stem = name.split('.').next().unwrap_or("").trim_end();
    let upper = stem.to_ascii_uppercase();
    if matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) {
        return true;
    }
    let mut chars = upper.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    if prefix != "COM" && prefix != "LPT" {
        return false;
    }
    let rest: Vec<char> = chars.collect();
    matches!(rest.as_slice(), [d] if d.is_ascii_digit() || matches!(d, '\u{B9}' | '\u{B2}' | '\u{B3}'))
}

/// Truncates to `max` bytes; keeps a short extension intact when possible.
fn truncate_keep_extension(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_owned();
    }
    if let Some((stem, ext)) = split_extension(name) {
        // `ext` includes the dot.
        if ext.len() <= MAX_KEPT_EXTENSION_BYTES {
            let room = max.saturating_sub(ext.len());
            let stem = trim_name(truncate_bytes(stem, room));
            if !stem.is_empty() {
                return format!("{stem}{ext}");
            }
        }
    }
    truncate_bytes(name, max).to_owned()
}

/// Splits `name.ext` into (`name`, `.ext`); `None` when there is no extension
/// (no dot, leading dot only like `.bashrc`, or trailing dot).
fn split_extension(name: &str) -> Option<(&str, &str)> {
    let dot = name.rfind('.')?;
    if dot == 0 {
        return None;
    }
    let (stem, ext) = name.split_at_checked(dot)?;
    if ext.len() <= 1 {
        None
    } else {
        Some((stem, ext))
    }
}

/// Lower-case last extension without the dot (`"A.TAR.GZ"` → `"gz"`).
pub fn extension(name: &str) -> Option<String> {
    split_extension(name)
        .and_then(|(_, ext)| ext.get(1..))
        .map(str::to_ascii_lowercase)
}

/// Last non-empty URL path segment, percent-decoded. Raw: sanitise before use.
pub fn filename_from_url(u: &Url) -> Option<String> {
    let seg = u.path_segments()?.next_back()?;
    if seg.is_empty() {
        return None;
    }
    let decoded = percent_decode_str(seg).decode_utf8_lossy().into_owned();
    if decoded.trim().is_empty() {
        None
    } else {
        Some(decoded)
    }
}

/// True for names like `invoice.pdf.exe` that hide an executable behind a decoy extension.
pub fn has_dangerous_double_extension(name: &str) -> bool {
    let lower = name.to_lowercase();
    let mut parts = lower.rsplit('.');
    let last = parts.next().unwrap_or("").trim();
    let Some(prev) = parts.next() else {
        return false;
    };
    // Need a stem before the decoy extension.
    if parts.next().is_none() {
        return false;
    }
    EXECUTABLE_EXTENSIONS.contains(&last) && DECOY_EXTENSIONS.contains(&prev.trim())
}

/// `"a.zip", 1` → `"a (1).zip"`; `"a.tar.gz", 1` → `"a (1).tar.gz"`; `"noext", 2` → `"noext (2)"`.
/// `n == 0` returns the name unchanged. The result never exceeds `limits::MAX_FILENAME_BYTES`.
pub fn auto_rename(name: &str, n: u32) -> String {
    if n == 0 {
        return truncate_keep_extension(name, limits::MAX_FILENAME_BYTES);
    }
    let lower = name.to_ascii_lowercase();
    let compound = COMPOUND_EXTENSIONS
        .iter()
        .find(|ext| lower.len() > ext.len() && lower.ends_with(*ext))
        .and_then(|ext| name.len().checked_sub(ext.len()))
        .and_then(|at| name.split_at_checked(at));
    let (stem, ext) = compound
        .or_else(|| split_extension(name))
        .unwrap_or((name, ""));
    let suffix = format!(" ({n})");
    let max = limits::MAX_FILENAME_BYTES;
    let (stem, ext) = if suffix.len().saturating_add(ext.len()) >= max {
        (name, "")
    } else {
        (stem, ext)
    };
    let room = max.saturating_sub(suffix.len()).saturating_sub(ext.len());
    let stem = trim_name(truncate_bytes(stem, room));
    format!("{stem}{suffix}{ext}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(raw: &str) -> String {
        sanitize_filename(raw)
    }

    fn assert_single_component(out: &str) {
        assert!(!out.is_empty());
        assert!(
            !out.contains('/') && !out.contains('\\') && !out.contains(':'),
            "{out:?}"
        );
        assert!(out != "." && out != "..");
        assert!(out.len() <= limits::MAX_FILENAME_BYTES);
        assert!(!out.ends_with('.') && !out.ends_with(' '));
        assert!(!out.chars().any(|c| c.is_control()));
    }

    #[test]
    fn ordinary_names_untouched() {
        assert_eq!(s("report 2024.pdf"), "report 2024.pdf");
        assert_eq!(s("ünïcødé – файл.zip"), "ünïcødé – файл.zip");
        assert_eq!(s(".bashrc"), ".bashrc");
    }

    #[test]
    fn hostile_filename_dot_dot() {
        for raw in ["..", ".", "...", "../..", "..\\..", "../", "a/.."] {
            let out = s(raw);
            assert_single_component(&out);
            assert_eq!(out, "download", "{raw:?}");
        }
        assert_eq!(s("../../../etc/passwd"), "passwd");
        assert_eq!(s("..\\..\\boot.ini"), "boot.ini");
    }

    #[test]
    fn hostile_filename_absolute_paths_and_drive_letters() {
        assert_eq!(s("/etc/shadow"), "shadow");
        assert_eq!(s("C:\\Windows\\System32\\evil.dll"), "evil.dll");
        assert_eq!(s("C:evil.exe"), "C_evil.exe");
        assert_eq!(s("\\\\server\\share\\x.bin"), "x.bin");
        assert_eq!(s("\\\\?\\C:\\x.bin"), "x.bin");
        assert_eq!(s("D:"), "D_");
        assert_eq!(s("file.txt:stream"), "file.txt_stream");
    }

    #[test]
    fn hostile_filename_separators() {
        for raw in ["a/b.txt", "a\\b.txt", "x/y\\b.txt", "/b.txt", "b.txt/"] {
            let out = s(raw);
            assert_single_component(&out);
        }
        assert_eq!(s("a/b.txt"), "b.txt");
        assert_eq!(s("b.txt/"), "download");
    }

    #[test]
    fn hostile_filename_nul_and_control_chars() {
        assert_eq!(s("a\0b.txt"), "ab.txt");
        assert_eq!(s("a\r\nb\t.txt"), "ab.txt");
        assert_eq!(s("a\u{7f}\u{85}\u{9b}b.txt"), "ab.txt");
        assert_eq!(s("\0\0\0"), "download");
        assert_eq!(s("a<b>c|d?e*f\"g.txt"), "a_b_c_d_e_f_g.txt");
    }

    #[test]
    fn hostile_filename_bidi_override() {
        // "invoice[RLO]fdp.exe" displays as "invoiceexe.pdf".
        assert_eq!(s("invoice\u{202E}fdp.exe"), "invoicefdp.exe");
        for c in [
            '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}', '\u{2066}', '\u{2067}',
            '\u{2068}', '\u{2069}', '\u{200E}', '\u{200F}', '\u{061C}', '\u{FEFF}',
        ] {
            let out = s(&format!("a{c}b.txt"));
            assert_eq!(out, "ab.txt", "{:?}", c);
        }
    }

    #[test]
    fn hostile_filename_windows_reserved_names() {
        let mut names: Vec<String> = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
            .iter()
            .map(|n| (*n).to_owned())
            .collect();
        for d in 0..=9 {
            names.push(format!("COM{d}"));
            names.push(format!("LPT{d}"));
        }
        names.push("COM\u{B9}".to_owned());
        for base in names {
            for variant in [base.clone(), base.to_lowercase(), mixed_case(&base)] {
                for name in [
                    variant.clone(),
                    format!("{variant}.txt"),
                    format!("{variant}.tar.gz"),
                    format!("{variant} .txt"),
                    format!("{variant}."),
                ] {
                    let out = s(&name);
                    assert!(out.starts_with('_'), "{name:?} -> {out:?}");
                    assert_single_component(&out);
                }
            }
        }
        // Not reserved.
        assert_eq!(s("CONSOLE.txt"), "CONSOLE.txt");
        assert_eq!(s("COM10.txt"), "COM10.txt");
        assert_eq!(s("LPT.txt"), "LPT.txt");
        assert_eq!(s("nul2"), "nul2");
    }

    fn mixed_case(s: &str) -> String {
        s.chars()
            .enumerate()
            .map(|(i, c)| {
                if i % 2 == 0 {
                    c.to_ascii_lowercase()
                } else {
                    c.to_ascii_uppercase()
                }
            })
            .collect()
    }

    #[test]
    fn hostile_filename_trailing_dots_and_spaces() {
        assert_eq!(s("file.txt..."), "file.txt");
        assert_eq!(s("file.txt . . "), "file.txt");
        assert_eq!(s("   spaced.txt   "), "spaced.txt");
        assert_eq!(s(" . . . "), "download");
    }

    #[test]
    fn hostile_filename_too_long() {
        let max = limits::MAX_FILENAME_BYTES;
        let out = s(&format!("{}.mp4", "a".repeat(max * 3)));
        assert_eq!(out.len(), max);
        assert!(out.ends_with(".mp4"));
        // Multi-byte chars are cut on a boundary.
        let out = s(&format!("{}.zip", "é".repeat(max)));
        assert!(out.len() <= max);
        assert!(out.ends_with(".zip"));
        assert!(out.trim_end_matches(".zip").chars().all(|c| c == 'é'));
        // Huge extension: plain truncation.
        let out = s(&format!("a.{}", "x".repeat(max * 2)));
        assert_eq!(out.len(), max);
        // Truncation never leaves a trailing dot or space.
        let out = s(&format!("{}. .{}", "b".repeat(max - 3), "c".repeat(max)));
        assert_single_component(&out);
        // A name of only emoji.
        let out = s(&"😀".repeat(max));
        assert!(out.len() <= max && !out.is_empty());
    }

    #[test]
    fn hostile_filename_empty() {
        assert_eq!(s(""), "download");
        assert_eq!(s("   "), "download");
        assert_eq!(s("\u{202E}"), "download");
    }

    #[test]
    fn hostile_filename_dangerous_double_extension() {
        assert!(has_dangerous_double_extension("invoice.pdf.exe"));
        assert!(has_dangerous_double_extension("INVOICE.PDF.EXE"));
        assert!(has_dangerous_double_extension("photo.jpg.scr"));
        assert!(has_dangerous_double_extension("report.docx  .js"));
        // The bidi trick is neutralised by the sanitiser, which leaves an honest name.
        assert!(!has_dangerous_double_extension(&s(
            "invoice\u{202E}fdp.exe"
        )));
        assert!(!has_dangerous_double_extension("setup.exe"));
        assert!(!has_dangerous_double_extension("tool.v2.exe"));
        assert!(!has_dangerous_double_extension("archive.tar.gz"));
        assert!(!has_dangerous_double_extension("pdf.exe"));
        assert!(!has_dangerous_double_extension(""));
    }

    #[test]
    fn extensions() {
        assert_eq!(extension("a.ZIP").as_deref(), Some("zip"));
        assert_eq!(extension("a.tar.gz").as_deref(), Some("gz"));
        assert_eq!(extension(".bashrc"), None);
        assert_eq!(extension("noext"), None);
        assert_eq!(extension("trailing."), None);
    }

    #[test]
    fn url_filenames() {
        let u = Url::parse("https://example.com/dir/my%20file.zip?x=1#f").unwrap();
        assert_eq!(filename_from_url(&u).as_deref(), Some("my file.zip"));
        let u = Url::parse("https://example.com/dir/").unwrap();
        assert_eq!(filename_from_url(&u), None);
        let u = Url::parse("https://example.com").unwrap();
        assert_eq!(filename_from_url(&u), None);
        let u = Url::parse("https://example.com/..%2F..%2Fx.exe").unwrap();
        assert_eq!(sanitize_filename(&filename_from_url(&u).unwrap()), "x.exe");
    }

    #[test]
    fn auto_rename_forms() {
        assert_eq!(auto_rename("a.zip", 1), "a (1).zip");
        assert_eq!(auto_rename("a.tar.gz", 1), "a (1).tar.gz");
        assert_eq!(auto_rename("A.TAR.GZ", 3), "A (3).TAR.GZ");
        assert_eq!(auto_rename("noext", 2), "noext (2)");
        assert_eq!(auto_rename(".bashrc", 1), ".bashrc (1)");
        assert_eq!(auto_rename("a.zip", 0), "a.zip");
        assert_eq!(
            auto_rename("x.tar.gz", limits::MAX_AUTO_RENAME),
            "x (999).tar.gz"
        );
    }

    #[test]
    fn hostile_auto_rename_bounded() {
        let max = limits::MAX_FILENAME_BYTES;
        let long = sanitize_filename(&format!("{}.zip", "é".repeat(max)));
        let out = auto_rename(&long, limits::MAX_AUTO_RENAME);
        assert!(out.len() <= max, "{}", out.len());
        assert!(out.ends_with(" (999).zip"));
        let out = auto_rename(&format!("a.{}", "x".repeat(max)), u32::MAX);
        assert!(out.len() <= max);
        assert!(out.ends_with(&format!(" ({})", u32::MAX)));
    }
}
