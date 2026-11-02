//! Speech commands: transcribe, captions, models.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use indicatif::{HumanBytes, ProgressBar};

use crate::bar_style;
use crate::cli::{Command, SpeechArgs};
use crate::display;
use crate::edit::{Ctx, check_output, finish, progress_bar, resolve_encoder, run_job, show_progress};
use mx_core::ffmpeg::{self, ops};
use mx_core::speech::{self, CaptionStyle, Model, TranscribeOptions, Transcript, models, subtitles};
use mx_core::temp::TempFile;

/// Handles the speech subcommands; anything else is a caller bug.
pub fn run(command: Command, ctx: &Ctx) -> Result<()> {
    match command {
        Command::Transcribe { input, output, speech } => {
            let output = output.unwrap_or_else(|| input.with_extension("srt"));
            let format = output.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            if !["srt", "vtt", "txt", "json"].contains(&format.as_str()) {
                bail!("can't write .{format}; use .srt, .vtt, .txt or .json");
            }
            check_output(&output, ctx)?;
            let started = Instant::now();
            let transcript = transcribe(ctx, &input, &speech)?;
            let text = match format.as_str() {
                "srt" => subtitles::to_srt(&transcript),
                "vtt" => subtitles::to_vtt(&transcript),
                "txt" => subtitles::to_text(&transcript),
                _ => serde_json::to_string_pretty(&transcript)? + "\n",
            };
            std::fs::write(&output, text)?;
            let words: usize = transcript.segments.iter().map(|s| s.text.split_whitespace().count()).sum();
            println!(
                "done in {:.1}s: {} segments, {words} words\n\n{}",
                started.elapsed().as_secs_f64(),
                transcript.segments.len(),
                output.display()
            );
            Ok(())
        }
        Command::Captions { input, output, subs, save_srt, words, size, position, font, upper, no_pop, speech, encode } => {
            check_output(&output, ctx)?;
            let source = ffmpeg::probe(&input)?;
            let v = source.video().ok_or_else(|| anyhow!("{} has no video to caption", input.display()))?;
            let (Some(width), Some(height)) = (v.width, v.height) else {
                bail!("couldn't read the frame size of {}", input.display());
            };
            let video = resolve_encoder(&encode, Some(height))?;

            let transcript = match &subs {
                Some(file) => subtitles::parse(&std::fs::read_to_string(file)?)?,
                None => transcribe(ctx, &input, &speech)?,
            };
            if transcript.segments.is_empty() {
                bail!("no speech found to caption");
            }
            if save_srt {
                let srt = output.with_extension("srt");
                std::fs::write(&srt, subtitles::to_srt(&transcript))?;
                println!("saved    {}", srt.display());
            }

            let defaults = CaptionStyle::for_frame(width, height);
            let style = CaptionStyle {
                max_words: words.max(1),
                size: size / 100.0,
                position: position.unwrap_or(defaults.position),
                font,
                uppercase: upper,
                pop: !no_pop,
                ..defaults
            };
            let cues = speech::cues(&transcript, &style);
            println!("captions {} phrases from {} segments, {width}x{height}", cues.len(), transcript.segments.len());
            let ass = TempFile::with_contents("captions", "ass", &speech::to_ass(&cues, width, height, &style))?;
            let job = ops::burn_subtitles(&input, &output, ass.path(), video)?;
            finish(&input, &output, || run_job(ctx, "captions", job))
        }
        Command::Models { download } => {
            if let Some(name) = download {
                let model = Model::lookup(&name).ok_or_else(|| unknown_model(&name))?;
                let path = ensure_model(ctx, model)?;
                println!("{}", path.display());
                return Ok(());
            }
            println!("models in {}\n", models::models_dir().display());
            for m in models::KNOWN {
                let mark = if m.is_installed() { "installed" } else { "" };
                let lang = if m.english_only() { "English only" } else { "multilingual" };
                println!("  {:<22} {:>6} MB  {lang:<13} {mark}", m.name, m.mb);
            }
            println!(
                "\nbase is the default: about 4x realtime on an Intel laptop GPU. small and up are more\n\
                 accurate and slower; .en models are better at English than multilingual ones of the same size."
            );
            Ok(())
        }
        _ => unreachable!("not a speech command"),
    }
}

fn unknown_model(name: &str) -> anyhow::Error {
    let names: Vec<_> = models::KNOWN.iter().map(|m| m.name).collect();
    anyhow!("unknown model {name:?}; known: {} (or give a path to a .bin file)", names.join(", "))
}

/// The model file to use and the language to pass whisper.
fn resolve_model(ctx: &Ctx, args: &SpeechArgs) -> Result<(PathBuf, String)> {
    let language = args.language.to_ascii_lowercase();
    let model = match &args.model {
        Some(m) if m.ends_with(".bin") || Path::new(m).is_file() => {
            let path = PathBuf::from(m);
            if !path.is_file() {
                bail!("model file {} doesn't exist", path.display());
            }
            return Ok((path, language));
        }
        Some(name) => Model::lookup(name).ok_or_else(|| unknown_model(name))?,
        None => Model::lookup(if language == "en" { "base.en" } else { "base" }).expect("known model"),
    };
    let language = match (model.english_only(), language.as_str()) {
        (true, "auto" | "en") => "en".to_string(),
        (true, other) => bail!("{} only understands English; pick a model without .en for {other:?}", model.name),
        (false, _) => language,
    };
    Ok((ensure_model(ctx, model)?, language))
}

fn ensure_model(ctx: &Ctx, model: Model) -> Result<PathBuf> {
    if model.is_installed() {
        return Ok(model.path());
    }
    println!("model    downloading {} ({} MB) to {}", model.name, model.mb, models::models_dir().display());
    let bar = ProgressBar::new(model.mb as u64 * 1_000_000);
    bar.set_style(bar_style("{spinner} {prefix:<10} [{bar:30}] {bytes:>10}/{total_bytes:<10} {bytes_per_sec}"));
    bar.set_prefix("download");
    bar.enable_steady_tick(Duration::from_millis(120));
    let result = models::ensure(model, ctx.cancel, |done, total| {
        if let Some(total) = total {
            bar.set_length(total);
        }
        bar.set_position(done);
    });
    bar.finish_and_clear();
    let path = result?;
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("model    saved {} ({})", path.display(), HumanBytes(size));
    Ok(path)
}

fn transcribe(ctx: &Ctx, input: &Path, args: &SpeechArgs) -> Result<Transcript> {
    let (model, language) = resolve_model(ctx, args)?;
    let name = model.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let device = if args.no_gpu { "CPU" } else { "GPU if available" };
    println!("speech   {name}, language {language}, {device}");
    let opts = TranscribeOptions { language, use_gpu: !args.no_gpu };
    let started = Instant::now();
    let bar = progress_bar("listen");
    let result = speech::transcribe(input, &model, &opts, ctx.cancel, |stage, p| {
        if bar.prefix() != stage {
            bar.set_prefix(stage.to_string());
            bar.set_position(0);
        }
        show_progress(&bar, p);
    });
    bar.finish_and_clear();
    let transcript = result?;
    let spoken = transcript.segments.last().map_or(0.0, |s| s.end);
    println!(
        "speech   transcribed {} in {}",
        display::duration(spoken),
        display::duration(started.elapsed().as_secs_f64())
    );
    Ok(transcript)
}
