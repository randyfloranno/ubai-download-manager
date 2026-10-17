//! `cmdline.input`: a small command language for the control port and the CLI.
//!
//! ```text
//! add http://127.0.0.1:8099/files/big.bin --connections 8
//! pause d-000001
//! remove d-000001 --delete-file --confirm
//! limit.set_global --bytes-per-sec 524288
//! schedule.set --start-at 23:00 --days mon,tue
//! ```
//!
//! The first word is a short alias (`add`, `pause`, ...) or a full command id.
//! Positional words fill the command's required arguments in order; `--name value`
//! sets an argument by name (`-` and `_` are equivalent); `--flag` alone is `true`.
//! Values are typed by the command's [`ArgSpec`].

use udm_core::{ArgKind, ArgSpec, ArgValue, CommandArgs, CommandSpec, Message, limits};
use udm_engine::registry;

/// Short aliases for common commands.
const ALIASES: &[(&str, &str)] = &[
    ("add", "download.add"),
    ("pause", "download.pause"),
    ("resume", "download.resume"),
    ("remove", "download.remove"),
    ("restart", "download.restart"),
    ("verify", "download.verify"),
    ("pause-all", "download.pause_all"),
    ("resume-all", "download.resume_all"),
    ("start", "queue.start"),
    ("stop", "queue.stop"),
    ("limit", "limit.set_global"),
    ("batch", "batch.import"),
    ("grab", "grabber.start"),
    ("set", "settings.set"),
];

fn usize_u64(v: usize) -> u64 {
    u64::try_from(v).unwrap_or(u64::MAX)
}

/// Splits text into words. Double or single quotes group words; there are no escapes.
pub fn tokenize(text: &str) -> Result<Vec<String>, Message> {
    if text.len() > limits::MAX_CMDLINE_BYTES {
        return Err(Message::new("error-cmdline-too-long")
            .with_uint("max", usize_u64(limits::MAX_CMDLINE_BYTES)));
    }
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                in_word = true;
            }
            None if c.is_whitespace() => {
                if in_word {
                    push_word(&mut out, std::mem::take(&mut cur))?;
                    in_word = false;
                }
            }
            None => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err(Message::new("error-cmdline-unterminated-quote"));
    }
    if in_word {
        push_word(&mut out, cur)?;
    }
    Ok(out)
}

fn push_word(out: &mut Vec<String>, w: String) -> Result<(), Message> {
    if out.len() >= limits::MAX_CMDLINE_TOKENS {
        return Err(Message::new("error-cmdline-too-many-tokens")
            .with_uint("max", usize_u64(limits::MAX_CMDLINE_TOKENS)));
    }
    out.push(w);
    Ok(())
}

/// Resolves an alias or a full command id.
pub fn resolve(word: &str) -> Option<&'static CommandSpec> {
    let id = ALIASES
        .iter()
        .find(|(a, _)| *a == word)
        .map_or(word, |(_, id)| *id);
    registry::find(id)
}

/// Parses a command line into a command id and its arguments.
/// Argument validation (kinds, required, confirmation) is left to the engine.
pub fn parse(text: &str) -> Result<(&'static str, CommandArgs), Message> {
    let words = tokenize(text)?;
    let mut it = words.into_iter().peekable();
    let Some(first) = it.next() else {
        return Err(Message::new("error-cmdline-empty"));
    };
    let spec = resolve(&first)
        .ok_or_else(|| Message::new("error-command-unknown").with_str("command", first.as_str()))?;
    let mut args = CommandArgs::new();
    let mut positional = spec.args.iter().filter(|a| a.required);
    while let Some(w) = it.next() {
        if let Some(name) = w.strip_prefix("--") {
            let name = name.replace('-', "_");
            if name == "confirm" {
                args.set("confirm", ArgValue::Bool(true));
                continue;
            }
            let Some(arg) = spec.arg(&name) else {
                return Err(Message::new("error-command-unknown-arg")
                    .with_str("command", spec.id)
                    .with_str("arg", name.as_str()));
            };
            let value = if arg.kind == ArgKind::Bool {
                match it.peek().map(String::as_str) {
                    Some("true" | "false") => it.next(),
                    _ => None,
                }
            } else {
                it.next()
            };
            args.set(arg.name, typed(spec, arg, value.as_deref())?);
        } else {
            let arg = positional
                .by_ref()
                .find(|a| args.get(a.name).is_none())
                .ok_or_else(|| {
                    Message::new("error-cmdline-extra-word").with_str("word", w.as_str())
                })?;
            args.set(arg.name, typed(spec, arg, Some(&w))?);
        }
    }
    Ok((spec.id, args))
}

