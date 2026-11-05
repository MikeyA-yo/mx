//! Opening a file in the editor: what's in it, a playable preview, the
//! timeline's filmstrip and waveform, and the analyses that propose cuts.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use mx_core::analysis::{
    self, Envelope, Highlight, HighlightOptions, SilenceOptions, SilenceReport, Threshold, find_highlights, find_silences,
};
use mx_core::cache;
use mx_core::ffmpeg::ops::{self, PreviewKind};
use mx_core::ffmpeg::probe;
use mx_core::speech::{self, CaptionStyle, Segment, Transcript};
use mx_core::timeline::{Range, keep_ranges, total};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::tasks::{self, Task, blocking};

pub fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaSummary {
    path: String,
    name: String,
    duration: f64,
    size: u64,
    has_video: bool,
    has_audio: bool,
    width: Option<u32>,
    height: Option<u32>,
    fps: Option<f64>,
    video_codec: Option<String>,
    audio_codec: Option<String>,
}

#[tauri::command]
pub async fn probe_media(path: String) -> Result<MediaSummary, String> {
    blocking(move || {
        let input = PathBuf::from(&path);
        let p = probe(&input)?;
        let duration = p.duration_secs().ok_or_else(|| anyhow!("couldn't read the duration"))?;
        let v = p.video();
        let codec = |kind: &str| {
            p.streams.iter().find(|s| s.codec_type.as_deref() == Some(kind)).and_then(|s| s.codec_name.clone())
        };
        Ok(MediaSummary {
            name: file_name(&input),
            duration,
            size: std::fs::metadata(&input).map(|m| m.len()).unwrap_or(0),
            has_video: v.is_some(),
            has_audio: p.has_audio(),
            width: v.and_then(|v| v.width),
            height: v.and_then(|v| v.height),
            fps: v.and_then(|v| v.fps()),
            video_codec: codec("video"),
            audio_codec: codec("audio"),
            path,
        })
    })
    .await
}

#[derive(Debug, Serialize)]
pub struct Preview {
    kind: PreviewKind,
    /// The file the player should load.
    src: String,
}

#[tauri::command]
pub async fn prepare_preview(app: AppHandle, task_id: String, path: String) -> Result<Preview, String> {
    let input = PathBuf::from(&path);
    let title = format!("Preview of {}", file_name(&input));
    tasks::run(app, task_id, "preview", title, move |task| {
        let source = probe(&input)?;
        let out = cache::path("preview", 1, &input, "mp4")?;
        let (kind, job) = ops::preview(&input, &source, &out)?;
        let src = match job {
            None => path,
            Some(_) if out.is_file() => out.display().to_string(),
            Some(job) => {
                std::fs::create_dir_all(cache::dir())?;
                let stage = if kind == PreviewKind::Remux { "remuxing" } else { "making a preview copy" };
                task.run_job(stage, job)?;
                out.display().to_string()
            }
        };
        Ok(Preview { kind, src })
    })
    .await
}

const STRIP_COUNT: u32 = 64;
const STRIP_HEIGHT: u32 = 60;

#[tauri::command]
pub async fn filmstrip(app: AppHandle, task_id: String, path: String) -> Result<Option<ops::Filmstrip>, String> {
    let input = PathBuf::from(&path);
    let title = format!("Thumbnails of {}", file_name(&input));
    tasks::run(app, task_id, "preview", title, move |task| {
        let source = probe(&input)?;
        if source.video().is_none() {
            return Ok(None);
        }
        let out = cache::path("filmstrip", 1, &input, "jpg")?;
        let (strip, job) = ops::filmstrip(&input, &source, &out, STRIP_COUNT, STRIP_HEIGHT)?;
        if !out.is_file() {
            std::fs::create_dir_all(cache::dir())?;
            task.run_job("thumbnails", job)?;
        }
        Ok(Some(strip))
    })
    .await
}

/// The loudness envelope, measured once per file.
fn envelope(input: &Path, task: &mut Task) -> Result<Envelope> {
    cache::json("loudness", 1, input, || {
        let cancel = task.cancel.clone();
        analysis::measure_loudness(input, &cancel, |p| task.ffmpeg("measuring loudness", p))
    })
}

