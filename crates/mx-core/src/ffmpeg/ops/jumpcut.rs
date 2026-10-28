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

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::{VideoEncode, aac, audio_codec, finish};
use crate::ffmpeg::{Job, probe};
use crate::timeline::{Range, total};

/// A rendered-to-be jump cut. Holds the filter script file, which is
/// deleted when this is dropped.
#[derive(Debug)]
pub struct JumpCut {
    pub job: Job,
    /// The ranges actually kept, after snapping to video frames.
    pub kept: Vec<Range>,
    /// Held only so the file is deleted when the job is done with.
    _script: ScriptFile,
}

impl JumpCut {
    pub fn output_duration(&self) -> f64 {
        total(&self.kept)
    }
}

#[derive(Debug)]
struct ScriptFile(PathBuf);

impl Drop for ScriptFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Parse "30000/1001" or "30" into frames per second.
fn parse_rate(rate: &str) -> Option<f64> {
    let fps = match rate.split_once('/') {
        Some((n, d)) => n.parse::<f64>().ok()? / d.parse::<f64>().ok()?,
        None => rate.parse().ok()?,
    };
    (fps.is_finite() && fps > 0.0).then_some(fps)
}

fn snap(ranges: &[Range], fps: f64) -> Vec<Range> {
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
fn select_expr(ranges: &[Range], shift: f64) -> String {
    ranges
        .iter()
        .map(|r| format!("gte(t,{:.6})*lt(t,{:.6})", r.start - shift, r.end - shift))
        .collect::<Vec<_>>()
        .join("+")
}

/// `asplit` into one branch per range, `atrim` each, `concat` them back.
/// Ranges are in time order, so each branch is consumed as the audio is
/// decoded and nothing piles up in memory.
fn audio_chain(ranges: &[Range]) -> String {
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

fn filter_script(ranges: &[Range], video_rate: Option<(&str, f64)>) -> String {
    let audio = audio_chain(ranges);
    match video_rate {
        Some((rate, fps)) => format!(
            "[0:v:0]fps={rate},select='{}',setpts=N/FRAME_RATE/TB[v];\n{audio}",
            select_expr(ranges, 0.5 / fps)
        ),
        None => audio,
    }
}

/// Keep only `keep` from `input`. Works for video and for audio-only files
/// (podcasts); `video` is required only when the input has video.
pub fn jumpcut(input: &Path, output: &Path, keep: &[Range], video: Option<VideoEncode>) -> Result<JumpCut> {
    let source = probe(input)?;
    if !source.has_audio() {
        bail!("jump cuts are driven by audio, and {} has none", input.display());
    }
    let rate = source.video().and_then(|v| v.avg_frame_rate.clone());
    let video_rate = rate.as_deref().and_then(|r| Some((r, parse_rate(r)?)));

    let kept = match video_rate {
        Some((_, fps)) => snap(keep, fps),
        None => keep.to_vec(),
    };
    if kept.is_empty() {
        bail!("nothing left to keep");
    }

    // Hundreds of ranges overflow Windows' 32k command-line limit, so the
    // graph goes in a file, loaded with ffmpeg 7+'s `-/option file` syntax.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let script = ScriptFile(std::env::temp_dir().join(format!("mx-jumpcut-{}-{nonce}.txt", std::process::id())));
    std::fs::write(&script.0, filter_script(&kept, video_rate))
        .with_context(|| format!("couldn't write {}", script.0.display()))?;

    let mut job = Job::new(output)
        .input(input)
        .arg("-/filter_complex")
        .arg(&script.0)
        .duration(total(&kept));
    job = match (video_rate, video) {
        (Some(_), Some(video)) => job.args(["-map", "[v]", "-map", "[a]"]).args(video.args()).args(aac(192)),
        (Some(_), None) => bail!("{} has video, so a video encoder is needed", input.display()),
        (None, _) => job.args(["-map", "[a]"]).args(audio_codec(output)),
    };
    Ok(JumpCut { job: finish(job, output), kept, _script: script })
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

    #[test]
    fn script_shifts_video_boundaries_by_half_a_frame() {
        let s = filter_script(&[Range::new(1.0, 2.0), Range::new(3.0, 4.0)], Some(("25/1", 25.0)));
        assert!(s.contains("[0:v:0]fps=25/1,select='gte(t,0.980000)*lt(t,1.980000)+gte(t,2.980000)*lt(t,3.980000)'"), "{s}");
        assert!(s.contains("[0:a:0]asplit=2[as0][as1];"), "{s}");
        assert!(s.contains("[as0]atrim=start=1.000000:end=2.000000,asetpts=PTS-STARTPTS[at0];"), "{s}");
        assert!(s.contains("[as1]atrim=start=3.000000:end=4.000000,asetpts=PTS-STARTPTS[at1];"), "{s}");
        assert!(s.contains("[at0][at1]concat=n=2:v=0:a=1[a]"), "{s}");
    }

    #[test]
    fn audio_only_script_has_no_video_chain() {
        let s = filter_script(&[Range::new(0.0, 1.0)], None);
        assert!(!s.contains("0:v"), "{s}");
    }
}
