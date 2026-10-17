//! Segment board: which byte ranges are done, in flight, or waiting.
//!
//! Workers *claim* a range, *reserve* bytes before writing them, *commit*
//! them after the write and *release* the slot on error. Dynamic
//! segmentation splits the slot with the most remaining bytes when a worker
//! is idle. Reserved-but-uncommitted bytes count as taken for splitting and
//! as not done for checkpoints, so a checkpoint never claims unwritten bytes.

use std::collections::VecDeque;

use udm_core::{ByteRange, CoverageMap, limits};
use udm_proto::{ActiveSegment, plan_segments, split_for_idle};

/// An active slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    /// Slot id (unique per board).
    pub id: u64,
    /// Current range; `end` may shrink when the slot is split.
    pub range: ByteRange,
    /// Bytes `[range.start, pos)` are written.
    pub pos: u64,
    /// Bytes `[pos, reserved)` are being written.
    pub reserved: u64,
}

/// Result of a claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claim {
    /// Slot id.
    pub slot: u64,
    /// Range to fetch.
    pub range: ByteRange,
}

/// The board.
#[derive(Clone, Debug)]
pub struct Board {
    size: Option<u64>,
    done: CoverageMap,
    slots: Vec<Slot>,
    pending: VecDeque<ByteRange>,
    next_slot: u64,
    min_segment: u64,
    single: bool,
}

/// Upper bound used for the slot of a download of unknown size.
const UNKNOWN_END: u64 = limits::MAX_FILE_SIZE_BYTES;

impl Board {
    /// Creates a board from saved coverage.
    ///
    /// `size == None` or `!resumable` → one connection from the first gap.
    pub fn new(size: Option<u64>, connections: u32, resumable: bool, done: CoverageMap) -> Self {
        let min_segment = limits::MIN_SEGMENT_BYTES;
        let mut board = Self {
            size,
            done,
            slots: Vec::new(),
            pending: VecDeque::new(),
            next_slot: 1,
            min_segment,
            single: false,
        };
        match size {
            Some(s) if resumable => {
                let conns = connections.clamp(1, limits::MAX_CONNECTIONS_PER_DOWNLOAD);
                board.pending = plan_segments(s, conns, min_segment, &board.done).into();
            }
            Some(s) => {
                // No ranges: everything must come from offset 0 in one stream.
                board.done.clear();
                board.single = true;
                if let Some(r) = ByteRange::new(0, s)
                    && !r.is_empty()
                {
                    board.pending.push_back(r);
                }
            }
            None => {
                board.done.clear();
                board.single = true;
                if let Some(r) = ByteRange::new(0, UNKNOWN_END) {
                    board.pending.push_back(r);
                }
            }
        }
        board
    }

    /// Expected size.
    pub fn size(&self) -> Option<u64> {
        self.size
    }

    /// True when only one connection may be used.
    pub fn is_single(&self) -> bool {
        self.single
    }

    /// Takes the next range: a pending one, or half of the busiest slot.
    pub fn claim(&mut self) -> Option<Claim> {
        if let Some(range) = self.pending.pop_front() {
            return Some(self.open_slot(range));
        }
        if self.single {
            return None;
        }
        let active: Vec<ActiveSegment> = self
            .slots
            .iter()
            .map(|s| ActiveSegment {
                range: s.range,
                pos: s.reserved,
            })
            .collect();
        let (index, split_at) = split_for_idle(&active, self.min_segment)?;
        let slot = self.slots.get_mut(index)?;
        let tail = ByteRange::new(split_at, slot.range.end())?;
        let head = ByteRange::new(slot.range.start(), split_at)?;
        if split_at < slot.reserved {
            return None;
        }
        slot.range = head;
        Some(self.open_slot(tail))
    }

    fn open_slot(&mut self, range: ByteRange) -> Claim {
        let id = self.next_slot;
        self.next_slot = self.next_slot.saturating_add(1);
        self.slots.push(Slot {
            id,
            range,
            pos: range.start(),
            reserved: range.start(),
        });
        Claim { slot: id, range }
    }

    fn slot_mut(&mut self, id: u64) -> Option<&mut Slot> {
        self.slots.iter_mut().find(|s| s.id == id)
    }

    /// Current position of a slot (where the next byte goes), or `None` if
    /// the slot no longer exists.
    pub fn position(&self, id: u64) -> Option<u64> {
        self.slots.iter().find(|s| s.id == id).map(|s| s.pos)
    }

