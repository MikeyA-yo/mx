//! A timeline of several clips with music and text on top: what the
//! desktop editor builds and [`crate::ffmpeg::ops::render_project`]
//! renders in one encode.
//!
//! Clips play one after another. Each clip keeps parts of its source, and
//! anything attached to a clip (titles, its transcript) is timed on that
//! source, so reordering or recutting clips never strands a title in the
//! wrong place. [`Project::layout`] works out where everything lands on
//! the output timeline.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ffmpeg::ops::Frame;
use crate::speech::captions::{ass_color, ass_text, ass_time};
use crate::speech::{CaptionStyle, Transcript};
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
    /// Burn the clips' transcripts in as captions, in this style.
    #[serde(default)]
    pub captions: Option<CaptionStyle>,
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
    /// Background noise reduction on the clip's own sound.
    #[serde(default)]
    pub denoise: Denoise,
    /// Text on screen, timed on this clip's source.
    #[serde(default)]
    pub titles: Vec<Title>,
    /// What's said, timed on this clip's source, for captions.
    #[serde(default)]
    pub transcript: Option<Transcript>,
    /// How this clip takes over from the one before it. `None` is a cut.
    #[serde(default)]
    pub transition: Option<Transition>,
    /// Zoom and pan over time, eased between keyframes. Empty = still.
    #[serde(default)]
    pub motion: Vec<Keyframe>,
    /// Audio that starts at a moment of this clip: a voiceover, an effect.
    #[serde(default)]
    pub sounds: Vec<Sound>,
    /// Video shown over the picture from a moment of this clip: b-roll
    /// while the clip's voice carries on, or a picture-in-picture.
    #[serde(default)]
    pub overlays: Vec<Overlay>,
}

/// How long an image layer shows when no length is given.
pub const IMAGE_SECONDS: f64 = 3.0;

/// Whether `path` is a still image, going by its extension. (A GIF moves,
/// so it counts as a video.)
pub fn is_image(path: &Path) -> bool {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    ["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff"].contains(&ext.as_str())
}

/// How much background noise to take out of a clip's sound: see
/// [`crate::ffmpeg::ops::denoise`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Denoise {
    #[default]
    Off,
    /// Learns the noise from a pause and takes some of it away; the voice
    /// stays natural.
    Light,
    /// A speech-trained neural network: much quieter background.
    Strong,
}

/// A video or an image laid over the main picture, started at a moment of
/// a clip and shown for its length, across later clips like a [`Sound`].
/// An image holds still, and keeps any transparency it has (a PNG logo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Overlay {
    pub source: PathBuf,
    /// When it starts, in its clip's source time.
    pub at: f64,
    /// Where in the file to start, seconds (videos only).
    #[serde(default)]
    pub offset: f64,
    /// How much to show; `None` plays a video to its end, and shows an
    /// image for [`IMAGE_SECONDS`].
    #[serde(default)]
    pub duration: Option<f64>,
    /// `None` covers the frame (fitted like the clips); `Some` shows it in
    /// a box.
    #[serde(default)]
    pub pip: Option<Pip>,
    /// Its own sound. 0 (the default) keeps it silent, so the voice
    /// underneath carries on.
    #[serde(default)]
    pub volume: f64,
    /// Fade in and out, seconds.
    #[serde(default = "overlay_fade")]
    pub fade: f64,
}

fn overlay_fade() -> f64 {
    0.2
}

/// A picture-in-picture box.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pip {
    /// Centre, 0..1 across and down the frame.
    pub x: f64,
    pub y: f64,
    /// Width as a fraction of the frame's.
    pub size: f64,
}

/// A piece of audio started at a moment of a clip. It plays on from there
/// to its own end, across later clips, cuts and transitions: a voiceover
/// keeps going while the pictures change. Anchoring it to a clip keeps it
/// in sync with that clip when clips are reordered or recut.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sound {
    pub source: PathBuf,
    /// When it starts, in its clip's source time.
    pub at: f64,
    /// Where in the file to start, seconds.
    #[serde(default)]
    pub offset: f64,
    /// How much of the file to play; `None` plays to its end.
    #[serde(default)]
    pub duration: Option<f64>,
    /// Linear gain; 1 is as recorded.
    #[serde(default = "one")]
    pub volume: f64,
    #[serde(default)]
    pub fade_in: f64,
    #[serde(default)]
    pub fade_out: f64,
}

/// One point of a clip's zoom/pan path. Between keyframes the view eases
/// (slow in, slow out); before the first and after the last it holds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// In the clip's source time.
    pub time: f64,
    /// 1 shows the whole picture; 2 shows half its width.
    pub zoom: f64,
    /// Centre of the view, 0..1 across and down the picture.
    pub x: f64,
    pub y: f64,
}

fn one() -> f64 {
    1.0
}

impl Clip {
    pub fn new(source: impl Into<PathBuf>) -> Self {
        Self {
            source: source.into(),
            keep: Vec::new(),
            volume: 1.0,
            denoise: Denoise::Off,
            titles: Vec::new(),
            transcript: None,
            transition: None,
            motion: Vec::new(),
            sounds: Vec::new(),
            overlays: Vec::new(),
        }
    }
}

/// The picture and sound of two clips overlapping as one hands over to
/// the next. It overlaps them, so it shortens the video by its duration.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub kind: TransitionKind,
    /// Seconds.
    pub duration: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransitionKind {
    /// Crossfade (dissolve).
    Fade,
    /// Fade through black.
    FadeBlack,
    WipeLeft,
    WipeRight,
    SlideLeft,
    SlideRight,
    CircleOpen,
    ZoomIn,
}

