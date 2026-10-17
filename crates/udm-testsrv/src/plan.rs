//! Protocol-independent routing: a request becomes a [`Plan`] (status, headers,
//! body generator with faults) that the HTTP/1.1 and HTTP/2 transports execute.

use crate::fixture::{DEFAULT_VERSION, Fixture, fixture_version, to_hex};
use crate::util::{Req, crc32_update, parse_query};
use crate::{AUTH_PASSWORD, AUTH_REALM, AUTH_USER};
use base64::Engine as _;
use bytes::{BufMut, Bytes, BytesMut};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

/// Raw chunk size used by the body generator (also the gzip stored-block size).
const CHUNK: u64 = 16 * 1024;

/// Size of the single header sent by `huge_header=1`.
pub const HUGE_HEADER_BYTES: usize = 1024 * 1024;

/// Shared mutable state of one HTTP test server instance.
pub(crate) struct HttpState {
    pub port: u16,
    pub tls: bool,
    pub versions: Mutex<HashMap<String, u32>>,
    pub fault_hits: Mutex<HashMap<String, u32>>,
    pub nonces: Mutex<HashSet<String>>,
    pub nonce_ctr: AtomicU64,
    pub downgrade_target: Mutex<Option<String>>,
    pub requests: AtomicUsize,
}

impl HttpState {
    pub fn new(port: u16, tls: bool) -> Self {
        HttpState {
            port,
            tls,
            versions: Mutex::new(HashMap::new()),
            fault_hits: Mutex::new(HashMap::new()),
            nonces: Mutex::new(HashSet::new()),
            nonce_ctr: AtomicU64::new(0),
            downgrade_target: Mutex::new(None),
            requests: AtomicUsize::new(0),
        }
    }

    pub fn version(&self, name: &str) -> u32 {
        self.versions
            .lock()
            .ok()
            .and_then(|v| v.get(name).copied())
            .unwrap_or(DEFAULT_VERSION)
    }

    pub fn fixture(&self, name: &str) -> Option<Fixture> {
        fixture_version(name, self.version(name))
    }

    /// Returns true when the fault keyed by `key` should still fire
    /// (fires for the first `times` requests).
    fn take_fault(&self, key: &str, times: u32) -> bool {
        let Ok(mut hits) = self.fault_hits.lock() else {
            return true;
        };
        let n = hits.entry(key.to_string()).or_insert(0);
        if *n < times {
            *n += 1;
            true
        } else {
            false
        }
    }

    fn new_nonce(&self) -> String {
        let c = self.nonce_ctr.fetch_add(1, Ordering::Relaxed);
        let mut h = Sha256::new();
        h.update(self.port.to_le_bytes());
        h.update(c.to_le_bytes());
        h.update(b"udm-testsrv-nonce");
        let n = to_hex(&h.finalize())[..32].to_string();
        if let Ok(mut set) = self.nonces.lock() {
            set.insert(n.clone());
        }
        n
    }
}

/// What a transport must do for one request.
pub(crate) enum Plan {
    /// Accept the request and never answer.
    Stall,
    Respond(Resp),
}

pub(crate) struct Resp {
    pub status: u16,
    pub headers: Vec<(String, Vec<u8>)>,
    pub body: BodySpec,
    pub head_only: bool,
    /// Stream headers forever (HTTP/1.1); on HTTP/2 a very long header list.
    pub endless_headers: bool,
    /// Close (h1) / reset (h2) after this many body bytes.
    pub cut: Option<u64>,
    /// Throttle in bytes per second.
    pub slow: Option<u64>,
}

pub(crate) enum BodySpec {
    Bytes(Vec<u8>),
    Fixture {
        fx: Fixture,
        start: u64,
        /// Raw bytes to send (range length + extra bytes).
        raw_len: u64,
        gzip: bool,
    },
}

impl Resp {
    fn simple(status: u16, ctype: &str, body: impl Into<Vec<u8>>) -> Self {
        let body = body.into();
        Resp {
            status,
            headers: vec![
                ("Content-Type".into(), ctype.as_bytes().to_vec()),
                ("Content-Length".into(), body.len().to_string().into_bytes()),
            ],
            body: BodySpec::Bytes(body),
            head_only: false,
            endless_headers: false,
            cut: None,
            slow: None,
        }
    }

