//! Turns one extension message into a call on the app's restricted capture
//! endpoint (port and token from `ipc.json`) and builds the reply.
//!
//! Messages: `{"type":"ping"}` and
//! `{"type":"capture","url":..,"file_name"?,"size"?,"referer"?,"cookie"?}`.

use std::path::Path;

use serde_json::{Map, Value, json};
use udm_control::{ControlClient, IpcInfo, message_to_json};
use udm_core::{Message, limits};

/// A validated extension message.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    /// Is the app running?
    Ping,
    /// Hand a captured download to the app; `args` are `capture.browser` args.
    Capture(Map<String, Value>),
}

fn bad(field: &str) -> Message {
    Message::new("error-native-bad-message").with_str("field", field)
}

fn opt_str<'a>(o: &'a Map<String, Value>, k: &str, max: usize) -> Result<Option<&'a str>, Message> {
    match o.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.len() <= max && !s.contains('\0') => Ok(Some(s.as_str())),
        Some(_) => Err(bad(k)),
    }
}

/// Parses and bounds a message body.
pub fn parse(body: &[u8]) -> Result<Incoming, Message> {
    let v: Value =
        serde_json::from_slice(body).map_err(|_| Message::new("error-native-bad-json"))?;
    let o = v.as_object().ok_or_else(|| bad("type"))?;
    match o.get("type").and_then(Value::as_str) {
        Some("ping") => Ok(Incoming::Ping),
        Some("capture") => {
            let url = opt_str(o, "url", limits::MAX_URL_BYTES)?.ok_or_else(|| bad("url"))?;
            if !(url.starts_with("http://")
                || url.starts_with("https://")
                || url.starts_with("ftp://"))
            {
                return Err(bad("url"));
            }
            let mut args = Map::new();
            args.insert("url".into(), Value::from(url));
            if let Some(n) = opt_str(o, "file_name", limits::MAX_PATH_BYTES)? {
                args.insert("file_name".into(), Value::from(n));
            }
            if let Some(r) = opt_str(o, "referer", limits::MAX_URL_BYTES)? {
                args.insert("referer".into(), Value::from(r));
            }
            if let Some(c) = opt_str(o, "cookie", limits::MAX_COOKIE_BYTES)? {
                args.insert("cookie".into(), Value::from(c));
            }
            match o.get("size") {
                None | Some(Value::Null) => {}
                Some(Value::Number(n)) => {
                    let s = n
                        .as_u64()
                        .filter(|s| *s <= limits::MAX_FILE_SIZE_BYTES)
                        .ok_or_else(|| bad("size"))?;
                    args.insert("size".into(), Value::from(s));
                }
                Some(_) => return Err(bad("size")),
            }
            Ok(Incoming::Capture(args))
        }
        _ => Err(bad("type")),
    }
}

/// Reply frame body for a result.
pub fn reply(r: Result<Value, Message>) -> Value {
    match r {
        Ok(v) => json!({ "ok": true, "result": v }),
        Err(m) => json!({ "ok": false, "error": message_to_json(&m) }),
    }
}

/// Sends a parsed message to the running app.
pub async fn forward(msg: Incoming, ipc_file: &Path) -> Result<Value, Message> {
    let info = IpcInfo::load(ipc_file)
        .map_err(|_| Message::new("error-native-ipc-unreadable"))?
        .ok_or_else(|| Message::new("error-native-app-not-running"))?;
    let mut c = ControlClient::connect(info.port)
        .await
        .map_err(|_| Message::new("error-native-app-not-running"))?;
    match msg {
        Incoming::Ping => c.call("app.ping", json!({ "token": info.token })).await,
        Incoming::Capture(args) => {
            c.call(
                "engine.execute",
                json!({ "token": info.token, "command": "capture.browser", "args": args }),
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ping_and_capture() {
        assert_eq!(parse(br#"{"type":"ping"}"#).unwrap(), Incoming::Ping);
        let Incoming::Capture(a) =
            parse(br#"{"type":"capture","url":"https://h/f.zip","size":10,"cookie":"a=b"}"#)
                .unwrap()
        else {
            panic!()
        };
        assert_eq!(a["url"], "https://h/f.zip");
        assert_eq!(a["size"], 10);
        assert_eq!(a["cookie"], "a=b");
    }

    #[test]
    fn hostile_native_messages_rejected() {
        let long_url = format!(
            r#"{{"type":"capture","url":"https://h/{}"}}"#,
            "a".repeat(limits::MAX_URL_BYTES)
        );
        let long_cookie = format!(
            r#"{{"type":"capture","url":"https://h/f","cookie":"{}"}}"#,
            "c".repeat(limits::MAX_COOKIE_BYTES + 1)
        );
        for bad in [
            &b"not json"[..],
            b"[]",
            br#"{"type":"exec","cmd":"rm"}"#,
            br#"{"type":"capture"}"#,
            br#"{"type":"capture","url":"file:///etc/passwd"}"#,
            br#"{"type":"capture","url":"javascript:alert(1)"}"#,
            br#"{"type":"capture","url":"https://h/f","size":-1}"#,
            br#"{"type":"capture","url":"https://h/f","size":"9"}"#,
            br#"{"type":"capture","url":"https://h/f","referer":7}"#,
            br#"{"type":"capture","url":"https://h/\u0000f"}"#,
            long_url.as_bytes(),
            long_cookie.as_bytes(),
        ] {
            assert!(parse(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn errors_carry_keys_not_secrets() {
        let r = reply(Err(Message::new("error-native-app-not-running")));
        assert_eq!(r["ok"], false);
        assert_eq!(r["error"]["key"], "error-native-app-not-running");
    }

    #[tokio::test]
    async fn missing_ipc_file_means_app_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let err = forward(Incoming::Ping, &dir.path().join("ipc.json"))
            .await
            .unwrap_err();
        assert_eq!(err.key, "error-native-app-not-running");
    }
}
