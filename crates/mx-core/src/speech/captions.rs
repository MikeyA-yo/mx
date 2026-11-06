//! Burn-in caption styling: short phrases in big outlined text, the look
//! of Shorts/Reels/TikTok captions, written as an ASS subtitle file for
//! libass to draw.
//!
//! The whisper filter only reports segment times, not word times, so a
//! segment's duration is shared among its phrases by character count.
//! Speech is close enough to constant-rate within one segment (a few
//! seconds) for that to stay in step with the voice.

use serde::{Deserialize, Serialize};

use super::transcribe::{Segment, Transcript};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptionStyle {
    /// Most words on screen at once.
    pub max_words: usize,
    /// Most characters on screen at once (long words make fewer fit).
    pub max_chars: usize,
    /// Text height as a fraction of the frame's shorter side.
    pub size: f64,
    /// Where the bottom of the text sits, 0 (top) to 1 (bottom).
    pub position: f64,
    pub font: String,
    pub uppercase: bool,
    /// Each phrase pops in, growing from 85% to full size.
    pub pop: bool,
}

impl CaptionStyle {
    /// Defaults for a `width`x`height` frame: higher up on portrait video,
    /// clear of the like/comment buttons and caption overlay on Shorts.
    pub fn for_frame(width: u32, height: u32) -> Self {
        Self {
            max_words: 4,
            max_chars: 22,
            size: 0.075,
            position: if height > width { 0.72 } else { 0.9 },
            font: "Arial".to_string(),
            uppercase: false,
            pop: true,
        }
    }
}

/// Split `transcript` into on-screen phrases. A phrase also ends after
/// punctuation that closes a clause, so captions follow the speech's rhythm.
pub fn cues(transcript: &Transcript, style: &CaptionStyle) -> Vec<Segment> {
    let mut out = Vec::new();
    for seg in &transcript.segments {
        let words: Vec<&str> = seg.text.split_whitespace().collect();
        let mut phrases: Vec<Vec<&str>> = Vec::new();
        let mut current: Vec<&str> = Vec::new();
        for word in words {
            let chars: usize = current.iter().map(|w| w.chars().count() + 1).sum::<usize>() + word.chars().count();
            if !current.is_empty() && (current.len() >= style.max_words || chars > style.max_chars) {
                phrases.push(std::mem::take(&mut current));
            }
            current.push(word);
            if word.ends_with(['.', ',', '?', '!', ';', ':']) {
                phrases.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            phrases.push(current);
        }

        let texts: Vec<String> = phrases.iter().map(|p| p.join(" ")).collect();
        let total_chars: usize = texts.iter().map(|t| t.chars().count()).sum();
        if total_chars == 0 {
            continue;
        }
        let mut t = seg.start;
        for text in texts {
            let share = text.chars().count() as f64 / total_chars as f64;
            let end = t + share * (seg.end - seg.start);
            out.push(Segment { start: t, end, text });
            t = end;
        }
    }
    out
}

/// `h:mm:ss.cc`, ASS's centisecond timestamps.
pub(crate) fn ass_time(secs: f64) -> String {
    let cs = (secs.max(0.0) * 100.0).round() as u64;
    format!("{}:{:02}:{:02}.{:02}", cs / 360_000, cs / 6000 % 60, cs / 100 % 60, cs % 100)
}

/// ASS treats `{...}` as override tags and `\` as an escape; neither may
/// reach libass from transcribed text.
pub(crate) fn ass_text(s: &str, uppercase: bool) -> String {
    let s = s.replace('\\', "/").replace('{', "(").replace('}', ")");
    if uppercase { s.to_uppercase() } else { s }
}

/// A complete ASS file drawing `cues` on a `width`x`height` frame.
pub fn to_ass(cues: &[Segment], width: u32, height: u32, style: &CaptionStyle) -> String {
    let short = width.min(height) as f64;
    let size = (short * style.size).round().max(8.0);
    let outline = (size * 0.08).round().max(1.0);
    let shadow = (size * 0.04).round();
    let margin_v = ((1.0 - style.position.clamp(0.0, 1.0)) * height as f64).round();
    let margin_h = (width as f64 * 0.06).round();
    let pop = if style.pop { r"{\fscx85\fscy85\t(0,90,\fscx100\fscy100)}" } else { "" };
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
         Style: Caption,{font},{size},&H00FFFFFF,&H00FFFFFF,&H00000000,&H80000000,\
         -1,0,0,0,100,100,0,0,1,{outline},{shadow},2,{margin_h},{margin_h},{margin_v},1\n\
         \n\
         [Events]\n\
         Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        font = style.font
    );
    for c in cues {
        ass += &format!(
            "Dialogue: 0,{},{},Caption,,0,0,0,,{pop}{}\n",
            ass_time(c.start),
            ass_time(c.end),
            ass_text(&c.text, style.uppercase)
        );
    }
    ass
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transcript(start: f64, end: f64, text: &str) -> Transcript {
        Transcript { segments: vec![Segment { start, end, text: text.into() }] }
    }

    #[test]
    fn phrases_break_on_word_count_length_and_punctuation() {
        let style = CaptionStyle::for_frame(1080, 1920);
        let t = transcript(10.0, 14.0, "So, this is a test of how captions get split up nicely");
        let texts: Vec<String> = cues(&t, &style).into_iter().map(|c| c.text).collect();
        assert_eq!(texts, ["So,", "this is a test", "of how captions get", "split up nicely"]);
    }

    #[test]
    fn phrase_times_split_the_segment_by_characters_and_cover_it() {
        let style = CaptionStyle::for_frame(1920, 1080);
        let c = cues(&transcript(2.0, 4.0, "Hello there, everyone"), &style);
        assert_eq!(c.len(), 2);
        assert!((c[0].start - 2.0).abs() < 1e-9 && (c[1].end - 4.0).abs() < 1e-9);
        assert!((c[0].end - c[1].start).abs() < 1e-9);
        // "Hello there," is 12 of 20 characters.
        assert!((c[0].end - 3.2).abs() < 1e-9, "{c:?}");
    }

    #[test]
    fn ass_has_scaled_style_and_escaped_text() {
        let style = CaptionStyle { uppercase: true, ..CaptionStyle::for_frame(1080, 1920) };
        let ass = to_ass(&[Segment { start: 61.234, end: 62.5, text: r"a {b} c\d".into() }], 1080, 1920, &style);
        // 7.5% of 1080 = 81 px; bottom at 72% of 1920 -> 538 px margin.
        assert!(ass.contains("Style: Caption,Arial,81,"), "{ass}");
        assert!(ass.contains(",2,65,65,538,1\n"), "{ass}");
        assert!(ass.contains(r"Dialogue: 0,0:01:01.23,0:01:02.50,Caption,,0,0,0,,{\fscx85\fscy85\t(0,90,\fscx100\fscy100)}A (B) C/D"), "{ass}");
    }
}
