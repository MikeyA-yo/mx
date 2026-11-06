//! A timeline of several clips with music and text on top: what the
//! desktop editor builds and [`crate::ffmpeg::ops::render_project`]
//! renders in one encode.
//!
//! Clips play one after another. Each clip keeps parts of its source, and
//! anything attached to a clip (titles, its transcript) is timed on that
//! source, so reordering or recutting clips never strands a title in the
//! wrong place. [`Project::layout`] works out where everything lands on
//! the output timeline.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ffmpeg::ops::Frame;
use crate::speech::captions::{ass_text, ass_time};
use crate::speech::{Segment, Transcript};
use crate::timeline::{Range, map_range, total};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Project {
    pub clips: Vec<Clip>,
    /// Output frame. `None` takes the first video clip's size, and other
    /// clips are fitted into it with a blurred background.
    #[serde(default)]
    pub frame: Option<Frame>,
    #[serde(default)]
    pub music: Option<Music>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub source: PathBuf,
    /// Source ranges to keep; empty keeps the whole source.
    #[serde(default)]
    pub keep: Vec<Range>,
    /// 1.0 leaves the clip's own sound as it is; 0 mutes it.
    #[serde(default = "one")]
    pub volume: f64,
    /// Text on screen, timed on this clip's source.
    #[serde(default)]
    pub titles: Vec<Title>,
}

fn one() -> f64 {
    1.0
}

impl Clip {
    pub fn new(source: impl Into<PathBuf>) -> Self {
        Self { source: source.into(), keep: Vec::new(), volume: 1.0, titles: Vec::new() }
    }
}

/// A soundtrack under the whole project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Music {
    pub source: PathBuf,
    /// Where in the song to start, in seconds.
    #[serde(default)]
    pub offset: f64,
    /// Linear gain; 0.25 sits under speech without fighting it.
    #[serde(default = "music_volume")]
    pub volume: f64,
    /// Turn the music down while the clips have sound (speech), and back
    /// up in the gaps.
    #[serde(default = "yes")]
    pub duck: bool,
    /// Fade-out at the end, seconds.
    #[serde(default = "fade")]
    pub fade_out: f64,
}

impl Default for Music {
    fn default() -> Self {
        Self { source: PathBuf::new(), offset: 0.0, volume: music_volume(), duck: true, fade_out: fade() }
    }
}

fn music_volume() -> f64 {
    0.25
}
fn yes() -> bool {
    true
}
fn fade() -> f64 {
    2.0
}

/// Text placed on the picture: a title, a label, a call to action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Title {
    pub text: String,
    /// When it shows, in its clip's source time.
    pub start: f64,
    pub end: f64,
    /// Centre of the text, 0..1 across and down the frame.
    pub x: f64,
    pub y: f64,
    /// Height as a fraction of the frame's shorter side.
    pub size: f64,
    /// `#rrggbb`.
    pub color: String,
    pub font: String,
    /// On a dark box instead of with an outline.
    #[serde(default)]
    pub boxed: bool,
}

/// Where one clip lands on the output timeline.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// Output time at which the clip starts.
    pub offset: f64,
    /// Its kept source ranges (the whole source when it keeps everything).
    pub keep: Vec<Range>,
}

impl Placement {
    pub fn duration(&self) -> f64 {
        total(&self.keep)
    }

    /// Source range -> the output ranges it plays at.
    pub fn place(&self, r: Range) -> Vec<Range> {
        map_range(&self.keep, r).into_iter().map(|p| Range::new(p.start + self.offset, p.end + self.offset)).collect()
    }
}

impl Project {
    /// Each clip's place on the output timeline, given the source
    /// durations (in clip order).
    pub fn layout(&self, durations: &[f64]) -> Vec<Placement> {
        let mut offset = 0.0;
        self.clips
            .iter()
            .zip(durations)
            .map(|(clip, &d)| {
                let keep = if clip.keep.is_empty() { vec![Range::new(0.0, d)] } else { clip.keep.clone() };
                let p = Placement { offset, keep };
                offset += p.duration();
                p
            })
            .collect()
    }
}

/// Per-clip transcripts (source-timed, `None` for clips without one) as
/// one transcript on the output timeline.
pub fn output_transcript(layout: &[Placement], transcripts: &[Option<Transcript>]) -> Transcript {
    let mut segments = Vec::new();
    for (place, t) in layout.iter().zip(transcripts) {
        let Some(t) = t else { continue };
        for s in &t.segments {
            let pieces = place.place(Range::new(s.start, s.end));
            if let (Some(first), Some(last)) = (pieces.first(), pieces.last()) {
                segments.push(Segment { start: first.start, end: last.end, text: s.text.clone() });
            }
        }
    }
    Transcript { segments }
}

/// `#rrggbb` -> ASS `&H00BBGGRR` (white if it doesn't parse).
fn ass_color(hex: &str) -> String {
    let h = hex.trim_start_matches('#');
    if h.len() == 6 && u32::from_str_radix(h, 16).is_ok() {
        format!("&H00{}{}{}", &h[4..6], &h[2..4], &h[0..2]).to_uppercase()
    } else {
        "&H00FFFFFF".to_string()
    }
}