fn typed(spec: &CommandSpec, arg: &ArgSpec, value: Option<&str>) -> Result<ArgValue, Message> {
    let bad = || {
        Message::new("error-command-bad-arg")
            .with_str("command", spec.id)
            .with_str("arg", arg.name)
    };
    match (arg.kind, value) {
        (ArgKind::Bool, None) => Ok(ArgValue::Bool(true)),
        (_, None) => Err(Message::new("error-command-missing-arg")
            .with_str("command", spec.id)
            .with_str("arg", arg.name)),
        (ArgKind::Bool, Some(v)) => v.parse().map(ArgValue::Bool).map_err(|_| bad()),
        (ArgKind::UInt, Some(v)) => v.parse().map(ArgValue::UInt).map_err(|_| bad()),
        (ArgKind::StrList, Some(v)) => {
            let items: Vec<String> = v
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            if items.len() > limits::MAX_ARG_LIST_ITEMS {
                return Err(bad());
            }
            Ok(ArgValue::List(items))
        }
        (ArgKind::Str | ArgKind::DownloadId | ArgKind::Enum(_), Some(v)) => {
            Ok(ArgValue::Str(v.to_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_with_options() {
        let (id, a) =
            parse("add http://127.0.0.1:8099/files/big.bin --connections 8 --start false").unwrap();
        assert_eq!(id, "download.add");
        assert_eq!(a.str("url"), Some("http://127.0.0.1:8099/files/big.bin"));
        assert_eq!(a.uint("connections"), Some(8));
        assert_eq!(a.bool("start"), Some(false));
        a.validate(registry::find(id).unwrap()).unwrap();
    }

    #[test]
    fn full_id_flags_and_lists() {
        let (id, a) = parse("remove d-000001 --delete-file --confirm").unwrap();
        assert_eq!(id, "download.remove");
        assert_eq!(a.bool("delete_file"), Some(true));
        assert_eq!(a.bool("confirm"), Some(true));
        a.validate(registry::find(id).unwrap()).unwrap();
        let (id, a) =
            parse("schedule.set --start-at 23:00 --days 'mon, tue' --after-done sleep").unwrap();
        assert_eq!(id, "schedule.set");
        assert_eq!(
            a.list("days"),
            Some(&["mon".to_owned(), "tue".to_owned()][..])
        );
        a.validate(registry::find(id).unwrap()).unwrap();
    }

    #[test]
    fn quoted_words() {
        assert_eq!(
            tokenize(r#"add "http://x/a b.bin" --dir 'C:\My Files'"#).unwrap(),
            vec!["add", "http://x/a b.bin", "--dir", r"C:\My Files"]
        );
    }

    #[test]
    fn hostile_cmdline_garbage() {
        assert_eq!(parse("").unwrap_err().key, "error-cmdline-empty");
        assert_eq!(
            parse("frobnicate").unwrap_err().key,
            "error-command-unknown"
        );
        assert_eq!(
            parse("pause d-1 extra").unwrap_err().key,
            "error-cmdline-extra-word"
        );
        assert_eq!(
            parse("add x --bogus 1").unwrap_err().key,
            "error-command-unknown-arg"
        );
        assert_eq!(
            parse("add x --connections many").unwrap_err().key,
            "error-command-bad-arg"
        );
        assert_eq!(
            parse("add x --connections").unwrap_err().key,
            "error-command-missing-arg"
        );
        assert_eq!(
            parse("add \"x").unwrap_err().key,
            "error-cmdline-unterminated-quote"
        );
    }

    #[test]
    fn hostile_cmdline_too_long() {
        let text = "a".repeat(limits::MAX_CMDLINE_BYTES + 1);
        assert_eq!(tokenize(&text).unwrap_err().key, "error-cmdline-too-long");
    }

    #[test]
    fn hostile_cmdline_too_many_tokens() {
        let text = "a ".repeat(limits::MAX_CMDLINE_TOKENS + 1);
        assert_eq!(
            tokenize(&text).unwrap_err().key,
            "error-cmdline-too-many-tokens"
        );
        let ok = "a ".repeat(limits::MAX_CMDLINE_TOKENS);
        assert_eq!(tokenize(&ok).unwrap().len(), limits::MAX_CMDLINE_TOKENS);
    }
}
