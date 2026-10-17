//! Output: JSON on stdout, localized errors on stderr.
#![allow(clippy::print_stdout)]

use serde_json::Value;
use udm_core::Message;
use udm_i18n::Localizer;

/// One JSON value per line.
pub fn json(v: &Value) {
    println!("{v}");
}

/// A raw response line.
pub fn line(l: &str) {
    println!("{l}");
}

/// A startup notice (e.g. a corrupt state file), as JSON.
pub fn notice(m: &Message) {
    println!(
        "{}",
        serde_json::json!({ "notice": udm_control::message_to_json(m) })
    );
}

/// A localized error with its key.
pub fn error(loc: &Localizer, m: &Message) {
    eprintln!("{} [{}]", loc.message(m), m.key);
}
