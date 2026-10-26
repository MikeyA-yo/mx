//! Video to GIF that doesn't look like a 1998 webpage.
//!
//! GIF frames hold at most 256 colours. ffmpeg's default is a fixed generic
//! palette, which bands badly. Instead we let `palettegen` pick the 256
//! colours this particular clip uses most, then `paletteuse` maps every
//! frame onto them with light dithering.

use std::path::Path;

use anyhow::{Result, bail};

use super::input_duration;
use crate::ffmpeg::Job;

#[derive(Debug, Clone, Copy)]
pub struct GifOptions {
    pub fps: u32,
    /// Output width in pixels; never upscales. Height keeps the aspect ratio.
    pub width: u32,
    pub start: f64,
    pub end: Option<f64>,
}

impl Default for GifOptions {
    fn default() -> Self {
        Self { fps: 15, width: 480, start: 0.0, end: None }
    }
}

pub fn gif(input: &Path, output: &Path, opts: GifOptions) -> Result<Job> {
    let total = input_duration(input)?;
    let end = opts.end.unwrap_or(total).min(total);
    if opts.start >= end {
        bail!("start must be before end");
    }
    let duration = end - opts.start;
    let GifOptions { fps, width, .. } = opts;
    let graph = format!(
        "[0:v:0]fps={fps},scale='min({width},iw)':-1:flags=lanczos,split[a][b];\
         [a]palettegen=stats_mode=diff[p];\
         [b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle"
    );
    Ok(Job::new(output)
        .args(["-ss", &format!("{:.6}", opts.start)])
        .input(input)
        .args(["-t", &format!("{duration:.6}")])
        .args(["-filter_complex", &graph])
        .args(["-loop", "0"])
        .duration(duration))
}
