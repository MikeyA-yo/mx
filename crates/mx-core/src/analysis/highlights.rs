//! Picking the most eventful stretches of a recording.
//!
//! Every candidate window (one per second of the timeline) is described by
//! up to four signals:
//!
//! - loudness: how loud it is (shouting, laughter, a music drop);
//! - activity: how little of it is silence (no dead air);
//! - motion: how much the picture moves, not counting cuts;
//! - pace: how many scene cuts it has.
//!
//! Raw values aren't comparable across recordings (a podcast's "loud" is a
//! concert's "quiet"), so each signal is turned into a rank among this
//! recording's own windows, 0 for the lowest and 1 for the highest. A
//! signal that barely varies, like the motion of a static podcast shot,
//! would only rank noise, so it's left out. The score is a weighted mean of
//! the ranks that remain.
//!
//! The best windows are picked greedily, kept apart from each other, and
//! their edges moved onto a nearby pause or scene cut so a clip doesn't
//! start or end mid-word.

use std::path::Path;
use std::thread;

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use super::loudness::{Envelope, SILENCE_DB, measure_loudness};
use super::motion::{MotionTrack, measure_motion};
use super::silence::{SilenceOptions, find_silences, percentile};
use crate::cancel::CancelToken;
use crate::ffmpeg::{Progress, probe};
use crate::timeline::Range;

/// The raw measurements highlights are scored from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signals {
    pub duration: f64,
    pub loudness: Option<Envelope>,
    pub motion: Option<MotionTrack>,
}

/// Measure loudness and motion at the same time (two ffmpeg processes;
/// the video side is slower, so its progress is the one reported).
pub fn measure_signals(input: &Path, cancel: &CancelToken, on_progress: impl FnMut(&Progress)) -> Result<Signals> {
    let source = probe(input)?;
    let duration = source
        .duration_secs()
        .ok_or_else(|| anyhow!("couldn't read the duration of {}", input.display()))?;
    let (has_video, has_audio) = (source.video().is_some(), source.has_audio());
    if !has_video && !has_audio {
        bail!("{} has neither audio nor video", input.display());
    }
    thread::scope(|s| {
        if !has_video {
            let loudness = measure_loudness(input, cancel, on_progress)?;
            return Ok(Signals { duration, loudness: Some(loudness), motion: None });
        }
        let audio = has_audio.then(|| s.spawn(|| measure_loudness(input, cancel, |_| {})));
        let motion = measure_motion(input, cancel, on_progress);
        let loudness = audio.map(|h| h.join().expect("loudness thread panicked")).transpose()?;
        Ok(Signals { duration, loudness, motion: Some(motion?) })
    })
}

#[derive(Debug, Clone, Copy)]
pub struct HighlightOptions {
    pub count: usize,
    /// Target clip length in seconds; snapping can change it by up to 20%.
    pub length: f64,
    /// Only search this part of the recording, e.g. to leave out rolling
    /// credits, which look like action to every signal here (loud music,
    /// fast-moving text). `None` searches everything.
    pub within: Option<Range>,
}

impl Default for HighlightOptions {
    fn default() -> Self {
        Self { count: 3, length: 30.0, within: None }
    }
}

/// Each signal's rank for one highlight, 0..1; `None` when the signal
/// wasn't available or didn't vary enough to count.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Breakdown {
    pub loudness: Option<f64>,
    pub activity: Option<f64>,
    pub motion: Option<f64>,
    pub pace: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Highlight {
    pub range: Range,
    /// 0..1, relative to the rest of this recording, not across recordings.
    pub score: f64,
    pub breakdown: Breakdown,
}

#[derive(Debug, Clone, Copy)]
enum Signal {
    Loudness,
    Activity,
    Motion,
    Pace,
}

impl Signal {
    fn weight(self) -> f64 {
        match self {
            Self::Loudness => 0.35,
            Self::Motion => 0.30,
            Self::Activity => 0.20,
            Self::Pace => 0.15,
        }
    }

