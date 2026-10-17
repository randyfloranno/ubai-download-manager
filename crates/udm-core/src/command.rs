//! Command specifications shared by every front-end (GUI, CLI, control port,
//! native host, scheduler). Every user action is a `CommandSpec`.

use crate::id::DownloadId;
use crate::limits;
use crate::message::Message;

/// Kind of a command argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgKind {
    /// UTF-8 text.
    Str,
    /// Unsigned integer.
    UInt,
    /// Boolean.
    Bool,
    /// List of strings.
    StrList,
    /// A download id (`d-000001`).
    DownloadId,
    /// One of a fixed set of strings.
    Enum(&'static [&'static str]),
}

/// One argument of a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArgSpec {
    /// Argument name in lower snake case.
    pub name: &'static str,
    /// Expected kind.
    pub kind: ArgKind,
    /// Whether the argument is required.
    pub required: bool,
}

impl ArgSpec {
    /// Required argument.
    pub const fn required(name: &'static str, kind: ArgKind) -> Self {
        Self {
            name,
            kind,
            required: true,
        }
    }

    /// Optional argument.
    pub const fn optional(name: &'static str, kind: ArgKind) -> Self {
        Self {
            name,
            kind,
            required: false,
        }
    }
}

/// A registered command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    /// `<area>.<verb>` in lower snake case.
    pub id: &'static str,
    /// i18n key of the label.
    pub label_key: &'static str,
    /// i18n key of the description.
    pub description_key: &'static str,
    /// Arguments.
    pub args: &'static [ArgSpec],
    /// Whether `"confirm": true` is required.
    pub needs_confirmation: bool,
}

impl CommandSpec {
    /// Checks that the id follows `<area>.<verb>` in lower snake case.
    pub fn id_is_well_formed(&self) -> bool {
        let mut parts = self.id.split('.');
        let ok_part = |p: &str| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                && p.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        };
        matches!((parts.next(), parts.next(), parts.next()), (Some(a), Some(v), None) if ok_part(a) && ok_part(v))
    }

    /// Looks up an argument spec.
    pub fn arg(&self, name: &str) -> Option<&ArgSpec> {
        self.args.iter().find(|a| a.name == name)
    }
}

/// A typed argument value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgValue {
    /// Absent / null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Unsigned integer.
    UInt(u64),
    /// Text.
    Str(String),
    /// List of texts.
    List(Vec<String>),
}

/// Arguments of one command invocation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandArgs {
    values: Vec<(String, ArgValue)>,
}

impl CommandArgs {
    /// Empty argument set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets (or replaces) an argument.
    pub fn set(&mut self, name: &str, value: ArgValue) {
        if let Some(slot) = self.values.iter_mut().find(|(n, _)| n == name) {
            slot.1 = value;
        } else {
            self.values.push((name.to_owned(), value));
        }
    }

    /// Builder form of [`CommandArgs::set`].
    pub fn with(mut self, name: &str, value: ArgValue) -> Self {
        self.set(name, value);
        self
    }

    /// All values.
    pub fn iter(&self) -> impl Iterator<Item = &(String, ArgValue)> {
        self.values.iter()
    }

    /// Raw lookup; `Null` counts as absent.
    pub fn get(&self, name: &str) -> Option<&ArgValue> {
        self.values
            .iter()
            .find(|(n, v)| n == name && *v != ArgValue::Null)
            .map(|(_, v)| v)
    }

