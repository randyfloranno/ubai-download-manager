//! Link extraction (site grabber, clipboard), URL lists and batch patterns.
//! All scanners are linear, iterative and bounded by `limits`.

use std::collections::HashSet;

use ::url::Url;
use udm_core::limits;

use crate::error::{ProtoError, usize_to_u64};
use crate::text::{parse_u64_strict, truncate_bytes};
use crate::url::validate_download_url;

/// Schemes the grabber follows or lists.
const LINK_SCHEMES: [&str; 4] = ["http", "https", "ftp", "ftps"];

/// Extracts `href`/`src` targets from HTML, resolved against `base` (or a `<base href>`).
///
/// Only `http`, `https`, `ftp` and `ftps` links are returned, without fragments,
/// deduplicated in document order. The input is truncated to
/// `limits::MAX_GRABBER_PAGE_BYTES` and at most `limits::MAX_LINKS_PER_PAGE` links are returned.
pub fn extract_links(html: &str, base: &Url) -> Vec<Url> {
    let html = truncate_bytes(html, limits::MAX_GRABBER_PAGE_BYTES);
    let mut base = base.clone();
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    let mut scanner = TagScanner::new(html);
    while let Some(tag) = scanner.next_tag() {
        for (name, value) in tag.attrs {
            let is_link = name.eq_ignore_ascii_case("href") || name.eq_ignore_ascii_case("src");
            if !is_link || value.len() > limits::MAX_URL_BYTES {
                continue;
            }
            let value = decode_entities(value);
            let Ok(mut u) = base.join(value.trim()) else {
                continue;
            };
            if !LINK_SCHEMES.contains(&u.scheme()) || u.host_str().is_none_or(str::is_empty) {
                continue;
            }
            u.set_fragment(None);
            if tag.name.eq_ignore_ascii_case("base") && name.eq_ignore_ascii_case("href") {
                base = u;
                continue;
            }
            if u.as_str().len() <= limits::MAX_URL_BYTES && seen.insert(u.as_str().to_owned()) {
                out.push(u);
                if out.len() >= limits::MAX_LINKS_PER_PAGE {
                    return out;
                }
            }
        }
    }
    out
}

struct Tag<'a> {
    name: &'a str,
    attrs: Vec<(&'a str, &'a str)>,
}

/// Minimal, forgiving HTML tag/attribute scanner. Works on byte offsets of ASCII
/// delimiters only, so every slice falls on a char boundary.
struct TagScanner<'a> {
    src: &'a str,
    pos: usize,
}

/// Attributes kept per tag (the rest of the tag is still skipped linearly).
const MAX_ATTRS_PER_TAG: usize = 64;

