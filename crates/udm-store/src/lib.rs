//! UDM atomic storage: state, queue, settings and history files.
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

mod atomic;
mod datadir;
mod envelope;
mod error;
mod hash;
mod history;

pub use atomic::atomic_write;
pub use datadir::DataDir;
pub use envelope::{load_envelope, save_envelope};
pub use error::StoreError;
pub use history::HistoryLog;
