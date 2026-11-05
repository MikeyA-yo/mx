//! Run ffmpeg with machine-readable progress.
//!
//! `-progress pipe:1` makes ffmpeg write `key=value` lines to stdout, in
//! blocks terminated by `progress=continue` (or `progress=end`). We fold each
//! block into a [`Progress`] snapshot. Human logs go to stderr at `-v error`,
//! so stderr holds only the reason when something fails.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::Serialize;

use crate::cancel::CancelToken;
use crate::process;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Progress {
    pub frame: Option<u64>,
    pub fps: Option<f64>,
    /// How far into the *output* timeline ffmpeg has written.
    pub out_time_secs: Option<f64>,
    pub total_size: Option<u64>,
    pub bitrate_kbps: Option<f64>,
    /// Encoding speed relative to realtime: 2.0 = twice as fast as playback.
    pub speed: Option<f64>,
    /// 0..=100, only when the output duration is known.
    pub percent: Option<f64>,
    pub done: bool,
}

/// Accumulates `-progress` lines. ffmpeg reports "N/A" for values it doesn't
/// know yet (early on, or for some muxers); those keep the previous value
/// instead of blanking it, so a UI doesn't flicker.
#[derive(Debug, Default)]
pub struct ProgressParser {
    current: Progress,
    duration_secs: Option<f64>,
}

impl ProgressParser {
    pub fn new(duration_secs: Option<f64>) -> Self {
        Self { current: Progress::default(), duration_secs }
    }

    /// Feed one line; returns a snapshot when a block completes.
    pub fn feed(&mut self, line: &str) -> Option<Progress> {
        let (key, value) = line.split_once('=')?;
        let value = value.trim();
        let num = || value.parse::<f64>().ok();
        let p = &mut self.current;
        match key.trim() {
            "frame" => p.frame = value.parse().ok().or(p.frame),
            "fps" => p.fps = num().or(p.fps),
            "total_size" => p.total_size = value.parse().ok().or(p.total_size),
            // `out_time_ms` is also microseconds (a long-standing ffmpeg quirk),
            // so we only read the unambiguous key.
            "out_time_us" => {
                p.out_time_secs = value.parse::<i64>().ok().map(|us| us.max(0) as f64 / 1e6).or(p.out_time_secs)
            }
            "bitrate" => {
                p.bitrate_kbps = value.trim_end_matches("kbits/s").trim().parse().ok().or(p.bitrate_kbps)
            }
            "speed" => p.speed = value.trim_end_matches('x').trim().parse().ok().or(p.speed),
            "progress" => {
                p.done = value == "end";
                p.percent = match (self.duration_secs, p.out_time_secs) {
                    _ if p.done && self.duration_secs.is_some() => Some(100.0),
                    (Some(total), Some(t)) if total > 0.0 => Some((t / total * 100.0).clamp(0.0, 100.0)),
                    _ => None,
                };
                return Some(p.clone());
            }
            _ => {}
        }
        None
    }
}

/// An ffmpeg invocation. Arguments are kept in the order given, because
/// ffmpeg's meaning depends on position: options before `-i` apply to that
/// input, options after the last input apply to the output.
#[derive(Debug, Clone)]
pub struct Job {
    args: Vec<OsString>,
    first_input: Option<PathBuf>,
    output: OsString,
    duration_secs: Option<f64>,
    overwrite: bool,
}

impl Job {
    pub fn new(output: impl AsRef<OsStr>) -> Self {
        Self {
            args: Vec::new(),
            first_input: None,
            output: output.as_ref().to_owned(),
            duration_secs: None,
            overwrite: false,
        }
    }

    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args.extend(args.into_iter().map(|a| a.as_ref().to_owned()));
        self
    }

    /// `-i <path>`. The first input's duration is used for percentages
    /// unless [`Job::duration`] says otherwise.
    pub fn input(mut self, path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        self.first_input.get_or_insert_with(|| path.to_owned());
        self.args.push("-i".into());
        self.args.push(path.as_os_str().to_owned());
        self
    }

    /// Expected output duration, for jobs that change it (trims, speed-ups).
    pub fn duration(mut self, secs: f64) -> Self {
        self.duration_secs = Some(secs);
        self
    }

    pub fn overwrite(mut self, yes: bool) -> Self {
        self.overwrite = yes;
        self
    }

    /// Full argument list after the program name, for logging and tests.
    pub fn command_line(&self) -> Vec<OsString> {
        let fixed = ["-hide_banner", "-nostdin", "-v", "error", "-nostats"];
        let progress = ["-progress", "pipe:1", "-stats_period", "0.5"];
        fixed
            .iter()
            .chain(&progress)
            .map(OsString::from)
            .chain([OsString::from(if self.overwrite { "-y" } else { "-n" })])
            .chain(self.args.iter().cloned())
            .chain([self.output.clone()])
            .collect()
    }

    pub fn run(&self, cancel: &CancelToken, on_progress: impl FnMut(&Progress)) -> Result<()> {
        self.run_with_lines(cancel, on_progress, |_| {})
    }

    /// Like [`Job::run`], but also hands every stdout line to `on_line`.
    ///
    /// Analysis filters print their measurements to stdout
    /// (`ametadata=mode=print:file='pipe\:1'`), interleaved with the progress
    /// records. The two never collide: progress lines are `key=value` with
    /// known keys, metadata lines are `frame:N ...` or `lavfi.*=...`.
    pub fn run_with_lines(
        &self,
        cancel: &CancelToken,
        mut on_progress: impl FnMut(&Progress),
        mut on_line: impl FnMut(&str),
    ) -> Result<()> {
        let duration = self
            .duration_secs
            .or_else(|| super::probe(self.first_input.as_ref()?).ok()?.duration_secs());
        let output = Path::new(&self.output);
        // "-" is stdout (e.g. `-f null -` in benchmarks), never a file to clean up.
        let is_file = self.output != "-";
        let existed = is_file && output.exists();
        // Checked here rather than left to `-n`: ffmpeg 8 refuses the file
        // but still exits 0, which would look like a successful job.
        if existed && !self.overwrite {
            bail!("{} already exists", output.display());
        }

        let mut cmd = crate::process::command(super::tool("ffmpeg"));
        cmd.args(self.command_line());
        let mut parser = ProgressParser::new(duration);
        let result = process::run_streaming(cmd, cancel, |line| {
            if let Some(p) = parser.feed(line) {
                on_progress(&p);
            }
            on_line(line);
        });

        let failed = !matches!(&result, Ok(f) if f.status.success());
        // A half-written file is worse than none. Leave pre-existing files
        // alone unless we were allowed to overwrite them anyway.
        if failed && is_file && (!existed || self.overwrite) {
            let _ = std::fs::remove_file(output);
        }

        let finished = result?;
        if !finished.status.success() {
            bail!("ffmpeg failed ({}):\n  {}", finished.status, summarize_stderr(&finished.stderr));
        }
        Ok(())
    }
}