    fn redirect(location: &str) -> Self {
        let mut r = Resp::simple(302, "text/plain", "redirect\n");
        let loc: Vec<u8> = location
            .bytes()
            .filter(|b| *b != b'\r' && *b != b'\n')
            .collect();
        r.headers.push(("Location".into(), loc));
        r
    }

    fn header(mut self, name: &str, value: impl Into<Vec<u8>>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }
}

/// Reason phrase for a status code.
pub(crate) fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        416 => "Range Not Satisfiable",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Status",
    }
}

fn q_get<'a>(q: &'a [(String, Vec<u8>)], key: &str) -> Option<&'a [u8]> {
    q.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_slice())
}

fn q_flag(q: &[(String, Vec<u8>)], key: &str) -> bool {
    q_get(q, key).is_some_and(|v| v != b"0" && v != b"false")
}

fn q_num(q: &[(String, Vec<u8>)], key: &str) -> Option<u64> {
    q_get(q, key).and_then(|v| std::str::from_utf8(v).ok()?.parse().ok())
}

fn with_query(path: &str, query: &str) -> String {
    if query.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{query}")
    }
}

/// Routes one request.
pub(crate) fn route(state: &HttpState, req: &Req) -> Plan {
    let head_only = req.method == "HEAD";
    if req.method != "GET" && !head_only {
        return Plan::Respond(
            Resp::simple(405, "text/plain", "method not allowed\n").header("Allow", "GET, HEAD"),
        );
    }
    let mut plan = route_inner(state, req);
    if let Plan::Respond(r) = &mut plan {
        r.head_only = head_only;
    }
    plan
}

fn route_inner(state: &HttpState, req: &Req) -> Plan {
    let path = req.path.as_str();
    let q = parse_query(&req.query);
    if path == "/" {
        return Plan::Respond(Resp::simple(200, "text/plain", "udm-testsrv\n"));
    }
    if let Some(name) = path.strip_prefix("/files/") {
        return serve_fixture(state, req, name, &q, Vec::new());
    }
    if path == "/redirect-loop" {
        return Plan::Respond(Resp::redirect(&with_query("/redirect-loop", &req.query)));
    }
    if path == "/redirect-abs" {
        return match q_get(&q, "to") {
            Some(to) => Plan::Respond(Resp::redirect(&String::from_utf8_lossy(to))),
            None => Plan::Respond(Resp::simple(400, "text/plain", "missing to\n")),
        };
    }
    if let Some(rest) = path.strip_prefix("/redirect/") {
        let Some((n, tail)) = rest.split_once('/') else {
            return not_found();
        };
        let Ok(n) = n.parse::<u32>() else {
            return not_found();
        };
        let loc = if n <= 1 {
            format!("/{tail}")
        } else {
            format!("/redirect/{}/{tail}", n - 1)
        };
        return Plan::Respond(Resp::redirect(&with_query(&loc, &req.query)));
    }
    if path == "/downgrade" || path.starts_with("/downgrade/") {
        let base = state
            .downgrade_target
            .lock()
            .ok()
            .and_then(|t| t.clone())
            .unwrap_or_else(|| format!("http://127.0.0.1:{}", state.port));
        let tail = path.strip_prefix("/downgrade").unwrap_or("");
        let tail = if tail.is_empty() || tail == "/" {
            "/files/tiny.txt"
        } else {
            tail
        };
        let loc = format!(
            "{}{}",
            base.trim_end_matches('/'),
            with_query(tail, &req.query)
        );
        return Plan::Respond(Resp::redirect(&loc));
    }
    if path == "/disposition" {
        let file = q_get(&q, "file")
            .map(|f| String::from_utf8_lossy(f).into_owned())
            .unwrap_or_else(|| "tiny.txt".to_string());
        let value: Vec<u8> = q_get(&q, "v")
            .unwrap_or(b"attachment")
            .iter()
            .copied()
            .filter(|b| *b != b'\r' && *b != b'\n')
            .collect();
        return serve_fixture(
            state,
            req,
            &file,
            &q,
            vec![("Content-Disposition".into(), value)],
        );
    }
    if let Some(name) = path.strip_prefix("/auth/basic/files/") {
        if basic_ok(req) {
            return serve_fixture(state, req, name, &q, Vec::new());
        }
        return Plan::Respond(
            Resp::simple(401, "text/plain", "unauthorized\n")
                .header("WWW-Authenticate", format!("Basic realm=\"{AUTH_REALM}\"")),
        );
    }
    if let Some(name) = path.strip_prefix("/auth/digest/files/") {
        if digest_ok(state, req) {
            return serve_fixture(state, req, name, &q, Vec::new());
        }
        let nonce = state.new_nonce();
        return Plan::Respond(Resp::simple(401, "text/plain", "unauthorized\n").header(
            "WWW-Authenticate",
            format!(
                "Digest realm=\"{AUTH_REALM}\", qop=\"auth\", algorithm=SHA-256, nonce=\"{nonce}\", opaque=\"udm-opaque\""
            ),
        ));
    }
    if let Some(page) = path.strip_prefix("/maze/") {
        let fan = q_num(&q, "fan").unwrap_or(2u64).clamp(1, 20);
        return match page
            .strip_suffix(".html")
            .and_then(|n| n.parse::<u64>().ok())
        {
            Some(n) => Plan::Respond(Resp::simple(
                200,
                "text/html; charset=utf-8",
                maze_page(n, fan),
            )),
            None => not_found(),
        };
    }
    if let Some(page) = path.strip_prefix("/site/") {
        return match site_page(page) {
            Some(html) => Plan::Respond(Resp::simple(200, "text/html; charset=utf-8", html)),
            None => not_found(),
        };
    }
    if let Some(code) = path.strip_prefix("/status/") {
        return match code.parse::<u16>() {
            Ok(c) if (200..=599).contains(&c) => {
                let body = if c == 204 || c == 304 {
                    String::new()
                } else {
                    format!("status {c}\n")
                };
                let mut r = Resp::simple(c, "text/plain", body);
                if c == 204 || c == 304 {
                    r.headers.retain(|(k, _)| k != "Content-Length");
                }
                Plan::Respond(r)
            }
            _ => Plan::Respond(Resp::simple(400, "text/plain", "bad status\n")),
        };
    }
    not_found()
}

