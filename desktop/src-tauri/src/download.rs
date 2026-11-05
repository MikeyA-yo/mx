//! Fetching from the web through yt-dlp.

use std::path::PathBuf;

use anyhow::Result;
use mx_core::ytdlp::progress::Event;
use mx_core::ytdlp::{Auth, DownloadRequest, Selection, YtDlp};
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::tasks::{self, blocking, clock};

fn ytdlp(cookies_from_browser: Option<String>) -> Result<YtDlp> {
    let auth = Auth { cookies_file: None, cookies_from_browser: cookies_from_browser.filter(|b| !b.is_empty()) };
    YtDlp::locate(None, auth, false)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkInfo {
    url: String,
    title: Option<String>,
    uploader: Option<String>,
    site: Option<String>,
    duration: Option<f64>,
    thumbnail: Option<String>,
    /// Video heights on offer, tallest first.
    heights: Vec<u32>,
    is_playlist: bool,
    entries: usize,
}

#[tauri::command]
pub async fn fetch_info(url: String, cookies_from_browser: Option<String>) -> Result<LinkInfo, String> {
    blocking(move || {
        let info = ytdlp(cookies_from_browser)?.info(&url, false)?;
        let mut heights: Vec<u32> =
            info.formats.iter().filter(|f| f.has_video()).filter_map(|f| f.height).filter(|&h| h > 0).collect();
        heights.sort_unstable_by(|a, b| b.cmp(a));
        heights.dedup();
        Ok(LinkInfo {
            url: info.webpage_url.clone().unwrap_or(url),
            title: info.title.clone(),
            uploader: info.uploader.clone().or(info.channel.clone()),
            site: info.extractor_key.clone(),
            duration: info.duration,
            thumbnail: info.thumbnail.clone(),
            heights,
            is_playlist: info.is_playlist(),
            entries: info.entries.len(),
        })
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadOptions {
    url: String,
    title: Option<String>,
    output_dir: String,
    max_height: Option<u32>,
    audio_only: bool,
    /// mp3, m4a, ... `None` keeps the original audio stream.
    audio_format: Option<String>,
    /// H.264/AAC instead of VP9/AV1/Opus: edits and plays everywhere.
    compat: bool,
    playlist: bool,
    cookies_from_browser: Option<String>,
}

#[tauri::command]
pub async fn download(app: AppHandle, task_id: String, opts: DownloadOptions) -> Result<Vec<String>, String> {
    let title = opts.title.clone().unwrap_or_else(|| opts.url.clone());
    tasks::run(app, task_id, "download", title, move |task| {
        let yt = ytdlp(opts.cookies_from_browser)?;
        let selection = match opts.audio_only {
            true => Selection::Audio { codec: opts.audio_format.filter(|f| !f.is_empty()) },
            false => Selection::Best { max_height: opts.max_height },
        };
        let req = DownloadRequest {
            output_dir: PathBuf::from(&opts.output_dir),
            selection,
            playlist: opts.playlist,
            compat: opts.compat,
            container: None,
            info_json: false,
        };
        task.progress("starting", None, None);
        let cancel = task.cancel.clone();
        let outcome = yt.download(&opts.url, &req, &cancel, |event| match event {
            Event::Download(p) => {
                let pct = p.total.map(|t| p.downloaded as f64 / t.max(1) as f64 * 100.0);
                let mut detail = Vec::new();
                if let Some(speed) = p.speed {
                    detail.push(format!("{:.1} MB/s", speed / 1e6));
                }
                if let Some(eta) = p.eta_secs {
                    detail.push(format!("{} left", clock(eta as f64)));
                }
                task.progress("downloading", pct, Some(detail.join(" · ")));
            }
            Event::PostProcess { status, name } if status == "started" => task.progress(&name, None, None),
            Event::PostProcess { .. } | Event::Done(_) => {}
        })?;
        let files: Vec<String> = outcome.files.iter().map(|f| f.display().to_string()).collect();
        if let Some(first) = files.first() {
            task.set_output(first.clone());
        }
        Ok(files)
    })
    .await
}
