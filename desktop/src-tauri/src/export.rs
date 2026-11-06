//! Rendering: the project as one file, size-targeted copies and GIFs.

use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Result, anyhow, bail};
use mx_core::ffmpeg::encoders::{self, Codec, Encoder};
use mx_core::ffmpeg::ops::{self, CompressCodec, GifOptions, VideoEncode};
use mx_core::ffmpeg::probe;
use mx_core::temp::TempFile;
use mx_core::project::Project;
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
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    /// Clips, frame, music and caption style, as `mx render` reads them.
    project: Project,
    output: String,
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
        task.progress("preparing", None, None);
        let project = req.project;
        let Some(first) = project.clips.first() else { bail!("there are no clips to export") };
        let first = probe(&first.source)?;
        let frame_size = match (project.frame, first.video()) {
            (Some(f), _) => Some(f.aspect.frame(f.short_edge)),
            (None, Some(v)) => v.width.zip(v.height),
            (None, None) => None,
        };
        let video = match frame_size {
            Some((_, h)) => Some(choose(&req.codec, req.gpu, Some(h), req.quality)?),
            None => None,
        };

        // A size target needs a second encode: two-pass plans its bitrate
        // against the finished picture, so the edit is rendered first.
        let intermediate = match req.target_mb {
            Some(_) => Some(TempFile::with_contents("render", "mkv", "")?),
            None => None,
        };
        let render_to = intermediate.as_ref().map_or(output.as_path(), |t| t.path());
        let render = ops::render_project(&project, &[], render_to, video)?;
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
