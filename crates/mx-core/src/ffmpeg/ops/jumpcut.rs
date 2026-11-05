//! Rendering a list of kept ranges as one continuous file: jump cuts.
//!
//! Video: one `select` pass keeps the frames inside the ranges and
//! `setpts` closes the gaps. Audio: an `atrim` per range, joined by
//! `concat`. (`aselect` would mirror the video side, but in ffmpeg 8.0.1
//! it passes every frame whatever the expression, even `aselect=0`.)
//!
//! The care goes into keeping audio and video in sync:
//! - Video becomes constant frame rate first (`fps`), so frame N sits at
//!   exactly N/fps. Phones record variable frame rate, which would make
//!   `N/FRAME_RATE` timestamps drift.
//! - Every cut point is snapped to that frame grid. Otherwise each cut
//!   keeps up to half a frame more or less video than audio, and over a
//!   few hundred cuts the errors add up to visible lip-sync drift.
//! - `atrim` cuts audio at the exact sample, so it matches the snapped
//!   video cut.
//!
//! Cuts land inside pauses, so both sides of each join are quiet and the
//! audio needs no crossfade to avoid clicks.

use std::path::Path;

use anyhow::{Result, bail};

use super::render::{Edit, Render, render};
use super::VideoEncode;
use crate::ffmpeg::probe;
use crate::timeline::Range;

/// Parse "30000/1001" or "30" into frames per second.
pub(super) fn parse_rate(rate: &str) -> Option<f64> {
    let fps = match rate.split_once('/') {
        Some((n, d)) => n.parse::<f64>().ok()? / d.parse::<f64>().ok()?,
        None => rate.parse().ok()?,
    };
    (fps.is_finite() && fps > 0.0).then_some(fps)
}

pub(super) fn snap(ranges: &[Range], fps: f64) -> Vec<Range> {
    let grid = |t: f64| (t * fps).round() / fps;
    ranges
        .iter()
        .map(|r| Range::new(grid(r.start), grid(r.end)))
        .filter(|r| !r.is_empty())
        .collect()
}

/// `gte(t,a)*lt(t,b)+...`: true inside any range. `shift` moves every
/// boundary half a frame early, so a frame exactly on a boundary is
/// decided by rounding, not float noise.
pub(super) fn select_expr(ranges: &[Range], shift: f64) -> String {
    ranges
        .iter()
        .map(|r| format!("gte(t,{:.6})*lt(t,{:.6})", r.start - shift, r.end - shift))
        .collect::<Vec<_>>()
        .join("+")
}

/// `asplit` into one branch per range, `atrim` each, `concat` them back.
/// Ranges are in time order, so each branch is consumed as the audio is
/// decoded and nothing piles up in memory.
pub(super) fn audio_chain(ranges: &[Range]) -> String {
    let n = ranges.len();
    let labels: String = (0..n).map(|i| format!("[as{i}]")).collect();
    let mut chain = format!("[0:a:0]asplit={n}{labels};
");
    for (i, r) in ranges.iter().enumerate() {
        chain += &format!("[as{i}]atrim=start={:.6}:end={:.6},asetpts=PTS-STARTPTS[at{i}];
", r.start, r.end);
    }
    let trimmed: String = (0..n).map(|i| format!("[at{i}]")).collect();
    chain + &format!("{trimmed}concat=n={n}:v=0:a=1[a]")
}

/// Keep only `keep` from `input`. Works for video and for audio-only files
/// (podcasts); `video` is required only when the input has video.
pub fn jumpcut(input: &Path, output: &Path, keep: &[Range], video: Option<VideoEncode>) -> Result<Render> {
    if !probe(input)?.has_audio() {
        bail!("jump cuts are driven by audio, and {} has none", input.display());
    }
    render(input, output, &Edit { keep: keep.to_vec(), ..Edit::default() }, video)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frame_rates() {
        assert_eq!(parse_rate("30/1"), Some(30.0));
        assert!((parse_rate("30000/1001").unwrap() - 29.97).abs() < 0.01);
        assert_eq!(parse_rate("25"), Some(25.0));
        assert_eq!(parse_rate("0/0"), None);
    }

    #[test]
    fn snapping_puts_every_cut_on_the_frame_grid() {
        let snapped = snap(&[Range::new(0.51, 1.234), Range::new(2.0, 2.01)], 25.0);
        // 2.0..2.01 is shorter than half a frame and vanishes.
        assert_eq!(snapped, [Range::new(0.52, 1.24)]);
        for r in &snapped {
            for t in [r.start, r.end] {
                assert!(((t * 25.0) - (t * 25.0).round()).abs() < 1e-9);
            }
        }
    }
}
