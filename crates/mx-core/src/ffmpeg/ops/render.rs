//! A whole edit of one source, rendered in a single encode: the parts to
//! keep, a new frame shape and burned-in captions. Doing it in one pass is
//! both faster and better than chaining `jumpcut`, `reframe` and
//! `captions`: the video is decoded and encoded once, so it loses quality
//! once.
//!
//! The cutting is the jump-cut machinery (see [`super::jumpcut`]): video
//! is made constant frame rate and `select`ed frame-exactly, audio is
//! `atrim`med at the matching sample. Reframing and subtitles run on the
//! joined result, so captions are timed on the output timeline.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use super::clip::Frame;
use super::jumpcut::{audio_chain, parse_rate, select_expr, snap};
use super::reframe::filtergraph;
use super::{VideoEncode, aac, audio_codec, finish};
use crate::ffmpeg::{Job, filter_path, probe};
use crate::temp::TempFile;
use crate::timeline::{Range, total};

/// What to make from a source.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Edit {
    /// Source ranges to keep. Empty keeps everything. They're played in
    /// time order, and overlapping ranges are merged.
    #[serde(default)]
    pub keep: Vec<Range>,
    /// A new frame shape, e.g. 9:16.
    #[serde(default)]
    pub frame: Option<Frame>,
    /// Subtitles (ASS keeps its styling, SRT works too) drawn into the
    /// picture. They're timed on the output, after cuts.
    #[serde(default)]
    pub subtitles: Option<PathBuf>,
}

/// A render ready to run. Holds the filter script, which is deleted when
/// this is dropped.
#[derive(Debug)]
pub struct Render {
    pub job: Job,
    /// The ranges actually kept, after snapping to video frames.
    pub kept: Vec<Range>,
    _script: Option<TempFile>,
}

impl Render {
    pub fn output_duration(&self) -> f64 {
        total(&self.kept)
    }
}