    /// Below this spread between quiet (10th percentile) and busy (90th
    /// percentile) windows, in the signal's own units, the signal is noise.
    fn min_spread(self) -> f64 {
        match self {
            Self::Loudness => 1.5, // dB
            Self::Activity => 0.05,
            Self::Motion => 0.1,
            Self::Pace => 0.5, // cuts per window
        }
    }

    fn set(self, breakdown: &mut Breakdown, value: f64) {
        let field = match self {
            Self::Loudness => &mut breakdown.loudness,
            Self::Activity => &mut breakdown.activity,
            Self::Motion => &mut breakdown.motion,
            Self::Pace => &mut breakdown.pace,
        };
        *field = Some(value);
    }
}

/// A clip starts this long before speech resumes after a pause, and ends
/// this long after speech stops.
const LEAD: f64 = 0.15;

/// Mean power of the 20 ms windows in each second, in dB.
fn loudness_bins(env: &Envelope, bins: usize) -> Vec<f64> {
    let mut power = vec![0.0; bins];
    let mut count = vec![0usize; bins];
    for &(t, db) in &env.points {
        let i = (t.max(0.0) as usize).min(bins - 1);
        power[i] += 10f64.powf(db / 10.0);
        count[i] += 1;
    }
    power
        .iter()
        .zip(&count)
        .map(|(&p, &n)| if n == 0 { SILENCE_DB } else { (10.0 * (p / n as f64).log10()).max(SILENCE_DB) })
        .collect()
}

/// Fraction of each second that isn't inside a pause.
fn activity_bins(silences: &[Range], bins: usize) -> Vec<f64> {
    let mut silent = vec![0.0; bins];
    for s in silences {
        let mut t = s.start.max(0.0);
        while t < s.end && (t as usize) < bins {
            let i = t as usize;
            let edge = ((i + 1) as f64).min(s.end);
            silent[i] += edge - t;
            t = edge;
        }
    }
    silent.into_iter().map(|s| (1.0 - s).clamp(0.0, 1.0)).collect()
}

/// Mean frame difference in each second, leaving out cut frames (a cut is
/// a change of shot, not movement).
fn motion_bins(track: &MotionTrack, bins: usize) -> Vec<f64> {
    let mut sum = vec![0.0; bins];
    let mut count = vec![0usize; bins];
    for p in track.points.iter().filter(|p| !p.is_cut()) {
        let i = (p.time.max(0.0) as usize).min(bins - 1);
        sum[i] += p.mafd;
        count[i] += 1;
    }
    sum.iter().zip(&count).map(|(&s, &n)| if n == 0 { 0.0 } else { s / n as f64 }).collect()
}

fn cut_bins(cuts: &[f64], bins: usize) -> Vec<f64> {
    let mut count = vec![0.0; bins];
    for &t in cuts {
        count[(t.max(0.0) as usize).min(bins - 1)] += 1.0;
    }
    count
}

/// Sum of every `span` consecutive values.
fn window_sums(values: &[f64], span: usize) -> Vec<f64> {
    let mut prefix = vec![0.0];
    for v in values {
        prefix.push(prefix.last().unwrap() + v);
    }
    (0..=values.len() - span).map(|i| prefix[i + span] - prefix[i]).collect()
}

fn window_means(values: &[f64], span: usize) -> Vec<f64> {
    window_sums(values, span).into_iter().map(|s| s / span as f64).collect()
}