/// An ASS file drawing every clip's titles at their output times on a
/// `width`x`height` frame. Titles on parts that were cut don't show; a
/// title spanning a cut shows from its first kept moment to its last.
pub fn titles_ass(clips: &[Clip], layout: &[Placement], width: u32, height: u32) -> String {
    let short = width.min(height) as f64;
    let mut ass = format!(
        "[Script Info]\n\
         ScriptType: v4.00+\n\
         PlayResX: {width}\n\
         PlayResY: {height}\n\
         WrapStyle: 0\n\
         ScaledBorderAndShadow: yes\n\
         \n\
         [V4+ Styles]\n\
         Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, \
         Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, \
         Alignment, MarginL, MarginR, MarginV, Encoding\n\
         Style: Title,Arial,48,&H00FFFFFF,&H00FFFFFF,&H00000000,&H80000000,-1,0,0,0,100,100,0,0,1,3,1,5,0,0,0,1\n\
         Style: Boxed,Arial,48,&H00FFFFFF,&H00FFFFFF,&H50000000,&H00000000,-1,0,0,0,100,100,0,0,3,10,0,5,0,0,0,1\n\
         \n\
         [Events]\n\
         Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"
    );
    for (clip, place) in clips.iter().zip(layout) {
        for t in &clip.titles {
            let pieces = place.place(Range::new(t.start, t.end));
            let (Some(first), Some(last)) = (pieces.first(), pieces.last()) else { continue };
            let size = (short * t.size).round().max(8.0);
            let (x, y) = ((t.x.clamp(0.0, 1.0) * width as f64).round(), (t.y.clamp(0.0, 1.0) * height as f64).round());
            let edge = if t.boxed { (size * 0.25).round() } else { (size * 0.07).round().max(1.0) };
            ass += &format!(
                "Dialogue: 1,{},{},{},,0,0,0,,{{\\an5\\pos({x},{y})\\fn{}\\fs{size}\\bord{edge}\\1c{}&\\fad(150,150)}}{}\n",
                ass_time(first.start),
                ass_time(last.end),
                if t.boxed { "Boxed" } else { "Title" },
                t.font.replace(['{', '}', '\\'], ""),
                ass_color(&t.color),
                ass_text(&t.text, false).replace('\n', "\\N"),
            );
        }
    }
    ass
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: f64, end: f64) -> Range {
        Range::new(start, end)
    }

    fn title(text: &str, start: f64, end: f64) -> Title {
        Title { text: text.into(), start, end, x: 0.5, y: 0.2, size: 0.1, color: "#ffcc00".into(), font: "Impact".into(), boxed: false }
    }

    #[test]
    fn clips_are_laid_out_back_to_back() {
        let mut a = Clip::new("a.mp4");
        a.keep = vec![r(0.0, 2.0), r(5.0, 6.0)];
        let p = Project { clips: vec![a, Clip::new("b.mp4")], ..Project::default() };
        let layout = p.layout(&[10.0, 4.0]);
        assert_eq!(layout[0].offset, 0.0);
        assert_eq!(layout[1], Placement { offset: 3.0, keep: vec![r(0.0, 4.0)] });
        // Source 5.5 in clip a plays at 2.5; source 1 in clip b at 4.
        assert_eq!(layout[0].place(r(5.5, 5.6))[0].start, 2.5);
        assert_eq!(layout[1].place(r(1.0, 2.0)), [r(4.0, 5.0)]);
    }

    #[test]
    fn transcripts_join_on_the_output_timeline() {
        let p = Project { clips: vec![Clip::new("a"), Clip::new("b")], ..Project::default() };
        let layout = p.layout(&[3.0, 3.0]);
        let t = |s: f64| Some(Transcript { segments: vec![Segment { start: s, end: s + 1.0, text: "x".into() }] });
        let out = output_transcript(&layout, &[t(1.0), None]);
        assert_eq!(out.segments.len(), 1);
        let out = output_transcript(&layout, &[t(1.0), t(0.5)]);
        assert_eq!((out.segments[1].start, out.segments[1].end), (3.5, 4.5));
    }

    #[test]
    fn titles_become_positioned_ass_lines_at_output_times() {
        let mut a = Clip::new("a.mp4");
        a.keep = vec![r(2.0, 6.0)];
        a.titles = vec![title("Hi {there}", 3.0, 4.0), title("cut away", 0.0, 1.0)];
        let p = Project { clips: vec![a], ..Project::default() };
        let ass = titles_ass(&p.clips, &p.layout(&[10.0]), 1080, 1920);
        assert!(ass.contains("Dialogue: 1,0:00:01.00,0:00:02.00,Title,,0,0,0,,{\\an5\\pos(540,384)\\fnImpact\\fs108\\bord8\\1c&H0000CCFF&\\fad(150,150)}Hi (there)"), "{ass}");
        // The second title sits entirely in the cut part.
        assert!(!ass.contains("cut away"));
        assert_eq!(ass_color("nonsense"), "&H00FFFFFF");
    }
}
