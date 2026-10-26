//! ffmpeg / ffprobe access.
//!
//! - [`probe`]: what's inside a file
//! - [`job`]: run any ffmpeg command with progress and cancellation
//! - [`encoders`]: which encoders (incl. GPU) actually work on this machine
//! - [`ops`]: ready-made jobs (transcode today; trims, reframes, ... next)

pub mod encoders;
pub mod job;
pub mod ops;
pub mod probe;

use std::process::Command;

pub use job::{Job, Progress};
pub use probe::{Probe, probe};

/// Executable for `name`, overridable with e.g. `FFMPEG_PATH`.
pub(crate) fn tool(name: &str) -> String {
    let var = format!("{}_PATH", name.to_uppercase());
    std::env::var(var).unwrap_or_else(|_| name.to_string())
}

/// First line of `<tool> -version`, or None if the tool isn't runnable.
pub fn version(name: &str) -> Option<String> {
    let out = Command::new(tool(name)).arg("-version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().next().map(str::to_string)
}
