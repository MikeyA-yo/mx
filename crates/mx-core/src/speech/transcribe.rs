//! Speech to text with ffmpeg's `whisper` filter (whisper.cpp).
//!
//! whisper.cpp always works on a 30 s window, however little audio it's
//! given, and the filter hands it whatever has queued up (3 s by default).
//! So the defaults cost a full window per 3 s of audio (about realtime on
//! a laptop), and every 3 s piece is transcribed without context: words
//! repeat or break at the seams ("epoll" came out as "E-Pole"). The
//! filter's own voice detection fixes the seams, but it re-scans the whole
//! queue for every incoming audio frame and ran at 0.05x realtime.
//!
//! Instead the audio is cut into chunks of up to 29 s at pauses (from the
//! loudness envelope), each chunk is padded with silence to exactly 30 s,
//! and the queue is set to 30 s. Every whisper call then sees one whole
//! chunk, and chunk `i` occupies `[30i, 30i + 30)` of the padded stream, so
//! its timestamps map straight back. One ffmpeg process, one model load:
//! 2 minutes of speech took 31 s with base.en on an Intel iGPU (118 s on
//! the CPU alone).

use std::path::Path;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::analysis::{SilenceOptions, find_silences, measure_loudness};
use crate::cancel::CancelToken;
use crate::ffmpeg::{Job, Progress, filter_path};
use crate::temp::TempFile;
use crate::timeline::Range;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Transcript {
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone)]
pub struct TranscribeOptions {
    /// "auto" to detect (multilingual models only), or a code like "en".
    pub language: String,
    pub use_gpu: bool,
}

/// whisper.cpp's fixed window.
const WINDOW: f64 = 30.0;
/// Longest chunk; the margin keeps a chunk plus resampling slack under the window.
const MAX_CHUNK: f64 = 29.0;
/// A pause is only used as a cut if the chunk before it is at least this long.
const MIN_CHUNK: f64 = 8.0;
/// Pauses at least this far into a chunk are preferred.
const PREFERRED_CHUNK: f64 = 15.0;
/// Silence kept at each side of a cut, so word edges aren't clipped.
const CUT_PADDING: f64 = 0.25;
/// Audio frames are made this many samples (0.1 s at 16 kHz). The filter
/// flushes when the next frame wouldn't fit, so frames that divide 30 s
/// make each flush exactly one padded chunk.
const FRAME_SAMPLES: u32 = 1600;

/// Split `[0, duration]` into chunks of at most `MAX_CHUNK` seconds, cutting
/// inside pauses where possible and leaving out the silent part of each
/// pause. Chunks with (almost) no sound in them are dropped: whisper invents
/// text for silence ("Thank you.").
fn plan_chunks(duration: f64, silences: &[Range]) -> Vec<Range> {
    let mut chunks = Vec::new();
    let mut cursor = 0.0;
    // Start after a leading silence.
    if let Some(s) = silences.iter().find(|s| s.start <= 0.0) {
        cursor = (s.end - CUT_PADDING).max(0.0);
    }
    while duration - cursor > 0.05 {
        if duration - cursor <= MAX_CHUNK {
            chunks.push(Range::new(cursor, duration));
            break;
        }
        let limit = cursor + MAX_CHUNK;
        // The longest pause is the likeliest end of a sentence (the last
        // one was often a breath mid-phrase, and whisper then ended the
        // chunk with an invented full stop). Pauses past PREFERRED_CHUNK
        // come first, so chunks stay long and whisper calls few.
        let fits = |s: &&Range, from: f64| s.start - cursor >= from && s.start + CUT_PADDING <= limit;
        let longest = |from: f64| silences.iter().filter(|s| fits(s, from)).max_by(|a, b| a.len().total_cmp(&b.len()));
        let pause = longest(PREFERRED_CHUNK).or_else(|| longest(MIN_CHUNK));
        match pause {
            Some(s) => {
                let mid = (s.start + s.end) / 2.0;
                chunks.push(Range::new(cursor, (s.start + CUT_PADDING).min(mid)));
                cursor = (s.end - CUT_PADDING).max(mid);
            }
            None => {
                chunks.push(Range::new(cursor, limit));
                cursor = limit;
            }
        }
    }
    let silent_in = |c: &Range| -> f64 {
        silences.iter().map(|s| (s.end.min(c.end) - s.start.max(c.start)).max(0.0)).sum()
    };
    chunks.retain(|c| c.len() - silent_in(c) >= 0.3);
    chunks
}

