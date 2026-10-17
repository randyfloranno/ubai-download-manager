//! Segmentation planner and dynamic re-splitting (ADR 0003 §7). Pure and deterministic.

use udm_core::{ByteRange, CoverageMap, limits};

/// A segment currently being fetched: bytes `[range.start, pos)` are done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveSegment {
    /// Assigned range.
    pub range: ByteRange,
    /// Next byte to fetch (`range.start ..= range.end`).
    pub pos: u64,
}

/// Plans the segments that fetch every uncovered byte of `[0, size)`.
///
/// - Segments are sorted, disjoint and lie within `[0, size)`; they cover every gap
///   of `coverage` and, when there are at most `limits::MAX_SEGMENTS` gaps, nothing else.
/// - With more than `MAX_SEGMENTS` gaps (only possible from a damaged state file),
///   consecutive gaps are merged so the plan stays bounded; merged segments
///   re-fetch the covered bytes between them, which is safe.
/// - When there are fewer gaps than `connections` (clamped to
///   `1..=limits::MAX_CONNECTIONS_PER_DOWNLOAD`), the largest gaps are split into
///   pieces of at least `min_segment` bytes (the last piece of a gap may be smaller);
///   inner split points are aligned down to `limits::SEGMENT_ALIGN_BYTES` when possible.
pub fn plan_segments(
    size: u64,
    connections: u32,
    min_segment: u64,
    coverage: &CoverageMap,
) -> Vec<ByteRange> {
    let min_segment = min_segment.max(1);
    let connections =
        usize::try_from(connections.clamp(1, limits::MAX_CONNECTIONS_PER_DOWNLOAD)).unwrap_or(1);
    let gaps = bound_gaps(coverage.gaps(size), limits::MAX_SEGMENTS);
    if gaps.is_empty() {
        return gaps;
    }
    // Pieces per gap: start with one each, hand out the rest to the gap whose pieces
    // are currently the largest (ties: lowest index), respecting min_segment.
    let mut pieces: Vec<u64> = vec![1; gaps.len()];
    let mut spare = connections.saturating_sub(gaps.len());
    while spare > 0 {
        let mut best: Option<(usize, u64)> = None;
        for (i, (gap, k)) in gaps.iter().zip(pieces.iter()).enumerate() {
            let next = k.saturating_add(1);
            let max_pieces = gap.len().checked_div(min_segment).unwrap_or(0);
            if next > max_pieces {
                continue;
            }
            let per_piece = gap.len().checked_div(*k).unwrap_or(0);
            if best.is_none_or(|(_, b)| per_piece > b) {
                best = Some((i, per_piece));
            }
        }
        let Some((i, _)) = best else { break };
        if let Some(k) = pieces.get_mut(i) {
            *k = k.saturating_add(1);
        }
        spare = spare.saturating_sub(1);
    }
    let mut out = Vec::with_capacity(gaps.len().saturating_add(connections));
    for (gap, k) in gaps.iter().zip(pieces.iter()) {
        split_gap(*gap, *k, min_segment, &mut out);
    }
    out
}

/// Merges consecutive gaps into at most `max` ranges (deterministic, O(n)).
fn bound_gaps(gaps: Vec<ByteRange>, max: usize) -> Vec<ByteRange> {
    if gaps.len() <= max || max == 0 {
        return gaps;
    }
    let per = gaps.len().div_ceil(max);
    gaps.chunks(per)
        .filter_map(|c| match (c.first(), c.last()) {
            (Some(a), Some(b)) => ByteRange::new(a.start(), b.end()),
            _ => None,
        })
        .collect()
}

/// Splits one gap into at most `k` pieces; all but the last are ≥ `min_segment`.
fn split_gap(gap: ByteRange, k: u64, min_segment: u64, out: &mut Vec<ByteRange>) {
    let len = u128::from(gap.len());
    let k128 = u128::from(k.max(1));
    let mut prev = gap.start();
    let mut i: u64 = 1;
    while i < k {
        // raw = start + floor(i * len / k), computed in u128 (cannot overflow).
        let offset = u128::from(i)
            .checked_mul(len)
            .and_then(|v| v.checked_div(k128));
        let raw = offset
            .and_then(|o| u64::try_from(o).ok())
            .and_then(|o| gap.start().checked_add(o));
        if let Some(raw) = raw {
            let aligned = align_down(raw, limits::SEGMENT_ALIGN_BYTES);
            let cut = if aligned > prev && aligned.saturating_sub(prev) >= min_segment {
                aligned
            } else {
                raw
            };
            let fits = cut.saturating_sub(prev) >= min_segment && cut < gap.end();
            if let Some(piece) = ByteRange::new(prev, cut).filter(|_| fits) {
                out.push(piece);
                prev = cut;
            }
        }
        i = i.saturating_add(1);
    }
    if let Some(tail) = ByteRange::new(prev, gap.end()).filter(|r| !r.is_empty()) {
        out.push(tail);
    }
}