    /// Reserves up to `n` bytes at the slot's position. Returns the offset
    /// and the number of bytes the worker may write (0 = range finished,
    /// `None` = slot gone, stop).
    pub fn reserve(&mut self, id: u64, n: u64) -> Option<(u64, u64)> {
        let slot = self.slot_mut(id)?;
        let room = slot.range.end().saturating_sub(slot.reserved);
        let allowed = n.min(room);
        let offset = slot.reserved;
        slot.reserved = slot.reserved.saturating_add(allowed);
        Some((offset, allowed))
    }

    /// Commits the bytes reserved earlier. Returns `true` when the slot has
    /// reached its (possibly shrunk) end and has been closed.
    pub fn commit(&mut self, id: u64, n: u64) -> bool {
        let Some(slot) = self.slot_mut(id) else {
            return true;
        };
        slot.pos = slot.pos.saturating_add(n).min(slot.reserved);
        if slot.pos >= slot.range.end() {
            self.close(id);
            return true;
        }
        false
    }

    /// Un-reserves bytes that were reserved but not written (write error).
    pub fn unreserve(&mut self, id: u64) {
        if let Some(slot) = self.slot_mut(id) {
            slot.reserved = slot.pos;
        }
    }

    fn close(&mut self, id: u64) {
        if let Some(i) = self.slots.iter().position(|s| s.id == id) {
            let slot = self.slots.swap_remove(i);
            if let Some(r) = ByteRange::new(slot.range.start(), slot.pos) {
                // Overlap is impossible by construction; ignore defensively.
                self.done.insert(r);
            }
        }
    }

    /// Gives a slot back after an error: written bytes become done, the rest
    /// pending again (front of the queue, so it is retried first).
    pub fn release(&mut self, id: u64) {
        if let Some(i) = self.slots.iter().position(|s| s.id == id) {
            let slot = self.slots.swap_remove(i);
            if let Some(r) = ByteRange::new(slot.range.start(), slot.pos) {
                self.done.insert(r);
            }
            if let Some(rest) = ByteRange::new(slot.pos, slot.range.end())
                && !rest.is_empty()
            {
                self.pending.push_front(rest);
            }
        }
    }

    /// The server ignored `Range` (answered 200): drop all progress and let
    /// slot `id` take the whole file from offset 0 on one connection.
    /// Returns the new range of that slot.
    pub fn go_single(&mut self, id: u64) -> Option<ByteRange> {
        let end = self.size.unwrap_or(UNKNOWN_END);
        let range = ByteRange::new(0, end)?;
        // Unknown slot: change nothing.
        self.slots.iter().find(|s| s.id == id)?;
        self.done.clear();
        self.pending.clear();
        self.slots.retain(|s| s.id == id);
        self.single = true;
        let slot = self.slot_mut(id)?;
        slot.range = range;
        slot.pos = 0;
        slot.reserved = 0;
        Some(range)
    }

    /// Unknown size: the stream ended at the slot's position, which is now
    /// the size.
    pub fn finish_unknown(&mut self, id: u64) -> Option<u64> {
        let pos = self.position(id)?;
        self.size = Some(pos);
        if let Some(slot) = self.slot_mut(id) {
            slot.range = ByteRange::new(slot.range.start(), pos)?;
        }
        self.close(id);
        Some(pos)
    }

    /// Bytes written (done + committed in-flight).
    pub fn downloaded(&self) -> u64 {
        let in_flight: u64 = self
            .slots
            .iter()
            .map(|s| s.pos.saturating_sub(s.range.start()))
            .fold(0u64, u64::saturating_add);
        self.done.covered_len().saturating_add(in_flight)
    }

    /// Coverage of written bytes, for checkpoints (call *before* `sync_data`).
    pub fn snapshot(&self) -> CoverageMap {
        let mut map = self.done.clone();
        for s in &self.slots {
            if let Some(r) = ByteRange::new(s.range.start(), s.pos) {
                map.insert(r);
            }
        }
        map
    }

    /// True when the whole expected size is covered.
    pub fn is_complete(&self) -> bool {
        match self.size {
            Some(size) => self.slots.is_empty() && self.done.is_complete(size),
            None => false,
        }
    }

    /// True when no work is left to claim or in flight.
    pub fn is_idle(&self) -> bool {
        self.slots.is_empty() && self.pending.is_empty()
    }