/// Rank of each value among all of them, 0 (lowest) to 1 (highest); equal
/// values share their average rank.
fn ranks(values: &[f64]) -> Vec<f64> {
    let n = values.len();
    if n < 2 {
        return vec![1.0; n];
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let mut out = vec![0.0; n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && values[order[j + 1]] == values[order[i]] {
            j += 1;
        }
        let rank = (i + j) as f64 / 2.0 / (n - 1) as f64;
        for &k in &order[i..=j] {
            out[k] = rank;
        }
        i = j + 1;
    }
    out
}

fn spread(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    percentile(&sorted, 0.9) - percentile(&sorted, 0.1)
}

/// The candidate in `points` nearest to `t`, if one is within `tolerance`.
fn nearest(points: impl IntoIterator<Item = f64>, t: f64, tolerance: f64) -> Option<f64> {
    points
        .into_iter()
        .filter(|p| (p - t).abs() <= tolerance)
        .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
}

/// Move each edge onto a pause if one is near, else onto a scene cut,
/// staying inside `area`.
fn snap(range: Range, silences: &[Range], cuts: &[f64], area: Range, tolerance: f64) -> Range {
    let speech_starts = silences.iter().map(|s| (s.end - LEAD).max(s.start)).chain([area.start]);
    let speech_ends = silences.iter().map(|s| (s.start + LEAD).min(s.end)).chain([area.end]);
    let start = nearest(speech_starts, range.start, tolerance)
        .or_else(|| nearest(cuts.iter().copied(), range.start, tolerance))
        .unwrap_or(range.start);
    let end = nearest(speech_ends, range.end, tolerance)
        .or_else(|| nearest(cuts.iter().copied(), range.end, tolerance))
        .unwrap_or(range.end);
    Range::new(start.max(area.start), end.min(area.end))
}

/// The `opts.count` best non-overlapping windows, best first. Fewer come
/// back when the recording is too short to hold that many.
pub fn find_highlights(signals: &Signals, opts: &HighlightOptions) -> Vec<Highlight> {
    let duration = signals.duration;
    let whole = Range::new(0.0, duration);
    let area = opts.within.map_or(whole, |r| Range::new(r.start.max(0.0), r.end.min(duration)));
    if area.is_empty() || opts.count == 0 || opts.length <= 0.0 {
        return Vec::new();
    }
    let length = opts.length.min(area.len());
    let bins = duration.ceil() as usize;
    let span = (length.round() as usize).clamp(1, bins);
    let windows = bins - span + 1;
    // Window w covers [w, w + length); keep those inside the area.
    let first = (area.start.ceil() as usize).min(windows - 1);
    let last = ((area.end - length + 1e-9).floor().max(0.0) as usize).clamp(first, windows - 1);
    let candidates: Vec<usize> = (first..=last).collect();

    let silences = match &signals.loudness {
        Some(env) => find_silences(env, &SilenceOptions::default()).silences,
        None => Vec::new(),
    };
    let cuts = signals.motion.as_ref().map(MotionTrack::scene_changes).unwrap_or_default();

    let mut features: Vec<(Signal, Vec<f64>)> = Vec::new();
    if let Some(env) = &signals.loudness {
        features.push((Signal::Loudness, window_means(&loudness_bins(env, bins), span)));
        features.push((Signal::Activity, window_means(&activity_bins(&silences, bins), span)));
    }
    if let Some(track) = &signals.motion {
        features.push((Signal::Motion, window_means(&motion_bins(track, bins), span)));
        features.push((Signal::Pace, window_sums(&cut_bins(&cuts, bins), span)));
    }
    // From here on, values are per candidate: ranked against each other only.
    let features: Vec<(Signal, Vec<f64>)> = features
        .into_iter()
        .map(|(signal, values)| (signal, candidates.iter().map(|&w| values[w]).collect::<Vec<_>>()))
        .filter(|(signal, values)| spread(values) >= signal.min_spread())
        .map(|(signal, values)| (signal, ranks(&values)))
        .collect();

    let total_weight: f64 = features.iter().map(|(s, _)| s.weight()).sum();
    let scores: Vec<f64> = (0..candidates.len())
        .map(|c| {
            let sum: f64 = features.iter().map(|(s, r)| s.weight() * r[c]).sum();
            if total_weight > 0.0 { sum / total_weight } else { 0.0 }
        })
        .collect();

    // Best first; among equal scores, earliest first.
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]).then(a.cmp(&b)));
    // Half a clip of space between picks, so two halves of one moment
    // don't come back as two highlights.
    let gap = length / 2.0;
    let mut picked: Vec<usize> = Vec::new();
    for w in order {
        if picked.len() == opts.count {
            break;
        }
        let start = candidates[w] as f64;
        if picked.iter().all(|&p| (candidates[p] as f64 - start).abs() >= length + gap) {
            picked.push(w);
        }
    }

    let tolerance = (length * 0.1).clamp(0.5, 3.0);
    picked
        .into_iter()
        .map(|c| {
            let start = candidates[c] as f64;
            let range = snap(Range::new(start, (start + length).min(area.end)), &silences, &cuts, area, tolerance);
            let mut breakdown = Breakdown::default();
            for (signal, r) in &features {
                signal.set(&mut breakdown, r[c]);
            }
            Highlight { range, score: scores[c], breakdown }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::loudness::WINDOW_SECS;
    use crate::analysis::motion::MotionPoint;

    /// Loudness from (seconds, dB) segments at 20 ms resolution.
    fn envelope(segments: &[(f64, f64)]) -> Envelope {
        let mut points = Vec::new();
        let mut t = 0.0;
        for &(secs, db) in segments {
            for _ in 0..(secs / WINDOW_SECS).round() as usize {
                points.push((t, db));
                t += WINDOW_SECS;
            }
        }
        Envelope { points }
    }

    /// Motion at 10 frames per second from (seconds, mafd) segments.
    fn motion(segments: &[(f64, f64)], cuts: &[f64]) -> MotionTrack {
        let mut points = Vec::new();
        let mut t = 0.0;
        for &(secs, mafd) in segments {
            for _ in 0..(secs * 10.0).round() as usize {
                points.push(MotionPoint { time: t, mafd, score: 0.0 });
                t += 0.1;
            }
        }
        for &c in cuts {
            points.push(MotionPoint { time: c, mafd: 40.0, score: 40.0 });
        }
        points.sort_by(|a, b| a.time.total_cmp(&b.time));
        MotionTrack { points }
    }

    #[test]
    fn ranks_spread_from_zero_to_one_and_share_ties() {
        assert_eq!(ranks(&[3.0, 1.0, 2.0]), [1.0, 0.0, 0.5]);
        assert_eq!(ranks(&[1.0, 1.0, 5.0]), [0.25, 0.25, 1.0]);
        assert_eq!(ranks(&[7.0]), [1.0]);
    }

    #[test]
    fn window_sums_slide_over_every_start() {
        assert_eq!(window_sums(&[1.0, 2.0, 3.0, 4.0], 2), [3.0, 5.0, 7.0]);
        assert_eq!(window_means(&[1.0, 3.0], 2), [2.0]);
    }

    #[test]
    fn activity_counts_the_unsilent_part_of_each_second() {
        let a = activity_bins(&[Range::new(0.5, 2.25)], 3);
        assert_eq!(a, [0.5, 0.0, 0.75]);
    }

    #[test]
    fn the_loud_busy_stretch_wins() {
        // 60 s: calm talk with pauses, then 10 s of shouting and fast motion
        // at 30..40, then calm again.
        let mut audio = Vec::new();
        for _ in 0..10 {
            audio.extend([(2.5, -24.0), (0.5, -50.0)]);
        }
        audio.push((10.0, -10.0));
        for _ in 0..7 {
            audio.extend([(2.5, -24.0), (0.5, -50.0)]);
        }
        let signals = Signals {
            duration: 61.0,
            loudness: Some(envelope(&audio)),
            motion: Some(motion(&[(30.0, 1.0), (10.0, 12.0), (21.0, 1.0)], &[31.0, 33.0, 35.0, 37.0])),
        };
        let found = find_highlights(&signals, &HighlightOptions { count: 1, length: 10.0, within: None });
        assert_eq!(found.len(), 1);
        let h = &found[0];
        assert!((h.range.start - 30.0).abs() <= 1.0 && (h.range.end - 40.0).abs() <= 1.0, "{h:?}");
        assert!(h.score > 0.9, "{h:?}");
        assert!(h.breakdown.loudness.unwrap() > 0.9 && h.breakdown.motion.unwrap() > 0.9, "{h:?}");
    }

    #[test]
    fn flat_signals_are_left_out() {
        // A static shot: the picture never changes, so motion and pace
        // don't count, and only the audio decides.
        let signals = Signals {
            duration: 30.0,
            loudness: Some(envelope(&[(20.0, -30.0), (10.0, -12.0)])),
            motion: Some(motion(&[(30.0, 0.5)], &[])),
        };
        let found = find_highlights(&signals, &HighlightOptions { count: 1, length: 5.0, within: None });
        let b = found[0].breakdown;
        assert!(b.loudness.is_some() && b.motion.is_none() && b.pace.is_none(), "{b:?}");
        // The quiet part counts as one long pause, so the start snaps to
        // just before the loud part begins.
        assert!((found[0].range.start - 19.85).abs() < 1e-9, "{:?}", found[0]);
    }

    #[test]
    fn picks_are_kept_apart_and_limited_by_length() {
        let signals = Signals {
            duration: 100.0,
            loudness: None,
            motion: Some(motion(&[(50.0, 2.0), (2.0, 20.0), (48.0, 2.0)], &[])),
        };
        let found = find_highlights(&signals, &HighlightOptions { count: 10, length: 20.0, within: None });
        // 100 s holds at most three 20 s clips with 10 s between them.
        assert!(found.len() <= 3, "{found:?}");
        for (i, a) in found.iter().enumerate() {
            for b in &found[i + 1..] {
                assert!(a.range.end <= b.range.start || b.range.end <= a.range.start, "{found:?}");
            }
        }
        // The burst of motion is in the best one.
        assert!(found[0].range.start <= 50.0 && found[0].range.end >= 52.0, "{found:?}");
    }

    #[test]
    fn edges_snap_to_pauses_first_then_cuts() {
        let silences = [Range::new(9.0, 10.0), Range::new(19.5, 20.5)];
        let r = snap(Range::new(11.0, 21.0), &silences, &[12.0, 22.0], Range::new(0.0, 60.0), 2.0);
        // Start: speech resumes at 10.0, so 9.85. End: speech stops at 19.5.
        assert!((r.start - 9.85).abs() < 1e-9 && (r.end - 19.65).abs() < 1e-9, "{r:?}");
        let r = snap(Range::new(11.0, 21.0), &[], &[12.0, 22.0], Range::new(0.0, 60.0), 2.0);
        assert_eq!(r, Range::new(12.0, 22.0));
        let r = snap(Range::new(11.0, 21.0), &[], &[], Range::new(0.0, 60.0), 2.0);
        assert_eq!(r, Range::new(11.0, 21.0));
    }

    #[test]
    fn within_limits_the_search_and_ranks_against_that_part_only() {
        // Wild motion in the "credits" after 60 s; calmer, varied motion before.
        let signals = Signals {
            duration: 90.0,
            loudness: None,
            motion: Some(motion(&[(20.0, 1.0), (10.0, 3.0), (30.0, 1.0), (30.0, 9.0)], &[])),
        };
        let all = find_highlights(&signals, &HighlightOptions { count: 1, length: 10.0, within: None });
        assert!(all[0].range.start >= 60.0, "{all:?}");
        let story = Range::new(0.0, 60.0);
        let found = find_highlights(&signals, &HighlightOptions { count: 3, length: 10.0, within: Some(story) });
        assert!((found[0].range.start - 20.0).abs() < 1e-9, "{found:?}");
        assert!(found[0].breakdown.motion.unwrap() > 0.99, "{found:?}");
        assert!(found.iter().all(|h| h.range.start >= 0.0 && h.range.end <= 60.0), "{found:?}");
    }

    #[test]
    fn a_recording_shorter_than_the_length_is_one_highlight() {
        let signals = Signals { duration: 8.0, loudness: Some(envelope(&[(8.0, -20.0)])), motion: None };
        let found = find_highlights(&signals, &HighlightOptions { count: 3, length: 30.0, within: None });
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].range, Range::new(0.0, 8.0));
    }
}
