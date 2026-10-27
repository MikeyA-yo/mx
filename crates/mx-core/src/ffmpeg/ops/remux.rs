//! Changing the container without touching the streams: lossless, and
//! about as fast as copying the file.
//!
//! Video and audio are copied as-is (every track, so alternate languages
//! survive). Subtitles need care, because each container takes different
//! subtitle formats:
//!
//! - mkv takes anything, so they're copied.
//! - mp4/mov only take `mov_text`, webm only `webvtt`. Text subtitles (SRT,
//!   ASS, ...) are converted, which keeps every word but drops ASS styling.
//!   Image subtitles (Blu-ray PGS, DVD) can't be converted without OCR, so
//!   they're left out and reported.

use std::path::Path;

use anyhow::{Result, bail};
use serde::Serialize;

use super::finish;
use crate::ffmpeg::{Job, probe, probe::Probe};

#[derive(Debug, Clone)]
pub struct Remux {
    pub job: Job,
    pub subtitles: SubtitlePlan,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SubtitlePlan {
    pub copied: usize,
    /// Text subtitles converted to the target's subtitle format.
    pub converted: usize,
    /// Codec names of image subtitles that had to be left out.
    pub dropped: Vec<String>,
}

const TEXT_SUBTITLES: &[&str] = &["subrip", "srt", "ass", "ssa", "webvtt", "mov_text", "text"];

/// What the target container does with subtitles.
enum SubtitleTarget {
    CopyAll,
    ConvertTextTo(&'static str),
    /// Containers we don't know the rules for: leave subtitles out rather
    /// than fail the whole remux.
    None,
}

fn subtitle_target(output: &Path) -> SubtitleTarget {
    let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "mkv" | "mka" => SubtitleTarget::CopyAll,
        "mp4" | "m4v" | "mov" => SubtitleTarget::ConvertTextTo("mov_text"),
        "webm" => SubtitleTarget::ConvertTextTo("webvtt"),
        _ => SubtitleTarget::None,
    }
}

/// Build the remux job for an already-probed input.
pub fn remux_with(input: &Path, output: &Path, source: &Probe) -> Remux {
    let mut job = Job::new(output)
        .input(input)
        .args(["-map", "0:v?", "-map", "0:a?"]);

    let mut plan = SubtitlePlan::default();
    let target = subtitle_target(output);
    for s in source.streams.iter().filter(|s| s.codec_type.as_deref() == Some("subtitle")) {
        let codec = s.codec_name.clone().unwrap_or_default();
        let is_text = TEXT_SUBTITLES.contains(&codec.as_str());
        let keep = match target {
            SubtitleTarget::CopyAll => {
                plan.copied += 1;
                true
            }
            SubtitleTarget::ConvertTextTo(_) if is_text => {
                plan.converted += 1;
                true
            }
            _ => {
                plan.dropped.push(codec);
                false
            }
        };
        if keep {
            job = job.args(["-map".to_string(), format!("0:{}", s.index)]);
        }
    }

    job = job.args(["-c", "copy"]);
    if let SubtitleTarget::ConvertTextTo(codec) = target
        && plan.converted > 0
    {
        job = job.args(["-c:s", codec]);
    }
    Remux { job: finish(job, output), subtitles: plan }
}

pub fn remux(input: &Path, output: &Path) -> Result<Remux> {
    // Reading and writing the same file would truncate it before it's read.
    if let (Ok(a), Ok(b)) = (input.canonicalize(), output.canonicalize())
        && a == b
    {
        bail!("input and output are the same file; pick a different output name");
    }
    Ok(remux_with(input, output, &probe(input)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Probe {
        serde_json::from_str(
            r#"{"format":{"duration":"60"},"streams":[
                {"index":0,"codec_type":"video","codec_name":"vp9"},
                {"index":1,"codec_type":"audio","codec_name":"opus"},
                {"index":2,"codec_type":"audio","codec_name":"aac"},
                {"index":3,"codec_type":"subtitle","codec_name":"subrip"},
                {"index":4,"codec_type":"subtitle","codec_name":"hdmv_pgs_subtitle"},
                {"index":5,"codec_type":"subtitle","codec_name":"ass"}]}"#,
        )
        .unwrap()
    }

    fn args(r: &Remux) -> String {
        super::super::args_of(&r.job)
    }

    #[test]
    fn mp4_converts_text_subs_and_drops_image_subs() {
        let r = remux_with(Path::new("in.mkv"), Path::new("out.mp4"), &source());
        let a = args(&r);
        assert!(a.contains("-map 0:v? -map 0:a? -map 0:3 -map 0:5 -c copy -c:s mov_text"), "{a}");
        assert!(!a.contains("0:4"), "{a}");
        assert!(a.contains("+faststart"));
        assert_eq!(
            r.subtitles,
            SubtitlePlan { copied: 0, converted: 2, dropped: vec!["hdmv_pgs_subtitle".into()] }
        );
    }

    #[test]
    fn mkv_copies_every_subtitle() {
        let r = remux_with(Path::new("in.mp4"), Path::new("out.MKV"), &source());
        let a = args(&r);
        assert!(a.contains("-map 0:3 -map 0:4 -map 0:5 -c copy"), "{a}");
        assert!(!a.contains("-c:s"));
        assert_eq!(r.subtitles.copied, 3);
    }

    #[test]
    fn webm_converts_to_webvtt_and_unknown_containers_skip_subtitles() {
        let a = args(&remux_with(Path::new("in.mkv"), Path::new("out.webm"), &source()));
        assert!(a.contains("-c:s webvtt"), "{a}");

        let r = remux_with(Path::new("in.mkv"), Path::new("out.ts"), &source());
        assert!(!args(&r).contains("0:3"));
        assert_eq!(r.subtitles.dropped.len(), 3);
    }
}
