//! ffmpeg / ffprobe access.
//!
//! - [`probe`]: what's inside a file
//! - [`job`]: run any ffmpeg command with progress and cancellation
//! - [`encoders`]: which encoders (incl. GPU) actually work on this machine
//! - [`ops`]: ready-made jobs: transcode, trim, reframe, jumpcut, ...

pub mod encoders;
pub mod job;
pub mod ops;
pub mod probe;

use std::path::Path;
use std::process::Command;

use anyhow::{Result, bail};

pub use job::{Job, Progress};
pub use probe::{Probe, probe};

/// A file path as a filter option value, e.g. `model='C\:/x/m.bin'`.
///
/// Filter options are parsed twice. The option parser splits on `:`, so a
/// Windows drive colon is escaped as `\:`; the quotes then stop the graph
/// parser from touching that backslash or any `[],;=` in the path.
/// Backslash separators become `/`, which Windows accepts too.
pub(crate) fn filter_path(path: &Path) -> Result<String> {
    let s = path.to_string_lossy();
    if s.contains('\'') {
        bail!("ffmpeg filters can't take a path containing a quote: {s}");
    }
    Ok(format!("'{}'", s.replace('\\', "/").replace(':', "\\:")))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_paths_escape_drive_colons_and_use_forward_slashes() {
        let p = filter_path(Path::new(r"C:\Users\me\mx models\ggml-base.en.bin")).unwrap();
        assert_eq!(p, r"'C\:/Users/me/mx models/ggml-base.en.bin'");
        assert!(filter_path(Path::new("it's.bin")).is_err());
    }
}
