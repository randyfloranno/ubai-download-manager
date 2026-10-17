//! UDM control protocol: newline-delimited JSON over TCP on 127.0.0.1, the
//! `cmdline.input` parser, and the restricted capture IPC endpoint used by the
//! native messaging host.
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

pub mod client;
pub mod cmdline;
pub mod handler;
pub mod ipc;
pub mod protocol;
pub mod server;

pub use client::ControlClient;
pub use handler::{BoxFuture, CaptureHandler, EngineHandler, Handler, UiBridge};
pub use ipc::IpcInfo;
pub use protocol::{Request, message_to_json};
pub use server::{ControlServer, ServerHandle};