fn not_found() -> Plan {
    Plan::Respond(Resp::simple(404, "text/plain", "not found\n"))
}

/// Parses a `Range` header against `total`.
/// `Ok(None)`: not a bytes range (ignored). `Err(())`: unsatisfiable/invalid.
pub(crate) fn parse_range(h: &str, total: u64) -> Result<Option<(u64, u64)>, ()> {
    let h = h.trim();
    let Some(spec) = h.strip_prefix("bytes=") else {
        return Ok(None);
    };
    let spec = spec.trim();
    if spec.contains(',') {
        return Err(());
    }
    let (a, b) = spec.split_once('-').ok_or(())?;
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() {
        let n: u64 = b.parse().map_err(|_| ())?;
        if n == 0 || total == 0 {
            return Err(());
        }
        let n = n.min(total);
        return Ok(Some((total - n, total - 1)));
    }
    let a: u64 = a.parse().map_err(|_| ())?;
    if a >= total {
        return Err(());
    }
    let b = if b.is_empty() {
        total - 1
    } else {
        let b: u64 = b.parse().map_err(|_| ())?;
        if b < a {
            return Err(());
        }
        b.min(total - 1)
    };
    Ok(Some((a, b)))
}

/// Total wire length of a gzip member made of stored deflate blocks.
pub fn gzip_len(raw: u64) -> u64 {
    10 + raw + 5 * raw.div_ceil(CHUNK) + 5 + 8
}

