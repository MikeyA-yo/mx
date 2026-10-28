//! Measuring media instead of transforming it.
//!
//! ffmpeg does the signal processing and prints raw measurements; the
//! decisions (what counts as silence, what's a highlight) are made here in
//! Rust, where they can be tuned and unit-tested without running ffmpeg.

pub mod highlights;
pub mod loudness;
pub mod motion;
pub mod silence;

pub use highlights::{Breakdown, Highlight, HighlightOptions, Signals, find_highlights, measure_signals};
pub use loudness::{Envelope, measure_loudness};
pub use motion::{MotionTrack, measure_motion};
pub use silence::{SilenceOptions, SilenceReport, Threshold, find_silences};
