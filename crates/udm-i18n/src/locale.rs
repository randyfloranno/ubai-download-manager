//! Supported UI locales.

/// A supported UI locale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Locale {
    /// Indonesian (default).
    #[default]
    Id,
    /// English (fallback).
    En,
}

impl Locale {
    /// All supported locales.
    pub const ALL: [Locale; 2] = [Locale::Id, Locale::En];

    /// BCP 47 tag.
    pub const fn as_str(self) -> &'static str {
        match self {
            Locale::Id => "id",
            Locale::En => "en",
        }
    }

    /// Parses a tag such as `id`, `id-ID`, `en`, `en-US` (case-insensitive).
    pub fn parse(tag: &str) -> Option<Self> {
        let primary = tag
            .split(['-', '_'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match primary.as_str() {
            "id" | "in" => Some(Locale::Id),
            "en" => Some(Locale::En),
            _ => None,
        }
    }

    /// Decimal separator.
    pub const fn decimal_separator(self) -> char {
        match self {
            Locale::Id => ',',
            Locale::En => '.',
        }
    }

    /// Thousands grouping separator.
    pub const fn group_separator(self) -> char {
        match self {
            Locale::Id => '.',
            Locale::En => ',',
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tags() {
        assert_eq!(Locale::parse("id-ID"), Some(Locale::Id));
        assert_eq!(Locale::parse("EN_us"), Some(Locale::En));
        assert_eq!(Locale::parse("fr"), None);
        assert_eq!(Locale::parse(""), None);
    }
}
