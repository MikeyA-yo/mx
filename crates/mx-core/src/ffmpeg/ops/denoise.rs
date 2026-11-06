//! Background noise reduction for a voice track.
//!
//! Two strengths, both ffmpeg built-ins:
//! - **Light**: an FFT denoiser (`afftdn`) that learns the room's noise
//!   from a pause in the recording, then takes about 10 dB of it away
//!   while leaving the voice as it was. The pause is played to it first,
//!   so the whole clip is cleaned, the pause included.
//! - **Strong**: RNNoise (`arnndn`), a small neural network trained on
//!   speech with recording noise, which removes about 25 dB of fans, hum
//!   and hiss. Anything that isn't speech goes too, music included.
//!
//! Both start with a high-pass at 80 Hz, below any voice, which takes out
//! rumble and handling noise.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::analysis::{SilenceOptions, find_silences, measure_loudness};
use crate::cache;
use crate::cancel::CancelToken;
use crate::ffmpeg::{Job, filter_path, has_filter, probe};
use crate::project::Denoise;
use crate::timeline::Range;

/// The RNNoise model, built into mx and written out when first needed.
const MODEL: &str = include_str!("../../../assets/rnnoise/sh.rnnn");
const MODEL_FILE: &str = "rnnoise-sh-2018-09-01.rnnn";

/// Where `arnndn` finds the model.
pub fn model_path() -> Result<PathBuf> {
    let path = cache::dir().join(MODEL_FILE);
    let current = std::fs::metadata(&path).map(|m| m.len() == MODEL.len() as u64).unwrap_or(false);
    if !current {
        std::fs::create_dir_all(cache::dir())?;
        // Written whole and renamed into place, so a reader never sees half.
        let partial = path.with_extension("part");
        std::fs::write(&partial, MODEL)?;
        std::fs::rename(&partial, &path).context("couldn't install the noise reduction model")?;
    }
    Ok(path)
}

/// A pause in `input` to learn its noise from: the longest one, kept clear
/// of the words either side, at most 3 s. `None` if it has no pauses.
pub fn quiet_stretch(input: &Path) -> Result<Option<Range>> {
    // The same measurement (and cache entry) as the waveform and pauses.
    let env = cache::json("loudness", 1, input, || measure_loudness(input, &CancelToken::new(), |_| {}))?;
    let report = find_silences(&env, &SilenceOptions::default());
    let best = report
        .silences
        .iter()
        .map(|r| Range::new(r.start + 0.1, r.end - 0.1))
        .filter(|r| r.end - r.start >= 0.25)
        .max_by(|a, b| (a.end - a.start).total_cmp(&(b.end - b.start)));
    Ok(best.map(|r| Range::new(r.start, r.end.min(r.start + 3.0))))
}

/// How a clip's sound is cleaned, ready to go into a filter graph.
#[derive(Debug, Clone, PartialEq)]
pub enum Denoiser {
    /// Light, with a pause: the pause is played to the filter first, as
    /// its own stream `length` seconds long, so it knows the noise before
    /// the clip starts, and then cut off again.
    Learned { length: f64 },
    /// Light without a pause: it follows the noise as it goes, which
    /// removes less.
    Tracking,
    /// Strong: the neural network, with its model's path as a filter value.
    Neural { model: String },
}

impl Denoiser {
    /// `pause` is how long the pause to learn from is, when there is one.
    pub fn new(denoise: Denoise, pause: Option<f64>) -> Result<Option<Self>> {
        Ok(match denoise {
            Denoise::Off => None,
            Denoise::Light => Some(pause.map_or(Self::Tracking, |length| Self::Learned { length })),
            Denoise::Strong => {
                if !has_filter("arnndn") {
                    bail!("this ffmpeg has no arnndn filter for strong noise reduction; the gyan.dev or BtbN builds have it");
                }
                Some(Self::Neural { model: filter_path(&model_path()?)? })
            }
        })
    }

    /// Whether it wants the pause fed in as a stream of its own.
    pub fn wants_pause(&self) -> bool {
        matches!(self, Self::Learned { .. })
    }

