//! Results derived from a media file, kept on disk between runs.
//!
//! Measuring a long video takes a minute or more, and both front ends ask
//! for the same measurements repeatedly (re-running `highlights` with new
//! options, reopening a file in the editor). Entries live in the temp
//! directory and are keyed by the file's path, size and modification time,
//! so an edited file is measured again.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub fn dir() -> PathBuf {
    std::env::temp_dir().join("mx-cache")
}

/// Where the `kind` result for `input` lives, e.g.
/// `mx-cache/signals-1a2b....json`. `version` is bumped whenever what's
/// stored under `kind` changes, which retires the old entries.
pub fn path(kind: &str, version: u32, input: &Path, ext: &str) -> Result<PathBuf> {
    let meta = std::fs::metadata(input)?;
    let mut key = std::collections::hash_map::DefaultHasher::new();
    (version, std::fs::canonicalize(input)?, meta.len(), meta.modified().ok()).hash(&mut key);
    Ok(dir().join(format!("{kind}-{:016x}.{ext}", key.finish())))
}

/// The cached `kind` value for `input`, if there is one.
pub fn get<T: DeserializeOwned>(kind: &str, version: u32, input: &Path) -> Option<T> {
    let bytes = std::fs::read(path(kind, version, input, "json").ok()?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The cached `kind` value for `input`, or `compute`'s result, stored for
/// next time. A failed write only costs the next run its speed-up.
pub fn json<T: Serialize + DeserializeOwned>(
    kind: &str,
    version: u32,
    input: &Path,
    compute: impl FnOnce() -> Result<T>,
) -> Result<T> {
    if let Some(value) = get(kind, version, input) {
        return Ok(value);
    }
    let path = path(kind, version, input, "json")?;
    let value = compute()?;
    if std::fs::create_dir_all(dir()).is_ok()
        && let Ok(bytes) = serde_json::to_vec(&value)
    {
        let _ = std::fs::write(&path, bytes);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_once_then_reads_back() {
        let file = crate::temp::TempFile::with_contents("cache-test", "txt", "media").unwrap();
        let mut calls = 0;
        let mut get = || {
            json("test", 1, file.path(), || {
                calls += 1;
                Ok(vec![1.5, 2.5])
            })
            .unwrap()
        };
        assert_eq!(get(), [1.5, 2.5]);
        assert_eq!(get(), [1.5, 2.5]);
        assert_eq!(calls, 1);
        // A new version is a new entry.
        let other = path("test", 2, file.path(), "json").unwrap();
        assert_ne!(other, path("test", 1, file.path(), "json").unwrap());
        let _ = std::fs::remove_file(path("test", 1, file.path(), "json").unwrap());
    }
}