fn serve_fixture(
    state: &HttpState,
    req: &Req,
    name: &str,
    q: &[(String, Vec<u8>)],
    extra_headers: Vec<(String, Vec<u8>)>,
) -> Plan {
    let Some(fx) = state.fixture(name) else {
        return not_found();
    };
    if q_flag(q, "stall") {
        return Plan::Stall;
    }
    let weak = q_flag(q, "weak_etag");
    let no_validators = q_flag(q, "no_validators");
    let no_range = q_flag(q, "no_range");
    let ignore_range = q_flag(q, "ignore_range");
    let gzip = q_flag(q, "gzip");
    let extra = q_num(q, "extra_bytes").unwrap_or(0);
    let etag = if weak { fx.weak_etag() } else { fx.etag() };
    let last_modified = fx.last_modified();
    let total = fx.len;

    let mut status = 200u16;
    let mut start = 0u64;
    let mut len = total;
    let mut content_range: Option<String> = None;

    if let Some(range) = req.header("range").filter(|_| !no_range && !ignore_range) {
        let if_range_ok = match req.header("if-range") {
            None => true,
            Some(ir) => {
                let ir = ir.trim();
                !no_validators
                    && ((!weak && !ir.starts_with("W/") && ir == etag) || ir == last_modified)
            }
        };
        if if_range_ok {
            match parse_range(range, total) {
                Ok(Some((a, b))) => {
                    status = 206;
                    start = a;
                    len = b - a + 1;
                    let shown_start = if q_flag(q, "bad_content_range") {
                        a + 1
                    } else {
                        a
                    };
                    content_range = Some(format!("bytes {shown_start}-{b}/{total}"));
                }
                Ok(None) => {}
                Err(()) => {
                    let mut r = Resp::simple(416, "text/plain", "")
                        .header("Content-Range", format!("bytes */{total}"));
                    if !no_range {
                        r = r.header("Accept-Ranges", "bytes");
                    }
                    return Plan::Respond(r);
                }
            }
        }
    }

    let raw_len = len.saturating_add(extra);
    let wire_len = if gzip { gzip_len(raw_len) } else { raw_len };
    let declared = if q_flag(q, "no_length") {
        None
    } else if q_flag(q, "short_length") {
        Some(wire_len + (wire_len / 2).max(1))
    } else if q_flag(q, "long_length") {
        Some(wire_len / 2)
    } else if extra > 0 && !gzip {
        Some(len)
    } else {
        Some(wire_len)
    };

    let mut headers: Vec<(String, Vec<u8>)> =
        vec![("Content-Type".into(), b"application/octet-stream".to_vec())];
    if let Some(d) = declared {
        headers.push(("Content-Length".into(), d.to_string().into_bytes()));
    }
    if !no_range {
        headers.push(("Accept-Ranges".into(), b"bytes".to_vec()));
    }
    if let Some(cr) = content_range {
        headers.push(("Content-Range".into(), cr.into_bytes()));
    }
    if !no_validators {
        headers.push(("ETag".into(), etag.into_bytes()));
        headers.push(("Last-Modified".into(), last_modified.into_bytes()));
    }
    if gzip {
        headers.push(("Content-Encoding".into(), b"gzip".to_vec()));
    }
    if q_flag(q, "huge_header") {
        headers.push(("X-Huge".into(), vec![b'a'; HUGE_HEADER_BYTES]));
    }
    headers.extend(extra_headers);

    let fault_key = with_query(&req.path, &req.query);
    let times = q_num(q, "drop_times")
        .map(|t| t.min(u64::from(u32::MAX)) as u32)
        .unwrap_or(1);
    let cut = q_num(q, "drop_after")
        .or_else(|| q_num(q, "rst_after"))
        .filter(|_| req.method != "HEAD")
        .filter(|_| state.take_fault(&fault_key, times));

    Plan::Respond(Resp {
        status,
        headers,
        body: BodySpec::Fixture {
            fx,
            start,
            raw_len,
            gzip,
        },
        head_only: false,
        endless_headers: q_flag(q, "endless_headers"),
        cut,
        slow: q_num(q, "slow").filter(|b| *b > 0),
    })
}

fn basic_ok(req: &Req) -> bool {
    let Some(h) = req.header("authorization") else {
        return false;
    };
    let Some((scheme, b64)) = h.split_once(' ') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("basic") {
        return false;
    }
    let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(b64.trim()) else {
        return false;
    };
    raw == format!("{AUTH_USER}:{AUTH_PASSWORD}").as_bytes()
}

/// Parses `k=v, k="quoted, value"` auth parameters (keys lower-cased).
pub(crate) fn parse_auth_params(s: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i] == b' ' || b[i] == b',' || b[i] == b'\t') {
            i += 1;
        }
        let ks = i;
        while i < b.len() && b[i] != b'=' && b[i] != b',' {
            i += 1;
        }
        let key = s[ks..i].trim().to_ascii_lowercase();
        if i >= b.len() || b[i] != b'=' {
            continue;
        }
        i += 1;
        let mut val = String::new();
        if i < b.len() && b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' && i + 1 < b.len() {
                    i += 1;
                }
                val.push(b[i] as char);
                i += 1;
            }
            i += 1;
        } else {
            let vs = i;
            while i < b.len() && b[i] != b',' {
                i += 1;
            }
            val = s[vs..i].trim().to_string();
        }
        if !key.is_empty() {
            out.insert(key, val);
        }
    }
    out
}

