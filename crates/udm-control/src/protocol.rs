//! Wire format: one JSON object per line.
//!
//! Request `{"id":N,"method":"...","params":{...}}`; response
//! `{"id":N,"ok":true,"result":...}` or `{"id":N,"ok":false,"error":{"key":"error-...","args":{...}}}`.

use serde_json::{Map, Value, json};
use udm_core::{Message, MsgArg, limits};

/// Maximum length of a method name.
const MAX_METHOD_BYTES: usize = 64;

/// A parsed request.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// Caller-chosen id echoed in the response (number, string or null).
    pub id: Value,
    /// Method, e.g. `engine.execute`.
    pub method: String,
    /// Parameters (`null` when absent).
    pub params: Value,
}

impl Request {
    /// Builds a request (used by clients).
    pub fn new(id: u64, method: &str, params: Value) -> Self {
        Self {
            id: Value::from(id),
            method: method.to_owned(),
            params,
        }
    }

    /// Serialises to one line (without the newline).
    pub fn to_line(&self) -> String {
        json!({ "id": self.id, "method": self.method, "params": self.params }).to_string()
    }

    /// String parameter.
    pub fn str_param(&self, name: &str) -> Option<&str> {
        self.params.get(name).and_then(Value::as_str)
    }
}

/// Parses one line. On error returns the id (if it could be read) and the message.
pub fn parse_request(line: &[u8]) -> Result<Request, (Value, Message)> {
    if line.len() > limits::MAX_CONTROL_MESSAGE_BYTES {
        return Err((Value::Null, too_large()));
    }
    let v: Value = serde_json::from_slice(line)
        .map_err(|_| (Value::Null, Message::new("error-control-bad-json")))?;
    let Value::Object(mut obj) = v else {
        return Err((Value::Null, Message::new("error-control-bad-request")));
    };
    let id = match obj.remove("id") {
        None => Value::Null,
        Some(id @ (Value::Null | Value::Number(_))) => id,
        Some(Value::String(s)) if s.len() <= MAX_METHOD_BYTES => Value::String(s),
        Some(_) => return Err((Value::Null, Message::new("error-control-bad-request"))),
    };
    let method = match obj.remove("method") {
        Some(Value::String(m)) if !m.is_empty() && m.len() <= MAX_METHOD_BYTES => m,
        _ => return Err((id, Message::new("error-control-bad-request"))),
    };
    let params = obj.remove("params").unwrap_or(Value::Null);
    if !matches!(params, Value::Null | Value::Object(_)) {
        return Err((id, Message::new("error-control-bad-request")));
    }
    Ok(Request { id, method, params })
}

/// The error for an over-long line.
pub fn too_large() -> Message {
    Message::new("error-control-message-too-large").with_uint(
        "max",
        u64::try_from(limits::MAX_CONTROL_MESSAGE_BYTES).unwrap_or(u64::MAX),
    )
}

/// JSON form of a [`Message`]: `{"key":..., "args":{...}}`.
pub fn message_to_json(m: &Message) -> Value {
    let mut args = Map::new();
    for (name, arg) in &m.args {
        let v = match arg {
            MsgArg::Str(s) => Value::from(s.as_str()),
            MsgArg::Int(i) => Value::from(*i),
            MsgArg::UInt(u) | MsgArg::Bytes(u) => Value::from(*u),
        };
        args.insert(name.clone(), v);
    }
    json!({ "key": m.key, "args": args })
}

/// Success response line.
pub fn ok_line(id: &Value, result: Value) -> String {
    json!({ "id": id, "ok": true, "result": result }).to_string()
}

/// Error response line.
pub fn err_line(id: &Value, m: &Message) -> String {
    json!({ "id": id, "ok": false, "error": message_to_json(m) }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_request() {
        let r = parse_request(br#"{"id":3,"method":"engine.query","params":{"what":"downloads"}}"#)
            .unwrap();
        assert_eq!(r.id, json!(3));
        assert_eq!(r.method, "engine.query");
        assert_eq!(r.str_param("what"), Some("downloads"));
        assert_eq!(parse_request(r.to_line().as_bytes()).unwrap(), r);
    }

    #[test]
    fn hostile_control_request_shapes() {
        for bad in [
            &b"not json"[..],
            b"[1,2]",
            b"{\"id\":1}",
            b"{\"id\":1,\"method\":5}",
            b"{\"id\":{},\"method\":\"x\"}",
            b"{\"id\":1,\"method\":\"x\",\"params\":[1]}",
        ] {
            assert!(
                parse_request(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        let long = format!(
            r#"{{"id":1,"method":"{}"}}"#,
            "m".repeat(MAX_METHOD_BYTES + 1)
        );
        assert!(parse_request(long.as_bytes()).is_err());
    }

    #[test]
    fn hostile_control_message_too_large() {
        let big = vec![b' '; limits::MAX_CONTROL_MESSAGE_BYTES + 1];
        let (_, m) = parse_request(&big).unwrap_err();
        assert_eq!(m.key, "error-control-message-too-large");
    }

    #[test]
    fn error_line_shape() {
        let m = Message::new("error-x")
            .with_str("a", "b")
            .with_bytes("n", 5);
        let v: Value = serde_json::from_str(&err_line(&json!(1), &m)).unwrap();
        assert_eq!(v["ok"], json!(false));
        assert_eq!(v["error"]["key"], json!("error-x"));
        assert_eq!(v["error"]["args"]["n"], json!(5));
    }
}
