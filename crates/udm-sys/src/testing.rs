//! Test doubles for [`FileWriter`](crate::FileWriter) (feature `test-util`).

use std::io;
use std::sync::Mutex;

use crate::writer::FileWriter;

fn poisoned() -> io::Error {
    io::Error::other("test writer lock poisoned")
}

/// The error a [`FaultyWriter`] injects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// `io::ErrorKind::StorageFull`.
    DiskFull,
    /// The platform's raw "disk full" OS error (`ENOSPC` 28 / `ERROR_DISK_FULL` 112).
    DiskFullRaw,
    /// A generic I/O error (e.g. a removed drive).
    Other,
}

impl Fault {
    fn to_error(self) -> io::Error {
        match self {
            Fault::DiskFull => io::Error::from(io::ErrorKind::StorageFull),
            #[cfg(windows)]
            Fault::DiskFullRaw => io::Error::from_raw_os_error(112),
            #[cfg(not(windows))]
            Fault::DiskFullRaw => io::Error::from_raw_os_error(28),
            Fault::Other => io::Error::other("injected write failure"),
        }
    }
}

/// Wraps a writer and fails once more than `fail_after` bytes have been written in total.
///
/// The write that crosses the budget stores the bytes that still fit, then fails, like a real
/// disk filling up mid-write. Every later write fails too.
#[derive(Debug)]
pub struct FaultyWriter<W> {
    inner: W,
    fail_after: u64,
    fault: Fault,
    written: Mutex<u64>,
}

impl<W: FileWriter> FaultyWriter<W> {
    /// Wraps `inner`; fails with `fault` after `fail_after` bytes.
    pub fn new(inner: W, fail_after: u64, fault: Fault) -> Self {
        Self {
            inner,
            fail_after,
            fault,
            written: Mutex::new(0),
        }
    }

    /// The wrapped writer.
    pub fn inner(&self) -> &W {
        &self.inner
    }

    /// Total bytes accepted so far.
    pub fn written(&self) -> u64 {
        self.written.lock().map(|g| *g).unwrap_or(0)
    }
}

impl<W: FileWriter> FileWriter for FaultyWriter<W> {
    fn write_at(&self, offset: u64, buf: &[u8]) -> io::Result<()> {
        let mut written = self.written.lock().map_err(|_| poisoned())?;
        let budget = self.fail_after.saturating_sub(*written);
        let len = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        if len <= budget {
            self.inner.write_at(offset, buf)?;
            *written = written.saturating_add(len);
            return Ok(());
        }
        let fit = usize::try_from(budget).unwrap_or(usize::MAX);
        if let Some(part) = buf.get(..fit)
            && !part.is_empty()
        {
            self.inner.write_at(offset, part)?;
            *written = written.saturating_add(budget);
        }
        Err(self.fault.to_error())
    }

    fn sync_data(&self) -> io::Result<()> {
        self.inner.sync_data()
    }

    fn len(&self) -> io::Result<u64> {
        self.inner.len()
    }
}

/// An in-memory [`FileWriter`]. Writes past the end grow the buffer with zeros.
#[derive(Debug, Default)]
pub struct MemWriter {
    data: Mutex<Vec<u8>>,
    syncs: Mutex<u64>,
}

impl MemWriter {
    /// Empty writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Writer pre-sized to `len` zero bytes (like a pre-allocated part file).
    pub fn with_len(len: usize) -> Self {
        Self {
            data: Mutex::new(vec![0; len]),
            syncs: Mutex::new(0),
        }
    }

    /// Copy of the current contents.
    pub fn contents(&self) -> Vec<u8> {
        self.data.lock().map(|d| d.clone()).unwrap_or_default()
    }

    /// Number of `sync_data` calls.
    pub fn sync_count(&self) -> u64 {
        self.syncs.lock().map(|g| *g).unwrap_or(0)
    }
}

impl FileWriter for MemWriter {
    fn write_at(&self, offset: u64, buf: &[u8]) -> io::Result<()> {
        let bad = || io::Error::new(io::ErrorKind::InvalidInput, "offset out of range");
        let start = usize::try_from(offset).map_err(|_| bad())?;
        let end = start.checked_add(buf.len()).ok_or_else(bad)?;
        let mut data = self.data.lock().map_err(|_| poisoned())?;
        if data.len() < end {
            data.resize(end, 0);
        }
        data.get_mut(start..end)
            .ok_or_else(bad)?
            .copy_from_slice(buf);
        Ok(())
    }

    fn sync_data(&self) -> io::Result<()> {
        let mut s = self.syncs.lock().map_err(|_| poisoned())?;
        *s = s.saturating_add(1);
        Ok(())
    }

    fn len(&self) -> io::Result<u64> {
        let data = self.data.lock().map_err(|_| poisoned())?;
        Ok(u64::try_from(data.len()).unwrap_or(u64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SysError, WriteFailure, classify_write_error};
    use udm_core::limits;

    #[test]
    fn mem_writer_basics() {
        let w = MemWriter::new();
        w.write_at(3, b"de").unwrap();
        w.write_at(0, b"abc").unwrap();
        w.sync_data().unwrap();
        assert_eq!(w.contents(), b"abcde");
        assert_eq!(w.len().unwrap(), 5);
        assert_eq!(w.sync_count(), 1);
        assert!(w.write_at(u64::MAX, b"x").is_err());
    }

    #[test]
    fn hostile_faulty_writer_disk_full_classified() {
        for fault in [Fault::DiskFull, Fault::DiskFullRaw] {
            let w = FaultyWriter::new(MemWriter::new(), 10, fault);
            w.write_at(0, b"12345").unwrap();
            let e = w.write_at(5, b"6789012345").unwrap_err();
            assert_eq!(
                classify_write_error(&e),
                WriteFailure::DiskFull,
                "{fault:?}"
            );
            // The bytes that fitted were written; nothing beyond.
            assert_eq!(w.inner().contents(), b"1234567890");
            assert_eq!(w.written(), 10);
            // Later writes keep failing; data already written is untouched.
            assert!(w.write_at(10, b"x").is_err());
            assert_eq!(w.inner().contents(), b"1234567890");
            let msg =
                SysError::from_write_error(std::path::Path::new("f.udmpart"), &e).to_message();
            assert_eq!(msg.key, "error-disk-full");
        }
    }

    #[test]
    fn hostile_faulty_writer_other_error_classified() {
        let w = FaultyWriter::new(MemWriter::new(), 0, Fault::Other);
        let e = w.write_at(0, b"x").unwrap_err();
        assert_eq!(classify_write_error(&e), WriteFailure::Other);
        let msg = SysError::from_write_error(std::path::Path::new("f.udmpart"), &e).to_message();
        assert_eq!(msg.key, "error-disk-write");
        assert!(w.inner().contents().is_empty());
    }

    #[test]
    fn hostile_faulty_writer_fills_at_checkpoint_boundary() {
        // Disk fills exactly after one checkpoint's worth of data.
        let budget = limits::CHECKPOINT_BYTES;
        let w = FaultyWriter::new(MemWriter::new(), budget, Fault::DiskFull);
        let chunk = vec![7u8; 65_536];
        let mut off = 0u64;
        let err = loop {
            match w.write_at(off, &chunk) {
                Ok(()) => off += chunk.len() as u64,
                Err(e) => break e,
            }
        };
        assert_eq!(classify_write_error(&err), WriteFailure::DiskFull);
        assert_eq!(w.written(), budget);
        assert_eq!(w.inner().len().unwrap(), budget);
    }
}
