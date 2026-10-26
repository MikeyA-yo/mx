//! Human-readable terminal output.

use indicatif::HumanBytes;

use mx_core::ffmpeg::Probe;
use mx_core::platform::Platform;
use mx_core::ytdlp::model::{Format, MediaInfo};

pub fn duration(secs: f64) -> String {
    let s = secs.round() as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

fn row(label: &str, value: impl std::fmt::Display) {
    println!("  {label:<11} {value}");
}

pub fn info(info: &MediaInfo, platform: Platform) {
    let title = info.title.as_deref().unwrap_or("(untitled)");
    let extractor = info.extractor_key.as_deref().unwrap_or("?");
    println!("{title}");
    match platform {
        Platform::Other => row("site", format!("extractor: {extractor}")),
        p => row("site", format!("{} (extractor: {extractor})", p.name())),
    }
    row("id", &info.id);
    if let Some(who) = info.uploader.as_ref().or(info.channel.as_ref()) {
        row("uploader", who);
    }
    if let Some(date) = &info.upload_date {
        row("uploaded", date);
    }
    if let Some(d) = info.duration {
        row("duration", duration(d));
    }
    if let (Some(w), Some(h)) = (info.width, info.height) {
        row("resolution", format!("{w}x{h}"));
    }
    if let Some(v) = info.view_count {
        row("views", v);
    }
    if let Some(status) = info.live_status.as_deref().filter(|s| *s != "not_live") {
        row("live", status);
    }

    if info.is_playlist() {
        let entries: Vec<_> = info.entries.iter().flatten().collect();
        row("entries", info.playlist_count.unwrap_or(entries.len() as u64));
        for (i, e) in entries.iter().enumerate().take(20) {
            let dur = e.duration.map(duration).unwrap_or_default();
            println!("    {:>3}. {:<8} {}", i + 1, dur, e.title.as_deref().unwrap_or(&e.id));
        }
        if entries.len() > 20 {
            println!("    ... {} more", entries.len() - 20);
        }
        return;
    }

    let media: Vec<_> = info.formats.iter().filter(|f| f.is_media()).collect();
    let best_height = media.iter().filter_map(|f| f.height).max();
    let mut summary = format!("{} available", media.len());
    if let Some(h) = best_height {
        summary += &format!(", up to {h}p");
    }
    row("formats", summary);

    let mut subs: Vec<_> = info.subtitles.keys().map(String::as_str).collect();
    subs.sort_unstable();
    if !subs.is_empty() {
        row("subtitles", subs.join(", "));
    }
    if !info.automatic_captions.is_empty() {
        row("auto-subs", format!("{} languages", info.automatic_captions.len()));
    }
}

fn codec(c: &Option<String>) -> &str {
    match c.as_deref() {
        None | Some("none") => "-",
        // "avc1.640028" -> "avc1": the profile suffix is noise at a glance.
        Some(c) => c.split('.').next().unwrap_or(c),
    }
}

pub fn formats(formats: &[Format]) {
    println!(
        "{:<18} {:<5} {:>10} {:>5} {:<6} {:<6} {:>8} {:>10}  {:<11} NOTE",
        "ID", "EXT", "RES", "FPS", "VCODEC", "ACODEC", "KBPS", "SIZE", "PROTO"
    );
    for f in formats.iter().filter(|f| f.is_media()) {
        let res = match (f.width, f.height, f.has_video()) {
            (Some(w), Some(h), _) => format!("{w}x{h}"),
            (None, Some(h), _) => format!("{h}p"),
            (_, _, false) => "audio".into(),
            _ => "?".into(),
        };
        let fps = f.fps.map(|v| format!("{v:.0}")).unwrap_or_default();
        let tbr = f.tbr.map(|v| format!("{v:.0}")).unwrap_or_default();
        let size = f.size().map(|s| HumanBytes(s).to_string()).unwrap_or_default();
        let mut note = f.format_note.clone().unwrap_or_default();
        if let Some(hdr) = f.dynamic_range.as_deref().filter(|r| *r != "SDR") {
            note = format!("{note} {hdr}");
        }
        if let Some(lang) = &f.language {
            note = format!("{note} [{lang}]");
        }
        println!(
            "{:<18} {:<5} {:>10} {:>5} {:<6} {:<6} {:>8} {:>10}  {:<11} {}",
            f.format_id,
            f.ext.as_deref().unwrap_or("?"),
            res,
            fps,
            codec(&f.vcodec),
            codec(&f.acodec),
            tbr,
            size,
            f.protocol.as_deref().unwrap_or(""),
            note.trim()
        );
    }
}

pub fn probe(p: &Probe) {
    let f = &p.format;
    let mut line = vec![f.format_long_name.clone().unwrap_or_default()];
    if let Some(d) = f.duration.as_deref().and_then(|d| d.parse().ok()) {
        line.push(duration(d));
    }
    if let Some(size) = f.size.as_deref().and_then(|s| s.parse().ok()) {
        line.push(HumanBytes(size).to_string());
    }
    if let Some(br) = f.bit_rate.as_deref().and_then(|b| b.parse::<u64>().ok()) {
        line.push(format!("{} kb/s", br / 1000));
    }
    println!("  container  {}", line.join(" | "));

    for s in &p.streams {
        let kind = s.codec_type.as_deref().unwrap_or("?");
        let mut parts = vec![s.codec_name.clone().unwrap_or_default()];
        if let Some(profile) = &s.profile {
            parts.push(profile.clone());
        }
        match kind {
            "video" => {
                if let (Some(w), Some(h)) = (s.width, s.height) {
                    parts.push(format!("{w}x{h}"));
                }
                if let Some(fps) = s.fps() {
                    parts.push(format!("{fps:.2} fps"));
                }
                parts.extend(s.pix_fmt.clone());
            }
            "audio" => {
                parts.extend(s.sample_rate.as_ref().map(|r| format!("{r} Hz")));
                parts.extend(s.channels.map(|c| format!("{c} ch")));
            }
            _ => {}
        }
        if let Some(br) = s.bit_rate.as_deref().and_then(|b| b.parse::<u64>().ok()) {
            parts.push(format!("{} kb/s", br / 1000));
        }
        if let Some(lang) = s.tags.get("language") {
            parts.push(format!("[{lang}]"));
        }
        println!("  #{:<2} {:<6} {}", s.index, kind, parts.join(" | "));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn formats_durations() {
        assert_eq!(super::duration(19.0), "0:19");
        assert_eq!(super::duration(596.46), "9:56");
        assert_eq!(super::duration(3725.0), "1:02:05");
    }
}
