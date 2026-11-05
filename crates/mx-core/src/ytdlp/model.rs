//! Typed view of yt-dlp's `--dump-single-json` output.
//!
//! Only the fields we use are modelled; everything is optional because
//! extractors fill in wildly different subsets. Numeric fields that some
//! extractors emit as floats are `f64` so deserialisation never fails on them.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MediaInfo {
    #[serde(default)]
    pub id: String,
    pub title: Option<String>,
    /// "video", "playlist", "url" (flat playlist entry), ...
    #[serde(rename = "_type")]
    pub kind: Option<String>,
    pub extractor_key: Option<String>,
    pub uploader: Option<String>,
    /// Cover image URL.
    pub thumbnail: Option<String>,
    pub webpage_url: Option<String>,
    pub channel: Option<String>,
    pub upload_date: Option<String>,
    pub duration: Option<f64>,
    pub view_count: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub live_status: Option<String>,
    pub playlist_count: Option<u64>,
    #[serde(default)]
    pub formats: Vec<Format>,
    #[serde(default)]
    pub entries: Vec<Option<MediaInfo>>,
    /// Language code -> available subtitle tracks (uploaded by the creator).
    #[serde(default)]
    pub subtitles: HashMap<String, serde_json::Value>,
    /// Language code -> auto-generated captions (e.g. YouTube ASR).
    #[serde(default)]
    pub automatic_captions: HashMap<String, serde_json::Value>,
}

impl MediaInfo {
    pub fn is_playlist(&self) -> bool {
        self.kind.as_deref() == Some("playlist")
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Format {
    pub format_id: String,
    pub format_note: Option<String>,
    pub ext: Option<String>,
    pub protocol: Option<String>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    /// Total bitrate in kbit/s.
    pub tbr: Option<f64>,
    pub filesize: Option<f64>,
    pub filesize_approx: Option<f64>,
    pub dynamic_range: Option<String>,
    pub language: Option<String>,
}

impl Format {
    fn codec_present(codec: &Option<String>) -> bool {
        codec.as_deref().is_some_and(|c| c != "none")
    }

    pub fn has_video(&self) -> bool {
        Self::codec_present(&self.vcodec)
    }

    /// Storyboard thumbnails and similar non-media formats.
    pub fn is_media(&self) -> bool {
        self.ext.as_deref() != Some("mhtml")
    }

    pub fn size(&self) -> Option<u64> {
        self.filesize.or(self.filesize_approx).map(|s| s as u64)
    }
}
