//! Shared helpers for udm-net integration tests (local test server only).
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use sha2::{Digest, Sha256};
use udm_core::ByteRange;
use udm_net::{BodyStream, Client, FetchBody, NetConfig, NetError, Request, Timeouts};

pub fn cfg() -> NetConfig {
    NetConfig {
        timeouts: Timeouts {
            connect_ms: 5_000,
            tls_ms: 5_000,
            first_byte_ms: 5_000,
            idle_ms: 5_000,
        },
        ..NetConfig::default()
    }
}

pub fn client() -> Client {
    Client::new(cfg()).unwrap()
}

pub fn req(url: &str) -> Request {
    Request::new(url::Url::parse(url).unwrap())
}

pub fn range(a: u64, b: u64) -> ByteRange {
    ByteRange::new(a, b).unwrap()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

/// Reads a body to the end; returns bytes read so far and the terminal result.
pub async fn read_all(body: &mut BodyStream) -> (Vec<u8>, Result<(), NetError>) {
    let mut out = Vec::new();
    loop {
        match body.next_chunk().await {
            Ok(Some(c)) => out.extend_from_slice(&c),
            Ok(None) => return (out, Ok(())),
            Err(e) => return (out, Err(e)),
        }
    }
}

pub fn unwrap_body(b: FetchBody) -> (bool, BodyStream) {
    match b {
        FetchBody::Partial(s) => (true, s),
        FetchBody::Full(s) => (false, s),
    }
}
