//! UDM networking: HTTP/1.1, HTTP/2, TLS, FTP/FTPS, proxies.
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

mod body;
mod client;
mod config;
mod connect;
mod error;
mod ftp;
mod http;
mod tls;

pub use body::BodyStream;
pub use client::{Client, FetchBody};
pub use config::{NetConfig, ProbeInfo, Protocol, ProxyConfig, Request, Timeouts, TlsException};
pub use error::{NetError, TimeoutPhase};
pub use tls::cert_fingerprint;