/// ffmpeg prints the root cause first ("Cannot load nvcuda.dll") and the
/// consequences last ("Nothing was written..."); keep both ends.
fn summarize_stderr(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() <= 6 {
        return lines.join("\n  ");
    }
    let (head, tail) = (&lines[..3], &lines[lines.len() - 2..]);
    format!("{}\n  ...\n  {}", head.join("\n  "), tail.join("\n  "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stderr_summary_keeps_root_cause_and_outcome() {
        let stderr: String = (1..=10).map(|i| format!("line {i}\n")).collect();
        assert_eq!(
            summarize_stderr(&stderr),
            "line 1\n  line 2\n  line 3\n  ...\n  line 9\n  line 10"
        );
        assert_eq!(summarize_stderr("a\n\nb\n"), "a\n  b");
    }

    /// Two blocks captured from ffmpeg 8.0.1, the second mid-run with N/A values.
    const BLOCKS: &str = "frame=17\nfps=0.00\nstream_0_0_q=29.0\nbitrate=   0.8kbits/s\ntotal_size=48\n\
        out_time_us=500000\nout_time_ms=500000\nout_time=00:00:00.500000\ndup_frames=0\ndrop_frames=0\n\
        speed=0.971x\nprogress=continue\n\
        frame=74\nfps=70.02\nbitrate=N/A\ntotal_size=786480\nout_time_us=N/A\nout_time_ms=N/A\n\
        out_time=N/A\nspeed=N/A\nprogress=continue\n";

    fn snapshots(input: &str, duration: Option<f64>) -> Vec<Progress> {
        let mut parser = ProgressParser::new(duration);
        input.lines().filter_map(|l| parser.feed(l)).collect()
    }

    #[test]
    fn parses_blocks_and_keeps_last_known_values_over_na() {
        let snaps = snapshots(BLOCKS, Some(2.0));
        assert_eq!(snaps.len(), 2);

        let first = &snaps[0];
        assert_eq!(first.frame, Some(17));
        assert_eq!(first.out_time_secs, Some(0.5));
        assert_eq!(first.bitrate_kbps, Some(0.8));
        assert_eq!(first.speed, Some(0.971));
        assert_eq!(first.percent, Some(25.0));

        let second = &snaps[1];
        assert_eq!(second.frame, Some(74));
        assert_eq!(second.total_size, Some(786480));
        assert_eq!(second.out_time_secs, Some(0.5), "N/A must not reset");
        assert_eq!(second.speed, Some(0.971));
        assert!(!second.done);
    }

    #[test]
    fn end_block_reports_done_and_full_percent() {
        let snaps = snapshots("out_time_us=1900000\nprogress=end\n", Some(2.0));
        assert!(snaps[0].done);
        assert_eq!(snaps[0].percent, Some(100.0));
    }

    #[test]
    fn percent_is_unknown_without_duration() {
        let snaps = snapshots("out_time_us=1000000\nprogress=continue\n", None);
        assert_eq!(snaps[0].percent, None);
    }

    #[test]
    fn command_line_keeps_argument_order() {
        let job = Job::new("out.mp4").arg("-ss").arg("5").input("in.mkv").args(["-c:v", "libx264"]);
        let line: Vec<_> = job.command_line().into_iter().map(|a| a.into_string().unwrap()).collect();
        let tail = &line[line.iter().position(|a| a == "-n").unwrap()..];
        assert_eq!(tail, ["-n", "-ss", "5", "-i", "in.mkv", "-c:v", "libx264", "out.mp4"]);
    }
}