fn sha256_hex(s: &str) -> String {
    to_hex(&Sha256::digest(s.as_bytes()))
}

/// RFC 7616 `response` for algorithm SHA-256, qop=auth.
#[allow(clippy::too_many_arguments)]
pub fn digest_sha256_response(
    user: &str,
    realm: &str,
    password: &str,
    method: &str,
    uri: &str,
    nonce: &str,
    nc: &str,
    cnonce: &str,
) -> String {
    let ha1 = sha256_hex(&format!("{user}:{realm}:{password}"));
    let ha2 = sha256_hex(&format!("{method}:{uri}"));
    sha256_hex(&format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}"))
}

fn digest_ok(state: &HttpState, req: &Req) -> bool {
    let Some(h) = req.header("authorization") else {
        return false;
    };
    let Some((scheme, rest)) = h.split_once(' ') else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("digest") {
        return false;
    }
    let p = parse_auth_params(rest);
    let get = |k: &str| p.get(k).map(String::as_str).unwrap_or("");
    let alg_ok = p
        .get("algorithm")
        .is_none_or(|a| a.eq_ignore_ascii_case("SHA-256"));
    let nonce_known = state
        .nonces
        .lock()
        .map(|n| n.contains(get("nonce")))
        .unwrap_or(false);
    if !alg_ok
        || !nonce_known
        || get("username") != AUTH_USER
        || get("realm") != AUTH_REALM
        || get("uri") != req.target
        || get("qop") != "auth"
    {
        return false;
    }
    let expected = digest_sha256_response(
        AUTH_USER,
        AUTH_REALM,
        AUTH_PASSWORD,
        &req.method,
        get("uri"),
        get("nonce"),
        get("nc"),
        get("cnonce"),
    );
    expected.eq_ignore_ascii_case(get("response"))
}

/// An endless site: page `n` links to `fan` child pages and one file.
fn maze_page(n: u64, fan: u64) -> String {
    let mut html = format!(
        "<!doctype html>
<html><body><a href=\"/files/tiny.txt?m={n}\">f</a>
"
    );
    for i in 1..=fan {
        let child = n.saturating_mul(fan).saturating_add(i);
        html.push_str(&format!(
            "<a href=\"{child}.html?fan={fan}\">p</a>
"
        ));
    }
    html.push_str(
        "</body></html>
",
    );
    html
}

/// The small site for the grabber: depth 0 (index) .. depth 4 (p6).
fn site_page(page: &str) -> Option<String> {
    let links: &[&str] = match page {
        "" | "index.html" => &["p1.html", "/site/p2.html", "/files/tiny.txt"],
        "p1.html" => &["p3.html", "../files/small.bin", "index.html"],
        "p2.html" => &["p3.html", "p4.html", "/files/small.bin"],
        "p3.html" => &[
            "p5.html",
            "/files/medium.bin",
            "http://example.invalid/external.bin",
        ],
        "p4.html" => &["index.html", "./p2.html", "/files/tiny.txt"],
        "p5.html" => &["p6.html", "/files/big.bin"],
        "p6.html" => &["/files/huge.bin"],
        _ => return None,
    };
    let mut html = format!("<!doctype html>\n<html><head><title>{page}</title></head><body>\n");
    for l in links {
        html.push_str(&format!("<a href=\"{l}\">{l}</a><br>\n"));
    }
    html.push_str("</body></html>\n");
    Some(html)
}

/// Outcome of one generator step.
pub(crate) enum Next {
    Data(Bytes),
    Done,
    /// The configured cut point was reached: drop (h1) / reset (h2).
    Cut,
}

/// Streams a response body, applying gzip, cut and throttle faults.
pub(crate) struct BodyGen {
    body: Option<BodySpec>,
    pos: u64,
    gz_header_sent: bool,
    gz_trailer_sent: bool,
    crc: u32,
    pending: Bytes,
    sent: u64,
    cut: Option<u64>,
    slow: Option<u64>,
}

impl BodyGen {
    pub fn new(body: BodySpec, cut: Option<u64>, slow: Option<u64>) -> Self {
        BodyGen {
            body: Some(body),
            pos: 0,
            gz_header_sent: false,
            gz_trailer_sent: false,
            crc: 0,
            pending: Bytes::new(),
            sent: 0,
            cut,
            slow,
        }
    }

