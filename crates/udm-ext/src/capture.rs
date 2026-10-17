//! Capture decisions for the browser extension. Pure functions over JSON so the
//! JS glue only forwards browser events and acts on the returned decision.
//!
//! Everything from the browser is hostile: bounded, validated, then used.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use udm_core::{Message, MsgArg, limits};
use udm_i18n::{Locale, Localizer};
use url::Url;

/// Name of the native messaging host (`udm-native-host`).
pub const HOST_NAME: &str = udm_core::NATIVE_HOST_NAME;
/// Context-menu id for links.
pub const MENU_LINK: &str = "udm-link";
/// Context-menu id for media elements.
pub const MENU_MEDIA: &str = "udm-media";

/// Extensions intercepted by default (lower case, no dot).
const DEFAULT_EXTENSIONS: &[&str] = &[
    "zip", "rar", "7z", "tar", "gz", "bz2", "xz", "iso", "img", "exe", "msi", "dmg", "pkg", "deb",
    "rpm", "appimage", "apk", "mp4", "mkv", "avi", "mov", "webm", "mp3", "flac", "wav", "m4a",
    "ogg", "pdf", "epub", "doc", "docx", "xls", "xlsx", "ppt", "pptx",
];
/// Downloads smaller than this are left to the browser by default (1 MiB).
const DEFAULT_MIN_SIZE_BYTES: u64 = 1_048_576;

/// User settings kept in `storage.local`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Intercept browser downloads automatically.
    pub enabled: bool,
    /// Minimum known size to intercept.
    pub min_size_bytes: u64,
    /// Extensions to intercept.
    pub extensions: Vec<String>,
    /// Hosts never intercepted (exact match, lower case).
    pub skip_hosts: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            min_size_bytes: DEFAULT_MIN_SIZE_BYTES,
            extensions: DEFAULT_EXTENSIONS.iter().map(|s| (*s).to_owned()).collect(),
            skip_hosts: Vec::new(),
        }
    }
}

impl Settings {
    /// Parses settings; anything malformed or out of bounds gives the defaults.
    pub fn parse(text: &str) -> Self {
        if text.len() > limits::MAX_NATIVE_MESSAGE_BYTES {
            return Self::default();
        }
        let Ok(mut s) = serde_json::from_str::<Self>(text) else {
            return Self::default();
        };
        if s.extensions.len() > limits::MAX_CATEGORY_EXTENSIONS
            || s.skip_hosts.len() > limits::MAX_ARG_LIST_ITEMS
        {
            return Self::default();
        }
        for e in &mut s.extensions {
            *e = e.trim().trim_start_matches('.').to_ascii_lowercase();
        }
        for h in &mut s.skip_hosts {
            *h = h.trim().to_ascii_lowercase();
        }
        s.min_size_bytes = s.min_size_bytes.min(limits::MAX_FILE_SIZE_BYTES);
        s
    }
}

/// What the glue should do with an event.
#[derive(Clone, Debug, PartialEq)]
pub enum Decision {
    /// Send `message` to the native host; attach cookies for `url` if asked.
    Capture {
        /// Native message (`type: capture`).
        message: Value,
        /// The URL whose cookies belong to this request.
        cookie_url: String,
    },
    /// Leave the browser alone.
    Ignore(&'static str),
}

impl Decision {
    /// JSON handed back to the glue.
    pub fn to_json(&self) -> Value {
        match self {
            Self::Capture {
                message,
                cookie_url,
            } => json!({ "action": "capture", "message": message, "cookie_url": cookie_url }),
            Self::Ignore(reason) => json!({ "action": "ignore", "reason": reason }),
        }
    }
}

fn str_field<'a>(o: &'a Map<String, Value>, key: &str, max: usize) -> Option<&'a str> {
    o.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= max && !s.contains('\0'))
}

