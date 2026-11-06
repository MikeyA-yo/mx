//! Is everything mx needs installed?

use mx_core::ffmpeg;
use mx_core::ytdlp::{Auth, YtDlp};
use serde::Serialize;

use crate::tasks::blocking;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    name: &'static str,
    /// Version or detail when it's there.
    found: Option<String>,
    /// What it's for and how to get it, when it isn't.
    fix: &'static str,
}

#[tauri::command]
pub async fn doctor() -> Result<Vec<Check>, String> {
    blocking(|| {
        let yt = YtDlp::locate(None, Auth::default(), false).ok();
        let whisper = ffmpeg::has_filter("whisper").then(|| "whisper filter available".to_string());
        Ok(vec![
            Check {
                name: "ffmpeg",
                found: ffmpeg::version("ffmpeg"),
                fix: "Needed for everything. Install a full build (gyan.dev on Windows) and put it on PATH.",
            },
            Check { name: "ffprobe", found: ffmpeg::version("ffprobe"), fix: "Ships with ffmpeg." },
            Check {
                name: "yt-dlp",
                found: yt.as_ref().and_then(|y| y.version().ok()),
                fix: "Needed for downloads: pip install -U \"yt-dlp[default,curl-cffi]\"",
            },
            Check {
                name: "browser impersonation",
                found: yt.as_ref().and_then(|y| y.can_impersonate().ok()).filter(|&ok| ok).map(|_| "curl_cffi".into()),
                fix: "TikTok and others block downloads without it: pip install -U \"yt-dlp[default,curl-cffi]\"",
            },
            Check {
                name: "speech",
                found: whisper,
                fix: "Captions need an ffmpeg 8 build with the whisper filter (the gyan.dev full build has it).",
            },
        ])
    })
    .await
}

/// Files given on the command line ("Open with" in Explorer), to open
/// in the editor at startup, in order.
#[tauri::command]
pub fn startup_files() -> Vec<String> {
    std::env::args().skip(1).filter(|a| !a.starts_with('-') && std::path::Path::new(a).is_file()).collect()
}
