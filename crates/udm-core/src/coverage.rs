//! Byte ranges and the coverage map of completed bytes of a download.

use crate::message::Message;

/// Half-open byte range `[start, end)`. Always `start <= end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteRange {
    start: u64,
    end: u64,
}

impl ByteRange {
    /// Creates a range; `None` if `start > end`.
    pub const fn new(start: u64, end: u64) -> Option<Self> {
        if start <= end {
            Some(Self { start, end })
        } else {
            None
        }
    }

    /// Inclusive start.
    pub const fn start(self) -> u64 {
        self.start
    }

    /// Exclusive end.
    pub const fn end(self) -> u64 {
        self.end
    }

    /// Length in bytes.
    pub const fn len(self) -> u64 {
        // start <= end is an invariant, so this cannot underflow.
        self.end.saturating_sub(self.start)
    }

    /// True when the range is empty.
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Inclusive last byte (`None` when empty), as used in `Range: bytes=a-b`.
    pub fn last_inclusive(self) -> Option<u64> {
        if self.is_empty() {
            None
        } else {
            self.end.checked_sub(1)
        }
    }

    /// True if the two ranges share at least one byte.
    pub const fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// True if `other` lies within `self`.
    pub const fn contains_range(self, other: Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }
}

/// Sorted, disjoint, non-adjacent set of completed ranges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CoverageMap {
    ranges: Vec<ByteRange>,
}

impl CoverageMap {
    /// Empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a map from stored ranges. Rejects overlapping ranges (a sign of a
    /// corrupt state file) but merges adjacent ones.
    pub fn from_ranges(ranges: &[ByteRange]) -> Result<Self, Message> {
        let mut sorted: Vec<ByteRange> = ranges.iter().copied().filter(|r| !r.is_empty()).collect();
        sorted.sort();
        let mut map = Self::new();
        for r in sorted {
            if map.ranges.last().is_some_and(|last| last.overlaps(r)) {
                return Err(Message::new("error-state-coverage-overlap"));
            }
            map.insert(r);
        }
        Ok(map)
    }

    /// Ranges in ascending order.
    pub fn ranges(&self) -> &[ByteRange] {
        &self.ranges
    }

    /// Adds a completed range, merging with neighbours.
    pub fn insert(&mut self, range: ByteRange) {
        if range.is_empty() {
            return;
        }
        let mut start = range.start;
        let mut end = range.end;
        let mut kept = Vec::with_capacity(self.ranges.len().saturating_add(1));
        let mut placed = false;
        for r in &self.ranges {
            if r.end < start {
                kept.push(*r);
            } else if r.start > end {
                if !placed {
                    kept.push(ByteRange { start, end });
                    placed = true;
                }
                kept.push(*r);
            } else {
                start = start.min(r.start);
                end = end.max(r.end);
            }
        }
        if !placed {
            kept.push(ByteRange { start, end });
        }
        self.ranges = kept;
    }

    /// Total covered bytes.
    pub fn covered_len(&self) -> u64 {
        self.ranges
            .iter()
            .fold(0u64, |acc, r| acc.saturating_add(r.len()))
    }

    /// True if `range` is fully covered.
    pub fn covers(&self, range: ByteRange) -> bool {
        range.is_empty() || self.ranges.iter().any(|r| r.contains_range(range))
    }

    /// Uncovered ranges within `[0, size)`.
    pub fn gaps(&self, size: u64) -> Vec<ByteRange> {
        let mut gaps = Vec::new();
        let mut cursor = 0u64;
        for r in &self.ranges {
            if r.start >= size {
                break;
            }
            if r.start > cursor {
                gaps.push(ByteRange {
                    start: cursor,
                    end: r.start,
                });
            }
            cursor = cursor.max(r.end);
        }
        if cursor < size {
            gaps.push(ByteRange {
                start: cursor,
                end: size,
            });
        }
        gaps
    }

    /// Finalisation check: exactly `[0, size)` covered, nothing beyond.
    pub fn is_complete(&self, size: u64) -> bool {
        match self.ranges.as_slice() {
            [] => size == 0,
            [only] => only.start == 0 && only.end == size,
            _ => false,
        }
    }

    /// Largest covered end, used for progress of single-connection downloads.
    pub fn contiguous_prefix(&self) -> u64 {
        match self.ranges.first() {
            Some(r) if r.start == 0 => r.end,
            _ => 0,
        }
    }

    /// Clears all coverage (download restarted).
    pub fn clear(&mut self) {
        self.ranges.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(a: u64, b: u64) -> ByteRange {
        ByteRange::new(a, b).unwrap()
    }

    #[test]
    fn insert_merges_and_orders() {
        let mut m = CoverageMap::new();
        m.insert(r(10, 20));
        m.insert(r(0, 5));
        m.insert(r(5, 10));
        assert_eq!(m.ranges(), &[r(0, 20)]);
        m.insert(r(30, 40));
        m.insert(r(15, 35));
        assert_eq!(m.ranges(), &[r(0, 40)]);
        assert!(m.is_complete(40));
        assert!(!m.is_complete(41));
    }

    #[test]
    fn gaps_found() {
        let mut m = CoverageMap::new();
        m.insert(r(10, 20));
        m.insert(r(30, 40));
        assert_eq!(m.gaps(50), vec![r(0, 10), r(20, 30), r(40, 50)]);
        assert_eq!(m.covered_len(), 20);
    }

    #[test]
    fn hostile_state_coverage_overlap_rejected() {
        assert!(CoverageMap::from_ranges(&[r(0, 10), r(5, 15)]).is_err());
        assert!(CoverageMap::from_ranges(&[r(0, 10), r(10, 15)]).is_ok());
        assert!(ByteRange::new(5, 4).is_none());
    }

    #[test]
    fn coverage_beyond_size_not_complete() {
        let mut m = CoverageMap::new();
        m.insert(r(0, 60));
        assert!(!m.is_complete(50));
    }
}