    /// Graph text taking `input` (and `pause`, for `Learned`) to `output`.
    pub fn graph(&self, input: &str, pause: Option<&str>, output: &str) -> String {
        // The pause and the clip come from the same file, so they share a
        // channel layout; it's left alone (mono to stereo would cost 3 dB).
        const PREP: &str = "highpass=f=80,aformat=sample_fmts=fltp:sample_rates=48000";
        match (self, pause) {
            (Self::Learned { length }, Some(pause)) => format!(
                "[{pause}]{PREP},asetpts=PTS-STARTPTS[{output}l];[{input}]{PREP},asetpts=PTS-STARTPTS[{output}m];\
                 [{output}l][{output}m]concat=n=2:v=0:a=1,\
                 asendcmd=c='0.000 afftdn sample_noise start; {length:.3} afftdn sample_noise stop',afftdn=nr=12:nf=-40,\
                 atrim=start={length:.6},asetpts=PTS-STARTPTS[{output}]"
            ),
            (Self::Neural { model }, _) => format!("[{input}]highpass=f=80,arnndn=m={model}[{output}]"),
            _ => format!("[{input}]highpass=f=80,afftdn=nr=12:nf=-50:tn=1[{output}]"),
        }
    }
}

/// A few seconds of `input` from `start`, as it is and with `denoise`, to
/// hear the difference: written to `before` and `after` (WAV).
pub fn preview(input: &Path, start: f64, length: f64, denoise: Denoise, before: &Path, after: &Path, cancel: &CancelToken) -> Result<()> {
    if !probe(input)?.has_audio() {
        bail!("{} has no sound", input.display());
    }
    let excerpt = |job: Job| job.args(["-ss", &format!("{start:.3}"), "-t", &format!("{length:.3}")]).input(input);
    let wav = ["-vn", "-ac", "2", "-ar", "48000"];
    excerpt(Job::new(before).overwrite(true)).args(wav).run(cancel, |_| {})?;

    let quiet = match denoise {
        Denoise::Light => quiet_stretch(input)?,
        _ => None,
    };
    let Some(d) = Denoiser::new(denoise, quiet.map(|q| q.end - q.start))? else {
        std::fs::copy(before, after)?;
        return Ok(());
    };
    let mut job = excerpt(Job::new(after).overwrite(true));
    if let (true, Some(q)) = (d.wants_pause(), quiet) {
        job = job.args(["-ss", &format!("{:.3}", q.start), "-t", &format!("{:.3}", q.end - q.start)]).input(input);
    }
    let graph = d.graph("0:a:0", d.wants_pause().then_some("1:a:0"), "out");
    job.args(["-filter_complex", &graph, "-map", "[out]"]).args(wav).run(cancel, |_| {})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_hears_the_pause_first_then_cuts_it_off() {
        let g = Denoiser::Learned { length: 1.25 }.graph("0:a:0", Some("3:a:0"), "dn0");
        assert!(g.starts_with("[3:a:0]highpass=f=80,"), "{g}");
        assert!(g.contains("[dn0l][dn0m]concat=n=2:v=0:a=1,asendcmd=c='0.000 afftdn sample_noise start; 1.250 afftdn sample_noise stop',"), "{g}");
        assert!(g.ends_with("atrim=start=1.250000,asetpts=PTS-STARTPTS[dn0]"), "{g}");
    }

    #[test]
    fn light_without_a_pause_tracks_the_noise() {
        let d = Denoiser::new(Denoise::Light, None).unwrap().unwrap();
        assert!(!d.wants_pause());
        assert_eq!(d.graph("0:a:0", None, "dn0"), "[0:a:0]highpass=f=80,afftdn=nr=12:nf=-50:tn=1[dn0]");
    }

    #[test]
    fn off_is_nothing() {
        assert_eq!(Denoiser::new(Denoise::Off, Some(1.0)).unwrap(), None);
    }

    #[test]
    fn the_model_is_installed_whole() {
        let path = model_path().unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap().len(), MODEL.len());
        assert!(MODEL.starts_with("rnnoise-nu model file"));
    }
}