/// A capturable URL: http(s)/ftp, bounded, no credentials.
fn capturable(raw: &str) -> Option<Url> {
    if raw.len() > limits::MAX_URL_BYTES {
        return None;
    }
    let u = Url::parse(raw).ok()?;
    let scheme_ok = matches!(u.scheme(), "http" | "https" | "ftp");
    // Credentials in a URL would leak into the queue and history.
    let no_creds = u.username().is_empty() && u.password().is_none();
    (scheme_ok && no_creds && u.host_str().is_some()).then_some(u)
}

/// Last path component of a browser-supplied path (`C:\Users\x\f.zip` → `f.zip`).
fn base_name(path: &str) -> Option<&str> {
    path.rsplit(['/', '\\']).next().filter(|s| !s.is_empty())
}

fn capture_message(
    url: &Url,
    file_name: Option<&str>,
    referer: Option<&str>,
    size: Option<u64>,
) -> Value {
    let mut m = Map::new();
    m.insert("type".into(), Value::from("capture"));
    m.insert("url".into(), Value::from(url.as_str()));
    if let Some(n) = file_name.filter(|n| n.len() <= limits::MAX_FILENAME_BYTES) {
        m.insert("file_name".into(), Value::from(n));
    }
    if let Some(r) = referer.and_then(capturable) {
        m.insert("referer".into(), Value::from(r.as_str()));
    }
    if let Some(s) = size.filter(|s| *s > 0 && *s <= limits::MAX_FILE_SIZE_BYTES) {
        m.insert("size".into(), Value::from(s));
    }
    Value::Object(m)
}

/// Size reported by the browser (`fileSize`, `totalBytes`); unknown is `-1`/`0`.
fn item_size(o: &Map<String, Value>) -> Option<u64> {
    ["fileSize", "totalBytes"]
        .iter()
        .filter_map(|k| o.get(*k).and_then(Value::as_u64))
        .find(|s| *s > 0)
}

fn decide_download(s: &Settings, item: &Map<String, Value>) -> Decision {
    if !s.enabled {
        return Decision::Ignore("disabled");
    }
    if item.get("byExtensionId").is_some_and(|v| !v.is_null()) {
        return Decision::Ignore("by-extension");
    }
    if item
        .get("state")
        .and_then(Value::as_str)
        .is_some_and(|st| st != "in_progress")
    {
        return Decision::Ignore("not-in-progress");
    }
    let raw = str_field(item, "finalUrl", limits::MAX_URL_BYTES)
        .or_else(|| str_field(item, "url", limits::MAX_URL_BYTES));
    let Some(url) = raw.and_then(capturable) else {
        return Decision::Ignore("url");
    };
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if s.skip_hosts.contains(&host) {
        return Decision::Ignore("skip-host");
    }
    let name = str_field(item, "filename", limits::MAX_PATH_BYTES)
        .and_then(base_name)
        .map(str::to_owned)
        .or_else(|| udm_proto::filename_from_url(&url));
    let ext = name.as_deref().and_then(udm_proto::extension);
    if !ext.is_some_and(|e| s.extensions.contains(&e)) {
        return Decision::Ignore("extension");
    }
    let size = item_size(item);
    if size.is_some_and(|n| n < s.min_size_bytes) {
        return Decision::Ignore("small");
    }
    let referer = str_field(item, "referrer", limits::MAX_URL_BYTES);
    Decision::Capture {
        message: capture_message(&url, name.as_deref(), referer, size),
        cookie_url: url.as_str().to_owned(),
    }
}

/// An explicit context-menu click is always captured if the URL is usable.
fn decide_menu(info: &Map<String, Value>) -> Decision {
    let field = match info.get("menuItemId").and_then(Value::as_str) {
        Some(MENU_LINK) => "linkUrl",
        Some(MENU_MEDIA) => "srcUrl",
        _ => return Decision::Ignore("menu"),
    };
    let Some(url) = str_field(info, field, limits::MAX_URL_BYTES).and_then(capturable) else {
        return Decision::Ignore("url");
    };
    let referer = str_field(info, "pageUrl", limits::MAX_URL_BYTES);
    Decision::Capture {
        message: capture_message(&url, None, referer, None),
        cookie_url: url.as_str().to_owned(),
    }
}

