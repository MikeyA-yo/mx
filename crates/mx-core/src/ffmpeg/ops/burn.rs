//! Drawing subtitles into the picture ("burn-in"), for platforms and
//! players that don't show subtitle tracks, and for styled captions.

use std::path::Path;

use anyhow::Result;

use super::{VideoEncode, finish};
use crate::ffmpeg::{Job, filter_path};

/// Draw `subtitles` (ASS, SRT or WebVTT; ASS keeps its styling) onto the
/// video. Audio is copied untouched.
pub fn burn_subtitles(input: &Path, output: &Path, subtitles: &Path, video: VideoEncode) -> Result<Job> {
    let job = Job::new(output)
        .input(input)
        .args(["-vf", &format!("subtitles=filename={}", filter_path(subtitles)?)])
        .args(["-map", "0:v:0", "-map", "0:a?"])
        .args(video.args())
        .args(["-c:a", "copy"]);
    Ok(finish(job, output))
}
