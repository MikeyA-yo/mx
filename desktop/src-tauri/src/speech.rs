//! Transcripts: models, transcribing, saving subtitle files.

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use mx_core::cache;
use mx_core::speech::{self, Model, TranscribeOptions, Transcript, models, subtitles};
use mx_core::timeline::Range;
use serde::Serialize;
use tauri::AppHandle;

use crate::media::file_name;
use crate::tasks::{self, blocking};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    name: &'static str,
    mb: u32,
    installed: bool,
    english_only: bool,
}

#[tauri::command]
pub fn list_models() -> Vec<ModelInfo> {
    models::KNOWN
        .iter()
        .map(|m| ModelInfo { name: m.name, mb: m.mb, installed: m.is_installed(), english_only: m.english_only() })
        .collect()
}

#[tauri::command]
pub async fn transcribe(
    app: AppHandle,
    task_id: String,
    path: String,
    model: String,
    language: String,
    gpu: bool,
) -> Result<Transcript, String> {
    let input = PathBuf::from(&path);
    let title = format!("Transcribing {}", file_name(&input));
    tasks::run(app, task_id, "transcribe", title, move |task| {
        let model = Model::lookup(&model).ok_or_else(|| anyhow!("unknown model {model:?}"))?;
        let language = match (model.english_only(), language.to_ascii_lowercase().as_str()) {
            (true, "auto" | "en") => "en".to_string(),
            (true, other) => bail!("{} only understands English; pick a model without .en for {other:?}", model.name),
            (false, other) => other.to_string(),
        };
        // Same file, model and language: same transcript.
        cache::json(&format!("transcript-{}-{language}", model.name), 1, &input, || {
            let cancel = task.cancel.clone();
            let model_path = models::ensure(model, &cancel, |done, total| {
                let pct = total.map(|t| done as f64 / t.max(1) as f64 * 100.0);
                let detail = format!("{:.0} of {} MB", done as f64 / 1e6, model.mb);
                task.progress(&format!("downloading the {} model", model.name), pct, Some(detail));
            })?;
            let opts = TranscribeOptions { language: language.clone(), use_gpu: gpu };
            speech::transcribe(&input, &model_path, &opts, &cancel, |stage, p| {
                let stage = if stage == "listen" { "finding speech" } else { "transcribing" };
                task.ffmpeg(stage, p);
            })
        })
    })
    .await
}

/// Write `transcript` as .srt, .vtt, .txt or .json, timed on the output of
/// `keep` when it's given (so the file matches the exported video).
#[tauri::command]
pub async fn save_subtitles(transcript: Transcript, keep: Option<Vec<Range>>, path: String) -> Result<(), String> {
    blocking(move || {
        let path = PathBuf::from(path);
        let t = match keep.filter(|k| !k.is_empty()) {
            Some(keep) => transcript.retimed(&keep),
            None => transcript,
        };
        let text = match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
            "srt" => subtitles::to_srt(&t),
            "vtt" => subtitles::to_vtt(&t),
            "txt" => subtitles::to_text(&t),
            "json" => serde_json::to_string_pretty(&t)? + "\n",
            other => bail!("can't write .{other}; use .srt, .vtt, .txt or .json"),
        };
        Ok(std::fs::write(&path, text)?)
    })
    .await
}

/// Read a subtitle file someone corrected by hand.
#[tauri::command]
pub async fn load_subtitles(path: String) -> Result<Transcript, String> {
    blocking(move || -> Result<Transcript> { subtitles::parse(&std::fs::read_to_string(path)?) }).await
}
