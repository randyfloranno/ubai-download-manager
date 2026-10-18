//! UDM OS services: Downloads folder, disk space, pre-allocation, file writer, power, clipboard, notifications.
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

mod desktop;
mod error;
mod folders;
mod path;
mod power;
mod space;
mod writer;

/// Test doubles for other crates' tests (feature `test-util`).
#[cfg(any(test, feature = "test-util"))]
pub mod testing;

pub use desktop::{clipboard_text, notify, open_path};
pub use error::SysError;
pub use folders::downloads_dir;
pub use path::{long_path, rename_no_overwrite};
pub use power::{POWER_DRY_RUN_ENV, PowerAction, perform_power_action};
pub use space::{available_space, ensure_space};
pub use writer::{FileWriter, PartFile, WriteFailure, classify_write_error};
