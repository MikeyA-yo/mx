//! Rendering: the edit as one file, size-targeted copies and GIFs.

use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Result, anyhow, bail};
use mx_core::ffmpeg::encoders::{self, Codec, Encoder};
use mx_core::ffmpeg::ops::{self, CompressCodec, Edit, Frame, GifOptions, VideoEncode};
use mx_core::ffmpeg::probe;
use mx_core::speech::{self, CaptionStyle, Transcript};
use mx_core::temp::TempFile;
use mx_core::timeline::Range;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::media::file_name;
use crate::tasks::{self, blocking};

/// Encoder detection test-encodes with each one (about a second), and the
/// answer can't change while the app runs.
pub fn working_encoders() -> &'static [Encoder] {
    static WORKING: OnceLock<Vec<Encoder>> = OnceLock::new();
    WORKING.get_or_init(encoders::detect)
}

fn choose(codec: &str, gpu: bool, height: Option<u32>, quality: Option<u32>) -> Result<VideoEncode> {
    let codec = match codec {
        "hevc" => Codec::Hevc,
        "av1" => Codec::Av1,
        _ => Codec::H264,
    };
    let encoder = encoders::pick(working_encoders(), codec, gpu, height)
        .ok_or_else(|| anyhow!("no working {codec:?} encoder on this machine"))?;
    Ok(VideoEncode { encoder, quality })
}

#[derive(Debug, Deserialize)]
pub struct Captions {
    /// Timed on the source; retimed to the output here.
    transcript: Transcript,
    style: CaptionStyle,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    input: String,
    output: String,
    /// Source ranges to keep; empty keeps everything.
    #[serde(default)]
    keep: Vec<Range>,
    frame: Option<Frame>,
    captions: Option<Captions>,
    codec: String,
    quality: Option<u32>,
    gpu: bool,
    /// Two-pass the result to fit under this many megabytes.
    target_mb: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    output: String,
    size: u64,
    duration: f64,
    encoder: Option<&'static str>,
}

#[tauri::command]
pub async fn export(app: AppHandle, task_id: String, req: ExportRequest) -> Result<Exported, String> {
    let output = PathBuf::from(&req.output);
    let title = format!("Exporting {}", file_name(&output));
    tasks::run(app, task_id, "export", title, move |task| {
        let input = PathBuf::from(&req.input);
        task.progress("preparing", None, None);
        let source = probe(&input)?;
        let frame_size = match (req.frame, source.video()) {
            (Some(f), _) => Some(f.aspect.frame(f.short_edge)),
            (None, Some(v)) => v.width.zip(v.height),
            (None, None) => None,
        };
        let video = match source.video() {
            Some(_) => Some(choose(&req.codec, req.gpu, frame_size.map(|(_, h)| h), req.quality)?),
            None => None,
        };

        let mut subtitles = None;
        if let Some(c) = &req.captions {
            let Some((w, h)) = frame_size else { bail!("captions need video") };
            let timed = if req.keep.is_empty() { c.transcript.clone() } else { c.transcript.retimed(&req.keep) };
            let cues = speech::cues(&timed, &c.style);
            subtitles = Some(TempFile::with_contents("captions", "ass", &speech::to_ass(&cues, w, h, &c.style))?);
        }
        let edit = Edit {
            keep: req.keep.clone(),
            frame: req.frame,
            subtitles: subtitles.as_ref().map(|s| s.path().to_path_buf()),
        };
        // A size target needs a second encode: two-pass plans its bitrate
        // against the finished picture, so the edit is rendered first.
        let intermediate = match req.target_mb {
            Some(_) => Some(TempFile::with_contents("render", "mkv", "")?),
            None => None,
        };
        let render_to = intermediate.as_ref().map_or(output.as_path(), |t| t.path());
        let render = ops::render(&input, render_to, &edit, video)?;
        let duration = render.output_duration();
        task.run_job("rendering", render.job.clone())?;
        if let (Some(mb), Some(rendered)) = (req.target_mb, &intermediate) {
            let codec = if req.codec == "hevc" { CompressCodec::Hevc } else { CompressCodec::H264 };
            let plan = ops::plan_compression((mb * 1e6) as u64, &probe(rendered.path())?, codec)?;
            let cancel = task.cancel.clone();
            ops::compress(rendered.path(), &output, plan, codec).run(&cancel, true, |stage, p| {
                task.ffmpeg(&format!("fitting under {mb} MB, {stage}"), p)
            })?;
        }
        task.set_output(output.display().to_string());
        Ok(Exported {
            output: output.display().to_string(),
            size: std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0),
            duration,
            encoder: video.map(|v| v.encoder.name),
        })
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GifRequest {
    input: String,
    output: String,
    start: f64,
    end: f64,
    fps: u32,
    width: u32,
}

#[tauri::command]
pub async fn export_gif(app: AppHandle, task_id: String, req: GifRequest) -> Result<Exported, String> {
    let output = PathBuf::from(&req.output);
    let title = format!("Making {}", file_name(&output));
    tasks::run(app, task_id, "export", title, move |task| {
        let opts = GifOptions { fps: req.fps.clamp(1, 50), width: req.width.max(16), start: req.start, end: Some(req.end) };
        task.run_job("making the gif", ops::gif(&PathBuf::from(&req.input), &output, opts)?)?;
        task.set_output(output.display().to_string());
        Ok(Exported {
            output: output.display().to_string(),
            size: std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0),
            duration: req.end - req.start,
            encoder: None,
        })
    })
    .await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderInfo {
    name: &'static str,
    codec: Codec,
    hardware: bool,
}

#[tauri::command]
pub async fn list_encoders() -> Result<Vec<EncoderInfo>, String> {
    blocking(|| {
        Ok(working_encoders()
            .iter()
            .map(|e| EncoderInfo { name: e.name, codec: e.codec, hardware: e.is_hardware() })
            .collect())
    })
    .await
}
