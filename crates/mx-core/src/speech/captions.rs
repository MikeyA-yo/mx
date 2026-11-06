//! Burn-in caption styling: short phrases in big outlined text, the look
//! of Shorts/Reels/TikTok captions, written as an ASS subtitle file for
//! libass to draw.
//!
//! The whisper filter only reports segment times, not word times. Word
//! times are estimated when transcribing (each word gets a share of the
//! segment's voiced time by length; see
//! [`super::transcribe::estimate_words`]), and phrases are timed from
//! their words. Speech is close enough to constant-rate within one
//! segment (a few seconds) for that to stay in step with the voice.

use serde::{Deserialize, Serialize};

use super::transcribe::{Segment, Transcript, Word, estimate_words};

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
    /// Colour (`#rrggbb`) for the word being spoken, karaoke style;
    /// `None` keeps every word white.
    #[serde(default)]
    pub highlight: Option<String>,
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
            highlight: None,
        }
    }
}

/// Split `transcript` into on-screen phrases. A phrase also ends after
/// punctuation that closes a clause, so captions follow the speech's
/// rhythm. Each phrase carries its words, timed from the segment's word
/// estimates when it has them, else by sharing the segment by length.
/// Within a segment phrases follow on without gaps, so a pause keeps the
/// last phrase on screen instead of flashing nothing.
pub fn cues(transcript: &Transcript, style: &CaptionStyle) -> Vec<Segment> {
    let mut out = Vec::new();
    for seg in &transcript.segments {
        let count = seg.text.split_whitespace().count();
        let words = if seg.words.len() == count { seg.words.clone() } else { estimate_words(seg.start, seg.end, &seg.text, &[]) };
        let mut phrases: Vec<Vec<Word>> = Vec::new();
        let mut current: Vec<Word> = Vec::new();
        for word in words {
            let chars: usize = current.iter().map(|w| w.text.chars().count() + 1).sum::<usize>() + word.text.chars().count();
            if !current.is_empty() && (current.len() >= style.max_words || chars > style.max_chars) {
                phrases.push(std::mem::take(&mut current));
            }
            let closes = word.text.ends_with(['.', ',', '?', '!', ';', ':']);
            current.push(word);
            if closes {
                phrases.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            phrases.push(current);
        }

        let first = out.len();
        for words in phrases {
            let text = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
            out.push(Segment { start: words[0].start, end: words[words.len() - 1].end, text, words });
        }
        // Cover the whole segment, each phrase lasting until the next.
        let n = out.len();
        if n > first {
            out[first].start = out[first].start.min(seg.start);
            for i in first..n - 1 {
                out[i].end = out[i + 1].start;
            }
            out[n - 1].end = out[n - 1].end.max(seg.end);
        }
    }
    out
}

/// `#rrggbb` -> ASS `&H00BBGGRR` (white if it doesn't parse).
pub(crate) fn ass_color(hex: &str) -> String {
    let h = hex.trim_start_matches('#');
    if h.len() == 6 && u32::from_str_radix(h, 16).is_ok() {
        format!("&H00{}{}{}", &h[4..6], &h[2..4], &h[0..2]).to_uppercase()
    } else {
        "&H00FFFFFF".to_string()
    }
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
        let line = |start: f64, end: f64, pop: &str, text: &str| {
            format!("Dialogue: 0,{},{},Caption,,0,0,0,,{pop}{text}\n", ass_time(start), ass_time(end))
        };
        match &style.highlight {
            // One line per word, the phrase redrawn with that word in the
            // highlight colour, from the word's start to the next's.
            Some(color) if c.words.len() > 1 => {
                let on = ass_color(color);
                for (i, w) in c.words.iter().enumerate() {
                    let start = if i == 0 { c.start } else { w.start };
                    let end = c.words.get(i + 1).map_or(c.end, |next| next.start);
                    let text: Vec<String> = c
                        .words
                        .iter()
                        .enumerate()
                        .map(|(j, w)| {
                            let t = ass_text(&w.text, style.uppercase);
                            if j == i { format!(r"{{\1c{on}&}}{t}{{\1c&H00FFFFFF&}}") } else { t }
                        })
                        .collect();
                    ass += &line(start, end.max(start), if i == 0 { pop } else { "" }, &text.join(" "));
                }
            }
            _ => ass += &line(c.start, c.end, pop, &ass_text(&c.text, style.uppercase)),
        }
    }
    ass
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transcript(start: f64, end: f64, text: &str) -> Transcript {
        Transcript { segments: vec![Segment { start, end, text: text.into(), words: Vec::new() }] }
    }

    #[test]
    fn phrases_break_on_word_count_length_and_punctuation() {
        let style = CaptionStyle::for_frame(1080, 1920);
        let t = transcript(10.0, 14.0, "So, this is a test of how captions get split up nicely");
        let texts: Vec<String> = cues(&t, &style).into_iter().map(|c| c.text).collect();
        assert_eq!(texts, ["So,", "this is a test", "of how captions get", "split up nicely"]);
    }

    #[test]
    fn phrase_times_split_the_segment_by_length_and_cover_it() {
        let style = CaptionStyle::for_frame(1920, 1080);
        let c = cues(&transcript(2.0, 4.0, "Hello there, everyone"), &style);
        assert_eq!(c.len(), 2);
        assert!((c[0].start - 2.0).abs() < 1e-9 && (c[1].end - 4.0).abs() < 1e-9);
        assert!((c[0].end - c[1].start).abs() < 1e-9);
        // Words weigh their length plus one: "Hello there," is 13 of 22.
        assert!((c[0].end - (2.0 + 2.0 * 13.0 / 22.0)).abs() < 1e-9, "{c:?}");
        assert_eq!(c[0].words.len(), 2);
    }

    #[test]
    fn highlight_draws_a_line_per_word_with_that_word_coloured() {
        let style = CaptionStyle { highlight: Some("#ffd400".into()), ..CaptionStyle::for_frame(1080, 1920) };
        let c = cues(&transcript(0.0, 1.0, "big red dog"), &style);
        let ass = to_ass(&c, 1080, 1920, &style);
        let lines: Vec<&str> = ass.lines().filter(|l| l.starts_with("Dialogue")).collect();
        assert_eq!(lines.len(), 3, "{ass}");
        assert!(lines[1].ends_with(r",,big {\1c&H0000D4FF&}red{\1c&H00FFFFFF&} dog"), "{ass}");
        // Only the first word's line pops in.
        assert!(lines[0].contains(r"\fscx85") && !lines[1].contains(r"\fscx85"), "{ass}");
    }

    #[test]
    fn ass_has_scaled_style_and_escaped_text() {
        let style = CaptionStyle { uppercase: true, ..CaptionStyle::for_frame(1080, 1920) };
        let ass = to_ass(&[Segment { start: 61.234, end: 62.5, text: r"a {b} c\d".into(), words: Vec::new() }], 1080, 1920, &style);
        // 7.5% of 1080 = 81 px; bottom at 72% of 1920 -> 538 px margin.
        assert!(ass.contains("Style: Caption,Arial,81,"), "{ass}");
        assert!(ass.contains(",2,65,65,538,1\n"), "{ass}");
        assert!(ass.contains(r"Dialogue: 0,0:01:01.23,0:01:02.50,Caption,,0,0,0,,{\fscx85\fscy85\t(0,90,\fscx100\fscy100)}A (B) C/D"), "{ass}");
    }
}