impl<'a> TagScanner<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, pos: 0 }
    }

    fn byte(&self, i: usize) -> Option<u8> {
        self.src.as_bytes().get(i).copied()
    }

    fn find_from(&self, from: usize, needle: &str) -> Option<usize> {
        let rest = self.src.get(from..)?;
        rest.find(needle).and_then(|i| i.checked_add(from))
    }

    fn skip_while(&mut self, f: impl Fn(u8) -> bool) {
        while self.byte(self.pos).is_some_and(&f) {
            self.pos = self.pos.saturating_add(1);
        }
    }

    fn next_tag(&mut self) -> Option<Tag<'a>> {
        loop {
            let lt = self.find_from(self.pos, "<")?;
            self.pos = lt.saturating_add(1);
            if self
                .src
                .get(self.pos..)
                .is_some_and(|r| r.starts_with("!--"))
            {
                // Comment: skip to "-->" (or the end).
                self.pos = self
                    .find_from(self.pos, "-->")
                    .map_or(self.src.len(), |i| i.saturating_add(3));
                continue;
            }
            let name_start = self.pos;
            self.skip_while(|b| b.is_ascii_alphanumeric());
            if self.pos == name_start {
                continue; // "</x>", "<!DOCTYPE", "< 3", ...
            }
            let name = self.src.get(name_start..self.pos).unwrap_or("");
            let mut attrs = Vec::new();
            loop {
                self.skip_while(|b| b.is_ascii_whitespace() || b == b'/');
                match self.byte(self.pos) {
                    None => return Some(Tag { name, attrs }),
                    Some(b'>') => {
                        self.pos = self.pos.saturating_add(1);
                        break;
                    }
                    Some(_) => {}
                }
                let an_start = self.pos;
                self.skip_while(|b| !(b.is_ascii_whitespace() || matches!(b, b'=' | b'>' | b'/')));
                let attr_name = self.src.get(an_start..self.pos).unwrap_or("");
                if self.pos == an_start {
                    // Lone '=' or similar: make progress.
                    self.pos = self.pos.saturating_add(1);
                    continue;
                }
                self.skip_while(|b| b.is_ascii_whitespace());
                if self.byte(self.pos) != Some(b'=') {
                    continue; // boolean attribute
                }
                self.pos = self.pos.saturating_add(1);
                self.skip_while(|b| b.is_ascii_whitespace());
                let value = match self.byte(self.pos) {
                    Some(q @ (b'"' | b'\'')) => {
                        let v_start = self.pos.saturating_add(1);
                        let quote = if q == b'"' { "\"" } else { "'" };
                        let v_end = self.find_from(v_start, quote).unwrap_or(self.src.len());
                        self.pos = v_end.saturating_add(1).min(self.src.len());
                        self.src.get(v_start..v_end).unwrap_or("")
                    }
                    _ => {
                        let v_start = self.pos;
                        self.skip_while(|b| !(b.is_ascii_whitespace() || b == b'>'));
                        self.src.get(v_start..self.pos).unwrap_or("")
                    }
                };
                if attrs.len() < MAX_ATTRS_PER_TAG {
                    attrs.push((attr_name, value));
                }
            }
            return Some(Tag { name, attrs });
        }
    }
}

