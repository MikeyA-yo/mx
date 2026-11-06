//! Speech: transcription with whisper.cpp (through ffmpeg's `whisper`
//! filter), subtitle files, and styled burn-in captions.

pub mod captions;
pub mod models;
pub mod subtitles;
pub mod transcribe;

pub use captions::{CaptionStyle, cues, to_ass};
pub use models::Model;
pub use transcribe::{Segment, TranscribeOptions, Transcript, Word, estimate_words, transcribe};
