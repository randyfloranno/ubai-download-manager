//! Native messaging framing: a 4-byte little-endian length, then UTF-8 JSON.
//! Lengths above `limits::MAX_NATIVE_MESSAGE_BYTES` are refused without
//! reading the body (the stream cannot be trusted after that).

use std::io::{ErrorKind, Read, Write};

use udm_core::limits;

/// Framing errors.
#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    /// Declared length exceeds the limit.
    TooLarge(u64),
    /// Stream ended inside a frame.
    Truncated,
    /// Other I/O error.
    Io(ErrorKind),
}

impl FrameError {
    /// Message key for this error.
    pub fn key(&self) -> &'static str {
        match self {
            Self::TooLarge(_) => "error-native-too-large",
            Self::Truncated => "error-native-truncated",
            Self::Io(_) => "error-native-io",
        }
    }
}

fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<usize, FrameError> {
    let mut filled = 0usize;
    while let Some(rest) = buf.get_mut(filled..) {
        if rest.is_empty() {
            break;
        }
        match r.read(rest) {
            Ok(0) => break,
            Ok(n) => filled = filled.saturating_add(n),
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(e) => return Err(FrameError::Io(e.kind())),
        }
    }
    Ok(filled)
}

/// Reads one frame. `Ok(None)` on a clean end of stream (browser closed us).
pub fn read_frame<R: Read>(r: &mut R) -> Result<Option<Vec<u8>>, FrameError> {
    let mut len = [0u8; 4];
    match read_full(r, &mut len)? {
        0 => return Ok(None),
        4 => {}
        _ => return Err(FrameError::Truncated),
    }
    let n = u32::from_le_bytes(len);
    let n_usize = usize::try_from(n).map_err(|_| FrameError::TooLarge(u64::from(n)))?;
    if n_usize > limits::MAX_NATIVE_MESSAGE_BYTES {
        return Err(FrameError::TooLarge(u64::from(n)));
    }
    let mut body = vec![0u8; n_usize];
    if read_full(r, &mut body)? != n_usize {
        return Err(FrameError::Truncated);
    }
    Ok(Some(body))
}

/// Writes one frame and flushes.
pub fn write_frame<W: Write>(w: &mut W, body: &[u8]) -> Result<(), FrameError> {
    if body.len() > limits::MAX_NATIVE_MESSAGE_BYTES {
        return Err(FrameError::TooLarge(
            u64::try_from(body.len()).unwrap_or(u64::MAX),
        ));
    }
    let n = u32::try_from(body.len()).map_err(|_| FrameError::TooLarge(u64::MAX))?;
    let io = |e: std::io::Error| FrameError::Io(e.kind());
    w.write_all(&n.to_le_bytes()).map_err(io)?;
    w.write_all(body).map_err(io)?;
    w.flush().map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(body: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        write_frame(&mut v, body).unwrap();
        v
    }

    #[test]
    fn round_trip_and_clean_eof() {
        let mut data = framed(br#"{"type":"ping"}"#);
        data.extend(framed(b"{}"));
        let mut r = data.as_slice();
        assert_eq!(read_frame(&mut r).unwrap().unwrap(), br#"{"type":"ping"}"#);
        assert_eq!(read_frame(&mut r).unwrap().unwrap(), b"{}");
        assert_eq!(read_frame(&mut r).unwrap(), None);
    }

    #[test]
    fn hostile_native_message_too_large() {
        // MAX_NATIVE_MESSAGE_BYTES: refused from the header alone.
        let n = u32::try_from(limits::MAX_NATIVE_MESSAGE_BYTES + 1).unwrap();
        let mut r = n.to_le_bytes().to_vec();
        r.extend(b"x");
        let err = read_frame(&mut r.as_slice()).unwrap_err();
        assert_eq!(err, FrameError::TooLarge(u64::from(n)));
        let mut r = u32::MAX.to_le_bytes().to_vec();
        assert!(matches!(
            read_frame(&mut r.as_slice()),
            Err(FrameError::TooLarge(_))
        ));
        r.clear();
        // Exactly at the limit is fine.
        let body = vec![b' '; limits::MAX_NATIVE_MESSAGE_BYTES];
        assert_eq!(
            read_frame(&mut framed(&body).as_slice())
                .unwrap()
                .unwrap()
                .len(),
            body.len()
        );
        // Writing too much is refused too.
        let mut out = Vec::new();
        assert!(write_frame(&mut out, &vec![0u8; limits::MAX_NATIVE_MESSAGE_BYTES + 1]).is_err());
        assert!(out.is_empty());
    }

    #[test]
    fn hostile_native_message_truncated() {
        for bad in [&[1u8, 0][..], &[5, 0, 0, 0, b'a', b'b'][..]] {
            assert_eq!(
                read_frame(&mut &bad[..]).unwrap_err(),
                FrameError::Truncated
            );
        }
    }
}
