//! Loudness over time ("envelope"): the audio level in short windows.
//!
//! The audio is mixed to mono and resampled to 16 kHz (plenty for judging
//! loudness, and cheap), cut into 20 ms windows, and `astats` measures the
//! RMS level of each in dBFS. `ametadata` prints those measurements to
//! stdout, where [`Job::run_with_lines`] passes them to us.

use std::path::Path;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::cancel::CancelToken;
use crate::ffmpeg::{Job, Progress, probe};

pub const WINDOW_SECS: f64 = 0.02;

/// Level used for digital silence, which astats reports as `-inf`.
pub const SILENCE_DB: f64 = -120.0;

const LEVEL_KEY: &str = "lavfi.astats.Overall.RMS_level";

/// The filter chain. `file='pipe\:1'` is stdout: the colon is escaped for
/// the filter's option parser and the quotes protect it from the graph
/// parser.
const FILTER: &str = "aformat=channel_layouts=mono,aresample=16000,asetnsamples=n=320,\
    astats=metadata=1:reset=1:measure_perchannel=none:measure_overall=RMS_level,\
    ametadata=mode=print:key=lavfi.astats.Overall.RMS_level:file='pipe\\:1':direct=1";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Envelope {
    /// (window start in seconds, RMS level in dBFS), in time order.
    pub points: Vec<(f64, f64)>,
}

impl Envelope {
    pub fn levels(&self) -> impl Iterator<Item = f64> + '_ {
        self.points.iter().map(|&(_, db)| db)
    }

    /// The loudest level in each of `bins` equal slices of `duration`,
    /// scaled to 0..=1: what a waveform overview draws.
    ///
    /// Levels become amplitude relative to the recording's loud end (its
    /// 99.5th percentile, so one spike doesn't flatten the rest), with a
    /// gentle curve. On a dB scale speech sits within a few dB of the top
    /// everywhere and the overview is a solid block; on amplitude the
    /// words and the pauses between them stand apart.
    pub fn overview(&self, duration: f64, bins: usize) -> Vec<f32> {
        if bins == 0 || duration <= 0.0 {
            return Vec::new();
        }
        let mut peaks = vec![SILENCE_DB; bins];
        for &(t, db) in &self.points {
            let i = ((t / duration * bins as f64) as usize).min(bins - 1);
            peaks[i] = peaks[i].max(db);
        }
        let mut sorted: Vec<f64> = self.levels().filter(|&db| db > SILENCE_DB).collect();
        if sorted.is_empty() {
            return vec![0.0; bins];
        }
        sorted.sort_by(f64::total_cmp);
        let top = super::silence::percentile(&sorted, 0.995);
        peaks.iter().map(|&db| (10f64.powf((db - top) / 20.0).min(1.0).powf(0.6)) as f32).collect()
    }
}

/// Parse `ametadata` output: a `frame:N pts:P pts_time:T` line, then the
/// metadata value line for that frame.
fn parse_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> Envelope {
    let mut points = Vec::new();
    let mut time = None;
    for line in lines {
        if let Some(rest) = line.strip_prefix("frame:") {
            time = rest
                .split_whitespace()
                .find_map(|field| field.strip_prefix("pts_time:"))
                .and_then(|t| t.parse::<f64>().ok());
        } else if let Some(value) = line.strip_prefix(LEVEL_KEY).and_then(|v| v.strip_prefix('='))
            && let Some(t) = time.take()
        {
            let db = value.parse::<f64>().unwrap_or(SILENCE_DB).max(SILENCE_DB);
            points.push((t, db));
        }
    }
    Envelope { points }
}

/// Measure the first audio track of `input`.
pub fn measure_loudness(
    input: &Path,
    cancel: &CancelToken,
    on_progress: impl FnMut(&Progress),
) -> Result<Envelope> {
    if !probe(input)?.has_audio() {
        bail!("{} has no audio track to analyse", input.display());
    }
    let mut lines = Vec::new();
    Job::new("-")
        .input(input)
        .args(["-map", "0:a:0", "-af", FILTER, "-f", "null"])
        .run_with_lines(cancel, on_progress, |line| {
            if line.starts_with("frame:") || line.starts_with(LEVEL_KEY) {
                lines.push(line.to_string());
            }
        })?;
    Ok(parse_lines(lines.iter().map(String::as_str)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_metadata_lines_and_ignores_progress() {
        // Captured from ffmpeg 8.0.1, with progress records interleaved.
        let out = "frame:0    pts:104     pts_time:0.0065\n\
                   lavfi.astats.Overall.RMS_level=-inf\n\
                   out_time_us=20000\n\
                   frame:1    pts:440     pts_time:0.0275\n\
                   lavfi.astats.Overall.RMS_level=-31.104177\n\
                   progress=continue\n\
                   frame:2    pts:760     pts_time:0.0475\n\
                   lavfi.astats.Overall.RMS_level=-26.808652\n";
        let env = parse_lines(out.lines());
        assert_eq!(env.points, [(0.0065, SILENCE_DB), (0.0275, -31.104177), (0.0475, -26.808652)]);
    }

    #[test]
    fn overview_scales_bins_between_floor_and_peak() {
        let points = (0..100).map(|i| (i as f64 * 0.1, if i < 50 { -60.0 } else { -20.0 })).collect();
        let o = Envelope { points }.overview(10.0, 4);
        assert_eq!(o.len(), 4);
        assert!(o[0] < 0.1 && o[1] < 0.1, "{o:?}");
        assert!((o[3] - 1.0).abs() < 1e-6, "{o:?}");
        assert!(Envelope::default().overview(10.0, 3).iter().all(|&v| v == 0.0));
    }
}