impl TransitionKind {
    /// The name ffmpeg's `xfade` filter knows it by.
    pub fn xfade(self) -> &'static str {
        match self {
            Self::Fade => "fade",
            Self::FadeBlack => "fadeblack",
            Self::WipeLeft => "wipeleft",
            Self::WipeRight => "wiperight",
            Self::SlideLeft => "slideleft",
            Self::SlideRight => "slideright",
            Self::CircleOpen => "circleopen",
            Self::ZoomIn => "zoomin",
        }
    }
}

/// Shortest clip a transition may eat into: at most half of either clip
/// overlaps, so a 1 s transition between 1 s clips becomes 0.5 s.
fn overlap(t: Option<Transition>, before: f64, after: f64) -> f64 {
    let d = t.map_or(0.0, |t| t.duration).min(before / 2.0).min(after / 2.0);
    if d < 0.04 { 0.0 } else { d }
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
    /// How long it overlaps the clip before it (its transition in).
    pub overlap: f64,
}

impl Placement {
    pub fn duration(&self) -> f64 {
        total(&self.keep)
    }

    /// Source range -> the output ranges it plays at.
    pub fn place(&self, r: Range) -> Vec<Range> {
        map_range(&self.keep, r).into_iter().map(|p| Range::new(p.start + self.offset, p.end + self.offset)).collect()
    }

    /// Source moment -> the output moment it plays at. A moment that was
    /// cut snaps to the next kept one (or the clip's end).
    pub fn at(&self, t: f64) -> f64 {
        let mut out = 0.0;
        for k in &self.keep {
            if t < k.start {
                break;
            }
            if t < k.end {
                return self.offset + out + t - k.start;
            }
            out += k.len();
        }
        self.offset + out
    }
}

impl Project {
    /// Each clip's place on the output timeline, given the source
    /// durations (in clip order).
    pub fn layout(&self, durations: &[f64]) -> Vec<Placement> {
        let keeps = self
            .clips
            .iter()
            .zip(durations)
            .map(|(clip, &d)| if clip.keep.is_empty() { vec![Range::new(0.0, d)] } else { clip.keep.clone() })
            .collect();
        self.arrange(keeps)
    }

    /// Place clips that keep `keeps` (one list per clip) back to back,
    /// each overlapping the one before by its transition.
    pub fn arrange(&self, keeps: Vec<Vec<Range>>) -> Vec<Placement> {
        let mut out: Vec<Placement> = Vec::with_capacity(keeps.len());
        for (clip, keep) in self.clips.iter().zip(keeps) {
            let len = total(&keep);
            let (offset, overlap) = match out.last() {
                None => (0.0, 0.0),
                Some(prev) => {
                    let o = overlap(clip.transition, prev.duration(), len);
                    (prev.offset + prev.duration() - o, o)
                }
            };
            out.push(Placement { offset, keep, overlap });
        }
        out
    }
}

/// How long `layout` plays for.
pub fn length(layout: &[Placement]) -> f64 {
    layout.last().map_or(0.0, |p| p.offset + p.duration())
}

/// Per-clip transcripts (source-timed, `None` for clips without one) as
/// one transcript on the output timeline.
pub fn output_transcript(layout: &[Placement], transcripts: &[Option<Transcript>]) -> Transcript {
    let mut segments = Vec::new();
    for (place, t) in layout.iter().zip(transcripts) {
        let Some(t) = t else { continue };
        segments.extend(t.segments.iter().filter_map(|s| s.placed(|r| place.place(r))));
    }
    Transcript { segments }
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
    use crate::speech::Segment;

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
        assert_eq!(layout[1], Placement { offset: 3.0, keep: vec![r(0.0, 4.0)], overlap: 0.0 });
        // Source 5.5 in clip a plays at 2.5; source 1 in clip b at 4.
        assert_eq!(layout[0].place(r(5.5, 5.6))[0].start, 2.5);
        assert_eq!(layout[1].place(r(1.0, 2.0)), [r(4.0, 5.0)]);
    }

    #[test]
    fn transitions_overlap_clips_and_are_capped_at_half_a_clip() {
        let fade = |duration| Some(Transition { kind: TransitionKind::Fade, duration });
        let mut b = Clip::new("b");
        b.transition = fade(1.0);
        let mut c = Clip::new("c");
        c.transition = fade(5.0);
        let p = Project { clips: vec![Clip::new("a"), b, c], ..Project::default() };
        let layout = p.layout(&[4.0, 3.0, 2.0]);
        // b starts 1 s before a ends; c's 5 s ask is capped at half of c.
        assert_eq!((layout[1].offset, layout[1].overlap), (3.0, 1.0));
        assert_eq!((layout[2].offset, layout[2].overlap), (5.0, 1.0));
        assert_eq!(length(&layout), 7.0);
    }

    #[test]
    fn moments_in_cut_time_snap_to_the_next_kept_moment() {
        let p = Placement { offset: 10.0, keep: vec![r(0.0, 2.0), r(5.0, 6.0)], overlap: 0.0 };
        assert_eq!(p.at(1.5), 11.5);
        assert_eq!(p.at(3.0), 12.0);
        assert_eq!(p.at(5.5), 12.5);
        assert_eq!(p.at(9.0), 13.0);
    }

    #[test]
    fn transcripts_join_on_the_output_timeline() {
        let p = Project { clips: vec![Clip::new("a"), Clip::new("b")], ..Project::default() };
        let layout = p.layout(&[3.0, 3.0]);
        let t = |s: f64| Some(Transcript { segments: vec![Segment { start: s, end: s + 1.0, text: "x".into(), words: Vec::new() }] });
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
