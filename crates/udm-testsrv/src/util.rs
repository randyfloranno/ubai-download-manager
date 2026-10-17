//! Small helpers: dates, percent-decoding, CRC-32, bounded head reading.

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};

/// Maximum request head size accepted by the HTTP server and the proxies.
pub const MAX_REQUEST_HEAD_BYTES: usize = 64 * 1024;

/// Object-safe async byte stream.
pub(crate) trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

/// IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`) for a Unix timestamp.
pub fn http_date(unix: u64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = unix / 86_400;
    let secs = unix % 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        DAYS[(days % 7) as usize],
        d,
        MONTHS[(m - 1) as usize],
        y,
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )
}

/// `YYYYMMDDHHMMSS` (FTP MDTM format) for a Unix timestamp.
pub fn mdtm_time(unix: u64) -> String {
    let (y, m, d) = civil_from_days((unix / 86_400) as i64);
    let secs = unix % 86_400;
    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        y,
        m,
        d,
        secs / 3600,
        (secs / 60) % 60,
        secs % 60
    )
}

// Howard Hinnant's days-from-civil inverse.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Percent-decodes `s` into raw bytes (`+` is kept literally).
pub fn pct_decode(s: &str) -> Vec<u8> {
    percent_encoding::percent_decode_str(s).collect()
}

/// Parses `a=1&b=2` into decoded pairs.
pub fn parse_query(q: &str) -> Vec<(String, Vec<u8>)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (
                String::from_utf8_lossy(&pct_decode(k)).into_owned(),
                pct_decode(v),
            )
        })
        .collect()
}

fn crc_table() -> &'static [u32; 256] {
    static T: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *e = c;
        }
        t
    })
}

/// Incremental CRC-32 (IEEE). Start with `0`.
pub fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    let t = crc_table();
    let mut c = !crc;
    for b in data {
        c = t[((c ^ u32::from(*b)) & 0xFF) as usize] ^ (c >> 8);
    }
    !c
}

/// Reads until `\r\n\r\n`. Returns `(head, leftover)`; `Ok(None)` on clean EOF
/// before any byte; `InvalidData` when the head exceeds `max`.
pub(crate) async fn read_head<S: AsyncRead + Unpin>(
    s: &mut S,
    max: usize,
) -> io::Result<Option<(Vec<u8>, Vec<u8>)>> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            let left = buf.split_off(pos + 4);
            return Ok(Some((buf, left)));
        }
        if buf.len() > max {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "head too large"));
        }
        let n = s.read(&mut chunk).await?;
        if n == 0 {
            if buf.is_empty() {
                return Ok(None);
            }
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "eof in head"));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

pub(crate) fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// A parsed HTTP request head.
#[derive(Clone, Debug, Default)]
pub(crate) struct Req {
    pub method: String,
    /// Origin-form target (path + optional `?query`) or absolute/authority form.
    pub target: String,
    pub path: String,
    pub query: String,
    /// Lower-cased names.
    pub headers: Vec<(String, String)>,
}

impl Req {
    pub fn new(method: &str, target: &str, headers: Vec<(String, String)>) -> Self {
        let (path, query) = match target.split_once('?') {
            Some((p, q)) => (p.to_string(), q.to_string()),
            None => (target.to_string(), String::new()),
        };
        Req {
            method: method.to_string(),
            target: target.to_string(),
            path,
            query,
            headers,
        }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Parses an HTTP/1.x request head.
pub(crate) fn parse_request(head: &[u8]) -> Option<Req> {
    let text = String::from_utf8_lossy(head);
    let mut lines = text.split("\r\n");
    let first = lines.next()?;
    let mut parts = first.split(' ');
    let method = parts.next()?;
    let target = parts.next()?;
    let version = parts.next()?;
    if !version.starts_with("HTTP/1.") || method.is_empty() || target.is_empty() {
        return None;
    }
    let headers = lines
        .filter(|l| !l.is_empty())
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            Some((k.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect();
    Some(Req::new(method, target, headers))
}

/// Resolves a proxy target, allowing loopback only (the test servers never
/// reach the internet).
pub(crate) fn resolve_loopback(host: &str, port: u16) -> Option<SocketAddr> {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    if h.eq_ignore_ascii_case("localhost") {
        return Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));
    }
    let ip: IpAddr = h.parse().ok()?;
    ip.is_loopback().then_some(SocketAddr::new(ip, port))
}

/// Splits `host:port` (IPv6 in brackets allowed).
pub(crate) fn split_host_port(authority: &str) -> Option<(String, u16)> {
    let (h, p) = authority.rsplit_once(':')?;
    Some((h.to_string(), p.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_formats() {
        assert_eq!(http_date(784_111_777), "Sun, 06 Nov 1994 08:49:37 GMT");
        assert_eq!(http_date(1_445_412_480), "Wed, 21 Oct 2015 07:28:00 GMT");
        assert_eq!(mdtm_time(1_445_412_480), "20151021072800");
    }

    #[test]
    fn crc_known() {
        assert_eq!(crc32_update(0, b"123456789"), 0xCBF4_3926);
        let a = crc32_update(0, b"1234");
        assert_eq!(crc32_update(a, b"56789"), 0xCBF4_3926);
    }
}
