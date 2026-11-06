//! Subtitle files: writing transcripts as SRT, WebVTT or plain text, and
//! reading SRT/WebVTT back (to burn in captions someone already has).

use anyhow::{Result, bail};

use super::transcribe::{Segment, Transcript};

/// `hh:mm:ss<sep>mmm`
fn timestamp(secs: f64, sep: char) -> String {
    let ms = (secs.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}{sep}{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

pub fn to_srt(t: &Transcript) -> String {
    t.segments
        .iter()
        .enumerate()
        .map(|(i, s)| format!("{}\n{} --> {}\n{}\n", i + 1, timestamp(s.start, ','), timestamp(s.end, ','), s.text))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn to_vtt(t: &Transcript) -> String {
    let cues: String = t
        .segments
        .iter()
        .map(|s| format!("\n{} --> {}\n{}\n", timestamp(s.start, '.'), timestamp(s.end, '.'), s.text))
        .collect();
    format!("WEBVTT\n{cues}")
}

pub fn to_text(t: &Transcript) -> String {
    t.segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n") + "\n"
}

/// `01:02:03,456`, `01:02:03.456` or WebVTT's short `02:03.456`.
fn parse_timestamp(s: &str) -> Option<f64> {
    let s = s.trim().replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    let (h, m, sec) = match parts.as_slice() {
        [h, m, sec] => (h.parse::<f64>().ok()?, m.parse::<f64>().ok()?, sec.parse::<f64>().ok()?),
        [m, sec] => (0.0, m.parse::<f64>().ok()?, sec.parse::<f64>().ok()?),
        _ => return None,
    };
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// Drop markup such as `<i>`, `<b>` and VTT's `<00:01.000>` word tags.
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// Parse SRT or WebVTT. Cue numbers, headers, NOTE blocks and cue
/// settings (`align:start` after the times) are ignored.
pub fn parse(text: &str) -> Result<Transcript> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut segments = Vec::new();
    for block in text.split("\n\n") {
        let mut lines = block.lines().skip_while(|l| !l.contains("-->"));
        let Some(times) = lines.next() else { continue };
        let Some((a, b)) = times.split_once("-->") else { continue };
        let b = b.split_whitespace().next().unwrap_or("");
        let (Some(start), Some(end)) = (parse_timestamp(a), parse_timestamp(b)) else { continue };
        let body: Vec<String> = lines.map(strip_tags).filter(|l| !l.trim().is_empty()).collect();
        if !body.is_empty() {
            segments.push(Segment { start, end, text: body.join(" ").trim().to_string(), words: Vec::new() });
        }
    }
    if segments.is_empty() {
        bail!("no subtitle cues found (expected SRT or WebVTT)");
    }
    Ok(Transcript { segments })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Transcript {
        Transcript {
            segments: vec![
                Segment { start: 1.5, end: 3.25, text: "Hello there.".into(), words: Vec::new() },
                Segment { start: 3661.5, end: 3662.0, text: "An hour in.".into(), words: Vec::new() },
            ],
        }
    }

    #[test]
    fn writes_srt_and_vtt() {
        let srt = to_srt(&sample());
        assert_eq!(srt, "1\n00:00:01,500 --> 00:00:03,250\nHello there.\n\n2\n01:01:01,500 --> 01:01:02,000\nAn hour in.\n");
        let vtt = to_vtt(&sample());
        assert!(vtt.starts_with("WEBVTT\n\n00:00:01.500 --> 00:00:03.250\nHello there.\n"), "{vtt}");
    }

    #[test]
    fn round_trips_and_reads_real_world_vtt() {
        let back = parse(&to_srt(&sample())).unwrap();
        assert_eq!(back.segments, sample().segments);

        let vtt = "\u{feff}WEBVTT\r\nKind: captions\r\n\r\nNOTE made by hand\r\n\r\n\
                   00:01.000 --> 00:02.500 align:start position:0%\r\n<i>Two</i> <00:01.500>lines\r\nof text\r\n";
        let t = parse(vtt).unwrap();
        assert_eq!(t.segments, [Segment { start: 1.0, end: 2.5, text: "Two lines of text".into(), words: Vec::new() }]);
        assert!(parse("not subtitles").is_err());
    }
}