    /// Active slots, for the UI.
    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    /// Done coverage (finished slots only).
    pub fn done(&self) -> &CoverageMap {
        &self.done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const MIB: u64 = 1_048_576;

    fn drain(board: &mut Board, chunk: u64) {
        // Simulate workers round-robin until all work is done.
        let mut claims: Vec<u64> = Vec::new();
        for _ in 0..4 {
            if let Some(c) = board.claim() {
                claims.push(c.slot);
            }
        }
        let mut guard = 0u32;
        while !claims.is_empty() {
            guard += 1;
            assert!(guard < 1_000_000);
            let mut next = Vec::new();
            for id in claims {
                match board.reserve(id, chunk) {
                    Some((_, n)) if n > 0 => {
                        if board.commit(id, n) {
                            if let Some(c) = board.claim() {
                                next.push(c.slot);
                            }
                        } else {
                            next.push(id);
                        }
                    }
                    Some(_) => {
                        board.commit(id, 0);
                        if let Some(c) = board.claim() {
                            next.push(c.slot);
                        }
                    }
                    None => {}
                }
            }
            claims = next;
        }
    }

    #[test]
    fn board_completes_with_dynamic_splitting() {
        let mut b = Board::new(Some(10 * MIB), 4, true, CoverageMap::default());
        drain(&mut b, 64 * 1024);
        assert!(b.is_complete());
        assert_eq!(b.downloaded(), 10 * MIB);
    }

    #[test]
    fn split_keeps_reserved_bytes() {
        let mut b = Board::new(Some(8 * MIB), 1, true, CoverageMap::default());
        let c = b.claim().unwrap();
        let (off, n) = b.reserve(c.slot, MIB).unwrap();
        assert_eq!((off, n), (0, MIB));
        // Split while 1 MiB is in flight: the tail must start after it.
        let tail = b.claim().unwrap();
        assert!(tail.range.start() >= MIB);
        // Snapshot does not count reserved bytes.
        assert_eq!(b.snapshot().covered_len(), 0);
        b.commit(c.slot, n);
        assert_eq!(b.snapshot().covered_len(), MIB);
    }

    #[test]
    fn release_requeues_rest() {
        let mut b = Board::new(Some(MIB), 1, true, CoverageMap::default());
        let c = b.claim().unwrap();
        let (_, n) = b.reserve(c.slot, 1000).unwrap();
        b.commit(c.slot, n);
        b.release(c.slot);
        let again = b.claim().unwrap();
        assert_eq!(again.range.start(), 1000);
        assert_eq!(b.done().covered_len(), 1000);
    }

    #[test]
    fn go_single_resets() {
        let mut b = Board::new(Some(4 * MIB), 4, true, CoverageMap::default());
        let a = b.claim().unwrap();
        let other = b.claim().unwrap();
        let (_, n) = b.reserve(other.slot, 100).unwrap();
        b.commit(other.slot, n);
        let r = b.go_single(a.slot).unwrap();
        assert_eq!(r, ByteRange::new(0, 4 * MIB).unwrap());
        assert!(b.reserve(other.slot, 10).is_none());
        assert_eq!(b.downloaded(), 0);
        assert!(b.claim().is_none());
    }

    #[test]
    fn unknown_size_single_stream() {
        let mut b = Board::new(None, 8, true, CoverageMap::default());
        let c = b.claim().unwrap();
        assert!(b.claim().is_none());
        let (_, n) = b.reserve(c.slot, 500).unwrap();
        b.commit(c.slot, n);
        assert_eq!(b.finish_unknown(c.slot), Some(500));
        assert!(b.is_complete());
    }

    #[test]
    fn hostile_board_unknown_slot_ids() {
        let mut b = Board::new(Some(MIB), 2, true, CoverageMap::default());
        assert!(b.reserve(999, 10).is_none());
        assert!(b.commit(999, 10));
        b.release(999);
        b.unreserve(999);
        assert!(b.go_single(999).is_none());
        // Committing more than reserved never moves past reserved.
        let c = b.claim().unwrap();
        let (_, n) = b.reserve(c.slot, 10).unwrap();
        b.commit(c.slot, n + 1_000_000);
        assert_eq!(b.position(c.slot), Some(c.range.start() + 10));
    }

    proptest! {
        #[test]
        fn board_covers_exactly_once(size in 1u64..(16 * MIB), conns in 1u32..16, chunk in 4096u64..(2 * MIB)) {
            let mut b = Board::new(Some(size), conns, true, CoverageMap::default());
            drain(&mut b, chunk);
            prop_assert!(b.is_complete());
            prop_assert_eq!(b.done().covered_len(), size);
        }
    }
}