/// Decodes the few character references that occur in URLs (`&amp;`, `&quot;`,
/// `&apos;`, `&lt;`, `&gt;`, `&#NN;`, `&#xHH;`). Unknown references stay as-is.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        let (before, after) = rest.split_at_checked(amp).unwrap_or((rest, ""));
        out.push_str(before);
        let after_amp = after.get(1..).unwrap_or("");
        let semi = after_amp.find(';').filter(|i| *i <= 10);
        let decoded = semi.and_then(|i| after_amp.get(..i)).and_then(|name| {
            let c = match name {
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "lt" => Some('<'),
                "gt" => Some('>'),
                n => n.strip_prefix('#').and_then(|num| {
                    let v = match num.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => num.parse::<u32>().ok(),
                    };
                    v.and_then(char::from_u32).filter(|c| !c.is_control())
                }),
            };
            c.map(|c| (c, name.len()))
        });
        match decoded {
            Some((c, n)) => {
                out.push(c);
                rest = after_amp.get(n.saturating_add(1)..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after_amp;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Finds download URLs in free text (clipboard). Input is truncated to
/// `limits::MAX_CLIPBOARD_BYTES`; results are validated, deduplicated, in order,
/// and capped at `limits::MAX_BATCH_SIZE`.
pub fn extract_urls_from_text(text: &str) -> Vec<String> {
    let text = truncate_bytes(text, limits::MAX_CLIPBOARD_BYTES);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let is_delim =
        |c: char| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '"' | '\'' | '`');
    for token in text.split(is_delim) {
        let lower = token.to_ascii_lowercase();
        let start = ["https://", "http://", "ftps://", "ftp://"]
            .iter()
            .filter_map(|p| lower.find(p))
            .min();
        let Some(start) = start else { continue };
        let Some(candidate) = token.get(start..) else {
            continue;
        };
        let mut candidate = candidate.trim_end_matches(['.', ',', ';', ':', '!', '?', ']', '}']);
        if candidate.ends_with(')') && !candidate.contains('(') {
            candidate = candidate.trim_end_matches(')');
        }
        let Ok(u) = validate_download_url(candidate) else {
            continue;
        };
        if seen.insert(u.as_str().to_owned()) {
            out.push(u.as_str().to_owned());
            if out.len() >= limits::MAX_BATCH_SIZE {
                break;
            }
        }
    }
    out
}

/// Parses a URL list file: optional UTF-8 BOM, one entry per line, blank lines and
/// `#` comments ignored, entries trimmed. Entries are **not** validated here (the caller
/// reports invalid lines individually with [`validate_download_url`]).
///
/// Errors: input larger than `limits::MAX_URL_LIST_BYTES`, more than `limits::MAX_BATCH_SIZE` entries.
pub fn parse_url_list(text: &str) -> Result<Vec<String>, ProtoError> {
    let max = limits::MAX_URL_LIST_BYTES;
    if text.len() > max {
        return Err(ProtoError::UrlListTooLarge {
            bytes: usize_to_u64(text.len()),
            max: usize_to_u64(max),
        });
    }
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if out.len() >= limits::MAX_BATCH_SIZE {
            let count = text
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
                .count();
            return Err(ProtoError::BatchTooLarge {
                count: usize_to_u64(count),
                max: usize_to_u64(limits::MAX_BATCH_SIZE),
            });
        }
        out.push(line.to_owned());
    }
    Ok(out)
}

enum Piece<'a> {
    Lit(&'a str),
    Num { start: u64, end: u64, width: usize },
    Alpha { start: char, end: char },
}

/// Expands a batch pattern such as `http://h/img[001-100].jpg` or `http://h/[a-z].txt`.
///
/// Numeric groups keep the width of their start (`[001-100]` → `001`…`100`); letter
/// groups use one case. Several groups expand as a cartesian product (leftmost varies
/// slowest). A `[` that does not start a valid group (e.g. an IPv6 host) stays literal.
/// Errors: descending or mixed groups, pattern longer than `limits::MAX_URL_BYTES`,
/// more than `limits::MAX_BATCH_SIZE` results.
pub fn expand_batch_pattern(p: &str) -> Result<Vec<String>, ProtoError> {
    let max_url = limits::MAX_URL_BYTES;
    if p.len() > max_url {
        return Err(ProtoError::UrlTooLong {
            len: p.len(),
            max: max_url,
        });
    }
    let pieces = parse_pattern(p)?;
    let mut total: u64 = 1;
    for piece in &pieces {
        let n = piece_len(piece).ok_or(ProtoError::BatchPatternInvalid)?;
        total = total.saturating_mul(n);
    }
    let max = usize_to_u64(limits::MAX_BATCH_SIZE);
    if total > max {
        return Err(ProtoError::BatchTooLarge { count: total, max });
    }
    // Odometer over the groups, rightmost fastest.
    let mut idx: Vec<u64> = vec![0; pieces.len()];
    let mut out = Vec::with_capacity(usize::try_from(total).unwrap_or(0));
    loop {
        let mut s = String::with_capacity(p.len());
        for (piece, i) in pieces.iter().zip(idx.iter()) {
            match piece {
                Piece::Lit(l) => s.push_str(l),
                Piece::Num { start, width, .. } => {
                    let v = start.saturating_add(*i);
                    s.push_str(&format!("{v:0width$}", width = *width));
                }
                Piece::Alpha { start, .. } => {
                    let c = u32::try_from(*i)
                        .ok()
                        .and_then(|i| u32::from(*start).checked_add(i))
                        .and_then(char::from_u32)
                        .unwrap_or(*start);
                    s.push(c);
                }
            }
        }
        out.push(s);
        // Advance.
        let mut carry = true;
        for (piece, i) in pieces.iter().zip(idx.iter_mut()).rev() {
            if !carry {
                break;
            }
            let n = piece_len(piece).unwrap_or(1);
            let next = i.saturating_add(1);
            if next < n {
                *i = next;
                carry = false;
            } else {
                *i = 0;
            }
        }
        if carry || out.len() >= limits::MAX_BATCH_SIZE {
            break;
        }
    }
    Ok(out)
}

fn piece_len(p: &Piece<'_>) -> Option<u64> {
    match p {
        Piece::Lit(_) => Some(1),
        Piece::Num { start, end, .. } => end.checked_sub(*start)?.checked_add(1),
        Piece::Alpha { start, end } => {
            u64::from(u32::from(*end).checked_sub(u32::from(*start))?).checked_add(1)
        }
    }
}

/// Longest digit run accepted in a numeric group.
const MAX_GROUP_DIGITS: usize = 18;

fn parse_pattern(p: &str) -> Result<Vec<Piece<'_>>, ProtoError> {
    let mut pieces = Vec::new();
    let mut lit_start = 0usize;
    let mut search = 0usize;
    while let Some(rel) = p.get(search..).and_then(|r| r.find('[')) {
        let open = search.saturating_add(rel);
        let inner_start = open.saturating_add(1);
        let close = p
            .get(inner_start..)
            .and_then(|r| r.find(']'))
            .map(|c| inner_start.saturating_add(c));
        let group = close
            .and_then(|c| p.get(inner_start..c))
            .and_then(|inner| inner.split_once('-'));
        let parsed = match group {
            Some((a, b)) => parse_group(a, b)?,
            None => None,
        };
        match (parsed, close) {
            (Some(piece), Some(c)) => {
                if let Some(lit) = p.get(lit_start..open).filter(|l| !l.is_empty()) {
                    pieces.push(Piece::Lit(lit));
                }
                pieces.push(piece);
                lit_start = c.saturating_add(1);
                search = lit_start;
            }
            _ => search = inner_start,
        }
    }
    if let Some(lit) = p.get(lit_start..).filter(|l| !l.is_empty()) {
        pieces.push(Piece::Lit(lit));
    }
    Ok(pieces)
}

