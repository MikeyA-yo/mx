//! Finding pauses in a loudness envelope.
//!
//! A fixed threshold like -30 dB fails on real recordings: in a noisy room
//! the background never drops that low, so nothing counts as silence (true
//! of "Me at the zoo"). The automatic threshold adapts instead. It
//! estimates the noise floor (a quiet percentile) and the speech level (a
//! loud percentile), and puts the line a third of the way up from the floor.

use serde::Serialize;

use super::loudness::{Envelope, SILENCE_DB, WINDOW_SECS};
use crate::timeline::Range;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Threshold {
    Auto,
    Db(f64),
}

#[derive(Debug, Clone, Copy)]
pub struct SilenceOptions {
    pub threshold: Threshold,
    /// Pauses shorter than this are left alone (gaps between words).
    pub min_duration: f64,
}

impl Default for SilenceOptions {
    fn default() -> Self {
        Self { threshold: Threshold::Auto, min_duration: 0.4 }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SilenceReport {
    pub noise_floor_db: f64,
    pub speech_db: f64,
    pub threshold_db: f64,
    pub silences: Vec<Range>,
}

/// Where between noise floor and speech the silence line sits.
const AUTO_POSITION: f64 = 0.35;
/// Below this, a window is digital silence; leaving those out of the
/// noise-floor estimate stops a silent intro from dragging it to -120 dB.
const DIGITAL_SILENCE_DB: f64 = -90.0;
/// Median filter width in windows (5 x 20 ms = 100 ms): removes one-off
/// clicks inside pauses and brief dips inside words.
const SMOOTHING: usize = 5;

pub(super) fn percentile(sorted: &[f64], q: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * q).round() as usize]
}

fn median_filter(values: &[f64], width: usize) -> Vec<f64> {
    let half = width / 2;
    (0..values.len())
        .map(|i| {
            let lo = i.saturating_sub(half);
            let hi = (i + half + 1).min(values.len());
            let mut window = values[lo..hi].to_vec();
            window.sort_by(f64::total_cmp);
            window[window.len() / 2]
        })
        .collect()
}

pub fn find_silences(env: &Envelope, opts: &SilenceOptions) -> SilenceReport {
    let mut audible: Vec<f64> = env.levels().filter(|&db| db > DIGITAL_SILENCE_DB).collect();
    audible.sort_by(f64::total_cmp);
    let (noise_floor_db, speech_db) = if audible.is_empty() {
        (SILENCE_DB, SILENCE_DB)
    } else {
        (percentile(&audible, 0.10), percentile(&audible, 0.90))
    };
    let threshold_db = match opts.threshold {
        Threshold::Db(db) => db,
        Threshold::Auto => noise_floor_db + AUTO_POSITION * (speech_db - noise_floor_db),
    };

    let levels: Vec<f64> = env.levels().collect();
    let smoothed = median_filter(&levels, SMOOTHING);

    let mut silences = Vec::new();
    let mut run_start: Option<f64> = None;
    for (i, &db) in smoothed.iter().enumerate() {
        let t = env.points[i].0;
        match (db < threshold_db, run_start) {
            (true, None) => run_start = Some(t),
            (false, Some(start)) => {
                silences.push(Range::new(start, t));
                run_start = None;
            }
            _ => {}
        }
    }
    if let (Some(start), Some(&(last, _))) = (run_start, env.points.last()) {
        silences.push(Range::new(start, last + WINDOW_SECS));
    }
    silences.retain(|s| s.len() >= opts.min_duration);

    SilenceReport { noise_floor_db, speech_db, threshold_db, silences }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an envelope from (seconds, dB) segments at 20 ms resolution.
    fn envelope(segments: &[(f64, f64)]) -> Envelope {
        let mut points = Vec::new();
        let mut t = 0.0;
        for &(secs, db) in segments {
            let n = (secs / WINDOW_SECS).round() as usize;
            for _ in 0..n {
                points.push((t, db));
                t += WINDOW_SECS;
            }
        }
        Envelope { points }
    }

    fn spans(report: &SilenceReport) -> Vec<(f64, f64)> {
        report
            .silences
            .iter()
            .map(|r| ((r.start * 100.0).round() / 100.0, (r.end * 100.0).round() / 100.0))
            .collect()
    }

    #[test]
    fn auto_threshold_finds_pauses_above_a_noisy_floor() {
        // Room noise at -30 dB never drops below a fixed -35 dB line, but
        // the auto threshold sits between noise and speech.
        let env = envelope(&[(2.0, -18.0), (1.0, -30.0), (3.0, -18.0), (0.2, -30.0), (2.0, -18.0)]);
        let fixed = find_silences(&env, &SilenceOptions { threshold: Threshold::Db(-35.0), min_duration: 0.4 });
        assert!(fixed.silences.is_empty());

        let auto = find_silences(&env, &SilenceOptions::default());
        assert!((auto.noise_floor_db - -30.0).abs() < 1e-9 && (auto.speech_db - -18.0).abs() < 1e-9);
        assert!((auto.threshold_db - -25.8).abs() < 1e-9);
        // The 1 s pause is found; the 0.2 s gap between words is not.
        assert_eq!(spans(&auto), [(2.0, 3.0)]);
    }

    #[test]
    fn single_window_blips_do_not_split_a_pause() {
        let env = envelope(&[(1.0, -15.0), (0.5, -40.0), (0.02, -10.0), (0.5, -40.0), (1.0, -15.0)]);
        let report = find_silences(&env, &SilenceOptions::default());
        assert_eq!(spans(&report), [(1.0, 2.02)]);
    }

    #[test]
    fn digital_silence_does_not_drag_the_noise_floor_down() {
        let env = envelope(&[(3.0, SILENCE_DB), (2.0, -18.0), (1.0, -32.0), (2.0, -18.0)]);
        let report = find_silences(&env, &SilenceOptions::default());
        assert!(report.noise_floor_db > -40.0, "{report:?}");
        assert_eq!(spans(&report), [(0.0, 3.0), (5.0, 6.0)]);
    }

    #[test]
    fn trailing_silence_runs_to_the_end() {
        let env = envelope(&[(1.0, -15.0), (1.0, -50.0)]);
        let report = find_silences(&env, &SilenceOptions::default());
        assert_eq!(spans(&report), [(1.0, 2.0)]);
    }
}
