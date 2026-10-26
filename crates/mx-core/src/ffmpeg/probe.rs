use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Probe {
    pub format: ProbeFormat,
    #[serde(default)]
    pub streams: Vec<Stream>,
}

impl Probe {
    pub fn duration_secs(&self) -> Option<f64> {
        self.format.duration.as_deref()?.parse().ok()
    }

    pub fn video(&self) -> Option<&Stream> {
        self.streams.iter().find(|s| s.codec_type.as_deref() == Some("video"))
    }

    pub fn has_audio(&self) -> bool {
        self.streams.iter().any(|s| s.codec_type.as_deref() == Some("audio"))
    }
}

/// ffprobe reports most numbers as strings; we keep them that way and parse
/// at the point of use.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProbeFormat {
    pub format_long_name: Option<String>,
    pub duration: Option<String>,
    pub size: Option<String>,
    pub bit_rate: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Stream {
    pub index: u32,
    pub codec_type: Option<String>,
    pub codec_name: Option<String>,
    pub profile: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub pix_fmt: Option<String>,
    pub avg_frame_rate: Option<String>,
    pub sample_rate: Option<String>,
    pub channels: Option<u32>,
    pub bit_rate: Option<String>,
    #[serde(default)]
    pub tags: HashMap<String, String>,
}

impl Stream {
    /// "30000/1001" -> 29.97
    pub fn fps(&self) -> Option<f64> {
        let (n, d) = self.avg_frame_rate.as_deref()?.split_once('/')?;
        let (n, d): (f64, f64) = (n.parse().ok()?, d.parse().ok()?);
        (d != 0.0).then(|| n / d)
    }
}

/// Timestamp of the last video keyframe at or before `t`: where a
/// stream-copy cut requested at `t` will really begin.
///
/// Only keyframes are decoded (`-skip_frame nokey`), and `-read_intervals`
/// limits the scan to a window before `t`, so this stays fast on long files.
/// If the window holds no keyframe (very long GOPs) it rescans from 0.
pub fn keyframe_at_or_before(path: &Path, t: f64) -> Result<Option<f64>> {
    for window_start in [(t - 60.0).max(0.0), 0.0] {
        let out = Command::new(super::tool("ffprobe"))
            .args(["-v", "error", "-select_streams", "v:0", "-skip_frame", "nokey"])
            .args(["-show_entries", "frame=best_effort_timestamp_time", "-of", "csv=p=0"])
            .args(["-read_intervals", &format!("{window_start}%{}", t + 0.001)])
            .arg(path)
            .output()
            .context("failed to run ffprobe")?;
        if !out.status.success() {
            bail!("ffprobe: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        // Lines look like "5.000000" (sometimes with a trailing comma).
        let found = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.trim().trim_end_matches(',').parse::<f64>().ok())
            .filter(|&k| k <= t + 0.001)
            .reduce(f64::max);
        if found.is_some() || window_start == 0.0 {
            return Ok(found);
        }
    }
    Ok(None)
}

pub fn probe(path: &Path) -> Result<Probe> {
    let out = Command::new(super::tool("ffprobe"))
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .context("failed to run ffprobe (is ffmpeg installed and on PATH?)")?;
    if !out.status.success() {
        bail!("ffprobe: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    serde_json::from_slice(&out.stdout).context("ffprobe returned unexpected JSON")
}
