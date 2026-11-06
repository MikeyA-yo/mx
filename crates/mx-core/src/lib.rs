//! The mx engine: extraction through yt-dlp, processing through ffmpeg.
//!
//! Front ends (the `mx` CLI today, a desktop UI later) only build requests
//! and render events; every subprocess, parser and media decision lives here.
//! Long-running work takes a [`CancelToken`] and reports progress through a
//! callback, so a UI can drive it from a worker thread.

pub mod analysis;
pub mod cache;
pub mod cancel;
pub mod error;
pub mod ffmpeg;
pub mod platform;
pub mod project;
pub mod speech;
pub mod temp;
pub mod timeline;
pub mod units;
pub mod ytdlp;

mod process;

pub use cancel::{CancelToken, Cancelled};