/// Sorted, clamped to the source, overlaps merged.
fn normalize(keep: &[Range], duration: f64) -> Vec<Range> {
    let mut ranges: Vec<Range> = keep
        .iter()
        .map(|r| Range::new(r.start.max(0.0), r.end.min(duration)))
        .filter(|r| !r.is_empty())
        .collect();
    ranges.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut merged: Vec<Range> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// The filter graph pieces, and the pads to map.
#[derive(Debug, Default, PartialEq)]
struct Graph {
    parts: Vec<String>,
    video: Option<String>,
    audio: Option<String>,
}

struct Plan<'a> {
    /// Ranges relative to the (possibly seeked) input; `None` = no cuts.
    cuts: Option<&'a [Range]>,
    /// Frame rate as ffmpeg wrote it and as a number, if there's video.
    rate: Option<(&'a str, f64)>,
    frame: Option<Frame>,
    subtitles: Option<String>,
    audio: bool,
}

fn graph(plan: &Plan) -> Graph {
    let mut g = Graph::default();
    if let Some((rate, fps)) = plan.rate {
        let mut pad = "0:v:0".to_string();
        if let Some(cuts) = plan.cuts {
            g.parts.push(format!("[0:v:0]fps={rate},select='{}',setpts=N/FRAME_RATE/TB[vcut]", select_expr(cuts, 0.5 / fps)));
            pad = "vcut".into();
        }
        if let Some(f) = plan.frame {
            let (w, h) = f.aspect.frame(f.short_edge);
            g.parts.push(filtergraph(&pad, "vframe", f.fit, w, h));
            pad = "vframe".into();
        }
        if let Some(subs) = &plan.subtitles {
            g.parts.push(format!("[{pad}]subtitles=filename={subs}[vsub]"));
            pad = "vsub".into();
        }
        g.video = Some(if g.parts.is_empty() { pad } else { format!("[{pad}]") });
    }
    if plan.audio {
        match plan.cuts {
            Some(cuts) => {
                g.parts.push(audio_chain(cuts));
                g.audio = Some("[a]".into());
            }
            None => g.audio = Some("0:a:0".into()),
        }
    }
    g
}

/// Render `edit` of `input` to `output`. `video` is required when the
/// input has video.
pub fn render(input: &Path, output: &Path, edit: &Edit, video: Option<VideoEncode>) -> Result<Render> {
    let source = probe(input)?;
    let duration = source.duration_secs().ok_or_else(|| anyhow!("couldn't read the duration of {}", input.display()))?;
    let audio = source.has_audio();
    let rate_text = source.video().and_then(|v| v.avg_frame_rate.clone());
    let rate = rate_text.as_deref().and_then(|r| Some((r, parse_rate(r)?)));
    if rate.is_none() && (edit.frame.is_some() || edit.subtitles.is_some()) {
        bail!("{} has no video to reframe or caption", input.display());
    }

    let mut kept = if edit.keep.is_empty() { vec![Range::new(0.0, duration)] } else { normalize(&edit.keep, duration) };
    if let Some((_, fps)) = rate {
        kept = snap(&kept, fps);
    }
    if kept.is_empty() {
        bail!("nothing left to keep");
    }
    let whole = kept.len() == 1 && kept[0].start < 0.001 && kept[0].end > duration - 0.001;

    // Start decoding where the first kept range begins: exporting a 30 s
    // clip from the end of an hour-long file shouldn't decode the hour.
    // Seeking before `-i` restarts timestamps at 0, so ranges shift too.
    let seek = if whole { 0.0 } else { kept[0].start };
    let shifted: Vec<Range> = kept.iter().map(|r| Range::new(r.start - seek, r.end - seek)).collect();

    let subtitles = edit.subtitles.as_deref().map(filter_path).transpose()?;
    let g = graph(&Plan { cuts: (!whole).then_some(&shifted[..]), rate, frame: edit.frame, subtitles, audio });

    let mut job = Job::new(output);
    if seek > 0.0 {
        job = job.args(["-ss", &format!("{seek:.6}")]);
    }
    job = job.input(input);
    if !whole {
        job = job.args(["-t", &format!("{:.6}", kept[kept.len() - 1].end - seek)]);
    }
    job = job.duration(total(&kept));

    // Hundreds of ranges overflow Windows' 32k command-line limit, so the
    // graph goes in a file, loaded with ffmpeg 7+'s `-/option file` syntax.
    let script = match g.parts.is_empty() {
        true => None,
        false => Some(TempFile::with_contents("render", "txt", &g.parts.join(";\n"))?),
    };
    if let Some(s) = &script {
        job = job.arg("-/filter_complex").arg(s.path());
    }
    if let Some(pad) = &g.video {
        let video = video.ok_or_else(|| anyhow!("{} has video, so a video encoder is needed", input.display()))?;
        job = job.args(["-map", pad]).args(video.args());
    }
    if let Some(pad) = &g.audio {
        job = job.args(["-map", pad]);
        let codec = source.streams.iter().find(|s| s.codec_type.as_deref() == Some("audio")).and_then(|s| s.codec_name.as_deref());
        let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        job = if whole && codec == Some("aac") && ["mp4", "m4v", "mov", "mkv", "m4a"].contains(&ext.as_str()) {
            // Untouched audio is copied: re-encoding AAC is the slow part
            // of an otherwise GPU-encoded render.
            job.args(["-c:a", "copy"])
        } else if g.video.is_some() {
            job.args(aac(192))
        } else {
            job.args(audio_codec(output))
        };
    }
    Ok(Render { job: finish(job, output), kept, _script: script })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::ops::{Aspect, Fit};

    fn r(start: f64, end: f64) -> Range {
        Range::new(start, end)
    }

    #[test]
    fn keep_ranges_are_sorted_clamped_and_merged() {
        let n = normalize(&[r(5.0, 7.0), r(-1.0, 2.0), r(1.5, 3.0), r(9.0, 12.0), r(4.0, 4.0)], 10.0);
        assert_eq!(n, [r(0.0, 3.0), r(5.0, 7.0), r(9.0, 10.0)]);
    }

    #[test]
    fn cuts_shift_video_boundaries_by_half_a_frame() {
        let cuts = [r(1.0, 2.0), r(3.0, 4.0)];
        let g = graph(&Plan { cuts: Some(&cuts), rate: Some(("25/1", 25.0)), frame: None, subtitles: None, audio: true });
        let s = g.parts.join(";\n");
        assert!(s.contains("[0:v:0]fps=25/1,select='gte(t,0.980000)*lt(t,1.980000)+gte(t,2.980000)*lt(t,3.980000)'"), "{s}");
        assert!(s.contains("[0:a:0]asplit=2[as0][as1];"), "{s}");
        assert!(s.contains("[as1]atrim=start=3.000000:end=4.000000,asetpts=PTS-STARTPTS[at1];"), "{s}");
        assert!(s.contains("[at0][at1]concat=n=2:v=0:a=1[a]"), "{s}");
        assert_eq!((g.video.as_deref(), g.audio.as_deref()), (Some("[vcut]"), Some("[a]")));
    }

    #[test]
    fn frame_and_subtitles_chain_after_the_cuts() {
        let frame = Frame { aspect: Aspect { w: 9, h: 16 }, fit: Fit::Pad, short_edge: 720 };
        let cuts = [r(0.0, 1.0)];
        let plan = Plan { cuts: Some(&cuts), rate: Some(("30/1", 30.0)), frame: Some(frame), subtitles: Some("'c.ass'".into()), audio: false };
        let g = graph(&plan);
        assert!(g.parts[1].starts_with("[vcut]scale=720:1280") && g.parts[1].ends_with("[vframe]"), "{:?}", g.parts);
        assert_eq!(g.parts[2], "[vframe]subtitles=filename='c.ass'[vsub]");
        assert_eq!((g.video.as_deref(), g.audio), (Some("[vsub]"), None));
    }

    #[test]
    fn untouched_source_maps_streams_directly() {
        let g = graph(&Plan { cuts: None, rate: Some(("30/1", 30.0)), frame: None, subtitles: None, audio: true });
        assert_eq!(g, Graph { parts: vec![], video: Some("0:v:0".into()), audio: Some("0:a:0".into()) });
        // Audio-only sources never get a video chain.
        let g = graph(&Plan { cuts: Some(&[r(1.0, 2.0)]), rate: None, frame: None, subtitles: None, audio: true });
        assert!(g.video.is_none() && !g.parts.iter().any(|p| p.contains("0:v")));
    }
}
