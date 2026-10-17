//! Simple site grabber: breadth-first crawl of HTML pages up to a bounded
//! depth and page count, collecting links to files with chosen extensions.
//! Iterative (no recursion); every bound comes from `limits.rs`.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use serde::Serialize;
use udm_core::{Message, limits};
use udm_net::{Client, Request};
use url::Url;

use crate::model::StoredMessage;

/// What to crawl.
#[derive(Clone, Debug)]
pub struct GrabOptions {
    /// Start page.
    pub url: Url,
    /// Link depth (0 = only the start page), capped at `MAX_GRABBER_DEPTH`.
    pub depth: u32,
    /// File extensions to collect (lower case, no dot); empty = any non-HTML link.
    pub extensions: Vec<String>,
    /// Stay on the start host.
    pub same_host: bool,
}

/// State of a grabber job.
#[derive(Clone, Debug, Serialize)]
pub struct GrabJob {
    /// Job id.
    pub id: u64,
    /// Start URL.
    pub url: String,
    /// `running`, `done` or `failed`.
    pub state: String,
    /// Pages fetched.
    pub pages: u64,
    /// Files found.
    pub files: Vec<String>,
    /// Error of the start page, if it failed.
    pub error: Option<StoredMessage>,
}

/// Result of a crawl.
#[derive(Debug, Default)]
pub struct GrabResult {
    /// Pages fetched.
    pub pages: u64,
    /// File links found, in discovery order, unique.
    pub files: Vec<String>,
    /// Error fetching the start page (other page errors are skipped).
    pub error: Option<Message>,
}

fn looks_like_page(u: &Url) -> bool {
    let name = udm_proto::filename_from_url(u).unwrap_or_default();
    match udm_proto::extension(&name) {
        None => true,
        Some(e) => matches!(
            e.as_str(),
            "html" | "htm" | "php" | "asp" | "aspx" | "jsp" | "shtml"
        ),
    }
}

fn wanted(u: &Url, exts: &[String]) -> bool {
    let name = udm_proto::filename_from_url(u).unwrap_or_default();
    match udm_proto::extension(&name) {
        Some(e) if exts.is_empty() => !looks_like_page(u) || e.is_empty(),
        Some(e) => exts.iter().any(|x| x.eq_ignore_ascii_case(&e)),
        None => false,
    }
}

/// Crawls. Only http(s) links are followed.
pub async fn crawl(client: Arc<Client>, opts: GrabOptions) -> GrabResult {
    let depth_cap = opts.depth.min(limits::MAX_GRABBER_DEPTH);
    let max_pages = u64::try_from(limits::MAX_GRABBER_PAGES).unwrap_or(u64::MAX);
    let mut result = GrabResult::default();
    let mut seen_pages: HashSet<String> = HashSet::new();
    let mut seen_files: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(Url, u32)> = VecDeque::new();
    seen_pages.insert(opts.url.as_str().to_owned());
    queue.push_back((opts.url.clone(), 0));
    let host = opts.url.host_str().map(str::to_owned);

    while let Some((page, depth)) = queue.pop_front() {
        if result.pages >= max_pages {
            break;
        }
        let fetched = client
            .fetch_page(&Request::new(page.clone()), limits::MAX_GRABBER_PAGE_BYTES)
            .await;
        result.pages = result.pages.saturating_add(1);
        let (base, html) = match fetched {
            Ok(v) => v,
            Err(e) => {
                if depth == 0 {
                    result.error = Some(e.to_message());
                }
                continue;
            }
        };
        for link in udm_proto::extract_links(&html, &base)
            .into_iter()
            .take(limits::MAX_LINKS_PER_PAGE)
        {
            if !matches!(link.scheme(), "http" | "https") {
                continue;
            }
            if opts.same_host && link.host_str().map(str::to_owned) != host {
                continue;
            }
            if wanted(&link, &opts.extensions) {
                if result.files.len() < limits::MAX_BATCH_SIZE
                    && seen_files.insert(link.as_str().to_owned())
                {
                    result.files.push(link.as_str().to_owned());
                }
            } else if looks_like_page(&link)
                && depth < depth_cap
                && seen_pages.len() < limits::MAX_GRABBER_PAGES
                && seen_pages.insert(link.as_str().to_owned())
            {
                queue.push_back((link, depth.saturating_add(1)));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_and_file_detection() {
        let u = |s: &str| Url::parse(s).unwrap();
        assert!(looks_like_page(&u("http://h/a/")));
        assert!(looks_like_page(&u("http://h/a/index.html")));
        assert!(!looks_like_page(&u("http://h/a/f.zip")));
        assert!(wanted(&u("http://h/f.zip"), &["zip".into()]));
        assert!(!wanted(&u("http://h/f.zip"), &["pdf".into()]));
        assert!(wanted(&u("http://h/f.pdf"), &[]));
        assert!(!wanted(&u("http://h/x.html"), &[]));
    }
}
