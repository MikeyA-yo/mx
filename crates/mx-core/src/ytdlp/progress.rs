//! Machine-readable progress from yt-dlp.
//!
//! Instead of scraping yt-dlp's human progress bar, we hand it templates that
//! print one `|`-separated record per line, tagged with a prefix we can't
//! confuse with anything else on stdout. Missing values come through as "NA".

use std::path::PathBuf;

use serde::Serialize;

const PROGRESS: &str = "MXP|";
const POSTPROCESS: &str = "MXPP|";
const FILE: &str = "MXF|";

/// Arguments that make yt-dlp emit the lines `parse_line` understands.
/// `--print` implies `--quiet`, so stdout carries nothing but these records.
pub fn template_args() -> [&'static str; 8] {
    [
        "--newline",
        "--progress",
        "--progress-template",
        "download:MXP|%(progress.status)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s|%(info.format_id)s",
        "--progress-template",
        "postprocess:MXPP|%(progress.status)s|%(progress.postprocessor)s",
        "--print",
        "after_move:MXF|%(filepath)s",
    ]
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Event {
    Download(Progress),
    PostProcess { status: String, name: String },
    /// Final path of a finished item, after merging and post-processing.
    Done(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Progress {
    pub finished: bool,
    pub downloaded: u64,
    /// Exact size, or yt-dlp's estimate for fragmented (HLS/DASH) downloads.
    pub total: Option<u64>,
    pub speed: Option<f64>,
    pub eta_secs: Option<u64>,
    /// Which format is downloading; video and audio of a merge arrive separately.
    pub format_id: String,
}

fn num(field: &str) -> Option<f64> {
    field.parse::<f64>().ok()
}

pub fn parse_line(line: &str) -> Option<Event> {
    let line = line.trim_end();
    if let Some(rest) = line.strip_prefix(FILE) {
        return Some(Event::Done(PathBuf::from(rest)));
    }
    if let Some(rest) = line.strip_prefix(POSTPROCESS) {
        let (status, name) = rest.split_once('|')?;
        return Some(Event::PostProcess {
            status: status.to_string(),
            name: name.to_string(),
        });
    }
    let rest = line.strip_prefix(PROGRESS)?;
    let f: Vec<&str> = rest.splitn(7, '|').collect();
    let [status, downloaded, total, estimate, speed, eta, format_id] = f[..] else {
        return None;
    };
    Some(Event::Download(Progress {
        finished: status == "finished",
        downloaded: num(downloaded).unwrap_or(0.0) as u64,
        total: num(total).or_else(|| num(estimate)).map(|t| t as u64),
        speed: num(speed),
        eta_secs: num(eta).map(|e| e as u64),
        format_id: format_id.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_lines_captured_from_ytdlp() {
        let ev = parse_line("MXP|downloading|7168|46935223|NA|1204518.0717948717|39|0").unwrap();
        assert_eq!(
            ev,
            Event::Download(Progress {
                finished: false,
                downloaded: 7168,
                total: Some(46935223),
                speed: Some(1204518.0717948717),
                eta_secs: Some(39),
                format_id: "0".into(),
            })
        );
        let Event::Download(p) = parse_line("MXP|finished|46935223|46935223|NA|278439.4|NA|0").unwrap()
        else {
            panic!()
        };
        assert!(p.finished);
        assert_eq!(p.eta_secs, None);
    }

    #[test]
    fn falls_back_to_estimate_for_fragmented_downloads() {
        let Event::Download(p) = parse_line("MXP|downloading|100|NA|5000.5|NA|NA|hls-720p").unwrap()
        else {
            panic!()
        };
        assert_eq!(p.total, Some(5000));
        assert_eq!(p.speed, None);
        assert_eq!(p.format_id, "hls-720p");
    }

    #[test]
    fn parses_file_and_postprocess_lines() {
        assert_eq!(
            parse_line(r"MXF|C:\dl\Big Buck Bunny [x].mp4"),
            Some(Event::Done(PathBuf::from(r"C:\dl\Big Buck Bunny [x].mp4")))
        );
        assert_eq!(
            parse_line("MXPP|started|Merger"),
            Some(Event::PostProcess {
                status: "started".into(),
                name: "Merger".into()
            })
        );
    }

    #[test]
    fn ignores_unrelated_output() {
        assert_eq!(parse_line("[download] Destination: foo.mp4"), None);
        assert_eq!(parse_line("MXP|truncated|1"), None);
    }
}
