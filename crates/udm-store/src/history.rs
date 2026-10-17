//! Append-only history log in JSON Lines.
//!
//! Each line is `{"v":1,"crc":"<first 16 hex of sha256 of rec json>","rec":{...}}`. Lines that are
//! longer than `limits::MAX_HISTORY_LINE_BYTES`, are not valid JSON, have another version or a
//! wrong crc are skipped and counted, never used.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use udm_core::limits;

use crate::atomic::atomic_write;
use crate::error::StoreError;
use crate::hash::sha256_prefix16;

/// Line format version written by this build.
const HISTORY_LINE_VERSION: u32 = 1;

#[derive(Deserialize)]
struct LineIn<'a> {
    v: u32,
    crc: String,
    #[serde(borrow)]
    rec: &'a RawValue,
}

/// An append-only, crash-tolerant history log of records `T`.
pub struct HistoryLog<T> {
    path: PathBuf,
    file: File,
    /// The file does not end with a newline (torn last line): the next append starts a new line.
    needs_newline: bool,
    _marker: PhantomData<fn(&T) -> T>,
}

impl<T> std::fmt::Debug for HistoryLog<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistoryLog")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl<T: Serialize + DeserializeOwned> HistoryLog<T> {
    /// Opens (creating if missing) the log at `path`.
    ///
    /// Returns the log, the valid records (the newest `limits::MAX_HISTORY_RECORDS`, oldest
    /// first) and the number of skipped lines.
    pub fn open(path: &Path) -> Result<(Self, Vec<T>, usize), StoreError> {
        let mut records: VecDeque<T> = VecDeque::new();
        let mut skipped: usize = 0;
        let mut needs_newline = false;
        match File::open(path) {
            Ok(f) => {
                let ends_with_newline = scan_lines(BufReader::new(f), |line| {
                    match line.and_then(parse_line::<T>) {
                        Some(LineOutcome::Record(rec)) => {
                            records.push_back(rec);
                            if records.len() > limits::MAX_HISTORY_RECORDS {
                                records.pop_front();
                            }
                        }
                        Some(LineOutcome::Blank) => {}
                        None => skipped = skipped.saturating_add(1),
                    }
                })
                .map_err(|e| StoreError::io(path, &e))?;
                needs_newline = !ends_with_newline;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(StoreError::io(path, &e)),
        }
        let file = open_append(path)?;
        let log = Self {
            path: path.to_path_buf(),
            file,
            needs_newline,
            _marker: PhantomData,
        };
        Ok((log, records.into_iter().collect(), skipped))
    }

    /// Path of the log file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one record and makes it durable (`flush` + `sync_data`).
    ///
    /// A record whose line would exceed `limits::MAX_HISTORY_LINE_BYTES` is refused with
    /// [`StoreError::TooLarge`] (it could never be read back).
    pub fn append(&mut self, rec: &T) -> Result<(), StoreError> {
        let mut line = encode_line(&self.path, rec)?;
        line.push('\n');
        if self.needs_newline {
            line.insert(0, '\n');
        }
        self.file
            .write_all(line.as_bytes())
            .and_then(|()| self.file.flush())
            .and_then(|()| self.file.sync_data())
            .map_err(|e| StoreError::io(&self.path, &e))?;
        self.needs_newline = false;
        Ok(())
    }

    /// Atomically replaces the whole log with `all`, keeping only the newest
    /// `limits::MAX_HISTORY_RECORDS` records.
    pub fn rewrite(&mut self, all: &[T]) -> Result<(), StoreError> {
        let skip = all.len().saturating_sub(limits::MAX_HISTORY_RECORDS);
        let mut out = String::new();
        for rec in all.iter().skip(skip) {
            out.push_str(&encode_line(&self.path, rec)?);
            out.push('\n');
        }
        atomic_write(&self.path, out.as_bytes())?;
        // The old handle points at the replaced file; reopen the new one.
        self.file = open_append(&self.path)?;
        self.needs_newline = false;
        Ok(())
    }
}

fn open_append(path: &Path) -> Result<File, StoreError> {
    OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .map_err(|e| StoreError::io(path, &e))
}