fn filter_script(chunks: &[Range], model: &Path, opts: &TranscribeOptions) -> Result<String> {
    let n = chunks.len();
    let pad_to = (WINDOW * 16000.0) as u32;
    let mut g = String::from("[0:a:0]aformat=sample_fmts=fltp:channel_layouts=mono,aresample=16000,");
    g += &format!("asplit={n}{};\n", (0..n).map(|i| format!("[s{i}]")).collect::<String>());
    for (i, c) in chunks.iter().enumerate() {
        g += &format!(
            "[s{i}]atrim=start={:.6}:end={:.6},asetpts=PTS-STARTPTS,apad=whole_len={pad_to}[p{i}];\n",
            c.start, c.end
        );
    }
    g += &(0..n).map(|i| format!("[p{i}]")).collect::<String>();
    g += &format!(
        "concat=n={n}:v=0:a=1,asetnsamples=n={FRAME_SAMPLES},\
         whisper=model={}:language={}:use_gpu={}:queue={WINDOW}:format=json:destination=-[out]",
        filter_path(model)?,
        opts.language,
        u8::from(opts.use_gpu)
    );
    Ok(g)
}

/// One line of the filter's JSON output. The filter doesn't escape the
/// text (a quote in speech would make it invalid JSON), so the line is
/// taken apart by its fixed shape instead of with a JSON parser.
fn parse_line(line: &str) -> Option<(i64, i64, &str)> {
    let rest = line.trim_end().strip_prefix("{\"start\":")?;
    let (start, rest) = rest.split_once(",\"end\":")?;
    let (end, rest) = rest.split_once(",\"text\":\"")?;
    let text = rest.strip_suffix("\"}")?;
    Some((start.parse().ok()?, end.parse().ok()?, text))
}

/// Map padded-stream segments back onto the source timeline.
fn map_segments<'a>(lines: impl IntoIterator<Item = &'a str>, chunks: &[Range]) -> Vec<Segment> {
    let window_ms = (WINDOW * 1000.0) as i64;
    let mut segments = Vec::new();
    for (start_ms, end_ms, text) in lines.into_iter().filter_map(parse_line) {
        let text = text.trim();
        let index = (start_ms / window_ms) as usize;
        let Some(chunk) = chunks.get(index) else { continue };
        let offset = (index as i64 * window_ms) as f64 / 1000.0;
        let start = chunk.start + start_ms as f64 / 1000.0 - offset;
        let end = (chunk.start + end_ms as f64 / 1000.0 - offset).min(chunk.end);
        // Text placed in the padding is whisper inventing words for silence.
        if text.is_empty() || start >= chunk.end - 0.05 {
            continue;
        }
        segments.push(Segment { start, end: end.max(start), text: text.to_string() });
    }
    segments
}

