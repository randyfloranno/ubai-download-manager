//! Fluent-backed localiser with `en` fallback. Never panics: a key missing in
//! both catalogues renders as the key itself.

use std::borrow::Cow;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use udm_core::{Message, MsgArg};

use crate::format::{format_bytes, format_number};
use crate::locale::Locale;
use crate::{CATALOGUE_EN, CATALOGUE_ID};

/// An argument for [`Localizer::tr_args`].
#[derive(Clone, Debug)]
pub enum TrArg<'a> {
    /// Text.
    Str(Cow<'a, str>),
    /// Number (used for plural selection, rendered with locale grouping).
    Num(u64),
    /// Byte count rendered with IEC units.
    Bytes(u64),
}

/// Localiser for one active locale.
pub struct Localizer {
    locale: Locale,
    primary: FluentBundle<FluentResource>,
    fallback: Option<FluentBundle<FluentResource>>,
}

impl std::fmt::Debug for Localizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Localizer")
            .field("locale", &self.locale)
            .finish()
    }
}

fn build_bundle(locale: Locale) -> FluentBundle<FluentResource> {
    let source = match locale {
        Locale::Id => CATALOGUE_ID,
        Locale::En => CATALOGUE_EN,
    };
    let lang: unic_langid::LanguageIdentifier = locale.as_str().parse().unwrap_or_default();
    let mut bundle = FluentBundle::new_concurrent(vec![lang]);
    // Bidi isolation marks render as boxes in egui; our arguments are plain text.
    bundle.set_use_isolating(false);
    // A catalogue with syntax errors still yields every valid message;
    // `cargo xtask i18n` and the tests below guarantee there are none.
    let resource = match FluentResource::try_new(source.to_owned()) {
        Ok(r) => r,
        Err((r, _errors)) => r,
    };
    // Duplicate ids are rejected by xtask; ignore here to stay panic-free.
    let _ = bundle.add_resource(resource);
    bundle
}

impl Localizer {
    /// Creates a localiser; `en` is used as fallback for `id`.
    pub fn new(locale: Locale) -> Self {
        let fallback = match locale {
            Locale::En => None,
            Locale::Id => Some(build_bundle(Locale::En)),
        };
        Self {
            locale,
            primary: build_bundle(locale),
            fallback,
        }
    }

    /// Active locale.
    pub fn locale(&self) -> Locale {
        self.locale
    }

    /// True if the key exists in the active or fallback catalogue.
    pub fn has(&self, key: &str) -> bool {
        self.primary.has_message(key) || self.fallback.as_ref().is_some_and(|b| b.has_message(key))
    }

    /// Translates a key without arguments.
    pub fn tr(&self, key: &str) -> String {
        self.render(key, None)
    }

    /// Translates a key with arguments.
    pub fn tr_args(&self, key: &str, args: &[(&str, TrArg<'_>)]) -> String {
        let mut fargs = FluentArgs::new();
        for (name, value) in args {
            let v: FluentValue<'_> = match value {
                TrArg::Str(s) => FluentValue::from(s.to_string()),
                TrArg::Num(n) => FluentValue::from(format_number(self.locale, *n)),
                TrArg::Bytes(b) => FluentValue::from(format_bytes(self.locale, *b)),
            };
            fargs.set(*name, v);
            if let TrArg::Num(n) = value {
                // Numeric twin for plural selectors: `$count-n`.
                fargs.set(format!("{name}-n"), FluentValue::from(*n));
            }
        }
        self.render(key, Some(&fargs))
    }

    /// Renders a [`Message`] produced by engine or protocol code.
    pub fn message(&self, msg: &Message) -> String {
        let args: Vec<(&str, TrArg<'_>)> = msg
            .args
            .iter()
            .map(|(name, value)| {
                let v = match value {
                    MsgArg::Str(s) => TrArg::Str(Cow::Borrowed(s.as_str())),
                    MsgArg::UInt(n) => TrArg::Num(*n),
                    MsgArg::Int(n) => TrArg::Str(Cow::Owned(n.to_string())),
                    MsgArg::Bytes(b) => TrArg::Bytes(*b),
                };
                (name.as_str(), v)
            })
            .collect();
        self.tr_args(&msg.key, &args)
    }

    fn render(&self, key: &str, args: Option<&FluentArgs<'_>>) -> String {
        for bundle in std::iter::once(&self.primary).chain(self.fallback.as_ref()) {
            if let Some(pattern) = bundle.get_message(key).and_then(|m| m.value()) {
                let mut errors = Vec::new();
                return bundle
                    .format_pattern(pattern, args, &mut errors)
                    .into_owned();
            }
        }
        key.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogues_parse_without_errors() {
        for src in [CATALOGUE_ID, CATALOGUE_EN] {
            assert!(FluentResource::try_new(src.to_owned()).is_ok());
        }
    }

    #[test]
    fn default_is_indonesian() {
        let l = Localizer::new(Locale::default());
        assert_eq!(l.tr("status-paused"), "Dijeda");
        let en = Localizer::new(Locale::En);
        assert_eq!(en.tr("status-paused"), "Paused");
    }

    #[test]
    fn fallback_to_en_and_key() {
        let l = Localizer::new(Locale::Id);
        assert_eq!(l.tr("definitely-missing-key"), "definitely-missing-key");
        assert!(!l.has("definitely-missing-key"));
    }

    #[test]
    fn message_with_bytes_arg() {
        let l = Localizer::new(Locale::Id);
        let m = Message::new("error-sys-not-enough-space")
            .with_str("volume", "D:\\")
            .with_bytes("needed", 1_572_864)
            .with_bytes("available", 1_024)
            .with_bytes("shortfall", 1_571_840);
        let text = l.message(&m);
        assert!(text.contains("1,5 MiB"), "{text}");
        assert!(text.contains("D:\\"), "{text}");
    }
}
