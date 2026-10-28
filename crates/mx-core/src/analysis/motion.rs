//! Motion and scene changes over time.
//!
//! `scdet` compares each frame with the one before. It reports `mafd` (mean
//! absolute frame difference: how much the picture moved) and `score` (how
//! abruptly that changed: a hard cut scores high, steady motion low).
//!
//! Frames are shrunk to 160 px wide first; that's enough to judge motion.
//! Most of the time then goes into decoding, so the decoder skips its
//! deblocking filter as well (`-skip_loop_filter all`), which barely changes
//! the picture. Together these halved analysis time on 720p60 (107 s to
//! 50 s for 10.5 minutes; decoding alone takes 44 s). Hardware decoding was
//! slower on an Intel iGPU, because every frame has to come back to the CPU.

use std::path::Path;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::cancel::CancelToken;
use crate::ffmpeg::{Job, Progress, probe};

/// A frame scoring at least this much is a cut. 10 is `scdet`'s own
/// default; its documentation suggests 8 to 14.
pub const SCENE_CUT_SCORE: f64 = 10.0;

/// Two cuts closer than this are one cut (a flash frame scores twice).
const MIN_CUT_GAP: f64 = 0.25;

const FILTER: &str = "scale=160:-2:flags=fast_bilinear,scdet=threshold=100,\
    metadata=mode=print:file='pipe\\:1':direct=1";

const MAFD_KEY: &str = "lavfi.scd.mafd=";
const SCORE_KEY: &str = "lavfi.scd.score=";

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MotionPoint {
    pub time: f64,
    /// 0..100: how much the picture differs from the previous frame.
    pub mafd: f64,
    /// 0..100: how abruptly that difference changed; high on cuts.
    pub score: f64,
}

impl MotionPoint {
    pub fn is_cut(&self) -> bool {
        self.score >= SCENE_CUT_SCORE
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MotionTrack {
    /// One point per frame, in time order.
    pub points: Vec<MotionPoint>,
}

impl MotionTrack {
    /// Times of hard cuts.
    pub fn scene_changes(&self) -> Vec<f64> {
        let mut cuts: Vec<f64> = Vec::new();
        for p in self.points.iter().filter(|p| p.is_cut()) {
            if cuts.last().is_none_or(|&last| p.time - last >= MIN_CUT_GAP) {
                cuts.push(p.time);
            }
        }
        cuts
    }
}

/// Parse `metadata` output: a `frame:N pts:P pts_time:T` line, then one
/// line per `lavfi.scd.*` key for that frame.
fn parse_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> MotionTrack {
    let mut points = Vec::new();
    let mut current: Option<MotionPoint> = None;
    let mut flush = |p: Option<MotionPoint>| points.extend(p);
    for line in lines {
        if let Some(rest) = line.strip_prefix("frame:") {
            flush(current.take());
            current = rest
                .split_whitespace()
                .find_map(|field| field.strip_prefix("pts_time:"))
                .and_then(|t| t.parse::<f64>().ok())
                .map(|time| MotionPoint { time, mafd: 0.0, score: 0.0 });
        } else if let Some(p) = current.as_mut() {
            let value = |v: &str| v.parse::<f64>().unwrap_or(0.0);
            if let Some(v) = line.strip_prefix(MAFD_KEY) {
                p.mafd = value(v);
            } else if let Some(v) = line.strip_prefix(SCORE_KEY) {
                p.score = value(v);
            }
        }
    }
    flush(current);
    MotionTrack { points }
}

/// Measure the first video track of `input`.
pub fn measure_motion(
    input: &Path,
    cancel: &CancelToken,
    on_progress: impl FnMut(&Progress),
) -> Result<MotionTrack> {
    if probe(input)?.video().is_none() {
        bail!("{} has no video track to analyse", input.display());
    }
    let mut lines = Vec::new();
    Job::new("-")
        .args(["-skip_loop_filter", "all"])
        .input(input)
        .args(["-map", "0:v:0", "-vf", FILTER, "-f", "null"])
        .run_with_lines(cancel, on_progress, |line| {
            if line.starts_with("frame:") || line.starts_with("lavfi.scd.") {
                lines.push(line.to_string());
            }
        })?;
    Ok(parse_lines(lines.iter().map(String::as_str)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_metadata_lines_and_ignores_progress() {
        // Captured from ffmpeg 8.0.1 (the time key only appears on cuts).
        let out = "frame:0    pts:0       pts_time:0\n\
                   lavfi.scd.mafd=0.000\n\
                   lavfi.scd.score=0.000\n\
                   out_time_us=16667\n\
                   frame:1    pts:256     pts_time:0.0166667\n\
                   lavfi.scd.mafd=7.977\n\
                   lavfi.scd.score=40.5\n\
                   lavfi.scd.time=0.0166667\n\
                   progress=continue\n\
                   frame:2    pts:512     pts_time:0.0333333\n\
                   lavfi.scd.mafd=9.088\n\
                   lavfi.scd.score=1.111\n";
        let track = parse_lines(out.lines());
        assert_eq!(track.points.len(), 3);
        assert_eq!(track.points[1], MotionPoint { time: 0.0166667, mafd: 7.977, score: 40.5 });
        assert_eq!(track.points[2].mafd, 9.088);
        assert_eq!(track.scene_changes(), [0.0166667]);
    }

    #[test]
    fn a_flash_frame_is_one_cut() {
        let p = |time, score| MotionPoint { time, mafd: 5.0, score };
        let track = MotionTrack { points: vec![p(1.0, 30.0), p(1.04, 25.0), p(2.0, 1.0), p(3.0, 12.0)] };
        assert_eq!(track.scene_changes(), [1.0, 3.0]);
    }
}
