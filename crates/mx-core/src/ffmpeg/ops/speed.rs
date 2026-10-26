//! Speeding up or slowing down, keeping audio pitch natural.

use std::path::Path;

use anyhow::{Context, Result, bail};

use super::{MAP_AV, VideoEncode, aac, finish};
use crate::ffmpeg::{Job, probe};

/// ffmpeg's `atempo` only accepts 0.5..=100 per instance, so slower speeds
/// are built by chaining: 0.25 = 0.5 x 0.5.
fn atempo_chain(factor: f64) -> String {
    let mut remaining = factor;
    let mut stages = Vec::new();
    while remaining < 0.5 {
        stages.push("atempo=0.5".to_string());
        remaining /= 0.5;
    }
    stages.push(format!("atempo={remaining}"));
    stages.join(",")
}

/// Play `input` at `factor` x speed (2.0 = twice as fast, 0.5 = half).
///
/// Video timestamps are rescaled, then the `fps` filter drops or repeats
/// frames to get back to the source frame rate. Without it, 0.25x of a
/// 60 fps clip comes out as 15 fps. Slow motion therefore repeats frames
/// rather than inventing new ones (that's what interpolation, e.g. RIFE, is for).
pub fn speed(input: &Path, output: &Path, factor: f64, video: VideoEncode) -> Result<Job> {
    if !(0.1..=10.0).contains(&factor) {
        bail!("speed must be between 0.1x and 10x");
    }
    let source = probe(input)?;
    let duration = source
        .duration_secs()
        .with_context(|| format!("couldn't read the duration of {}", input.display()))?
        / factor;
    let keep_rate = source
        .video()
        .and_then(|v| v.avg_frame_rate.clone())
        .map(|rate| format!(",fps={rate}"))
        .unwrap_or_default();
    let job = Job::new(output)
        .input(input)
        .args(MAP_AV)
        .args(["-filter:v", &format!("setpts=PTS/{factor}{keep_rate}")])
        .args(["-filter:a", &atempo_chain(factor)])
        .args(video.args())
        .args(aac(192))
        .duration(duration);
    Ok(finish(job, output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chains_atempo_below_half_speed() {
        assert_eq!(atempo_chain(2.0), "atempo=2");
        assert_eq!(atempo_chain(0.5), "atempo=0.5");
        assert_eq!(atempo_chain(0.25), "atempo=0.5,atempo=0.5");
        assert_eq!(atempo_chain(0.3), "atempo=0.5,atempo=0.6");
    }
}
