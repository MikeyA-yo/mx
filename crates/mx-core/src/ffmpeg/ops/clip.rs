//! A self-contained clip: one time range, re-encoded, optionally in a new
//! frame shape. This is what highlights are exported with: trim and
//! reframe in a single encode instead of two.

use std::path::Path;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use super::reframe::{Aspect, Fit, filtergraph};
use super::{VideoEncode, aac, audio_codec, finish};
use crate::ffmpeg::{Job, probe};
use crate::timeline::Range;

/// A target frame shape (see [`super::reframe`]).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Frame {
    pub aspect: Aspect,
    pub fit: Fit,
    pub short_edge: u32,
}

/// Audio fades in and out over this long, so a clip doesn't start or stop
/// with a click.
const FADE: f64 = 0.05;

/// Cut `range` from `input`. `video` is required when the input has video.
pub fn clip(input: &Path, output: &Path, range: Range, frame: Option<Frame>, video: Option<VideoEncode>) -> Result<Job> {
    let source = probe(input)?;
    let len = range.len();
    if len <= 0.0 {
        bail!("empty clip range {:.3}..{:.3}", range.start, range.end);
    }

    // `-ss` before `-i` seeks the input, and timestamps then start at 0,
    // which is what the fade-out time is relative to.
    let mut job = Job::new(output)
        .args(["-ss", &format!("{:.6}", range.start)])
        .input(input)
        .args(["-t", &format!("{len:.6}")])
        .duration(len);
    let mut graph = Vec::new();
    let mut maps = Vec::new();

    if source.video().is_some() {
        let Some(video) = video else {
            bail!("{} has video, so a video encoder is needed", input.display());
        };
        match frame {
            Some(f) => {
                let (w, h) = f.aspect.frame(f.short_edge);
                graph.push(filtergraph("0:v:0", "v", f.fit, w, h));
                maps.push("[v]");
            }
            None => maps.push("0:v:0"),
        }
        job = job.args(video.args());
    }
    if source.has_audio() {
        let fade_out = (len - FADE).max(0.0);
        graph.push(format!("[0:a:0]afade=t=in:d={FADE},afade=t=out:st={fade_out:.6}:d={FADE}[a]"));
        maps.push("[a]");
        job = job.args(if source.video().is_some() { aac(192) } else { audio_codec(output) });
    }
    if !graph.is_empty() {
        job = job.args(["-filter_complex", &graph.join(";")]);
    }
    for map in maps {
        job = job.args(["-map", map]);
    }
    Ok(finish(job, output))
}
