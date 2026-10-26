//! Cutting a time range out of a file.
//!
//! Compressed video is a chain of keyframes (complete pictures) followed by
//! frames that only store differences. Copying packets without re-encoding
//! can therefore only *start* on a keyframe, so there are two kinds of cut:
//!
//! - [`Cut::Fast`]: stream copy. Instant and lossless, but the clip begins
//!   at the last keyframe at or before the requested start, which can be
//!   seconds earlier. We look that keyframe up and cut from it explicitly,
//!   so the caller learns the real start instead of being surprised.
//! - [`Cut::Exact`]: re-encode. ffmpeg decodes from the previous keyframe
//!   and discards frames until the exact requested time. Frame-accurate,
//!   costs an encode.

use std::path::Path;

use anyhow::{Result, bail};

use super::{MAP_AV, VideoEncode, aac, finish, input_duration};
use crate::ffmpeg::{Job, probe};

#[derive(Debug, Clone, Copy)]
pub enum Cut {
    Fast,
    Exact(VideoEncode),
}

#[derive(Debug, Clone)]
pub struct Trim {
    pub job: Job,
    /// Where the output really begins in the source timeline. Equal to the
    /// requested start for exact cuts, possibly earlier for fast ones.
    pub actual_start: f64,
    pub duration: f64,
}

/// Cut `[start, end)` from `input`. `end = None` means "to the end".
pub fn trim(input: &Path, output: &Path, start: f64, end: Option<f64>, cut: Cut) -> Result<Trim> {
    let total = input_duration(input)?;
    let end = end.unwrap_or(total).min(total);
    if start >= end {
        bail!("start ({start:.3}s) must be before end ({end:.3}s; input is {total:.3}s long)");
    }

    let actual_start = match cut {
        Cut::Fast => probe::keyframe_at_or_before(input, start)?.unwrap_or(0.0),
        Cut::Exact(_) => start,
    };
    let duration = end - actual_start;

    // `-ss` before `-i` seeks the input (fast); `-t` after it limits the output.
    let job = Job::new(output)
        .args(["-ss", &format!("{actual_start:.6}")])
        .input(input)
        .args(["-t", &format!("{duration:.6}")])
        .args(MAP_AV)
        .duration(duration);
    let job = match cut {
        // Timestamps of a copied clip don't start at zero; shift them so
        // players don't show a blank lead-in.
        Cut::Fast => job.args(["-c", "copy", "-avoid_negative_ts", "make_zero"]),
        Cut::Exact(video) => job.args(video.args()).args(aac(192)),
    };
    Ok(Trim { job: finish(job, output), actual_start, duration })
}
