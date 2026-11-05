//! What an editor shows while you work: a version of the file its web view
//! can play, and a filmstrip of thumbnails for the timeline.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::Serialize;

use super::finish;
use crate::ffmpeg::{Job, Probe};

/// Codecs and containers a Chromium web view (WebView2, Tauri's on
/// Windows) plays natively. HEVC and 10-bit H.264 depend on the hardware,
/// so they get a proxy.
const VIDEO_CODECS: &[&str] = &["h264", "vp8", "vp9", "av1"];
const AUDIO_CODECS: &[&str] = &["aac", "mp3", "opus", "vorbis", "flac"];
const CONTAINERS: &[&str] = &["mp4", "m4v", "m4a", "mov", "webm", "mp3", "ogg", "opus", "flac", "wav"];

/// Proxies are small: the preview pane is rarely bigger than this, and
/// below 720p the CPU encodes faster than the GPU anyway.
const PROXY_HEIGHT: u32 = 540;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PreviewKind {
    /// Play the file itself.
    Direct,
    /// Same streams in an mp4: lossless, takes seconds.
    Remux,
    /// Re-encoded small H.264/AAC copy with frequent keyframes for quick
    /// seeking. Times match the source exactly.
    Proxy,
}

/// How to get a playable preview of `input`. `None` for the job means the
/// file plays as it is.
pub fn preview(input: &Path, source: &Probe, output: &Path) -> Result<(PreviewKind, Option<Job>)> {
    let video = source.video();
    if video.is_none() && !source.has_audio() {
        bail!("{} has no audio or video to preview", input.display());
    }
    let ext = input.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let codec = |kind: &str| {
        source
            .streams
            .iter()
            .find(|s| s.codec_type.as_deref() == Some(kind))
            .and_then(|s| s.codec_name.clone())
            .unwrap_or_default()
    };
    let video_ok = video.is_none_or(|v| {
        VIDEO_CODECS.contains(&codec("video").as_str()) && !v.pix_fmt.as_deref().unwrap_or("").contains("10")
    });
    let audio_ok = !source.has_audio() || AUDIO_CODECS.contains(&codec("audio").as_str()) || codec("audio").starts_with("pcm_");

    if video_ok && audio_ok && CONTAINERS.contains(&ext.as_str()) {
        return Ok((PreviewKind::Direct, None));
    }
    let mut job = Job::new(output).input(input).args(["-map", "0:v:0?", "-map", "0:a:0?"]);
    if video_ok && audio_ok && !codec("audio").starts_with("pcm_") {
        job = job.args(["-c", "copy"]);
        return Ok((PreviewKind::Remux, Some(finish(job, output))));
    }
    if let Some(v) = video {
        let height = v.height.unwrap_or(PROXY_HEIGHT).min(PROXY_HEIGHT);
        job = job
            .args(["-vf", &format!("scale=-2:{height}")])
            .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "26", "-pix_fmt", "yuv420p"])
            // A keyframe every half second: scrubbing seeks constantly.
            .args(["-force_key_frames", "expr:gte(t,n_forced*0.5)"]);
    }
    if let Some(d) = source.duration_secs() {
        job = job.duration(d);
    }
    job = job.args(["-c:a", "aac", "-b:a", "128k"]);
    Ok((PreviewKind::Proxy, Some(finish(job, output))))
}

/// A row of evenly spaced thumbnails in one image.
#[derive(Debug, Clone, Serialize)]
pub struct Filmstrip {
    pub path: PathBuf,
    pub count: u32,
    pub thumb_width: u32,
    pub thumb_height: u32,
}

/// Render `count` thumbnails `height` pixels tall across `input` into one
/// JPEG, side by side. Only keyframes are decoded, which is many times
/// faster than decoding every frame and plenty for a timeline overview.
pub fn filmstrip(input: &Path, source: &Probe, output: &Path, count: u32, height: u32) -> Result<(Filmstrip, Job)> {
    let Some(v) = source.video() else {
        bail!("{} has no video", input.display());
    };
    let Some(duration) = source.duration_secs().filter(|d| *d > 0.0) else {
        bail!("couldn't read the duration of {}", input.display());
    };
    let (w, h) = (v.width.unwrap_or(16), v.height.unwrap_or(9));
    let thumb_width = ((height as f64 * w as f64 / h.max(1) as f64 / 2.0).round() as u32).max(1) * 2;
    let job = Job::new(output)
        .args(["-skip_frame", "nokey"])
        .input(input)
        .args(["-an", "-sn"])
        .args([
            "-vf",
            &format!(
                "fps={count}/{duration:.3},scale={thumb_width}:{height}:flags=fast_bilinear,format=yuvj420p,tile={count}x1"
            ),
        ])
        .args(["-frames:v", "1", "-q:v", "5", "-update", "1"])
        .duration(duration);
    let strip = Filmstrip { path: output.to_path_buf(), count, thumb_width, thumb_height: height };
    Ok((strip, job))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::probe::{ProbeFormat, Stream};

    fn stream(kind: &str, codec: &str, pix_fmt: Option<&str>) -> Stream {
        Stream {
            codec_type: Some(kind.into()),
            codec_name: Some(codec.into()),
            pix_fmt: pix_fmt.map(String::from),
            width: Some(1920),
            height: Some(1080),
            ..Default::default()
        }
    }

    fn source(streams: Vec<Stream>) -> Probe {
        Probe { format: ProbeFormat { duration: Some("60".into()), ..Default::default() }, streams }
    }

    fn kind(file: &str, streams: Vec<Stream>) -> PreviewKind {
        preview(Path::new(file), &source(streams), Path::new("p.mp4")).unwrap().0
    }

    #[test]
    fn plays_web_codecs_directly_and_remuxes_them_out_of_mkv() {
        let web = || vec![stream("video", "h264", Some("yuv420p")), stream("audio", "aac", None)];
        assert_eq!(kind("a.mp4", web()), PreviewKind::Direct);
        assert_eq!(kind("a.mkv", web()), PreviewKind::Remux);
        assert_eq!(kind("a.webm", vec![stream("video", "vp9", Some("yuv420p")), stream("audio", "opus", None)]), PreviewKind::Direct);
    }

    #[test]
    fn hevc_10_bit_and_odd_audio_get_a_proxy() {
        assert_eq!(kind("a.mp4", vec![stream("video", "hevc", Some("yuv420p"))]), PreviewKind::Proxy);
        assert_eq!(kind("a.mp4", vec![stream("video", "h264", Some("yuv420p10le"))]), PreviewKind::Proxy);
        assert_eq!(kind("a.mkv", vec![stream("video", "h264", Some("yuv420p")), stream("audio", "ac3", None)]), PreviewKind::Proxy);
        let (_, job) = preview(Path::new("a.mov"), &source(vec![stream("video", "prores", None)]), Path::new("p.mp4")).unwrap();
        let s = super::super::args_of(&job.unwrap());
        assert!(s.contains("scale=-2:540") && s.contains("-c:v libx264"), "{s}");
    }

    #[test]
    fn filmstrip_thumbs_keep_the_aspect_ratio() {
        let src = source(vec![stream("video", "h264", Some("yuv420p"))]);
        let (strip, job) = filmstrip(Path::new("a.mp4"), &src, Path::new("s.jpg"), 40, 72).unwrap();
        assert_eq!((strip.thumb_width, strip.thumb_height), (128, 72));
        let s = super::super::args_of(&job);
        assert!(s.contains("fps=40/60.000,scale=128:72") && s.contains("tile=40x1"), "{s}");
    }
}
