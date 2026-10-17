//! UDM pure protocol logic: URLs, headers, filenames, ranges, segmentation, links.
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

pub mod auth;
pub mod category;
pub mod checksum;
mod civil;
pub mod disposition;
pub mod error;
pub mod filename;
pub mod ftp;
pub mod headers;
pub mod links;
pub mod range;
pub mod segment;
mod text;
pub mod url;

pub use crate::url::{is_downgrade, resolve_redirect, split_credentials, validate_download_url};
pub use auth::{
    Challenge, DigestAlgorithm, DigestChallenge, basic_authorization, digest_authorization,
    parse_www_authenticate,
};
pub use category::{CategoryRule, classify, default_category_rules};
pub use checksum::{ChecksumAlgo, ChecksumSpec, Hasher, to_hex};
pub use disposition::filename_from_content_disposition;
pub use error::ProtoError;
pub use filename::{
    auto_rename, extension, filename_from_url, has_dangerous_double_extension, sanitize_filename,
};
pub use ftp::{FtpReply, FtpReplyParser, parse_epsv, parse_mdtm, parse_pasv};
pub use headers::{
    EntityTag, format_http_date, header_value_ok, parse_content_length, parse_http_date,
};
pub use links::{expand_batch_pattern, extract_links, extract_urls_from_text, parse_url_list};
pub use range::{ContentRange, range_header, verify_content_range};
pub use segment::{ActiveSegment, plan_segments, split_for_idle};
