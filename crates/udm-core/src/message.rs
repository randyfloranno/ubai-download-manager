//! Localisable messages: an i18n key plus typed arguments.
//! Engine and protocol code never format display text; they return a `Message`.

use core::fmt;

/// A typed message argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MsgArg {
    /// Plain text (file names, hosts, ...).
    Str(String),
    /// Signed integer.
    Int(i64),
    /// Unsigned integer (counts).
    UInt(u64),
    /// A byte count, formatted with IEC units by the localiser.
    Bytes(u64),
}

impl fmt::Display for MsgArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MsgArg::Str(s) => f.write_str(s),
            MsgArg::Int(v) => write!(f, "{v}"),
            MsgArg::UInt(v) | MsgArg::Bytes(v) => write!(f, "{v}"),
        }
    }
}

/// An i18n key with arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// Fluent message id, e.g. `error-net-timeout`.
    pub key: String,
    /// Named arguments.
    pub args: Vec<(String, MsgArg)>,
}

impl Message {
    /// Message without arguments.
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            args: Vec::new(),
        }
    }

    /// Adds a string argument.
    pub fn with_str(mut self, name: &str, value: impl Into<String>) -> Self {
        self.args.push((name.to_owned(), MsgArg::Str(value.into())));
        self
    }

    /// Adds an unsigned integer argument.
    pub fn with_uint(mut self, name: &str, value: u64) -> Self {
        self.args.push((name.to_owned(), MsgArg::UInt(value)));
        self
    }

    /// Adds a signed integer argument.
    pub fn with_int(mut self, name: &str, value: i64) -> Self {
        self.args.push((name.to_owned(), MsgArg::Int(value)));
        self
    }

    /// Adds a byte-count argument.
    pub fn with_bytes(mut self, name: &str, value: u64) -> Self {
        self.args.push((name.to_owned(), MsgArg::Bytes(value)));
        self
    }

    /// Looks up an argument by name.
    pub fn arg(&self, name: &str) -> Option<&MsgArg> {
        self.args.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
}

impl fmt::Display for Message {
    /// Debug-oriented rendering (`key{a=1}`); UIs must use the localiser.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.key)?;
        if !self.args.is_empty() {
            f.write_str("{")?;
            for (i, (n, v)) in self.args.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{n}={v}")?;
            }
            f.write_str("}")?;
        }
        Ok(())
    }
}

/// Anything that can be shown to the user as a localised message.
pub trait ToMessage {
    /// Converts into a localisable message.
    fn to_message(&self) -> Message;
}

impl ToMessage for Message {
    fn to_message(&self) -> Message {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_with_args() {
        let m = Message::new("error-x")
            .with_uint("n", 3)
            .with_str("f", "a.bin");
        assert_eq!(m.to_string(), "error-x{n=3, f=a.bin}");
        assert_eq!(m.arg("n"), Some(&MsgArg::UInt(3)));
    }
}
