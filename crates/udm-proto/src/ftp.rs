//! FTP control-channel reply parsing (RFC 959, RFC 2428 EPSV, RFC 3659 MDTM).

use udm_core::limits;

use crate::civil::unix_from_civil;
use crate::error::ProtoError;
use crate::text::parse_u64_strict;

/// A complete FTP reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FtpReply {
    /// Three-digit reply code (100..=599).
    pub code: u16,
    /// Text lines: the first and last without the `NNN-`/`NNN ` prefix, middle lines verbatim.
    pub lines: Vec<String>,
}

impl FtpReply {
    /// All lines joined with `\n` (for PASV/EPSV/MDTM parsing).
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }
}

/// Incremental reply parser, fed one line at a time.
///
/// Lines longer than `limits::MAX_FTP_LINE_BYTES` and replies with more than
/// `limits::MAX_FTP_REPLY_LINES` lines are errors (the connection should be dropped).
#[derive(Clone, Debug, Default)]
pub struct FtpReplyParser {
    pending: Option<(u16, Vec<String>)>,
}

impl FtpReplyParser {
    /// New parser.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one line (with or without the trailing CRLF). Returns the reply once complete.
    pub fn push_line(&mut self, line: &str) -> Result<Option<FtpReply>, ProtoError> {
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        let max = limits::MAX_FTP_LINE_BYTES;
        if line.len() > max {
            self.pending = None;
            return Err(ProtoError::FtpLineTooLong { max });
        }
        match self.pending.take() {
            None => {
                let (code, sep, text) = split_code(line).ok_or(ProtoError::FtpReplyInvalid)?;
                match sep {
                    Some('-') => {
                        self.pending = Some((code, vec![text.to_owned()]));
                        Ok(None)
                    }
                    Some(' ') | None => Ok(Some(FtpReply {
                        code,
                        lines: vec![text.to_owned()],
                    })),
                    Some(_) => Err(ProtoError::FtpReplyInvalid),
                }
            }
            Some((code, mut lines)) => {
                let max_lines = limits::MAX_FTP_REPLY_LINES;
                if lines.len() >= max_lines {
                    return Err(ProtoError::FtpTooManyLines { max: max_lines });
                }
                match split_code(line) {
                    Some((c, Some(' ') | None, text)) if c == code => {
                        lines.push(text.to_owned());
                        Ok(Some(FtpReply { code, lines }))
                    }
                    _ => {
                        lines.push(line.to_owned());
                        self.pending = Some((code, lines));
                        Ok(None)
                    }
                }
            }
        }
    }
}

/// Splits `NNN<sep>text`. `None` when the line does not start with a valid code.
fn split_code(line: &str) -> Option<(u16, Option<char>, &str)> {
    let digits = line.get(..3)?;
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let code = u16::try_from(parse_u64_strict(digits)?).ok()?;
    if !(100..=599).contains(&code) {
        return None;
    }
    let rest = line.get(3..)?;
    let mut chars = rest.chars();
    let sep = chars.next();
    Some((code, sep, chars.as_str()))
}

/// Parses a `227` reply: `Entering Passive Mode (h1,h2,h3,h4,p1,p2)` (parentheses optional).
///
/// Callers should connect to the control connection's host, not the returned address,
/// unless they have a reason to trust it (prevents FTP bounce to internal hosts).
pub fn parse_pasv(reply_text: &str) -> Option<([u8; 4], u16)> {
    if reply_text.len() > limits::MAX_FTP_LINE_BYTES {
        return None;
    }
    let inner = match reply_text.split_once('(') {
        Some((_, rest)) => rest.split(')').next()?,
        None => reply_text
            .split(|c: char| c.is_whitespace())
            .find(|t| t.matches(',').count() == 5)?
            .trim_end_matches(['.', ')']),
    };
    let mut nums = inner.split(',').map(|n| n.trim().parse::<u8>().ok());
    let mut next = || nums.next().flatten();
    let ip = [next()?, next()?, next()?, next()?];
    let port = u16::from_be_bytes([next()?, next()?]);
    if nums.next().is_some() || port == 0 {
        return None;
    }
    Some((ip, port))
}

/// Parses a `229` reply: `Entering Extended Passive Mode (|||port|)`.
pub fn parse_epsv(reply_text: &str) -> Option<u16> {
    if reply_text.len() > limits::MAX_FTP_LINE_BYTES {
        return None;
    }
    let (_, rest) = reply_text.split_once('(')?;
    let inner = rest.split(')').next()?;
    let mut chars = inner.chars();
    let d = chars.next()?;
    if !(' '..='~').contains(&d) || d.is_ascii_digit() {
        return None;
    }
    let mut parts = inner.split(d);
    // "", "", "", port, ""
    let (a, b, c, port, e) = (
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
        parts.next()?,
    );
    if !(a.is_empty() && b.is_empty() && c.is_empty() && e.is_empty()) || parts.next().is_some() {
        return None;
    }
    let port = u16::try_from(parse_u64_strict(port)?).ok()?;
    if port == 0 { None } else { Some(port) }
}

