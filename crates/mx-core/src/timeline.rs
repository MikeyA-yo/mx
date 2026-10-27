//! Time ranges on a media timeline: the building block of edit decision
//! lists. Jump cuts produce a list of ranges to keep; highlights and the
//! eventual timeline editor build on the same type.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Range {
    pub start: f64,
    pub end: f64,
}

impl Range {
    pub fn new(start: f64, end: f64) -> Self {
        Self { start, end }
    }

    pub fn len(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() <= 0.0
    }
}

pub fn total(ranges: &[Range]) -> f64 {
    ranges.iter().map(Range::len).sum()
}

/// Ranges shorter than this are dropped rather than kept as one-frame
/// slivers that would read as glitches.
const MIN_PIECE: f64 = 0.05;

/// The parts of `[0, duration]` to keep after removing `cuts`.
///
/// Every cut first gives back `padding` seconds at each edge that touches
/// kept material, so speech doesn't start or end abruptly on a cut. Cuts
/// that touch the start or end of the file only give back padding on their
/// inner side. A cut too short to survive the padding is skipped.
pub fn keep_ranges(duration: f64, cuts: &[Range], padding: f64) -> Vec<Range> {
    let mut cuts: Vec<Range> = cuts
        .iter()
        .map(|c| {
            let start = if c.start <= 0.0 { 0.0 } else { c.start + padding };
            let end = if c.end >= duration { duration } else { c.end - padding };
            Range::new(start.max(0.0), end.min(duration))
        })
        .filter(|c| c.len() >= MIN_PIECE)
        .collect();
    cuts.sort_by(|a, b| a.start.total_cmp(&b.start));

    let mut keep = Vec::new();
    let mut cursor = 0.0;
    for cut in cuts {
        if cut.start > cursor {
            keep.push(Range::new(cursor, cut.start));
        }
        cursor = cursor.max(cut.end);
    }
    if cursor < duration {
        keep.push(Range::new(cursor, duration));
    }
    keep.retain(|r| r.len() >= MIN_PIECE);
    keep
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: f64, end: f64) -> Range {
        Range::new(start, end)
    }

    fn approx(actual: &[Range], expected: &[(f64, f64)]) {
        assert_eq!(actual.len(), expected.len(), "{actual:?}");
        for (a, (s, e)) in actual.iter().zip(expected) {
            assert!((a.start - s).abs() < 1e-9 && (a.end - e).abs() < 1e-9, "{actual:?} vs {expected:?}");
        }
    }

    #[test]
    fn padding_gives_back_time_on_both_sides_of_inner_cuts() {
        let keep = keep_ranges(10.0, &[r(2.0, 4.0), r(6.0, 7.0)], 0.25);
        approx(&keep, &[(0.0, 2.25), (3.75, 6.25), (6.75, 10.0)]);
    }

    #[test]
    fn edge_cuts_only_pad_their_inner_side() {
        let keep = keep_ranges(10.0, &[r(0.0, 1.5), r(9.0, 10.0)], 0.25);
        approx(&keep, &[(1.25, 9.25)]);
    }

    #[test]
    fn cuts_too_short_for_the_padding_are_skipped() {
        // 0.4 s of silence minus 2 x 0.2 s padding leaves nothing to cut.
        let keep = keep_ranges(10.0, &[r(5.0, 5.4)], 0.2);
        approx(&keep, &[(0.0, 10.0)]);
    }

    #[test]
    fn unsorted_and_overlapping_cuts_are_handled() {
        let keep = keep_ranges(10.0, &[r(6.0, 8.0), r(1.0, 3.0), r(2.5, 7.0)], 0.0);
        approx(&keep, &[(0.0, 1.0), (8.0, 10.0)]);
        assert!((total(&keep) - 3.0).abs() < 1e-9);
    }
}
