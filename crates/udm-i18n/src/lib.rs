//! UDM localisation: Fluent catalogues (`id` default, `en` fallback) and
//! locale-aware formatting of numbers, sizes, speeds and durations.
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

mod format;
mod locale;
mod localizer;

pub use format::{format_bytes, format_duration, format_number, format_percent, format_speed};
pub use locale::Locale;
pub use localizer::{Localizer, TrArg};

/// Raw Indonesian catalogue.
pub const CATALOGUE_ID: &str = include_str!("../locales/id/udm.ftl");
/// Raw English catalogue.
pub const CATALOGUE_EN: &str = include_str!("../locales/en/udm.ftl");