    /// String argument.
    pub fn str(&self, name: &str) -> Option<&str> {
        match self.get(name) {
            Some(ArgValue::Str(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Unsigned argument.
    pub fn uint(&self, name: &str) -> Option<u64> {
        match self.get(name) {
            Some(ArgValue::UInt(v)) => Some(*v),
            _ => None,
        }
    }

    /// Boolean argument.
    pub fn bool(&self, name: &str) -> Option<bool> {
        match self.get(name) {
            Some(ArgValue::Bool(v)) => Some(*v),
            _ => None,
        }
    }

    /// List argument.
    pub fn list(&self, name: &str) -> Option<&[String]> {
        match self.get(name) {
            Some(ArgValue::List(v)) => Some(v.as_slice()),
            _ => None,
        }
    }

    /// Download-id argument.
    pub fn download_id(&self, name: &str) -> Option<DownloadId> {
        self.str(name).and_then(|s| s.parse().ok())
    }

    /// Validates against a spec: unknown args, kinds, required args and size bounds.
    /// `confirm` is always accepted as an optional boolean.
    pub fn validate(&self, spec: &CommandSpec) -> Result<(), Message> {
        if self.values.len() > limits::MAX_COMMAND_ARGS {
            return Err(Message::new("error-command-too-many-args")
                .with_uint("max", usize_to_u64(limits::MAX_COMMAND_ARGS)));
        }
        for (name, value) in &self.values {
            if name == "confirm" {
                if !matches!(value, ArgValue::Bool(_) | ArgValue::Null) {
                    return Err(bad_arg(spec, name));
                }
                continue;
            }
            let Some(arg) = spec.arg(name) else {
                return Err(Message::new("error-command-unknown-arg")
                    .with_str("command", spec.id)
                    .with_str("arg", name.as_str()));
            };
            if !value_matches(arg.kind, value) {
                return Err(bad_arg(spec, name));
            }
        }
        for arg in spec.args.iter().filter(|a| a.required) {
            if self.get(arg.name).is_none() {
                return Err(Message::new("error-command-missing-arg")
                    .with_str("command", spec.id)
                    .with_str("arg", arg.name));
            }
        }
        if spec.needs_confirmation && self.bool("confirm") != Some(true) {
            return Err(
                Message::new("error-command-needs-confirmation").with_str("command", spec.id)
            );
        }
        Ok(())
    }
}

fn usize_to_u64(v: usize) -> u64 {
    u64::try_from(v).unwrap_or(u64::MAX)
}

fn bad_arg(spec: &CommandSpec, name: &str) -> Message {
    Message::new("error-command-bad-arg")
        .with_str("command", spec.id)
        .with_str("arg", name)
}

fn value_matches(kind: ArgKind, value: &ArgValue) -> bool {
    match (kind, value) {
        (_, ArgValue::Null) => true,
        (ArgKind::Str, ArgValue::Str(s)) => s.len() <= limits::MAX_ARG_STRING_BYTES,
        (ArgKind::UInt, ArgValue::UInt(_)) | (ArgKind::Bool, ArgValue::Bool(_)) => true,
        (ArgKind::StrList, ArgValue::List(items)) => {
            items.len() <= limits::MAX_ARG_LIST_ITEMS
                && items
                    .iter()
                    .all(|s| s.len() <= limits::MAX_ARG_STRING_BYTES)
        }
        (ArgKind::DownloadId, ArgValue::Str(s)) => s.parse::<DownloadId>().is_ok(),
        (ArgKind::Enum(options), ArgValue::Str(s)) => options.contains(&s.as_str()),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: CommandSpec = CommandSpec {
        id: "download.pause",
        label_key: "cmd-download-pause",
        description_key: "cmd-download-pause-desc",
        args: &[
            ArgSpec::required("download_id", ArgKind::DownloadId),
            ArgSpec::optional("mode", ArgKind::Enum(&["soft", "hard"])),
        ],
        needs_confirmation: false,
    };

    #[test]
    fn id_well_formed() {
        assert!(SPEC.id_is_well_formed());
        let bad = CommandSpec {
            id: "Download.Pause",
            ..SPEC
        };
        assert!(!bad.id_is_well_formed());
        let bad = CommandSpec {
            id: "download",
            ..SPEC
        };
        assert!(!bad.id_is_well_formed());
        let bad = CommandSpec {
            id: "a.b.c",
            ..SPEC
        };
        assert!(!bad.id_is_well_formed());
    }

    #[test]
    fn validate_ok_and_missing() {
        let args = CommandArgs::new().with("download_id", ArgValue::Str("d-000001".into()));
        assert!(args.validate(&SPEC).is_ok());
        let err = CommandArgs::new().validate(&SPEC).unwrap_err();
        assert_eq!(err.key, "error-command-missing-arg");
    }

    #[test]
    fn hostile_command_args_unknown_and_wrong_kind() {
        let args = CommandArgs::new()
            .with("download_id", ArgValue::Str("d-000001".into()))
            .with("evil", ArgValue::Bool(true));
        assert_eq!(
            args.validate(&SPEC).unwrap_err().key,
            "error-command-unknown-arg"
        );
        let args = CommandArgs::new().with("download_id", ArgValue::UInt(1));
        assert_eq!(
            args.validate(&SPEC).unwrap_err().key,
            "error-command-bad-arg"
        );
        let args = CommandArgs::new()
            .with("download_id", ArgValue::Str("d-000001".into()))
            .with("mode", ArgValue::Str("nuke".into()));
        assert_eq!(
            args.validate(&SPEC).unwrap_err().key,
            "error-command-bad-arg"
        );
    }

    #[test]
    fn hostile_command_args_too_many_and_too_long() {
        let mut args = CommandArgs::new();
        for i in 0..=limits::MAX_COMMAND_ARGS {
            args.set(&format!("a{i}"), ArgValue::Null);
        }
        assert_eq!(
            args.validate(&SPEC).unwrap_err().key,
            "error-command-too-many-args"
        );
        const ARGS: &[ArgSpec] = &[
            ArgSpec::required("s", ArgKind::Str),
            ArgSpec::optional("l", ArgKind::StrList),
        ];
        let spec = CommandSpec { args: ARGS, ..SPEC };
        let long = "x".repeat(limits::MAX_ARG_STRING_BYTES + 1);
        let args = CommandArgs::new().with("s", ArgValue::Str(long));
        assert!(args.validate(&spec).is_err());
        let list = vec![String::new(); limits::MAX_ARG_LIST_ITEMS + 1];
        let args = CommandArgs::new()
            .with("s", ArgValue::Str("ok".into()))
            .with("l", ArgValue::List(list));
        assert!(args.validate(&spec).is_err());
    }

    #[test]
    fn confirmation_required() {
        let spec = CommandSpec {
            needs_confirmation: true,
            ..SPEC
        };
        let args = CommandArgs::new().with("download_id", ArgValue::Str("d-000001".into()));
        assert_eq!(
            args.validate(&spec).unwrap_err().key,
            "error-command-needs-confirmation"
        );
        let args = args.with("confirm", ArgValue::Bool(true));
        assert!(args.validate(&spec).is_ok());
    }
}
