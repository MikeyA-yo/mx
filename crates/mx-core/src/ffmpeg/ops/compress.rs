//! Hitting a file-size target ("under 10 MB for WhatsApp").
//!
//! Size = bitrate x duration, so the target fixes the average bitrate. The
//! only question is how to spend it, which is what two-pass encoding solves:
//! pass 1 analyses the whole video (which scenes are complex), pass 2
//! spends the bits where they matter while landing on the average. Constant
//! quality (CRF) can't do this: it fixes quality and lets size float.
//!
//! When the budget is tight we first drop high frame rates to 30 fps, then
//! lower the resolution, because a sharp 720p30 looks better than a smeared
//! 1080p60 at the same bitrate.
//!
//! Two-pass still lands a few percent off on short clips, so a size target
//! is treated as a hard limit: if pass 2 comes out too big, it's re-run at
//! a corrected bitrate, reusing pass 1's analysis.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::Serialize;

use super::{MAP_AV, aac, finish};
use crate::cancel::CancelToken;
use crate::ffmpeg::{Job, Progress, probe::Probe};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CompressCodec {
    H264,
    Hevc,
}

impl CompressCodec {
    /// Rough floor of bits per pixel per frame below which this codec turns
    /// to mush. A rule of thumb for choosing resolution, not a standard.
    fn min_bits_per_pixel(self) -> f64 {
        match self {
            Self::H264 => 0.05,
            Self::Hevc => 0.035,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompressPlan {
    pub target_bytes: u64,
    pub duration_secs: f64,
    pub video_kbps: u32,
    pub audio_kbps: u32,
    /// Downscale so the shorter side is this long ("720p" is 1280x720
    /// landscape, 720x1280 portrait), or `None` to keep the resolution.
    pub short_edge: Option<u32>,
    /// Whether the source is portrait, so `short_edge` is its width.
    pub portrait: bool,
    /// Cap the frame rate, or `None` to keep the source's.
    pub fps: Option<u32>,
    /// Downmix surround to stereo: 5.1 at 64-128 kb/s wastes the budget.
    pub stereo: bool,
}

/// Reserve for container overhead. Encoder overshoot is handled by retrying.
const HEADROOM: f64 = 0.97;
const LADDER: [u32; 8] = [2160, 1440, 1080, 720, 540, 480, 360, 240];
const FPS_CAP: u32 = 30;
const MAX_RETRIES: u32 = 2;

pub fn plan_compression(target_bytes: u64, source: &Probe, codec: CompressCodec) -> Result<CompressPlan> {
    let Some(duration) = source.duration_secs().filter(|d| *d > 0.0) else {
        bail!("couldn't read the input's duration");
    };
    let total_kbps = target_bytes as f64 * 8.0 / 1000.0 / duration * HEADROOM;

    let audio_kbps = match (source.has_audio(), total_kbps) {
        (false, _) => 0,
        (true, t) if t >= 1500.0 => 128,
        (true, t) if t >= 500.0 => 96,
        _ => 64,
    };
    let video_kbps = total_kbps - audio_kbps as f64;
    const MIN_VIDEO_KBPS: f64 = 100.0;
    if video_kbps < MIN_VIDEO_KBPS {
        let needed = (MIN_VIDEO_KBPS + audio_kbps as f64) * 1000.0 / 8.0 * duration / HEADROOM;
        bail!(
            "{:.1} MB is too small for {duration:.0}s of video; it needs at least {:.1} MB (or trim it first)",
            target_bytes as f64 / 1e6,
            needed / 1e6
        );
    }

    let mut short_edge = None;
    let mut fps_cap = None;
    let mut portrait = false;
    if let Some(v) = source.video()
        && let (Some(src_w), Some(src_h)) = (v.width, v.height)
    {
        // The ladder is in "p": the shorter side, whichever way up.
        portrait = src_h > src_w;
        let (src_short, src_long) = (src_w.min(src_h), src_w.max(src_h));
        let src_fps = v.fps().unwrap_or(30.0);
        let fits = |short: u32, fps: f64| {
            let long = src_long as f64 * short as f64 / src_short as f64;
            video_kbps * 1000.0 / (long * short as f64 * fps) >= codec.min_bits_per_pixel()
        };
        let src_h = src_short;
        if !fits(src_h, src_fps) {
            // Halving 60 -> 30 fps halves the pixels per second; try that
            // before giving up resolution.
            let fps = if src_fps > FPS_CAP as f64 + 1.0 {
                fps_cap = Some(FPS_CAP);
                FPS_CAP as f64
            } else {
                src_fps
            };
            if !fits(src_h, fps) {
                // Largest rung below the source that the budget can feed.
                let rung = LADDER
                    .iter()
                    .copied()
                    .filter(|&h| h < src_h)
                    .find(|&h| fits(h, fps))
                    .unwrap_or(LADDER[LADDER.len() - 1]);
                short_edge = Some(rung.min(src_h));
            }
        }
    }

    let max_channels = source.streams.iter().filter_map(|s| s.channels).max().unwrap_or(0);
    Ok(CompressPlan {
        target_bytes,
        duration_secs: duration,
        video_kbps: video_kbps as u32,
        audio_kbps,
        short_edge,
        portrait,
        fps: fps_cap,
        stereo: max_channels > 2,
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompressOutcome {
    pub bytes: u64,
    /// Pass-2 runs, 1 if the first landed under the target.
    pub attempts: u32,
    pub final_video_kbps: u32,
    /// Still over after all retries (only on pathological inputs).
    pub over_target: bool,
}

/// A prepared two-pass compression. Dropping it removes the pass-1 stats
/// files, so they never outlive the operation.
#[derive(Debug)]
pub struct Compress {
    pub plan: CompressPlan,
    input: PathBuf,
    output: PathBuf,
    codec: CompressCodec,
    stats: StatsFiles,
}

#[derive(Debug)]
struct StatsFiles {
    prefix: PathBuf,
}

impl Drop for StatsFiles {
    fn drop(&mut self) {
        // x264 writes <prefix>-0.log(.mbtree), x265 <prefix>(.cutree).
        let (Some(dir), Some(name)) = (self.prefix.parent(), self.prefix.file_name()) else {
            return;
        };
        let name = name.to_string_lossy();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(name.as_ref()) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
}

pub fn compress(input: &Path, output: &Path, plan: CompressPlan, codec: CompressCodec) -> Compress {
    let dir = output.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let prefix = dir.join(format!(".mx-2pass-{}-{nonce}", std::process::id()));
    Compress {
        plan,
        input: input.to_owned(),
        output: output.to_owned(),
        codec,
        stats: StatsFiles { prefix },
    }
}

impl Compress {
    fn video_args(&self, pass: u32, video_kbps: u32) -> Vec<String> {
        let prefix = self.stats.prefix.to_string_lossy();
        let rate = format!("{video_kbps}k");
        let mut args: Vec<String> = match self.codec {
            CompressCodec::H264 => ["-c:v", "libx264", "-b:v", &rate, "-pass", &pass.to_string(), "-passlogfile", &prefix]
                .map(String::from)
                .to_vec(),
            CompressCodec::Hevc => vec![
                "-c:v".into(),
                "libx265".into(),
                "-b:v".into(),
                rate,
                "-x265-params".into(),
                format!("pass={pass}:stats={}:log-level=error", quote_param(&prefix)),
            ],
        };
        args.extend(["-pix_fmt", "yuv420p"].map(String::from));
        let mut filters = Vec::new();
        if let Some(fps) = self.plan.fps {
            filters.push(format!("fps={fps}"));
        }
        if let Some(short) = self.plan.short_edge {
            // -2: the other side, keeping the aspect ratio and even.
            filters.push(match self.plan.portrait {
                true => format!("scale={short}:-2"),
                false => format!("scale=-2:{short}"),
            });
        }
        if !filters.is_empty() {
            args.extend(["-vf".to_string(), filters.join(",")]);
        }
        args
    }

    /// Pass 1: analysis only, video only, output discarded.
    pub fn first_pass(&self) -> Job {
        Job::new("-")
            .input(&self.input)
            .args(["-map", "0:v:0"])
            .args(self.video_args(1, self.plan.video_kbps))
            .args(["-an", "-f", "null"])
            .duration(self.plan.duration_secs)
    }

    /// Pass 2 at `video_kbps`, which differs from the plan on retries.
    pub fn second_pass(&self, video_kbps: u32) -> Job {
        let mut job = Job::new(&self.output)
            .input(&self.input)
            .args(MAP_AV)
            .args(self.video_args(2, video_kbps))
            .duration(self.plan.duration_secs);
        if self.plan.audio_kbps > 0 {
            job = job.args(aac(self.plan.audio_kbps));
            if self.plan.stereo {
                job = job.args(["-ac", "2"]);
            }
        }
        finish(job, &self.output)
    }

    /// Run both passes, re-running pass 2 at a lower bitrate while the
    /// result is over the target. `on_progress` gets a stage label
    /// ("pass 1/2", "pass 2/2", "retry 1") with each update.
    pub fn run(
        &self,
        cancel: &CancelToken,
        overwrite: bool,
        mut on_progress: impl FnMut(&str, &Progress),
    ) -> Result<CompressOutcome> {
        self.first_pass().run(cancel, |p| on_progress("pass 1/2", p))?;

        let target = self.plan.target_bytes;
        let mut video_kbps = self.plan.video_kbps;
        let mut attempts = 0;
        loop {
            attempts += 1;
            let label = if attempts == 1 { "pass 2/2".to_string() } else { format!("retry {}", attempts - 1) };
            // Retries replace our own output from the previous attempt.
            let overwrite = overwrite || attempts > 1;
            self.second_pass(video_kbps).overwrite(overwrite).run(cancel, |p| on_progress(&label, p))?;

            let bytes = std::fs::metadata(&self.output)?.len();
            if bytes <= target || attempts > MAX_RETRIES {
                return Ok(CompressOutcome {
                    bytes,
                    attempts,
                    final_video_kbps: video_kbps,
                    over_target: bytes > target,
                });
            }
            video_kbps = corrected_kbps(video_kbps, self.plan.audio_kbps, bytes, target, self.plan.duration_secs);
        }
    }
}

/// Quote a value inside an ffmpeg `key=value:key=value` option string.
///
/// `:` separates options, so an unquoted Windows path `C:\...` is cut at the
/// drive letter (x265 then writes its stats to a file named `C` in the
/// working directory). ffmpeg reads single-quoted text literally; a quote
/// inside is written as `'\''` (close, escaped quote, reopen).
fn quote_param(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// New video bitrate after an attempt at `video_kbps` produced `bytes`.
/// Overshoot is almost all video (AAC holds its rate), so the excess is
/// taken out of the video budget, plus 1% so we land under, not on, it.
fn corrected_kbps(video_kbps: u32, audio_kbps: u32, bytes: u64, target: u64, duration: f64) -> u32 {
    let to_kbps = |b: u64| b as f64 * 8.0 / 1000.0 / duration;
    let actual_video = to_kbps(bytes) - audio_kbps as f64;
    let allowed_video = to_kbps(target) * 0.99 - audio_kbps as f64;
    if actual_video <= allowed_video {
        return video_kbps;
    }
    let ratio = (allowed_video / actual_video).clamp(0.5, 1.0);
    ((video_kbps as f64 * ratio) as u32).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(json: &str) -> Probe {
        serde_json::from_str(json).unwrap()
    }

    fn landscape(duration: f64, height: u32, fps: u32, channels: u32) -> Probe {
        let width = height * 16 / 9;
        source(&format!(
            r#"{{"format":{{"duration":"{duration}"}},"streams":[
                {{"index":0,"codec_type":"video","width":{width},"height":{height},"avg_frame_rate":"{fps}/1"}},
                {{"index":1,"codec_type":"audio","channels":{channels}}}]}}"#
        ))
    }

    #[test]
    fn generous_budget_keeps_everything() {
        // 100 MB for 60 s = ~12.9 Mbit/s total.
        let plan = plan_compression(100_000_000, &landscape(60.0, 1080, 30, 2), CompressCodec::H264).unwrap();
        assert_eq!((plan.audio_kbps, plan.short_edge, plan.fps, plan.stereo), (128, None, None, false));
    }

    #[test]
    fn tight_budget_drops_fps_before_resolution() {
        // Measured case: 3 MB for a 30 s 720p60 5.1 clip. At 60 fps the
        // budget only affords 240p; at 30 fps it affords 480p.
        let plan = plan_compression(3_000_000, &landscape(30.0, 720, 60, 6), CompressCodec::H264).unwrap();
        assert_eq!(plan.fps, Some(30));
        assert_eq!(plan.short_edge, Some(480));
        assert!(plan.stereo);
    }

    #[test]
    fn portrait_video_steps_down_its_short_side() {
        // The 3 MB / 30 s / 720p60 case above, turned on its side.
        let src = source(
            r#"{"format":{"duration":"30"},"streams":[
                {"index":0,"codec_type":"video","width":720,"height":1280,"avg_frame_rate":"60/1"}]}"#,
        );
        let plan = plan_compression(3_000_000, &src, CompressCodec::H264).unwrap();
        assert_eq!((plan.short_edge, plan.portrait), (Some(480), true));
        let s = crate::ffmpeg::ops::args_of(&compress(Path::new("in.mp4"), Path::new("out.mp4"), plan, CompressCodec::H264).first_pass());
        assert!(s.contains("scale=480:-2"), "{s}");
    }

    #[test]
    fn hevc_affords_more_pixels_for_the_same_budget() {
        let h264 = plan_compression(10_000_000, &landscape(120.0, 1080, 30, 2), CompressCodec::H264).unwrap();
        let hevc = plan_compression(10_000_000, &landscape(120.0, 1080, 30, 2), CompressCodec::Hevc).unwrap();
        assert!(hevc.short_edge.unwrap() > h264.short_edge.unwrap(), "{hevc:?} vs {h264:?}");
    }

    #[test]
    fn impossible_budget_says_how_much_is_needed() {
        let err = plan_compression(1_000_000, &landscape(600.0, 1080, 30, 2), CompressCodec::H264).unwrap_err();
        assert!(err.to_string().contains("needs at least"), "{err}");
    }

    #[test]
    fn correction_takes_the_overshoot_out_of_video() {
        // Measured: aimed at 672k video + 96k audio for 3 MB / 30 s, got 3,029,355 bytes.
        let kbps = corrected_kbps(672, 96, 3_029_355, 3_000_000, 30.0);
        assert!((630..660).contains(&kbps), "{kbps}");
        // Never raises the rate, never more than halves it.
        assert_eq!(corrected_kbps(500, 64, 1_000, 3_000_000, 30.0), 500);
        assert_eq!(corrected_kbps(500, 64, 30_000_000, 3_000_000, 30.0), 250);
    }

    #[test]
    fn x265_stats_path_is_quoted_so_drive_colons_survive() {
        assert_eq!(quote_param(r"C:\out\.mx-2pass-1"), r"'C:\out\.mx-2pass-1'");
        assert_eq!(quote_param("it's"), r"'it'\''s'");

        if cfg!(windows) {
            let plan = plan_compression(3_000_000, &landscape(30.0, 720, 30, 2), CompressCodec::Hevc).unwrap();
            let c = compress(Path::new("in.mp4"), Path::new(r"C:\out\clip.mp4"), plan, CompressCodec::Hevc);
            let p1 = super::super::args_of(&c.first_pass());
            assert!(p1.contains(r"pass=1:stats='C:\out\.mx-2pass-"), "{p1}");
        }
    }

    #[test]
    fn passes_share_stats_and_apply_filters() {
        let plan = plan_compression(3_000_000, &landscape(30.0, 720, 60, 6), CompressCodec::H264).unwrap();
        let c = compress(Path::new("in.mp4"), Path::new("out/clip.mp4"), plan, CompressCodec::H264);
        let prefix = c.stats.prefix.to_string_lossy().into_owned();
        let p1 = super::super::args_of(&c.first_pass());
        let p2 = super::super::args_of(&c.second_pass(600));
        assert!(p1.contains(&format!("-pass 1 -passlogfile {prefix}")) && p1.ends_with("-an -f null -"), "{p1}");
        assert!(p2.contains("-b:v 600k -pass 2") && p2.contains("-vf fps=30,scale=-2:480"), "{p2}");
        assert!(p2.contains("-b:a 96k -ac 2") && p2.ends_with("out/clip.mp4"), "{p2}");
    }
}