/// Decides on one event: `{"kind":"download","item":{...}}` (a `downloads.onCreated`
/// item) or `{"kind":"menu","info":{...}}` (a `contextMenus.onClicked` info).
pub fn decide(settings: &Settings, event_json: &str) -> Decision {
    if event_json.len() > limits::MAX_NATIVE_MESSAGE_BYTES {
        return Decision::Ignore("too-large");
    }
    let Ok(Value::Object(ev)) = serde_json::from_str::<Value>(event_json) else {
        return Decision::Ignore("bad-json");
    };
    let kind = ev.get("kind").and_then(Value::as_str);
    match (kind, ev.get("item"), ev.get("info")) {
        (Some("download"), Some(Value::Object(item)), _) => decide_download(settings, item),
        (Some("menu"), _, Some(Value::Object(info))) => decide_menu(info),
        _ => Decision::Ignore("kind"),
    }
}

fn cookie_token_ok(s: &str, value: bool) -> bool {
    s.chars().all(|c| {
        let ctl = c.is_control() || c == ';';
        // `=` is allowed inside values (base64), never in names.
        !ctl && (value || (c != '=' && c != ' '))
    })
}

/// Builds a `Cookie` header from a `cookies.getAll` result for the captured URL.
/// Bad cookies are dropped; the header never exceeds `MAX_COOKIE_BYTES`.
pub fn cookie_header(cookies_json: &str) -> String {
    if cookies_json.len() > limits::MAX_NATIVE_MESSAGE_BYTES {
        return String::new();
    }
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(cookies_json) else {
        return String::new();
    };
    let mut out = String::new();
    for c in list.iter().take(limits::MAX_ARG_LIST_ITEMS) {
        let (Some(name), Some(value)) = (
            c.get("name").and_then(Value::as_str),
            c.get("value").and_then(Value::as_str),
        ) else {
            continue;
        };
        if name.is_empty() || !cookie_token_ok(name, false) || !cookie_token_ok(value, true) {
            continue;
        }
        let sep = if out.is_empty() { "" } else { "; " };
        let added = sep
            .len()
            .saturating_add(name.len())
            .saturating_add(1)
            .saturating_add(value.len());
        if out.len().saturating_add(added) > limits::MAX_COOKIE_BYTES {
            break;
        }
        out.push_str(sep);
        out.push_str(name);
        out.push('=');
        out.push_str(value);
    }
    out
}

fn message_from_json(v: &Value) -> Message {
    let key = v
        .get("key")
        .and_then(Value::as_str)
        .filter(|k| k.len() <= limits::MAX_ARG_STRING_BYTES)
        .unwrap_or("ext-reply-invalid");
    let mut m = Message::new(key);
    if let Some(args) = v.get("args").and_then(Value::as_object) {
        for (k, a) in args.iter().take(limits::MAX_COMMAND_ARGS) {
            let arg = match a {
                Value::String(s) => MsgArg::Str(s.clone()),
                Value::Number(n) => match n.as_u64() {
                    Some(u) => MsgArg::UInt(u),
                    None => MsgArg::Str(n.to_string()),
                },
                _ => continue,
            };
            m.args.push((k.clone(), arg));
        }
    }
    m
}

/// Turns a native-host reply (or `null` when the host is unreachable) into
/// `{"captured": bool, "badge": "...", "text": "..."}` for the toolbar button.
pub fn reply(locale_tag: &str, reply_json: &str) -> Value {
    let loc = Localizer::new(Locale::parse(locale_tag).unwrap_or_default());
    let parsed = if reply_json.len() > limits::MAX_NATIVE_MESSAGE_BYTES {
        Value::Null
    } else {
        serde_json::from_str::<Value>(reply_json).unwrap_or(Value::Null)
    };
    let (captured, msg) = match &parsed {
        Value::Null => (false, Message::new("ext-host-missing")),
        v if v.get("ok") == Some(&Value::Bool(true)) => (true, Message::new("ext-sent")),
        v => (
            false,
            v.get("error")
                .map_or_else(|| Message::new("ext-reply-invalid"), message_from_json),
        ),
    };
    // An unknown key from a newer host must not show a raw key.
    let msg = if loc.has(&msg.key) {
        msg
    } else {
        Message::new("ext-reply-invalid")
    };
    let badge = if captured { "" } else { "!" };
    json!({ "captured": captured, "badge": badge, "text": loc.message(&msg) })
}