/// Parses an `MDTM` reply `YYYYMMDDHHMMSS[.sss]` (UTC) into Unix seconds.
/// The reply code prefix (`213 `) may be present.
pub fn parse_mdtm(reply_text: &str) -> Option<i64> {
    if reply_text.len() > limits::MAX_FTP_LINE_BYTES {
        return None;
    }
    let token = reply_text.split_whitespace().find(|t| {
        t.len() >= 14
            && t.get(..14)
                .is_some_and(|d| d.bytes().all(|b| b.is_ascii_digit()))
    })?;
    let (stamp, frac) = match token.split_once('.') {
        Some((s, f)) => (s, Some(f)),
        None => (token, None),
    };
    if stamp.len() != 14
        || frac.is_some_and(|f| f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let field = |a: usize, b: usize| stamp.get(a..b).and_then(parse_u64_strict);
    let year = i64::try_from(field(0, 4)?).ok()?;
    let to_u32 = |v: u64| u32::try_from(v).ok();
    unix_from_civil(
        year,
        to_u32(field(4, 6)?)?,
        to_u32(field(6, 8)?)?,
        to_u32(field(8, 10)?)?,
        to_u32(field(10, 12)?)?,
        to_u32(field(12, 14)?)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_and_multi_line() {
        let mut p = FtpReplyParser::new();
        assert_eq!(
            p.push_line("220 Welcome\r\n").unwrap(),
            Some(FtpReply {
                code: 220,
                lines: vec!["Welcome".into()]
            })
        );
        assert_eq!(p.push_line("211-Features:\r\n").unwrap(), None);
        assert_eq!(p.push_line(" MDTM\r\n").unwrap(), None);
        assert_eq!(p.push_line("211-not the end\r\n").unwrap(), None);
        assert_eq!(p.push_line("200 other code is text\r\n").unwrap(), None);
        let r = p.push_line("211 End").unwrap().unwrap();
        assert_eq!(r.code, 211);
        assert_eq!(
            r.lines,
            vec![
                "Features:",
                " MDTM",
                "211-not the end",
                "200 other code is text",
                "End"
            ]
        );
        assert_eq!(p.push_line("200").unwrap().unwrap().code, 200);
    }

    #[test]
    fn hostile_ftp_line_too_long() {
        let mut p = FtpReplyParser::new();
        let line = format!("220 {}", "a".repeat(limits::MAX_FTP_LINE_BYTES));
        assert_eq!(
            p.push_line(&line),
            Err(ProtoError::FtpLineTooLong {
                max: limits::MAX_FTP_LINE_BYTES
            })
        );
        let ok = format!("220 {}", "a".repeat(limits::MAX_FTP_LINE_BYTES - 4));
        assert!(p.push_line(&ok).unwrap().is_some());
    }

    #[test]
    fn hostile_ftp_too_many_lines() {
        let mut p = FtpReplyParser::new();
        assert_eq!(p.push_line("211-start").unwrap(), None);
        let mut result = Ok(None);
        for _ in 0..limits::MAX_FTP_REPLY_LINES + 10 {
            result = p.push_line(" filler");
            if result.is_err() {
                break;
            }
        }
        assert_eq!(
            result,
            Err(ProtoError::FtpTooManyLines {
                max: limits::MAX_FTP_REPLY_LINES
            })
        );
        // Parser is reset afterwards.
        assert!(p.push_line("220 ok").unwrap().is_some());
    }

    #[test]
    fn hostile_ftp_reply_garbage() {
        for l in [
            "",
            "22",
            "abc def",
            "999 too big",
            "099 too small",
            "2200 x",
            "22a x",
            "\u{e9}20 x",
        ] {
            assert!(FtpReplyParser::new().push_line(l).is_err(), "{l:?}");
        }
    }

    #[test]
    fn pasv_epsv() {
        assert_eq!(
            parse_pasv("Entering Passive Mode (192,168,1,2,19,137)."),
            Some(([192, 168, 1, 2], 5001))
        );
        assert_eq!(
            parse_pasv("Entering Passive Mode 10,0,0,1,0,21"),
            Some(([10, 0, 0, 1], 21))
        );
        assert_eq!(
            parse_epsv("Entering Extended Passive Mode (|||6446|)"),
            Some(6446)
        );
        assert_eq!(
            parse_epsv("Entering Extended Passive Mode (!!!6446!)"),
            Some(6446)
        );
    }

    #[test]
    fn hostile_pasv_epsv_garbage() {
        for t in [
            "",
            "(1,2,3,4,5)",
            "(1,2,3,4,5,6,7)",
            "(256,1,1,1,1,1)",
            "(1,2,3,4,0,0)",
            "(a,b,c,d,e,f)",
            "(-1,2,3,4,5,6)",
        ] {
            assert_eq!(parse_pasv(t), None, "{t}");
        }
        for t in [
            "",
            "(||6446|)",
            "(|||70000|)",
            "(|||0|)",
            "(|||abc|)",
            "(1116461)",
            "(|||6446|x|)",
            "(|1|1|6446|)",
        ] {
            assert_eq!(parse_epsv(t), None, "{t}");
        }
        assert_eq!(
            parse_pasv(&format!("({})", "1,".repeat(limits::MAX_FTP_LINE_BYTES))),
            None
        );
    }

    #[test]
    fn mdtm() {
        assert_eq!(parse_mdtm("213 19941106084937"), Some(784_111_777));
        assert_eq!(parse_mdtm("19941106084937.123"), Some(784_111_777));
        for t in [
            "",
            "213 1994110608493",
            "19941306084937",
            "19941106084937.",
            "19941106084937.x",
            "19940230000000",
            "213 199411060849377",
        ] {
            assert_eq!(parse_mdtm(t), None, "{t}");
        }
    }
}