/// `Ok(None)`: not a group (stays literal). `Err`: looks like a group but is invalid.
fn parse_group<'a>(a: &str, b: &str) -> Result<Option<Piece<'a>>, ProtoError> {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    if digits(a) && digits(b) {
        if a.len() > MAX_GROUP_DIGITS || b.len() > MAX_GROUP_DIGITS {
            return Err(ProtoError::BatchPatternInvalid);
        }
        let start = parse_u64_strict(a).ok_or(ProtoError::BatchPatternInvalid)?;
        let end = parse_u64_strict(b).ok_or(ProtoError::BatchPatternInvalid)?;
        if start > end {
            return Err(ProtoError::BatchPatternInvalid);
        }
        return Ok(Some(Piece::Num {
            start,
            end,
            width: a.len(),
        }));
    }
    let mut ac = a.chars();
    let mut bc = b.chars();
    let (Some(x), None, Some(y), None) = (ac.next(), ac.next(), bc.next(), bc.next()) else {
        return Ok(None);
    };
    if x.is_ascii_alphabetic() && y.is_ascii_alphabetic() {
        let same_case = x.is_ascii_lowercase() == y.is_ascii_lowercase();
        if !same_case || x > y {
            return Err(ProtoError::BatchPatternInvalid);
        }
        return Ok(Some(Piece::Alpha { start: x, end: y }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://example.com/dir/page.html").unwrap()
    }

    #[test]
    fn links_resolved_and_deduplicated() {
        let html = r#"<html><a href="a.zip">A</a> <A HREF='/b.iso'>B</A>
            <img src=//cdn.example.com/c.png alt=x> <a href="a.zip#frag">dup</a>
            <a href="mailto:x@y"> <a href="javascript:void(0)"> <a href="data:,x">
            <!-- <a href="hidden.zip"> --> <a href="ftp://ftp.example.com/d.bin">
            <a href="q?x=1&amp;y=2"> <a href = "  spaced.bin  " ></html>"#;
        let links: Vec<String> = extract_links(html, &base())
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(
            links,
            vec![
                "https://example.com/dir/a.zip",
                "https://example.com/b.iso",
                "https://cdn.example.com/c.png",
                "ftp://ftp.example.com/d.bin",
                "https://example.com/dir/q?x=1&y=2",
                "https://example.com/dir/spaced.bin",
            ]
        );
    }

    #[test]
    fn base_href_respected() {
        let html = r#"<base href="https://mirror.example.org/pub/"><a href="x.bin">"#;
        let links = extract_links(html, &base());
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].as_str(), "https://mirror.example.org/pub/x.bin");
    }

    #[test]
    fn hostile_links_malformed_html() {
        for html in [
            "<",
            "<a",
            "<a href",
            "<a href=",
            "<a href=\"",
            "<a href='x",
            "<<<<>>>>",
            "<!--",
            "<a ==== href=x>",
            "< a href=x>",
            "<a href=\"\u{1F600}.zip\">",
            "<a\u{0}href=x>",
        ] {
            let _ = extract_links(html, &base());
        }
        let links = extract_links("<a href=\"x\u{e9}.zip", &base());
        assert_eq!(links.len(), 1);
    }

    #[test]
    fn hostile_links_bounded_count() {
        let mut html = String::new();
        for i in 0..(limits::MAX_LINKS_PER_PAGE + 50) {
            html.push_str(&format!("<a href=\"/f{i}\">"));
        }
        assert!(html.len() <= limits::MAX_GRABBER_PAGE_BYTES);
        assert_eq!(
            extract_links(&html, &base()).len(),
            limits::MAX_LINKS_PER_PAGE
        );
    }

    #[test]
    fn hostile_links_page_bytes_truncated() {
        let mut html = "x".repeat(limits::MAX_GRABBER_PAGE_BYTES - 3);
        html.push_str("é<a href=\"/after-limit.zip\">");
        assert!(extract_links(&html, &base()).is_empty());
        let mut html = "<a href=\"/early.zip\">".to_owned();
        html.push_str(&"<b ".repeat(limits::MAX_GRABBER_PAGE_BYTES / 3));
        assert_eq!(extract_links(&html, &base()).len(), 1);
    }

    #[test]
    fn entities() {
        assert_eq!(
            decode_entities("a&amp;b&#38;c&#x26;d&unknown;e&"),
            "a&b&c&d&unknown;e&"
        );
        assert_eq!(decode_entities("&#0;&#x110000;"), "&#0;&#x110000;");
    }

    #[test]
    fn clipboard_urls() {
        let t = "see https://example.com/a.zip, and (http://x.org/b.iso). \
                 dup https://example.com/a.zip <ftp://f.net/c> text:https://w.org/wiki/A_(b) file:///x";
        assert_eq!(
            extract_urls_from_text(t),
            vec![
                "https://example.com/a.zip",
                "http://x.org/b.iso",
                "ftp://f.net/c",
                "https://w.org/wiki/A_(b)",
            ]
        );
        assert!(extract_urls_from_text("nothing here").is_empty());
    }

    #[test]
    fn hostile_clipboard_text_bounded() {
        let mut t = " ".repeat(limits::MAX_CLIPBOARD_BYTES);
        t.push_str("https://example.com/after.zip");
        assert!(extract_urls_from_text(&t).is_empty());
        let t = "https://a.b/xyz ".repeat(limits::MAX_CLIPBOARD_BYTES);
        assert_eq!(extract_urls_from_text(&t).len(), 1);
        let t: String = (0..limits::MAX_CLIPBOARD_BYTES / 16)
            .map(|i| format!("http://h/{i} "))
            .collect();
        assert!(extract_urls_from_text(&t).len() <= limits::MAX_BATCH_SIZE);
    }

    #[test]
    fn url_list_parsing() {
        let t = "\u{FEFF}# list\r\nhttps://a/1\r\n\r\n  https://a/2  \n#c\nftp://b/3";
        assert_eq!(
            parse_url_list(t).unwrap(),
            vec!["https://a/1", "https://a/2", "ftp://b/3"]
        );
        assert!(parse_url_list("").unwrap().is_empty());
    }

    #[test]
    fn hostile_url_list_too_large() {
        let t = "a".repeat(limits::MAX_URL_LIST_BYTES + 1);
        assert!(matches!(
            parse_url_list(&t),
            Err(ProtoError::UrlListTooLarge { .. })
        ));
        let t = "http://h/x\n".repeat(limits::MAX_BATCH_SIZE + 1);
        let e = parse_url_list(&t).unwrap_err();
        assert_eq!(
            e,
            ProtoError::BatchTooLarge {
                count: (limits::MAX_BATCH_SIZE + 1) as u64,
                max: limits::MAX_BATCH_SIZE as u64
            }
        );
        let t = "http://h/x\n".repeat(limits::MAX_BATCH_SIZE);
        assert_eq!(parse_url_list(&t).unwrap().len(), limits::MAX_BATCH_SIZE);
    }

    #[test]
    fn batch_patterns() {
        let v = expand_batch_pattern("http://h/img[001-003].jpg").unwrap();
        assert_eq!(
            v,
            vec![
                "http://h/img001.jpg",
                "http://h/img002.jpg",
                "http://h/img003.jpg"
            ]
        );
        let v = expand_batch_pattern("http://h/[8-10]").unwrap();
        assert_eq!(v, vec!["http://h/8", "http://h/9", "http://h/10"]);
        let v = expand_batch_pattern("http://h/[a-c][1-2].txt").unwrap();
        assert_eq!(v.len(), 6);
        assert_eq!(v[0], "http://h/a1.txt");
        assert_eq!(v[1], "http://h/a2.txt");
        assert_eq!(v[5], "http://h/c2.txt");
        let v = expand_batch_pattern("http://[::1]:8080/f[1-2]").unwrap();
        assert_eq!(v, vec!["http://[::1]:8080/f1", "http://[::1]:8080/f2"]);
        assert_eq!(
            expand_batch_pattern("http://h/x").unwrap(),
            vec!["http://h/x"]
        );
        assert_eq!(
            expand_batch_pattern("http://h/[x]").unwrap(),
            vec!["http://h/[x]"]
        );
    }

    #[test]
    fn hostile_batch_pattern_too_large() {
        let p = format!("http://h/[1-{}]", limits::MAX_BATCH_SIZE + 1);
        let e = expand_batch_pattern(&p).unwrap_err();
        assert!(matches!(e, ProtoError::BatchTooLarge { .. }));
        // Product overflow across groups saturates and is rejected without allocating.
        let p = format!("http://h/{}", "[0-999999999999999999]".repeat(8));
        assert!(matches!(
            expand_batch_pattern(&p),
            Err(ProtoError::BatchTooLarge { .. })
        ));
        let p = format!("http://h/[1-{}]", limits::MAX_BATCH_SIZE);
        assert_eq!(
            expand_batch_pattern(&p).unwrap().len(),
            limits::MAX_BATCH_SIZE
        );
        let p = format!("http://h/{}", "a".repeat(limits::MAX_URL_BYTES));
        assert!(matches!(
            expand_batch_pattern(&p),
            Err(ProtoError::UrlTooLong { .. })
        ));
    }

    #[test]
    fn hostile_batch_pattern_invalid() {
        for p in [
            "http://h/[9-1]",
            "http://h/[z-a]",
            "http://h/[a-Z]",
            "http://h/[1-9999999999999999999]",
        ] {
            assert_eq!(
                expand_batch_pattern(p).unwrap_err(),
                ProtoError::BatchPatternInvalid,
                "{p}"
            );
        }
        for p in ["[", "]", "[[[[", "[-]", "[1-]", "[-1]", "[é-ü]", "[1-2"] {
            assert!(expand_batch_pattern(p).is_ok(), "{p}");
        }
    }
}
