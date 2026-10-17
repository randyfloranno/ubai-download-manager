//! Typed protocol errors. Every variant maps to an i18n key `error-proto-*`.

use core::fmt;

use udm_core::Message;

/// Errors produced by the pure protocol layer.
///
/// Variants never carry raw hostile input; only bounded numbers or values
/// re-formatted from parsed integers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProtoError {
    /// URL longer than `limits::MAX_URL_BYTES`.
    UrlTooLong {
        /// Actual length in bytes.
        len: usize,
        /// Allowed maximum.
        max: usize,
    },
    /// URL could not be parsed.
    UrlInvalid,
    /// URL scheme is not http, https, ftp or ftps.
    UrlScheme {
        /// The (bounded, lower-case) scheme that was refused.
        scheme: String,
    },
    /// URL has no host.
    UrlNoHost,
    /// A redirect `Location` could not be resolved to a valid download URL.
    RedirectInvalid,
    /// A header value is longer than `limits::MAX_HEADER_VALUE_BYTES` or has CR/LF/NUL.
    HeaderValueInvalid {
        /// Actual length in bytes.
        len: usize,
        /// Allowed maximum.
        max: usize,
    },
    /// A partial response had no `Content-Range` header.
    ContentRangeMissing,
    /// `Content-Range` could not be parsed.
    ContentRangeInvalid,
    /// `Content-Range` does not match the requested range or expected total.
    ContentRangeMismatch {
        /// What was requested, `start-end/total`.
        expected: String,
        /// What the server sent, re-formatted from parsed numbers.
        actual: String,
    },
    /// `WWW-Authenticate` could not be parsed.
    AuthChallengeInvalid,
    /// FTP control line longer than `limits::MAX_FTP_LINE_BYTES`.
    FtpLineTooLong {
        /// Allowed maximum.
        max: usize,
    },
    /// Multi-line FTP reply with more than `limits::MAX_FTP_REPLY_LINES` lines.
    FtpTooManyLines {
        /// Allowed maximum.
        max: usize,
    },
    /// FTP reply line without a valid three-digit code.
    FtpReplyInvalid,
    /// URL list larger than `limits::MAX_URL_LIST_BYTES`.
    UrlListTooLarge {
        /// Actual size in bytes.
        bytes: u64,
        /// Allowed maximum in bytes.
        max: u64,
    },
    /// More than `limits::MAX_BATCH_SIZE` URLs.
    BatchTooLarge {
        /// Number of URLs (saturated at `u64::MAX`).
        count: u64,
        /// Allowed maximum.
        max: u64,
    },
    /// A batch pattern group such as `[9-1]` or `[a-Z]` is invalid.
    BatchPatternInvalid,
    /// A checksum specification could not be parsed.
    ChecksumInvalid,
}

impl ProtoError {
    /// The i18n key of this error.
    pub fn key(&self) -> &'static str {
        match self {
            Self::UrlTooLong { .. } => "error-proto-url-too-long",
            Self::UrlInvalid => "error-proto-url-invalid",
            Self::UrlScheme { .. } => "error-proto-url-scheme",
            Self::UrlNoHost => "error-proto-url-no-host",
            Self::RedirectInvalid => "error-proto-redirect-invalid",
            Self::HeaderValueInvalid { .. } => "error-proto-header-value",
            Self::ContentRangeMissing => "error-proto-content-range-missing",
            Self::ContentRangeInvalid => "error-proto-content-range-invalid",
            Self::ContentRangeMismatch { .. } => "error-proto-content-range",
            Self::AuthChallengeInvalid => "error-proto-auth-challenge",
            Self::FtpLineTooLong { .. } => "error-proto-ftp-line-too-long",
            Self::FtpTooManyLines { .. } => "error-proto-ftp-too-many-lines",
            Self::FtpReplyInvalid => "error-proto-ftp-reply",
            Self::UrlListTooLarge { .. } => "error-proto-url-list-too-large",
            Self::BatchTooLarge { .. } => "error-proto-batch-too-large",
            Self::BatchPatternInvalid => "error-proto-batch-pattern",
            Self::ChecksumInvalid => "error-proto-checksum-invalid",
        }
    }

    /// Converts into a localisable message (key + typed arguments).
    pub fn to_message(&self) -> Message {
        let m = Message::new(self.key());
        match self {
            Self::UrlTooLong { len, max } | Self::HeaderValueInvalid { len, max } => m
                .with_uint("len", usize_to_u64(*len))
                .with_uint("max", usize_to_u64(*max)),
            Self::UrlScheme { scheme } => m.with_str("scheme", scheme.clone()),
            Self::ContentRangeMismatch { expected, actual } => m
                .with_str("expected", expected.clone())
                .with_str("actual", actual.clone()),
            Self::FtpLineTooLong { max } | Self::FtpTooManyLines { max } => {
                m.with_uint("max", usize_to_u64(*max))
            }
            Self::UrlListTooLarge { bytes, max } => {
                m.with_bytes("bytes", *bytes).with_bytes("max", *max)
            }
            Self::BatchTooLarge { count, max } => {
                m.with_uint("count", *count).with_uint("max", *max)
            }
            Self::UrlInvalid
            | Self::UrlNoHost
            | Self::RedirectInvalid
            | Self::ContentRangeMissing
            | Self::ContentRangeInvalid
            | Self::AuthChallengeInvalid
            | Self::FtpReplyInvalid
            | Self::BatchPatternInvalid
            | Self::ChecksumInvalid => m,
        }
    }
}

impl udm_core::message::ToMessage for ProtoError {
    fn to_message(&self) -> Message {
        ProtoError::to_message(self)
    }
}

impl fmt::Display for ProtoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.to_message(), f)
    }
}

impl std::error::Error for ProtoError {}

/// Lossless on every supported target (usize ≤ 64 bits); saturates otherwise.
pub(crate) fn usize_to_u64(v: usize) -> u64 {
    u64::try_from(v).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_proto_prefixed_and_args_present() {
        let e = ProtoError::UrlTooLong {
            len: 9000,
            max: 8192,
        };
        let m = e.to_message();
        assert_eq!(m.key, "error-proto-url-too-long");
        assert_eq!(m.arg("len"), Some(&udm_core::MsgArg::UInt(9000)));
        let e = ProtoError::ContentRangeMismatch {
            expected: "0-9/10".into(),
            actual: "1-9/10".into(),
        };
        assert_eq!(
            e.to_string(),
            "error-proto-content-range{expected=0-9/10, actual=1-9/10}"
        );
    }
}
