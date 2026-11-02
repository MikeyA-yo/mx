//! whisper.cpp models: which exist, where they're kept, and downloading
//! them on first use.
//!
//! Models are ggml files from the whisper.cpp project on Hugging Face. They
//! live in one shared directory (not next to each video) because they're
//! large and every transcription reuses them.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;

use crate::cancel::{CancelToken, Cancelled};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Model {
    pub name: &'static str,
    /// Approximate download size in megabytes (decimal).
    pub mb: u32,
}

const fn model(name: &'static str, mb: u32) -> Model {
    Model { name, mb }
}

/// Smallest to largest. `.en` models only understand English but are more
/// accurate at English than the multilingual model of the same size.
pub const KNOWN: &[Model] = &[
    model("tiny", 78),
    model("tiny.en", 78),
    model("base", 148),
    model("base.en", 148),
    model("small", 488),
    model("small.en", 488),
    model("medium", 1533),
    model("medium.en", 1533),
    model("large-v3-turbo-q5_0", 574),
    model("large-v3-turbo", 1624),
];

const BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

impl Model {
    pub fn lookup(name: &str) -> Option<Self> {
        KNOWN.iter().copied().find(|m| m.name == name)
    }

    pub fn file_name(&self) -> String {
        format!("ggml-{}.bin", self.name)
    }

    pub fn url(&self) -> String {
        format!("{BASE_URL}/{}", self.file_name())
    }

    pub fn english_only(&self) -> bool {
        self.name.ends_with(".en")
    }

    pub fn path(&self) -> PathBuf {
        models_dir().join(self.file_name())
    }

    pub fn is_installed(&self) -> bool {
        self.path().is_file()
    }
}

/// `MX_MODELS_DIR`, else the platform's per-user cache directory.
pub fn models_dir() -> PathBuf {
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(dir) = env("MX_MODELS_DIR") {
        return dir;
    }
    let cache = if cfg!(windows) {
        env("LOCALAPPDATA")
    } else {
        env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|h| h.join(".cache")))
    };
    cache.unwrap_or_else(std::env::temp_dir).join("mx").join("models")
}

/// Download `model` unless it's already there; returns its path.
/// `on_progress(bytes_so_far, total_bytes)` is called as data arrives.
/// The file is written as `.part` and renamed when complete, so an
/// interrupted download is never mistaken for a model.
pub fn ensure(model: Model, cancel: &CancelToken, mut on_progress: impl FnMut(u64, Option<u64>)) -> Result<PathBuf> {
    let path = model.path();
    if path.is_file() {
        return Ok(path);
    }
    let dir = path.parent().expect("models_dir() has a parent");
    std::fs::create_dir_all(dir).with_context(|| format!("couldn't create {}", dir.display()))?;
    let partial = path.with_extension("bin.part");
    let result = download(&model.url(), &partial, cancel, &mut on_progress);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&partial);
        return Err(e);
    }
    std::fs::rename(&partial, &path).with_context(|| format!("couldn't move the model into {}", path.display()))?;
    Ok(path)
}

fn download(url: &str, to: &Path, cancel: &CancelToken, on_progress: &mut impl FnMut(u64, Option<u64>)) -> Result<()> {
    let response = ureq::get(url).call().map_err(|e| anyhow!("couldn't download {url}: {e}"))?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok()?.parse::<u64>().ok());
    let mut body = response.into_body().into_reader();
    let mut file = File::create(to).with_context(|| format!("couldn't create {}", to.display()))?;
    let mut buf = vec![0; 1 << 16];
    let mut done = 0u64;
    loop {
        if cancel.is_cancelled() {
            return Err(Cancelled.into());
        }
        let n = body.read(&mut buf).with_context(|| format!("download of {url} broke off"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        done += n as u64;
        on_progress(done, total);
    }
    if total.is_some_and(|t| t != done) {
        bail!("download of {url} ended early ({done} of {} bytes)", total.unwrap_or(0));
    }
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_map_to_whisper_cpp_files() {
        let m = Model::lookup("base.en").unwrap();
        assert_eq!(m.file_name(), "ggml-base.en.bin");
        assert!(m.url().ends_with("/resolve/main/ggml-base.en.bin"));
        assert!(m.english_only() && !Model::lookup("base").unwrap().english_only());
        assert!(Model::lookup("huge").is_none());
    }
}
