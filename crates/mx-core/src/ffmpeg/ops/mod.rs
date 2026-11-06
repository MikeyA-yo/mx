//! Ready-made ffmpeg jobs. Each function only *builds* jobs; running them
//! (with progress and cancellation) is the caller's choice.
//!
//! Inputs are probed up front so every job knows its output duration, and
//! therefore reports a real percentage even when it changes the length.

mod burn;
mod clip;
mod compress;
mod gif;
mod jumpcut;
mod preview;
mod reframe;
mod remux;
mod render;
mod speed;
mod trim;

pub use burn::burn_subtitles;
pub use clip::{Frame, clip};
pub use compress::{Compress, CompressCodec, CompressOutcome, CompressPlan, compress, plan_compression};
pub use gif::{GifOptions, gif};
pub use jumpcut::jumpcut;
pub use preview::{Filmstrip, PreviewKind, filmstrip, preview};
pub use reframe::{Aspect, Fit, reframe};
pub use remux::{Remux, SubtitlePlan, remux, remux_with};
pub use render::{Edit, Render, render, render_project};
pub use speed::speed;
pub use trim::{Cut, Trim, trim};

use std::path::Path;

use anyhow::{Context, Result};

use super::Job;
use super::encoders::Encoder;

/// Video settings for any operation that re-encodes.
#[derive(Debug, Clone, Copy)]
pub struct VideoEncode {
    pub encoder: Encoder,
    /// Encoder-specific constant quality; `None` = the encoder's default.
    pub quality: Option<u32>,
}

impl VideoEncode {
    fn args(&self) -> Vec<String> {
        self.encoder.encode_args(self.quality)
    }
}

fn aac(kbps: u32) -> Vec<String> {
    ["-c:a", "aac", "-b:a", &format!("{kbps}k")].map(String::from).to_vec()
}

/// Audio codec for an audio-only output, by its extension.
fn audio_codec(output: &Path) -> Vec<String> {
    match output.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("mp3") => ["-c:a", "libmp3lame", "-q:a", "2"].map(String::from).to_vec(),
        // Lossless containers pick their own lossless codec.
        Some("wav" | "flac") => Vec::new(),
        _ => aac(192),
    }
}

/// Best video stream plus audio if there is any. Subtitle and data streams
/// are left out: most containers can't take every subtitle format, and
/// captions get their own stage.
const MAP_AV: [&str; 4] = ["-map", "0:v:0?", "-map", "0:a?"];

fn input_duration(input: &Path) -> Result<f64> {
    super::probe(input)?
        .duration_secs()
        .with_context(|| format!("couldn't read the duration of {}", input.display()))
}

/// Container-specific finishing flags.
fn finish(job: Job, output: &Path) -> Job {
    let mp4_family = output
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["mp4", "m4v", "mov"].contains(&e.to_ascii_lowercase().as_str()));
    if mp4_family {
        // Move the index to the front so playback can start before the
        // whole file has downloaded.
        job.args(["-movflags", "+faststart"])
    } else {
        job
    }
}

/// Re-encode video with `video.encoder` and audio to AAC.
pub fn transcode(input: &Path, output: &Path, video: VideoEncode) -> Job {
    let job = Job::new(output)
        .input(input)
        .args(MAP_AV)
        .args(video.args())
        .args(aac(192));
    finish(job, output)
}

#[cfg(test)]
pub(crate) fn args_of(job: &Job) -> String {
    job.command_line()
        .into_iter()
        .map(|a| a.into_string().unwrap())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(name: &str, quality: Option<u32>) -> VideoEncode {
        VideoEncode { encoder: Encoder::lookup(name).unwrap(), quality }
    }

    #[test]
    fn transcode_uses_encoder_defaults_and_faststart_for_mp4() {
        let s = args_of(&transcode(Path::new("in.mkv"), Path::new("out.MP4"), video("libx264", None)));
        assert!(s.contains("-c:v libx264 -crf 23 -pix_fmt yuv420p"), "{s}");
        assert!(s.contains("-movflags +faststart"), "{s}");
        assert!(s.ends_with("out.MP4"));
    }

    #[test]
    fn transcode_to_mkv_skips_faststart() {
        let s = args_of(&transcode(Path::new("in.mp4"), Path::new("out.mkv"), video("h264_qsv", Some(20))));
        assert!(s.contains("-c:v h264_qsv -global_quality 20 -c:a aac"), "{s}");
        assert!(!s.contains("faststart"));
    }
}
