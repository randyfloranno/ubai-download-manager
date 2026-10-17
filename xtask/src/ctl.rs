//! `cargo xtask ctl <PORT> '<json>'`: send one newline-delimited JSON request to the
//! control server on 127.0.0.1 and print the one-line response.
//!
//! PowerShell quoting: Windows PowerShell 5.1 strips the inner double quotes when passing a
//! single-quoted argument to a native program, so `'{"id":1,"method":"ui.set_locale"}'`
//! arrives as `{id:1,method:ui.set_locale}`. When the argument is not valid JSON, `ctl`
//! re-quotes bare keys and values: keys run up to `:`, values run up to the next `,`, `}`
//! or `]` (so URLs and `23:00` survive). Numbers, `true`, `false` and `null` stay bare.
//! Limitation: a repaired bare value cannot contain `,`, `}` or `]` — escape quotes as
//! `\"` (or use `--%`) for such values. Multiple arguments are joined with spaces.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(30);
/// Upper bound for one response line (control responses are small; screenshots are files).
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    ObjKey,
    ObjValue,
    Array,
}

/// Re-quote bare keys/values of a JSON-ish text stripped of its double quotes.
pub fn repair_json(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len() + 16);
    let mut stack: Vec<Ctx> = Vec::new();
    let mut i = 0usize;
    while let Some(&c) = chars.get(i) {
        match c {
            '{' => {
                stack.push(Ctx::ObjKey);
                out.push(c);
                i += 1;
            }
            '[' => {
                stack.push(Ctx::Array);
                out.push(c);
                i += 1;
            }
            '}' | ']' => {
                stack.pop();
                out.push(c);
                i += 1;
            }
            ':' => {
                if let Some(top) = stack.last_mut()
                    && *top == Ctx::ObjKey
                {
                    *top = Ctx::ObjValue;
                }
                out.push(c);
                i += 1;
            }
            ',' => {
                if let Some(top) = stack.last_mut()
                    && *top == Ctx::ObjValue
                {
                    *top = Ctx::ObjKey;
                }
                out.push(c);
                i += 1;
            }
            '"' => {
                // already quoted string: copy verbatim
                out.push(c);
                i += 1;
                while let Some(&d) = chars.get(i) {
                    out.push(d);
                    i += 1;
                    if d == '\\' {
                        if let Some(&e) = chars.get(i) {
                            out.push(e);
                            i += 1;
                        }
                    } else if d == '"' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {
                out.push(c);
                i += 1;
            }
            _ => {
                let ctx = stack.last().copied();
                let stop = |d: char| match ctx {
                    Some(Ctx::ObjKey) => matches!(d, ':' | ',' | '}'),
                    Some(Ctx::ObjValue) => matches!(d, ',' | '}' | ']'),
                    Some(Ctx::Array) => matches!(d, ',' | ']' | '}'),
                    None => false,
                };
                let start = i;
                while chars.get(i).is_some_and(|&d| !stop(d)) {
                    i += 1;
                }
                let tok: String = chars.get(start..i).unwrap_or_default().iter().collect();
                let trimmed = tok.trim_end();
                let trailing = tok.get(trimmed.len()..).unwrap_or_default();
                let is_literal = matches!(trimmed, "true" | "false" | "null")
                    || serde_json::from_str::<serde_json::Number>(trimmed).is_ok();
                if is_literal && ctx != Some(Ctx::ObjKey) {
                    out.push_str(trimmed);
                } else {
                    out.push_str(&serde_json::Value::String(trimmed.to_string()).to_string());
                }
                out.push_str(trailing);
            }
        }
    }
    out
}

/// Parse the request argument, repairing PowerShell-stripped quotes if needed.
pub fn parse_request(arg: &str) -> Result<serde_json::Value, String> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(arg) {
        return Ok(v);
    }
    let repaired = repair_json(arg);
    let v: serde_json::Value = serde_json::from_str(&repaired).map_err(|e| {
        format!("request is not valid JSON ({e}), even after re-quoting:\n  input:    {arg}\n  repaired: {repaired}")
    })?;
    if v.is_object() {
        Ok(v)
    } else {
        Err(format!("request must be a JSON object, got: {arg}"))
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let usage = "usage: cargo xtask ctl <PORT> '<json>'";
    let port: u16 = args
        .first()
        .ok_or(usage)?
        .parse()
        .map_err(|_| format!("invalid port\n{usage}"))?;
    let json = args.get(1..).unwrap_or(&[]).join(" ");
    if json.trim().is_empty() {
        return Err(usage.to_string());
    }
    let request = parse_request(&json)?;
    let line = serde_json::to_string(&request).map_err(|e| e.to_string())?;

    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&addr, TIMEOUT)
        .map_err(|e| format!("cannot connect to {addr}: {e}"))?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(TIMEOUT))
        .map_err(|e| e.to_string())?;
    stream
        .write_all(format!("{line}\n").as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|e| format!("cannot send request: {e}"))?;

    let mut reader = BufReader::new(std::io::Read::take(&stream, MAX_RESPONSE_BYTES));
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .map_err(|e| format!("cannot read response: {e}"))?;
    let response = response.trim_end();
    println!("{response}");
    let parsed: serde_json::Value =
        serde_json::from_str(response).map_err(|e| format!("response is not valid JSON: {e}"))?;
    if parsed.get("ok").and_then(serde_json::Value::as_bool) == Some(false) {
        return Err("server answered ok:false".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn valid_json_untouched() {
        let v = parse_request(r#"{"id":1,"method":"engine.query","params":{"what":"downloads"}}"#)
            .unwrap();
        assert_eq!(
            v,
            json!({"id":1,"method":"engine.query","params":{"what":"downloads"}})
        );
    }

    #[test]
    fn repairs_stripped_quotes() {
        let v = parse_request("{id:1,method:ui.set_locale,params:{locale:en}}").unwrap();
        assert_eq!(
            v,
            json!({"id":1,"method":"ui.set_locale","params":{"locale":"en"}})
        );
    }

    #[test]
    fn repairs_values_with_colons_spaces_and_literals() {
        let v = parse_request(
            "{id:2, method:cmdline.input, params:{text:add http://127.0.0.1:8099/files/big.bin --connections 8, start:true, n:-1.5e3, x:null, list:[a, 2, b c]}}",
        )
        .unwrap();
        assert_eq!(
            v,
            json!({"id":2,"method":"cmdline.input","params":{
                "text":"add http://127.0.0.1:8099/files/big.bin --connections 8",
                "start":true,"n":-1.5e3,"x":null,"list":["a",2,"b c"]}})
        );
    }

    #[test]
    fn repairs_times_and_paths() {
        let v = parse_request(
            r"{id:5,method:engine.execute,params:{command:schedule.set,args:{queue:main,start_at:23:00,dir:.udm-dev\out}}}",
        )
        .unwrap();
        assert_eq!(v["params"]["args"]["start_at"], "23:00");
        assert_eq!(v["params"]["args"]["dir"], ".udm-dev\\out");
    }

    #[test]
    fn mixed_quoted_and_bare() {
        let v =
            parse_request(r#"{id:3,"method":ui.screenshot,params:{"path":"a, b.png"}}"#).unwrap();
        assert_eq!(v["params"]["path"], "a, b.png");
        assert_eq!(v["method"], "ui.screenshot");
    }

    #[test]
    fn hopeless_input_errors() {
        assert!(parse_request("not json at all {").is_err());
    }
}