/// Context-menu titles for the browser UI language.
pub fn menus(locale_tag: &str) -> Value {
    let loc = Localizer::new(Locale::parse(locale_tag).unwrap_or_default());
    json!([
        { "id": MENU_LINK, "title": loc.tr("ext-menu-link"), "contexts": ["link"] },
        { "id": MENU_MEDIA, "title": loc.tr("ext-menu-media"), "contexts": ["video", "audio", "image"] },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dl(item: Value) -> String {
        json!({ "kind": "download", "item": item }).to_string()
    }

    #[test]
    fn big_zip_is_captured_with_basename_and_referer() {
        let d = decide(
            &Settings::default(),
            &dl(json!({
                "url": "https://h.example/get?id=1",
                "finalUrl": "https://cdn.example/files/big.zip",
                "filename": "C:\\Users\\me\\Downloads\\big.zip",
                "fileSize": 5_000_000,
                "referrer": "https://h.example/page",
                "state": "in_progress"
            })),
        );
        let Decision::Capture {
            message,
            cookie_url,
        } = d
        else {
            panic!("{d:?}")
        };
        assert_eq!(cookie_url, "https://cdn.example/files/big.zip");
        assert_eq!(message["type"], "capture");
        assert_eq!(message["url"], "https://cdn.example/files/big.zip");
        assert_eq!(message["file_name"], "big.zip");
        assert_eq!(message["referer"], "https://h.example/page");
        assert_eq!(message["size"], 5_000_000);
    }

    #[test]
    fn small_unknown_ext_disabled_and_skipped_hosts_are_ignored() {
        let s = Settings::default();
        let small = dl(json!({ "url": "https://h/f.zip", "fileSize": 10 }));
        assert_eq!(decide(&s, &small), Decision::Ignore("small"));
        let html = dl(json!({ "url": "https://h/page.html", "fileSize": 9_000_000 }));
        assert_eq!(decide(&s, &html), Decision::Ignore("extension"));
        let off = Settings {
            enabled: false,
            ..Settings::default()
        };
        assert_eq!(
            decide(&off, &dl(json!({ "url": "https://h/f.zip" }))),
            Decision::Ignore("disabled")
        );
        let skip = Settings {
            skip_hosts: vec!["h".into()],
            ..Settings::default()
        };
        assert_eq!(
            decide(&skip, &dl(json!({ "url": "https://H/f.zip" }))),
            Decision::Ignore("skip-host")
        );
        // Unknown size (-1) is captured: UDM probes it.
        let unknown = dl(json!({ "url": "https://h/f.iso", "fileSize": -1 }));
        assert!(matches!(decide(&s, &unknown), Decision::Capture { .. }));
    }

    #[test]
    fn menu_clicks_are_captured() {
        let ev = json!({ "kind": "menu", "info": {
            "menuItemId": MENU_MEDIA, "srcUrl": "https://h/v.mp4", "pageUrl": "https://h/" } });
        let Decision::Capture { message, .. } = decide(&Settings::default(), &ev.to_string())
        else {
            panic!()
        };
        assert_eq!(message["url"], "https://h/v.mp4");
        let ev =
            json!({ "kind": "menu", "info": { "menuItemId": "other", "linkUrl": "https://h/" } });
        assert_eq!(
            decide(&Settings::default(), &ev.to_string()),
            Decision::Ignore("menu")
        );
    }

    #[test]
    fn hostile_ext_events_are_ignored() {
        let s = Settings::default();
        for ev in [
            String::from("not json"),
            String::from("[]"),
            json!({ "kind": "download" }).to_string(),
            dl(json!({ "url": "blob:https://h/1234" })),
            dl(json!({ "url": "data:application/zip;base64,AAAA" })),
            dl(json!({ "url": "javascript:alert(1)//x.zip" })),
            dl(json!({ "url": "file:///etc/passwd.zip" })),
            dl(json!({ "url": "https://user:pw@h/f.zip" })),
            dl(json!({ "url": format!("https://h/{}.zip", "a".repeat(limits::MAX_URL_BYTES)) })),
            dl(json!({ "url": "https://h/f.zip", "byExtensionId": "abc" })),
            dl(json!({ "url": "https://h/f.zip", "state": "complete" })),
            "x".repeat(limits::MAX_NATIVE_MESSAGE_BYTES + 1),
        ] {
            assert!(
                matches!(decide(&s, &ev), Decision::Ignore(_)),
                "{}",
                ev.get(..80).unwrap_or(&ev)
            );
        }
        // A hostile referer or file name is dropped, not forwarded.
        let d = decide(
            &s,
            &dl(
                json!({ "url": "https://h/f.zip", "referrer": "javascript:x",
                "filename": format!("{}.zip", "n".repeat(limits::MAX_FILENAME_BYTES)) }),
            ),
        );
        let Decision::Capture { message, .. } = d else {
            panic!()
        };
        assert!(message.get("referer").is_none());
        assert!(message.get("file_name").is_none());
    }

    #[test]
    fn hostile_ext_cookie_header_bounded() {
        let ok = json!([{ "name": "a", "value": "1" }, { "name": "b", "value": "x=y" }]);
        assert_eq!(cookie_header(&ok.to_string()), "a=1; b=x=y");
        let bad = json!([
            { "name": "a;b", "value": "1" },
            { "name": "c", "value": "1\r\nX-Evil: 1" },
            { "name": "", "value": "1" },
            { "name": "d=e", "value": "1" },
            { "value": "1" },
            { "name": "f", "value": "2" }
        ]);
        assert_eq!(cookie_header(&bad.to_string()), "f=2");
        // MAX_COOKIE_BYTES: the header stops before the limit.
        let many: Vec<Value> = (0..2000)
            .map(|i| json!({ "name": format!("n{i}"), "value": "v".repeat(50) }))
            .collect();
        let h = cookie_header(&Value::Array(many).to_string());
        assert!(!h.is_empty() && h.len() <= limits::MAX_COOKIE_BYTES);
        assert_eq!(cookie_header("{}"), "");
    }

    #[test]
    fn settings_parse_normalises_and_bounds() {
        let s = Settings::parse(r#"{"extensions":[".ZIP"," Iso "],"skip_hosts":["A.example"]}"#);
        assert_eq!(s.extensions, vec!["zip", "iso"]);
        assert_eq!(s.skip_hosts, vec!["a.example"]);
        assert!(s.enabled);
        assert_eq!(Settings::parse("garbage"), Settings::default());
        let too_many: Vec<String> = (0..=limits::MAX_CATEGORY_EXTENSIONS)
            .map(|i| format!("e{i}"))
            .collect();
        let t = json!({ "extensions": too_many }).to_string();
        assert_eq!(Settings::parse(&t), Settings::default());
    }

    #[test]
    fn replies_are_localised() {
        let ok = reply(
            "en-US",
            r#"{"ok":true,"result":{"download_id":"d-000001"}}"#,
        );
        assert_eq!(ok["captured"], true);
        assert_eq!(ok["badge"], "");
        let missing = reply("id", "null");
        assert_eq!(missing["captured"], false);
        assert_eq!(missing["badge"], "!");
        let err = reply(
            "en",
            r#"{"ok":false,"error":{"key":"error-native-app-not-running","args":{}}}"#,
        );
        assert_eq!(err["captured"], false);
        assert_ne!(err["text"], "error-native-app-not-running");
        let unknown = reply("en", r#"{"ok":false,"error":{"key":"zzz-unknown"}}"#);
        assert!(!unknown["text"].as_str().unwrap().contains("zzz"));
        assert_eq!(menus("id")[0]["id"], MENU_LINK);
    }
}
