//! Rendering: the project as one file, size-targeted copies and GIFs.

use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Result, anyhow, bail};
use mx_core::ffmpeg::encoders::{self, Codec, Encoder};
use mx_core::ffmpeg::ops::{self, CompressCodec, Frame, GifOptions, VideoEncode};
use mx_core::ffmpeg::probe;
use mx_core::speech::{self, CaptionStyle, Transcript};
use mx_core::temp::TempFile;
use mx_core::project::{Clip, Music, Project, output_transcript, titles_ass};
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

/// One clip of the project, as the editor sends it.
#[derive(Debug, Deserialize)]
pub struct ClipRequest {
    #[serde(flatten)]
    clip: Clip,
    /// The clip's transcript, source-timed, without deleted lines.
    #[serde(default)]
    transcript: Option<Transcript>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    clips: Vec<ClipRequest>,
    frame: Option<Frame>,
    music: Option<Music>,
    /// Burn the clips' transcripts in with this style.
    captions: Option<CaptionStyle>,
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
        let (clips, transcripts): (Vec<Clip>, Vec<Option<Transcript>>) =
            req.clips.into_iter().map(|c| (c.clip, c.transcript)).unzip();
        let project = Project { clips, frame: req.frame, music: req.music };
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

        // Captions and titles are drawn on the output, so they're laid out
        // through the same clip placement the render uses.
        let durations = project
            .clips
            .iter()
            .map(|c| probe(&c.source)?.duration_secs().ok_or_else(|| anyhow!("couldn't read the duration of {}", c.source.display())))
            .collect::<Result<Vec<_>>>()?;
        let layout = project.layout(&durations);
        let mut overlays = Vec::new();
        if let (Some(style), Some((w, h))) = (&req.captions, frame_size) {
            let timed = output_transcript(&layout, &transcripts);
            if !timed.segments.is_empty() {
                let cues = speech::cues(&timed, style);
                overlays.push(TempFile::with_contents("captions", "ass", &speech::to_ass(&cues, w, h, style))?);
            }
        }
        if let Some((w, h)) = frame_size
            && project.clips.iter().any(|c| !c.titles.is_empty())
        {
            overlays.push(TempFile::with_contents("titles", "ass", &titles_ass(&project.clips, &layout, w, h))?);
        }
        let subtitles: Vec<PathBuf> = overlays.iter().map(|t| t.path().to_path_buf()).collect();

        // A size target needs a second encode: two-pass plans its bitrate
        // against the finished picture, so the edit is rendered first.
        let intermediate = match req.target_mb {
            Some(_) => Some(TempFile::with_contents("render", "mkv", "")?),
            None => None,
        };
        let render_to = intermediate.as_ref().map_or(output.as_path(), |t| t.path());
        let render = ops::render_project(&project, &subtitles, render_to, video)?;
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