/// Transcribe the first audio track of `input`. `on_progress(stage, p)`
/// reports "listen" (finding pauses), then "transcribe".
pub fn transcribe(
    input: &Path,
    model: &Path,
    opts: &TranscribeOptions,
    cancel: &CancelToken,
    mut on_progress: impl FnMut(&str, &Progress),
) -> Result<Transcript> {
    let envelope = measure_loudness(input, cancel, |p| on_progress("listen", p))?;
    let duration = envelope.points.last().map_or(0.0, |&(t, _)| t + crate::analysis::loudness::WINDOW_SECS);
    // Shorter pauses than jump cuts use: any breath is a fine place to cut.
    let silences = find_silences(&envelope, &SilenceOptions { min_duration: 0.25, ..Default::default() }).silences;
    let chunks = plan_chunks(duration, &silences);
    if chunks.is_empty() {
        return Ok(Transcript::default());
    }

    // Hundreds of chunks overflow Windows' command-line limit, as in jump cuts.
    let script = TempFile::with_contents("whisper", "txt", &filter_script(&chunks, model, opts)?)?;
    let mut lines = Vec::new();
    let result = Job::new("-")
        .input(input)
        .arg("-/filter_complex")
        .arg(script.path())
        .args(["-map", "[out]", "-f", "null"])
        .duration(chunks.len() as f64 * WINDOW)
        .run_with_lines(cancel, |p| on_progress("transcribe", p), |line| {
            if line.starts_with("{\"start\"") {
                lines.push(line.to_string());
            }
        });
    if let Err(e) = result {
        if e.to_string().contains("whisper") || e.to_string().contains("model") {
            bail!("{e:#}\n  (is {} a whisper.cpp ggml model?)", model.display());
        }
        return Err(e);
    }
    Ok(Transcript { segments: map_segments(lines.iter().map(String::as_str), &chunks) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_cut_inside_pauses_and_skip_their_silence() {
        let silences = [Range::new(0.0, 2.0), Range::new(20.0, 21.0), Range::new(40.0, 40.5)];
        let chunks = plan_chunks(60.0, &silences);
        let r = |c: &Range| ((c.start * 100.0).round() / 100.0, (c.end * 100.0).round() / 100.0);
        let got: Vec<_> = chunks.iter().map(r).collect();
        // Leading silence skipped; cuts at 20.25 / 20.75 and 40.25 / 40.25.
        assert_eq!(got, [(1.75, 20.25), (20.75, 40.25), (40.25, 60.0)]);
        assert!(chunks.iter().all(|c| c.len() <= MAX_CHUNK));
    }

    #[test]
    fn the_longest_pause_wins_over_a_later_breath() {
        // A real sentence break at 18-19 s, a 0.3 s breath at 26 s.
        let chunks = plan_chunks(40.0, &[Range::new(18.0, 19.0), Range::new(26.0, 26.3)]);
        assert!((chunks[0].end - 18.25).abs() < 1e-9, "{chunks:?}");
    }

    #[test]
    fn speech_without_pauses_is_cut_hard_at_the_limit() {
        let chunks = plan_chunks(70.0, &[]);
        assert_eq!(chunks, [Range::new(0.0, 29.0), Range::new(29.0, 58.0), Range::new(58.0, 70.0)]);
    }

    #[test]
    fn silent_chunks_are_dropped() {
        // 0-25 speech, then 35 s of silence.
        let chunks = plan_chunks(60.0, &[Range::new(25.0, 60.0)]);
        assert_eq!(chunks.len(), 1, "{chunks:?}");
        assert!(chunks[0].start == 0.0 && chunks[0].end <= 29.0);
    }

    #[test]
    fn parses_unescaped_text_and_maps_times_back() {
        let chunks = [Range::new(1.75, 20.25), Range::new(20.75, 40.25)];
        // Captured shape from ffmpeg 8.0.1; the second line has a raw quote.
        let out = [
            "{\"start\":0,\"end\":4240,\"text\":\"actually remember what you've learned.\"}",
            "out_time_us=30000000",
            "{\"start\":30000,\"end\":33500,\"text\":\" He said \"hi\" twice.\"}",
            // Placed in chunk 1's padding (it ends 19.5 s into the window).
            "{\"start\":49800,\"end\":51000,\"text\":\"Thank you.\"}",
        ];
        let segments = map_segments(out, &chunks);
        assert_eq!(segments.len(), 2, "{segments:?}");
        assert_eq!(segments[0], Segment { start: 1.75, end: 1.75 + 4.24, text: "actually remember what you've learned.".into() });
        assert!((segments[1].start - 20.75).abs() < 1e-9 && (segments[1].end - 24.25).abs() < 1e-9);
        assert_eq!(segments[1].text, "He said \"hi\" twice.");
    }

    #[test]
    fn script_pads_every_chunk_to_the_window() {
        let opts = TranscribeOptions { language: "en".into(), use_gpu: true };
        let s = filter_script(&[Range::new(0.0, 10.0), Range::new(12.0, 20.0)], Path::new("m.bin"), &opts).unwrap();
        assert!(s.contains("asplit=2[s0][s1];"), "{s}");
        assert!(s.contains("[s1]atrim=start=12.000000:end=20.000000,asetpts=PTS-STARTPTS,apad=whole_len=480000[p1]"), "{s}");
        assert!(s.contains("[p0][p1]concat=n=2:v=0:a=1,asetnsamples=n=1600,whisper=model='m.bin':language=en:use_gpu=1:queue=30"), "{s}");
    }
}
