//! wasm-bindgen exports used by `extension/glue/background.js`. Strings in,
//! strings out: the glue never interprets anything but the returned JSON.

use wasm_bindgen::prelude::wasm_bindgen;

use crate::capture::{self, Settings};

/// Native messaging host name.
#[wasm_bindgen]
pub fn host_name() -> String {
    capture::HOST_NAME.to_owned()
}

/// Default settings as JSON.
#[wasm_bindgen]
pub fn default_settings() -> String {
    serde_json::to_string(&Settings::default()).unwrap_or_default()
}

/// Decision for one browser event (see [`capture::decide`]).
#[wasm_bindgen]
pub fn decide(settings_json: &str, event_json: &str) -> String {
    capture::decide(&Settings::parse(settings_json), event_json)
        .to_json()
        .to_string()
}

/// `Cookie` header for a `cookies.getAll` result.
#[wasm_bindgen]
pub fn cookie_header(cookies_json: &str) -> String {
    capture::cookie_header(cookies_json)
}

/// Toolbar badge and text for a native-host reply.
#[wasm_bindgen]
pub fn reply(locale: &str, reply_json: &str) -> String {
    capture::reply(locale, reply_json).to_string()
}

/// Context menus to create.
#[wasm_bindgen]
pub fn menus(locale: &str) -> String {
    capture::menus(locale).to_string()
}