fn encode_line<T: Serialize>(path: &Path, rec: &T) -> Result<String, StoreError> {
    let rec_json = serde_json::to_string(rec).map_err(|e| StoreError::parse(path, &e))?;
    let crc = sha256_prefix16(rec_json.as_bytes());
    let line = format!(r#"{{"v":{HISTORY_LINE_VERSION},"crc":"{crc}","rec":{rec_json}}}"#);
    if line.len() > limits::MAX_HISTORY_LINE_BYTES {
        return Err(StoreError::TooLarge {
            path: path.to_path_buf(),
            size: u64::try_from(line.len()).unwrap_or(u64::MAX),
            max: u64::try_from(limits::MAX_HISTORY_LINE_BYTES).unwrap_or(u64::MAX),
        });
    }
    Ok(line)
}

enum LineOutcome<T> {
    Record(T),
    Blank,
}

/// Parses one line; `None` means "skip and count".
fn parse_line<T: DeserializeOwned>(line: &[u8]) -> Option<LineOutcome<T>> {
    if line.iter().all(u8::is_ascii_whitespace) {
        return Some(LineOutcome::Blank);
    }
    let parsed: LineIn<'_> = serde_json::from_slice(line).ok()?;
    if parsed.v != HISTORY_LINE_VERSION {
        return None;
    }
    if sha256_prefix16(parsed.rec.get().as_bytes()) != parsed.crc {
        return None;
    }
    serde_json::from_str(parsed.rec.get())
        .ok()
        .map(LineOutcome::Record)
}

/// Streams `reader` line by line without ever buffering more than
/// `limits::MAX_HISTORY_LINE_BYTES` per line. `on_line` receives `Some(bytes)` for a line within
/// the limit and `None` for an oversized one. Returns whether the data ended with a newline
/// (true for empty input).
fn scan_lines(
    mut reader: impl BufRead,
    mut on_line: impl FnMut(Option<&[u8]>),
) -> io::Result<bool> {
    let mut line: Vec<u8> = Vec::new();
    let mut oversized = false;
    let mut ends_with_newline = true;
    loop {
        let buf = match reader.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if buf.is_empty() {
            if !ends_with_newline {
                // Torn last line.
                on_line(if oversized { None } else { Some(&line) });
            }
            return Ok(ends_with_newline);
        }
        let newline = buf.iter().position(|&b| b == b'\n');
        let chunk_len = newline.unwrap_or(buf.len());
        let chunk = buf.get(..chunk_len).unwrap_or_default();
        if !oversized {
            if line.len().saturating_add(chunk.len()) > limits::MAX_HISTORY_LINE_BYTES {
                oversized = true;
                line.clear();
            } else {
                line.extend_from_slice(chunk);
            }
        }
        let consumed = match newline {
            Some(pos) => {
                on_line(if oversized { None } else { Some(&line) });
                line.clear();
                oversized = false;
                ends_with_newline = true;
                pos.saturating_add(1)
            }
            None => {
                ends_with_newline = false;
                chunk_len
            }
        };
        reader.consume(consumed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
    struct Rec {
        id: u64,
        name: String,
    }

    fn rec(id: u64) -> Rec {
        Rec {
            id,
            name: format!("file-{id}.bin"),
        }
    }

    #[test]
    fn append_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("history.jsonl");
        let (mut log, recs, skipped) = HistoryLog::<Rec>::open(&p).unwrap();
        assert!(recs.is_empty());
        assert_eq!(skipped, 0);
        log.append(&rec(1)).unwrap();
        log.append(&rec(2)).unwrap();
        drop(log);
        let (_, recs, skipped) = HistoryLog::<Rec>::open(&p).unwrap();
        assert_eq!(recs, vec![rec(1), rec(2)]);
        assert_eq!(skipped, 0);
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.starts_with(r#"{"v":1,"crc":""#));
    }

    #[test]
    fn rewrite_replaces_and_keeps_appending() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("history.jsonl");
        let (mut log, _, _) = HistoryLog::<Rec>::open(&p).unwrap();
        for i in 0..5 {
            log.append(&rec(i)).unwrap();
        }
        log.rewrite(&[rec(10), rec(11)]).unwrap();
        log.append(&rec(12)).unwrap();
        drop(log);
        let (_, recs, skipped) = HistoryLog::<Rec>::open(&p).unwrap();
        assert_eq!(recs, vec![rec(10), rec(11), rec(12)]);
        assert_eq!(skipped, 0);
    }

    #[test]
    fn rewrite_keeps_newest_max_history_records() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("history.jsonl");
        let (mut log, _, _) = HistoryLog::<u64>::open(&p).unwrap();
        let n = limits::MAX_HISTORY_RECORDS + 3;
        let all: Vec<u64> = (0..n as u64).collect();
        log.rewrite(&all).unwrap();
        drop(log);
        let (_, recs, _) = HistoryLog::<u64>::open(&p).unwrap();
        assert_eq!(recs.len(), limits::MAX_HISTORY_RECORDS);
        assert_eq!(recs.first(), Some(&3));
        assert_eq!(recs.last(), Some(&(n as u64 - 1)));
    }

    #[test]
    fn hostile_history_garbage_lines_skipped_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("history.jsonl");
        let (mut log, _, _) = HistoryLog::<Rec>::open(&p).unwrap();
        log.append(&rec(1)).unwrap();
        drop(log);
        let good = fs::read_to_string(&p).unwrap();
        let bad_crc = good.replace("file-1", "file-9");
        let mut content = good.clone();
        content.push_str("not json at all\n");
        content.push_str("\u{0}\u{1}\u{2}\n");
        content.push_str(&bad_crc); // valid JSON, crc mismatch
        content.push_str(&good.replace(r#""v":1"#, r#""v":2"#)); // unknown version
        content.push_str(r#"{"v":1,"crc":"0000000000000000","rec":"#); // truncated
        content.push('\n');
        content.push('\n'); // blank: not counted
        content.push_str(&good);
        fs::write(&p, content).unwrap();
        let (_, recs, skipped) = HistoryLog::<Rec>::open(&p).unwrap();
        assert_eq!(recs, vec![rec(1), rec(1)]);
        assert_eq!(skipped, 5);
    }

    #[test]
    fn hostile_history_oversized_line_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("history.jsonl");
        let (mut log, _, _) = HistoryLog::<Rec>::open(&p).unwrap();
        log.append(&rec(1)).unwrap();
        drop(log);
        let good = fs::read_to_string(&p).unwrap();
        let mut content = good.clone();
        content.push_str(&"x".repeat(limits::MAX_HISTORY_LINE_BYTES + 1));
        content.push('\n');
        // A 3-line-limit line with no newline at all at the end (torn and oversized).
        content.push_str(&good);
        content.push_str(&"y".repeat(limits::MAX_HISTORY_LINE_BYTES * 3));
        fs::write(&p, content).unwrap();
        let (mut log, recs, skipped) = HistoryLog::<Rec>::open(&p).unwrap();
        assert_eq!(recs, vec![rec(1), rec(1)]);
        assert_eq!(skipped, 2);
        // Appending after a torn tail starts a fresh line, so the new record survives.
        log.append(&rec(2)).unwrap();
        drop(log);
        let (_, recs, skipped) = HistoryLog::<Rec>::open(&p).unwrap();
        assert_eq!(recs, vec![rec(1), rec(1), rec(2)]);
        assert_eq!(skipped, 2);
    }

    #[test]
    fn hostile_history_torn_last_line() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("history.jsonl");
        let (mut log, _, _) = HistoryLog::<Rec>::open(&p).unwrap();
        log.append(&rec(1)).unwrap();
        log.append(&rec(2)).unwrap();
        drop(log);
        let full = fs::read(&p).unwrap();
        fs::write(&p, &full[..full.len() - 5]).unwrap();
        let (mut log, recs, skipped) = HistoryLog::<Rec>::open(&p).unwrap();
        assert_eq!(recs, vec![rec(1)]);
        assert_eq!(skipped, 1);
        log.append(&rec(3)).unwrap();
        drop(log);
        let (_, recs, skipped) = HistoryLog::<Rec>::open(&p).unwrap();
        assert_eq!(recs, vec![rec(1), rec(3)]);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn hostile_history_append_refuses_oversized_record() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("history.jsonl");
        let (mut log, _, _) = HistoryLog::<Rec>::open(&p).unwrap();
        let big = Rec {
            id: 1,
            name: "z".repeat(limits::MAX_HISTORY_LINE_BYTES),
        };
        assert!(matches!(log.append(&big), Err(StoreError::TooLarge { .. })));
        assert_eq!(fs::metadata(&p).unwrap().len(), 0);
    }

    #[test]
    fn scan_lines_small_buffer_boundaries() {
        let data = b"ab\ncd\n\nef";
        let reader = BufReader::with_capacity(1, &data[..]);
        let mut lines = Vec::new();
        let ended = scan_lines(reader, |l| lines.push(l.map(<[u8]>::to_vec))).unwrap();
        assert!(!ended);
        assert_eq!(
            lines,
            vec![
                Some(b"ab".to_vec()),
                Some(b"cd".to_vec()),
                Some(Vec::new()),
                Some(b"ef".to_vec())
            ]
        );
    }
}
