//! UDM core: ids, units, hard limits, i18n messages, command specs and status types.
//! This crate has no dependencies and must build for `wasm32-unknown-unknown`.
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::unreachable,
    clippy::indexing_slicing,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::arithmetic_side_effects,
    rust_2018_idioms,
    unused_must_use
)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )
)]

pub mod command;
pub mod coverage;
pub mod id;
pub mod limits;
pub mod message;
pub mod secret;
pub mod status;
pub mod units;

pub use command::{ArgKind, ArgSpec, ArgValue, CommandArgs, CommandSpec};
pub use coverage::{ByteRange, CoverageMap};
pub use id::DownloadId;
pub use message::{Message, MsgArg};
pub use secret::Secret;
pub use status::{DownloadStatus, PauseReason};

/// Native messaging host name shared by the extension, the host and its registration.
pub const NATIVE_HOST_NAME: &str = "dev.udm.native_host";
/// Firefox add-on id of the UDM extension.
pub const FIREFOX_EXTENSION_ID: &str = "udm@udm.dev";
