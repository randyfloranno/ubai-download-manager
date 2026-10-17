//! UDM download engine: segments, queues, scheduler, limiter, commands.
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

pub mod board;
pub mod checksum;
pub mod commands;
pub mod engine;
pub mod error;
pub mod grabber;
pub mod limiter;
pub mod model;
pub mod platform;
pub mod registry;
pub mod schedule;
pub mod settings;
pub mod task;

pub use commands::{AddSpec, args_from_json};
pub use engine::{Engine, EngineConfig, EngineEvent, MAIN_QUEUE, Stats};
pub use error::{EngineError, EngineResult};
pub use model::{DownloadView, HistoryEntry, QueueRecord, StoredMessage};
pub use platform::{Platform, SystemPlatform};
pub use settings::Settings;