    fn produce(&mut self) -> Option<Bytes> {
        match self.body.as_ref()? {
            BodySpec::Bytes(_) => {
                if let Some(BodySpec::Bytes(v)) = self.body.take() {
                    if v.is_empty() {
                        return None;
                    }
                    return Some(Bytes::from(v));
                }
                None
            }
            BodySpec::Fixture {
                fx,
                start,
                raw_len,
                gzip,
            } => {
                let (start, raw_len, gzip) = (*start, *raw_len, *gzip);
                if gzip && self.gz_trailer_sent {
                    return None;
                }
                let n = raw_len.saturating_sub(self.pos).min(CHUNK) as usize;
                if n == 0 && !gzip {
                    return None;
                }
                let mut data = vec![0u8; n];
                fx.fill_unbounded(start + self.pos, &mut data);
                self.pos += n as u64;
                if !gzip {
                    return Some(Bytes::from(data));
                }
                let mut out = BytesMut::with_capacity(n + 32);
                if !self.gz_header_sent {
                    out.put_slice(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff]);
                    self.gz_header_sent = true;
                }
                if n > 0 {
                    let l = n as u16;
                    out.put_u8(0);
                    out.put_u16_le(l);
                    out.put_u16_le(!l);
                    out.put_slice(&data);
                    self.crc = crc32_update(self.crc, &data);
                }
                if self.pos >= raw_len {
                    out.put_slice(&[1, 0, 0, 0xff, 0xff]);
                    out.put_u32_le(self.crc);
                    out.put_u32_le(raw_len as u32);
                    self.gz_trailer_sent = true;
                }
                Some(out.freeze())
            }
        }
    }

    pub async fn next(&mut self) -> Next {
        if self.cut.is_some_and(|c| self.sent >= c) {
            return Next::Cut;
        }
        if self.pending.is_empty() {
            match self.produce() {
                Some(b) => self.pending = b,
                None => return Next::Done,
            }
        }
        let mut take = self.pending.len() as u64;
        if let Some(c) = self.cut {
            take = take.min(c - self.sent);
        }
        if let Some(bps) = self.slow {
            take = take.min((bps / 20).max(1));
            tokio::time::sleep(Duration::from_secs_f64(take as f64 / bps as f64)).await;
        }
        let out = self.pending.split_to(take as usize);
        self.sent += take;
        Next::Data(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-9", 100), Ok(Some((0, 9))));
        assert_eq!(parse_range("bytes=90-", 100), Ok(Some((90, 99))));
        assert_eq!(parse_range("bytes=-10", 100), Ok(Some((90, 99))));
        assert_eq!(parse_range("bytes=-1000", 100), Ok(Some((0, 99))));
        assert_eq!(parse_range("bytes=50-1000", 100), Ok(Some((50, 99))));
        assert_eq!(parse_range("bytes=100-", 100), Err(()));
        assert_eq!(parse_range("bytes=9-3", 100), Err(()));
        assert_eq!(parse_range("bytes=0-1,5-6", 100), Err(()));
        assert_eq!(parse_range("bytes=x-", 100), Err(()));
        assert_eq!(parse_range("items=0-1", 100), Ok(None));
    }

    #[test]
    fn auth_params() {
        let p = parse_auth_params(r#"username="udm-user", realm="u, dm", nc=00000001, qop=auth"#);
        assert_eq!(p["username"], "udm-user");
        assert_eq!(p["realm"], "u, dm");
        assert_eq!(p["nc"], "00000001");
        assert_eq!(p["qop"], "auth");
    }

    #[tokio::test]
    async fn gzip_length_matches_formula() {
        let fx = crate::fixture("small.bin").unwrap();
        for raw in [0u64, 1, CHUNK, CHUNK + 1, 3 * CHUNK] {
            let mut g = BodyGen::new(
                BodySpec::Fixture {
                    fx: fx.clone(),
                    start: 0,
                    raw_len: raw,
                    gzip: true,
                },
                None,
                None,
            );
            let mut n = 0u64;
            while let Next::Data(b) = g.next().await {
                n += b.len() as u64;
            }
            assert_eq!(n, gzip_len(raw), "raw {raw}");
        }
    }
}