fn align_down(v: u64, align: u64) -> u64 {
    match v.checked_rem(align) {
        Some(r) => v.saturating_sub(r),
        None => v,
    }
}

fn align_up(v: u64, align: u64) -> Option<u64> {
    match v.checked_rem(align)? {
        0 => Some(v),
        r => v.checked_add(align.checked_sub(r)?),
    }
}

/// Dynamic segmentation: when a worker goes idle, pick the active segment with the
/// most remaining bytes (ties: lowest index) and split its remainder.
///
/// Returns `(index, split_at)`: the owner keeps `[range.start, split_at)`, the idle
/// worker takes `[split_at, range.end)`. `split_at` is
/// `align_up(pos + remaining / 2, limits::SEGMENT_ALIGN_BYTES)` and both halves of the
/// remainder (`[pos, split_at)` and `[split_at, end)`) must be ≥ `min_segment`.
/// Segments with `pos` outside their range are ignored.
pub fn split_for_idle(active: &[ActiveSegment], min_segment: u64) -> Option<(usize, u64)> {
    let min_segment = min_segment.max(1);
    let mut best: Option<(usize, u64)> = None;
    for (i, seg) in active.iter().enumerate() {
        if seg.pos < seg.range.start() || seg.pos > seg.range.end() {
            continue;
        }
        let rem = seg.range.end().saturating_sub(seg.pos);
        if best.is_none_or(|(_, b)| rem > b) {
            best = Some((i, rem));
        }
    }
    let (i, rem) = best?;
    let seg = active.get(i)?;
    let split_at = align_up(
        seg.pos.checked_add(rem.checked_div(2)?)?,
        limits::SEGMENT_ALIGN_BYTES,
    )?;
    let owner = split_at.checked_sub(seg.pos)?;
    let idle = seg.range.end().checked_sub(split_at)?;
    if owner >= min_segment && idle >= min_segment {
        Some((i, split_at))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn r(a: u64, b: u64) -> ByteRange {
        ByteRange::new(a, b).unwrap()
    }

    const MIB: u64 = 1_048_576;

    #[test]
    fn fresh_download_splits_evenly() {
        let plan = plan_segments(32 * MIB, 8, limits::MIN_SEGMENT_BYTES, &CoverageMap::new());
        assert_eq!(plan.len(), 8);
        assert!(plan.iter().all(|p| p.len() == 4 * MIB));
        assert_eq!(plan.first().unwrap().start(), 0);
        assert_eq!(plan.last().unwrap().end(), 32 * MIB);
    }

    #[test]
    fn small_file_single_segment() {
        let plan = plan_segments(1000, 8, limits::MIN_SEGMENT_BYTES, &CoverageMap::new());
        assert_eq!(plan, vec![r(0, 1000)]);
        assert!(plan_segments(0, 8, limits::MIN_SEGMENT_BYTES, &CoverageMap::new()).is_empty());
    }

    #[test]
    fn resume_plans_only_gaps() {
        let mut cov = CoverageMap::new();
        cov.insert(r(0, 10 * MIB));
        cov.insert(r(20 * MIB, 30 * MIB));
        let plan = plan_segments(32 * MIB, 2, limits::MIN_SEGMENT_BYTES, &cov);
        assert_eq!(plan, vec![r(10 * MIB, 20 * MIB), r(30 * MIB, 32 * MIB)]);
        let mut done = CoverageMap::new();
        done.insert(r(0, 32 * MIB));
        assert!(plan_segments(32 * MIB, 8, limits::MIN_SEGMENT_BYTES, &done).is_empty());
    }

    #[test]
    fn hostile_segment_count_bounded() {
        // A damaged state file with far more gaps than limits::MAX_SEGMENTS.
        let n = (limits::MAX_SEGMENTS as u64) * 3 + 7;
        let mut cov = CoverageMap::new();
        for i in 0..n {
            cov.insert(r(i * 10, i * 10 + 5));
        }
        let size = n * 10;
        let plan = plan_segments(size, u32::MAX, 1, &cov);
        assert!(plan.len() <= limits::MAX_SEGMENTS);
        check_disjoint_within(&plan, size);
        for g in cov.gaps(size) {
            assert!(plan.iter().any(|p| p.contains_range(g)));
        }
        // Connections are clamped.
        let plan = plan_segments(1 << 40, u32::MAX, 1, &CoverageMap::new());
        assert_eq!(plan.len(), limits::MAX_CONNECTIONS_PER_DOWNLOAD as usize);
        let plan = plan_segments(u64::MAX, 32, 1, &CoverageMap::new());
        assert_eq!(plan.len(), 32);
        check_disjoint_within(&plan, u64::MAX);
    }

    #[test]
    fn idle_split_picks_largest() {
        let a = ActiveSegment {
            range: r(0, 8 * MIB),
            pos: 6 * MIB,
        };
        let b = ActiveSegment {
            range: r(8 * MIB, 16 * MIB),
            pos: 9 * MIB,
        };
        let (i, at) = split_for_idle(&[a, b], limits::MIN_SEGMENT_BYTES).unwrap();
        assert_eq!(i, 1);
        assert_eq!(at, 9 * MIB + 3 * MIB + MIB / 2);
        assert_eq!(at % limits::SEGMENT_ALIGN_BYTES, 0);
        // Too small to split.
        let c = ActiveSegment {
            range: r(0, MIB),
            pos: MIB - 1000,
        };
        assert_eq!(split_for_idle(&[c], limits::MIN_SEGMENT_BYTES), None);
        assert_eq!(split_for_idle(&[], limits::MIN_SEGMENT_BYTES), None);
    }

    #[test]
    fn hostile_idle_split_bad_positions() {
        let bad = ActiveSegment {
            range: r(100, 200),
            pos: 50,
        };
        let past = ActiveSegment {
            range: r(0, u64::MAX),
            pos: u64::MAX,
        };
        assert_eq!(split_for_idle(&[bad, past], 1), None);
        let edge = ActiveSegment {
            range: r(0, u64::MAX),
            pos: 0,
        };
        let (_, at) = split_for_idle(&[edge], 1).unwrap();
        assert_eq!(at % limits::SEGMENT_ALIGN_BYTES, 0);
    }

    fn check_disjoint_within(plan: &[ByteRange], size: u64) {
        for w in plan.windows(2) {
            assert!(w[0].end() <= w[1].start(), "{w:?}");
        }
        assert!(plan.iter().all(|p| !p.is_empty() && p.end() <= size));
    }

    fn coverage_strategy() -> impl Strategy<Value = (u64, CoverageMap)> {
        (
            1u64..(64 * MIB),
            prop::collection::vec((0u64..(64 * MIB), 0u64..(8 * MIB)), 0..20),
        )
            .prop_map(|(size, raw)| {
                let mut cov = CoverageMap::new();
                for (s, l) in raw {
                    let s = s % size;
                    let e = (s + l).min(size);
                    cov.insert(r(s, e));
                }
                (size, cov)
            })
    }

    proptest! {
        #[test]
        fn plan_covers_gaps_exactly_once(
            (size, cov) in coverage_strategy(),
            connections in 0u32..64,
            min_segment in prop_oneof![Just(limits::MIN_SEGMENT_BYTES), 1u64..(2 * MIB)],
        ) {
            let plan = plan_segments(size, connections, min_segment, &cov);
            prop_assert!(plan.len() <= limits::MAX_SEGMENTS);
            check_disjoint_within(&plan, size);
            // Union of the plan == gaps (exactly once: disjoint + same total length).
            let gaps = cov.gaps(size);
            let total: u64 = plan.iter().map(|p| p.len()).sum();
            let gap_total: u64 = gaps.iter().map(|g| g.len()).sum();
            prop_assert_eq!(total, gap_total);
            for p in &plan {
                prop_assert!(gaps.iter().any(|g| g.contains_range(*p)));
            }
            // Pieces >= min_segment except the last piece of each gap.
            for g in &gaps {
                let inside: Vec<_> = plan.iter().filter(|p| g.contains_range(**p)).collect();
                for p in inside.iter().take(inside.len().saturating_sub(1)) {
                    prop_assert!(p.len() >= min_segment.max(1));
                }
            }
            let effective = connections.clamp(1, limits::MAX_CONNECTIONS_PER_DOWNLOAD) as usize;
            prop_assert!(plan.len() <= gaps.len().max(effective));
            // Deterministic.
            prop_assert_eq!(plan_segments(size, connections, min_segment, &cov), plan);
        }

        #[test]
        fn idle_split_halves_bounded_and_aligned(
            segs in prop::collection::vec((0u64..(1u64 << 40), 0u64..(1u64 << 32), 0u64..=100), 0..16),
            min_segment in 1u64..(4 * MIB),
        ) {
            let active: Vec<ActiveSegment> = segs
                .iter()
                .map(|(s, l, pct)| ActiveSegment { range: r(*s, s + l), pos: s + l * pct / 100 })
                .collect();
            let res = split_for_idle(&active, min_segment);
            prop_assert_eq!(split_for_idle(&active, min_segment), res);
            if let Some((i, at)) = res {
                let seg = active[i];
                prop_assert_eq!(at % limits::SEGMENT_ALIGN_BYTES, 0);
                prop_assert!(at > seg.pos && at < seg.range.end());
                prop_assert!(at - seg.pos >= min_segment);
                prop_assert!(seg.range.end() - at >= min_segment);
                let rem = seg.range.end() - seg.pos;
                for (j, other) in active.iter().enumerate() {
                    let orem = other.range.end() - other.pos;
                    prop_assert!(orem < rem || (orem == rem && j >= i));
                }
            }
        }
    }
}