#[tauri::command]
pub async fn waveform(app: AppHandle, task_id: String, path: String, bins: usize) -> Result<Vec<f32>, String> {
    let input = PathBuf::from(&path);
    let title = format!("Waveform of {}", file_name(&input));
    tasks::run(app, task_id, "analyse", title, move |task| {
        let source = probe(&input)?;
        let bins = bins.clamp(1, 20_000);
        if !source.has_audio() {
            return Ok(vec![0.0; bins]);
        }
        Ok(envelope(&input, task)?.overview(source.duration_secs().unwrap_or(0.0), bins))
    })
    .await
}

#[tauri::command]
pub async fn detect_silences(
    app: AppHandle,
    task_id: String,
    path: String,
    threshold_db: Option<f64>,
    min_silence: f64,
) -> Result<SilenceReport, String> {
    let input = PathBuf::from(&path);
    let title = format!("Finding pauses in {}", file_name(&input));
    tasks::run(app, task_id, "analyse", title, move |task| {
        let env = envelope(&input, task)?;
        let threshold = threshold_db.map_or(Threshold::Auto, Threshold::Db);
        Ok(find_silences(&env, &SilenceOptions { threshold, min_duration: min_silence.max(0.05) }))
    })
    .await
}

#[tauri::command]
pub async fn highlights(
    app: AppHandle,
    task_id: String,
    path: String,
    count: usize,
    length: f64,
    from: Option<f64>,
    to: Option<f64>,
) -> Result<Vec<Highlight>, String> {
    let input = PathBuf::from(&path);
    let title = format!("Finding highlights in {}", file_name(&input));
    tasks::run(app, task_id, "analyse", title, move |task| {
        let signals = cache::json(analysis::highlights::CACHE_KIND, analysis::highlights::CACHE_VERSION, &input, || {
            let cancel = task.cancel.clone();
            analysis::measure_signals(&input, &cancel, |p| task.ffmpeg("measuring sound and motion", p))
        })?;
        let within = (from.is_some() || to.is_some()).then(|| Range::new(from.unwrap_or(0.0), to.unwrap_or(f64::INFINITY)));
        Ok(find_highlights(&signals, &HighlightOptions { count: count.clamp(1, 50), length: length.max(1.0), within }))
    })
    .await
}

/// Everything that removes time from the source, from every tool.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditPlanRequest {
    duration: f64,
    /// Pauses from `detect_silences`, cut with `padding` left at each side.
    #[serde(default)]
    silences: Vec<Range>,
    #[serde(default)]
    padding: f64,
    /// Ranges cut by hand.
    #[serde(default)]
    cuts: Vec<Range>,
    /// Transcript lines deleted in the text editor.
    transcript: Option<Transcript>,
    #[serde(default)]
    removed: Vec<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditPlan {
    /// All cuts, merged, for drawing on the timeline.
    cuts: Vec<Range>,
    /// What plays, in order: what export renders.
    keep: Vec<Range>,
    output_duration: f64,
}

/// Combine every tool's cuts into the one list of ranges to keep.
#[tauri::command]
pub fn plan_edit(req: EditPlanRequest) -> EditPlan {
    let d = req.duration;
    // keep_ranges(d, x, 0) is the complement of x, merged: the padded
    // pauses are what's left after keeping their padded complement.
    let mut cuts = if req.silences.is_empty() {
        Vec::new()
    } else {
        keep_ranges(d, &keep_ranges(d, &req.silences, req.padding), 0.0)
    };
    cuts.extend(req.cuts);
    if let Some(t) = &req.transcript {
        cuts.extend(t.cuts(&req.removed));
    }
    let keep = keep_ranges(d, &cuts, 0.0);
    EditPlan { cuts: keep_ranges(d, &keep, 0.0), output_duration: total(&keep), keep }
}

#[tauri::command]
pub fn caption_defaults(width: u32, height: u32) -> CaptionStyle {
    CaptionStyle::for_frame(width, height)
}

/// On-screen phrases, timed on the output of `keep` (empty = no cuts).
#[tauri::command]
pub fn caption_cues(transcript: Transcript, style: CaptionStyle, keep: Vec<Range>) -> Vec<Segment> {
    let timed = if keep.is_empty() { transcript } else { transcript.retimed(&keep) };
    speech::cues(&timed, &style)
}
