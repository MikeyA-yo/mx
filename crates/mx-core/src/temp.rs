//! Files that live only as long as a job needs them: filter scripts too
//! long for the command line, subtitle files for burn-in.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{Context, Result};

/// A file in the temp directory, deleted when this is dropped.
#[derive(Debug)]
pub struct TempFile(PathBuf);

impl TempFile {
    /// Write `contents` to a new file named `mx-<prefix>-<unique>.<ext>`.
    pub fn with_contents(prefix: &str, ext: &str, contents: &str) -> Result<Self> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = format!("{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
        let path = std::env::temp_dir().join(format!("mx-{prefix}-{unique}.{ext}"));
        std::fs::write(&path, contents).with_context(|| format!("couldn't write {}", path.display()))?;
        Ok(Self(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
