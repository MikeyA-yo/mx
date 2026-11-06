//! Rendering: the project as one file, size-targeted copies and GIFs.

use std::path::PathBuf;
use std::sync::OnceLock;

use anyhow::{Result, anyhow, bail};
use mx_core::ffmpeg::encoders::{self, Codec, Encoder};
use mx_core::cache;
use mx_core::ffmpeg::ops::{self, Aspect, CompressCodec, Fit, Frame, GifOptions, VideoEncode};
use mx_core::ffmpeg::probe;
use mx_core::temp::TempFile;
use mx_core::project::{Clip, Project};
use mx_core::timeline::{Range, head, tail};
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

/// Seconds of each clip shown either side of a transition in its preview.
const PREVIEW_LEAD: f64 = 1.5;

/// A small render of the join into clip `index`: the end of the clip
/// before, the transition, the start of this one. Rendered by the same
/// code as the export, at 360p, so what plays is what the export does.
#[tauri::command]
pub async fn preview_transition(app: AppHandle, task_id: String, project: Project, index: usize) -> Result<String, String> {
    let title = format!("Previewing the transition into clip {}", index + 1);
    tasks::run(app, task_id, "preview", title, move |task| {
        if index == 0 || index >= project.clips.len() {
            bail!("there's no clip before clip {} to transition from", index + 1);
        }
        let (before, after) = (&project.clips[index - 1], &project.clips[index]);
        let Some(transition) = after.transition else { bail!("clip {} has no transition in", index + 1) };
        let kept = |c: &Clip| -> Result<Vec<Range>> {
            if !c.keep.is_empty() {
                return Ok(c.keep.clone());
            }
            let d = probe(&c.source)?.duration_secs().ok_or_else(|| anyhow!("couldn't read the duration of {}", c.source.display()))?;
            Ok(vec![Range::new(0.0, d)])
        };
        let window = transition.duration + PREVIEW_LEAD;
        let a = Clip { keep: tail(&kept(before)?, window), transition: None, ..before.clone() };
        let b = Clip { keep: head(&kept(after)?, window), ..after.clone() };

        let source = probe(&before.source)?;
        let (w, h) = source.video().and_then(|v| v.width.zip(v.height)).ok_or_else(|| anyhow!("transitions need video"))?;
        let frame = match project.frame {
            Some(f) => Frame { short_edge: f.short_edge.min(360), ..f },
            None => Frame { aspect: Aspect { w, h }, fit: Fit::Blur, short_edge: w.min(h).min(360) },
        };
        let preview = Project { clips: vec![a, b], frame: Some(frame), music: None, captions: project.captions.clone() };
        std::fs::create_dir_all(cache::dir())?;
        // A new name each time, so the web view doesn't replay a cached one.
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let output = cache::dir().join(format!("transition-{stamp}.mp4"));
        let video = choose("h264", false, Some(frame.aspect.frame(frame.short_edge).1), None)?;
        let render = ops::render_project(&preview, &[], &output, Some(video))?;
        task.run_job("rendering the preview", render.job.clone())?;
        Ok(output.display().to_string())
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
