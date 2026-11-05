//! Local processing commands: convert, remux, trim, reframe, silence,
//! jumpcut, highlights, compress, speed, gif.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use indicatif::{DecimalBytes, HumanBytes, ProgressBar};

use crate::cli::{CodecArg, Command, CompressCodecArg, EncodeArgs, FitArg, SilenceArgs};
use crate::{bar_style, display};
use mx_core::{CancelToken, Cancelled, cache};
use mx_core::ffmpeg::ops::{self, Aspect, CompressCodec, Cut, Fit, Frame, GifOptions, VideoEncode};
use mx_core::ffmpeg::{self, Job, Progress, encoders, encoders::Codec, encoders::Encoder};
use mx_core::analysis::{
    self, Breakdown, Highlight, HighlightOptions, SilenceOptions, SilenceReport, Signals, Threshold, find_highlights,
    find_silences, measure_loudness,
};
use mx_core::timeline::{self, Range};
use mx_core::units::format_time;

pub struct Ctx<'a> {
    pub cancel: &'a CancelToken,
    pub overwrite: bool,
    pub verbose: bool,
}

/// Handles the processing subcommands; anything else is a caller bug.
pub fn run(command: Command, ctx: &Ctx) -> Result<()> {
    match command {
        Command::Convert { input, output, encode } => {
            check_output(&output, ctx)?;
            let video = resolve_encoder(&encode, video_height(&input))?;
            let job = ops::transcode(&input, &output, video);
            finish(&input, &output, || run_job(ctx, "convert", job))
        }
        Command::Remux { input, output, to } => {
            let output = output.unwrap_or_else(|| input.with_extension(to.core().ext()));
            check_output(&output, ctx)?;
            let remux = ops::remux(&input, &output)?;
            let subs = &remux.subtitles;
            if subs.converted > 0 {
                println!("subtitles  {} text track(s) converted for this container", subs.converted);
            }
            if !subs.dropped.is_empty() {
                println!(
                    "note: left out {} subtitle track(s) ({}) this container can't hold; remux to .mkv to keep them",
                    subs.dropped.len(),
                    subs.dropped.join(", ")
                );
            }
            finish(&input, &output, || run_job(ctx, "remux", remux.job)).map_err(|e| {
                if e.is::<Cancelled>() {
                    e
                } else {
                    e.context("remux failed; .mkv holds almost any codec, or `mx convert` re-encodes into this one")
                }
            })
        }
        Command::Trim { input, output, from, to, length, fast, encode } => {
            check_output(&output, ctx)?;
            let cut = if fast { Cut::Fast } else { Cut::Exact(resolve_encoder(&encode, video_height(&input))?) };
            let end = to.or(length.map(|l| from + l));
            let trim = ops::trim(&input, &output, from, end, cut)?;
            if fast && from - trim.actual_start > 0.001 {
                println!(
                    "note: fast cut starts at the keyframe at {}, {:.2}s before --from {}\n      \
                     (drop --fast for a frame-exact start)",
                    format_time(trim.actual_start),
                    from - trim.actual_start,
                    format_time(from)
                );
            }
            println!("cutting  {} -> {}", format_time(trim.actual_start), format_time(trim.actual_start + trim.duration));
            finish(&input, &output, || run_job(ctx, "trim", trim.job))
        }
        Command::Reframe { input, output, aspect, fit, position, short_edge, encode } => {
            check_output(&output, ctx)?;
            let video = resolve_encoder(&encode, Some(aspect.frame(short_edge).1))?;
            let (w, h) = aspect.frame(short_edge);
            println!("frame    {}:{} -> {w}x{h}", aspect.w, aspect.h);
            let job = ops::reframe(&input, &output, aspect, fit_of(fit, position), short_edge, video);
            finish(&input, &output, || run_job(ctx, "reframe", job))
        }
        Command::Silence { input, detect } => {
            let duration = probe_duration(&input)?;
            let report = detect_silences(ctx, &input, &detect)?;
            print_levels(&report, &detect);
            const SHOWN: usize = 40;
            for s in report.silences.iter().take(SHOWN) {
                println!("  {} - {}  ({:.2}s)", format_time(s.start), format_time(s.end), s.len());
            }
            if report.silences.len() > SHOWN {
                println!("  ... {} more", report.silences.len() - SHOWN);
            }
            let silent = timeline::total(&report.silences);
            let kept = timeline::total(&timeline::keep_ranges(duration, &report.silences, detect.padding));
            println!(
                "
{} pauses, {} of silence. jumpcut would remove {} ({:.0}%): {} -> {}",
                report.silences.len(),
                display::duration(silent),
                display::duration(duration - kept),
                (duration - kept) / duration * 100.0,
                display::duration(duration),
                display::duration(kept)
            );
            Ok(())
        }
        Command::Jumpcut { input, output, detect, encode } => {
            check_output(&output, ctx)?;
            let source = ffmpeg::probe(&input)?;
            let duration = source.duration_secs().ok_or_else(|| anyhow!("couldn't read the duration"))?;
            let video = match source.video() {
                Some(v) => Some(resolve_encoder(&encode, v.height)?),
                None => None,
            };
            let report = detect_silences(ctx, &input, &detect)?;
            print_levels(&report, &detect);
            let keep = timeline::keep_ranges(duration, &report.silences, detect.padding);
            if (duration - timeline::total(&keep)) < 0.05 {
                println!("no pauses long enough to cut (try a higher --threshold or lower --min-silence)");
                return Ok(());
            }
            let cut = ops::jumpcut(&input, &output, &keep, video)?;
            let after = cut.output_duration();
            let (first, last) = (cut.kept[0], cut.kept[cut.kept.len() - 1]);
            let removed = cut.kept.len() - 1 + usize::from(first.start > 0.0) + usize::from(last.end < duration - 0.05);
            println!(
                "cutting  {removed} pauses: {} -> {} (-{}, {:.0}%)",
                display::duration(duration),
                display::duration(after),
                display::duration(duration - after),
                (duration - after) / duration * 100.0
            );
            finish(&input, &output, || run_job(ctx, "jumpcut", cut.job.clone()))
        }
        Command::Highlights {
            input,
            count,
            length,
            from,
            to,
            export,
            vertical,
            aspect,
            fit,
            position,
            short_edge,
            json,
            encode,
        } => {
            let source = ffmpeg::probe(&input)?;
            let aspect = if vertical { Some(Aspect { w: 9, h: 16 }) } else { aspect };
            let frame = match (aspect, source.video()) {
                (None, _) => None,
                (Some(_), None) => bail!("{} has no video to reframe", input.display()),
                (Some(aspect), Some(v)) => {
                    let source_short = v.width.zip(v.height).map(|(w, h)| w.min(h)).unwrap_or(1080);
                    let short_edge = short_edge.unwrap_or(source_short.min(1080));
                    Some(Frame { aspect, fit: fit_of(fit, position), short_edge })
                }
            };
            // Pick the encoder before the slow analysis, so a bad -e fails fast.
            let video = match (&export, source.video()) {
                (Some(_), Some(v)) => {
                    let height = frame.map(|f| f.aspect.frame(f.short_edge).1).or(v.height);
                    Some(resolve_encoder(&encode, height)?)
                }
                _ => None,
            };

            let signals = cached_signals(ctx, &input)?;
            let within = (from.is_some() || to.is_some())
                .then(|| Range::new(from.unwrap_or(0.0), to.unwrap_or(f64::INFINITY)));
            let found = find_highlights(&signals, &HighlightOptions { count, length, within });
            if json {
                println!("{}", serde_json::to_string_pretty(&found)?);
                return Ok(());
            }
            print_highlights(&signals, &found);
            match export {
                Some(dir) => export_highlights(ctx, &input, &dir, &found, frame, video),
                None => Ok(()),
            }
        }
        Command::Compress { input, output, size, codec } => {
            check_output(&output, ctx)?;
            compress(ctx, &input, &output, size, codec)
        }
        Command::Speed { input, output, factor, encode } => {
            check_output(&output, ctx)?;
            let video = resolve_encoder(&encode, video_height(&input))?;
            let job = ops::speed(&input, &output, factor, video)?;
            finish(&input, &output, || run_job(ctx, "speed", job))
        }
        Command::Gif { input, output, from, to, fps, width } => {
            check_output(&output, ctx)?;
            let opts = GifOptions { fps, width, start: from, end: to };
            let job = ops::gif(&input, &output, opts)?;
            finish(&input, &output, || run_job(ctx, "gif", job))
        }
        _ => unreachable!("not a processing command"),
    }
}

fn fit_of(fit: FitArg, position: f64) -> Fit {
    match fit {
        FitArg::Blur => Fit::Blur,
        FitArg::Crop => Fit::Crop { position },
        FitArg::Pad => Fit::Pad,
    }
}

/// Short display names for the signals a highlight was ranked on.
fn parts(b: Breakdown) -> [(&'static str, Option<f64>); 4] {
    [("loud", b.loudness), ("lively", b.activity), ("motion", b.motion), ("cuts", b.pace)]
}

fn print_highlights(signals: &Signals, found: &[Highlight]) {
    let Some(first) = found.first() else {
        println!("no highlights: the recording is empty");
        return;
    };
    let used: Vec<&str> = parts(first.breakdown).iter().filter(|(_, v)| v.is_some()).map(|(name, _)| *name).collect();
    let cuts = signals.motion.as_ref().map(|m| m.scene_changes().len());
    println!(
        "signals  {}{}",
        if used.is_empty() { "none varied enough to rank; showing the start".to_string() } else { used.join(", ") },
        cuts.map(|n| format!(" ({n} scene cuts in {})", display::duration(signals.duration))).unwrap_or_default()
    );
    println!();
    for (i, h) in found.iter().enumerate() {
        let detail: Vec<String> =
            parts(h.breakdown).into_iter().filter_map(|(name, v)| Some(format!("{name} {:.0}", v? * 100.0))).collect();
        println!(
            "#{}  {} - {}  ({:.1}s)  score {:>3.0}   {}",
            i + 1,
            format_time(h.range.start),
            format_time(h.range.end),
            h.range.len(),
            h.score * 100.0,
            detail.join(" · ")
        );
    }
}

/// The usual workflow runs `highlights` a few times (look, adjust
/// --from/--to, then export), so measurements are cached.
fn cached_signals(ctx: &Ctx, input: &Path) -> Result<Signals> {
    cache::json(analysis::highlights::CACHE_KIND, analysis::highlights::CACHE_VERSION, input, || {
        let bar = progress_bar("analyse");
        let signals = analysis::measure_signals(input, ctx.cancel, |p| show_progress(&bar, p));
        bar.finish_and_clear();
        signals
    })
}

fn export_highlights(
    ctx: &Ctx,
    input: &Path,
    dir: &Path,
    found: &[Highlight],
    frame: Option<Frame>,
    video: Option<VideoEncode>,
) -> Result<()> {
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "clip".into());
    let input_ext = input.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    // Clips are H.264/AAC, which webm can't hold; audio-only clips keep
    // the source format where the encoder for it is known.
    let ext = match (video.is_some(), input_ext.as_str()) {
        (true, "mkv") => "mkv",
        (true, _) => "mp4",
        (false, e @ ("mp3" | "m4a" | "wav" | "flac")) => e,
        (false, _) => "m4a",
    };
    let outputs: Vec<_> = found
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let at = h.range.start as u64;
            dir.join(format!("{stem}_h{}_{}m{:02}s.{ext}", i + 1, at / 60, at % 60))
        })
        .collect();
    for output in &outputs {
        check_output(output, ctx)?;
    }
    std::fs::create_dir_all(dir)?;
    if let Some(f) = frame {
        let (w, h) = f.aspect.frame(f.short_edge);
        println!("\nframe    {}:{} -> {w}x{h}", f.aspect.w, f.aspect.h);
    }
    let started = Instant::now();
    for (i, (h, output)) in found.iter().zip(&outputs).enumerate() {
        let job = ops::clip(input, output, h.range, frame, video)?;
        run_job(ctx, &format!("clip {}/{}", i + 1, found.len()), job)?;
        let size = std::fs::metadata(output).map(|m| m.len()).unwrap_or(0);
        println!("saved    {} ({})", output.display(), HumanBytes(size));
    }
    println!("done in {:.1}s", started.elapsed().as_secs_f64());
    Ok(())
}

fn probe_duration(input: &Path) -> Result<f64> {
    ffmpeg::probe(input)?
        .duration_secs()
        .ok_or_else(|| anyhow!("couldn't read the duration of {}", input.display()))
}

fn detect_silences(ctx: &Ctx, input: &Path, args: &SilenceArgs) -> Result<SilenceReport> {
    let bar = progress_bar("analyse");
    let envelope = measure_loudness(input, ctx.cancel, |p| show_progress(&bar, p));
    bar.finish_and_clear();
    let opts = SilenceOptions { threshold: args.threshold, min_duration: args.min_silence };
    Ok(find_silences(&envelope?, &opts))
}

fn print_levels(report: &SilenceReport, args: &SilenceArgs) {
    let mode = if args.threshold == Threshold::Auto { " (auto)" } else { "" };
    println!(
        "levels   noise floor {:.1} dB, speech {:.1} dB -> silence below {:.1} dB{mode}",
        report.noise_floor_db, report.speech_db, report.threshold_db
    );
}

/// Fail before any slow work (encoder detection, pass 1) rather than after.
pub(crate) fn check_output(output: &Path, ctx: &Ctx) -> Result<()> {
    if output.exists() && !ctx.overwrite {
        bail!("{} already exists (pass -y to replace it)", output.display());
    }
    Ok(())
}

/// Pick the encoder for an output `output_height` pixels tall (None if
/// unknown): an explicit `-e`, else the best working one for the codec.
pub(crate) fn resolve_encoder(args: &EncodeArgs, output_height: Option<u32>) -> Result<VideoEncode> {
    let mut why = String::new();
    let encoder = match &args.encoder {
        Some(name) => Encoder::lookup(name).ok_or_else(|| {
            let known: Vec<_> = encoders::KNOWN.iter().map(|e| e.name).collect();
            anyhow!("unknown encoder {name:?}; known: {}", known.join(", "))
        })?,
        None => {
            let codec = match args.codec {
                CodecArg::H264 => Codec::H264,
                CodecArg::Hevc => Codec::Hevc,
                CodecArg::Av1 => Codec::Av1,
            };
            let available = encoders::detect();
            let chosen = encoders::pick(&available, codec, !args.cpu, output_height)
                .ok_or_else(|| anyhow!("no working {codec:?} encoder found (see `mx doctor`)"))?;
            let gpu = encoders::pick(&available, codec, true, None).filter(|e| e.is_hardware());
            if let (false, Some(gpu), Some(h)) = (args.cpu, gpu, output_height)
                && !chosen.is_hardware()
            {
                why = format!(
                    " (CPU beats {} below {}p; this output is {h}p)",
                    gpu.name,
                    encoders::HARDWARE_MIN_HEIGHT
                );
            }
            chosen
        }
    };
    let quality = args.quality.unwrap_or_else(|| encoder.default_quality());
    println!("encoder  {} ({:?}), quality {quality}{why}", encoder.name, encoder.backend);
    Ok(VideoEncode { encoder, quality: Some(quality) })
}

/// Height of the input's video, if it has any.
fn video_height(input: &Path) -> Option<u32> {
    ffmpeg::probe(input).ok()?.video()?.height
}

pub(crate) fn run_job(ctx: &Ctx, label: &str, job: Job) -> Result<()> {
    let job = job.overwrite(ctx.overwrite);
    if ctx.verbose {
        let line: Vec<_> = job.command_line().iter().map(|a| a.to_string_lossy().into_owned()).collect();
        eprintln!("$ ffmpeg {}", line.join(" "));
    }
    let bar = progress_bar(label);
    let result = job.run(ctx.cancel, |p| show_progress(&bar, p));
    bar.finish_and_clear();
    result
}

pub(crate) fn progress_bar(label: &str) -> ProgressBar {
    let bar = ProgressBar::new(1000);
    bar.set_style(bar_style("{spinner} {prefix:<10} [{bar:30}] {percent:>3}% {msg}"));
    bar.set_prefix(label.to_string());
    bar.enable_steady_tick(Duration::from_millis(120));
    bar
}

pub(crate) fn show_progress(bar: &ProgressBar, p: &Progress) {
    if let Some(pct) = p.percent {
        bar.set_position((pct * 10.0) as u64);
    }
    let mut msg = Vec::new();
    if let Some(fps) = p.fps {
        msg.push(format!("{fps:.0} fps"));
    }
    if let Some(speed) = p.speed {
        msg.push(format!("{speed:.2}x"));
    }
    if let Some(size) = p.total_size {
        msg.push(HumanBytes(size).to_string());
    }
    // Remaining media time divided by encode speed = wall-clock left.
    if let (Some(pct), Some(t), Some(speed)) = (p.percent, p.out_time_secs, p.speed)
        && pct > 0.0
        && speed > 0.0
    {
        let remaining = t * (100.0 / pct - 1.0) / speed;
        msg.push(format!("eta {}", display::duration(remaining)));
    }
    bar.set_message(msg.join("  "));
}

/// Run `work`, then report time, size change and what's in the output.
pub(crate) fn finish(input: &Path, output: &Path, work: impl FnOnce() -> Result<()>) -> Result<()> {
    let started = Instant::now();
    work()?;
    let size = |p: &Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let (before, after) = (size(input), size(output));
    println!(
        "done in {:.1}s: {} -> {} ({:.0}% of original)",
        started.elapsed().as_secs_f64(),
        HumanBytes(before),
        HumanBytes(after),
        after as f64 / before.max(1) as f64 * 100.0
    );
    println!("\n{}", output.display());
    display::probe(&ffmpeg::probe(output)?);
    Ok(())
}

fn compress(ctx: &Ctx, input: &Path, output: &Path, target: u64, codec: CompressCodecArg) -> Result<()> {
    let codec = match codec {
        CompressCodecArg::H264 => CompressCodec::H264,
        CompressCodecArg::Hevc => CompressCodec::Hevc,
    };
    let source = ffmpeg::probe(input)?;
    let plan = ops::plan_compression(target, &source, codec)?;
    let mut changes = Vec::new();
    if let Some(fps) = plan.fps {
        changes.push(format!("{fps} fps"));
    }
    if let Some(p) = plan.short_edge {
        changes.push(format!("{p}p"));
    }
    if plan.stereo {
        changes.push("stereo".to_string());
    }
    let changes = if changes.is_empty() { String::new() } else { format!(" -> {}", changes.join(", ")) };
    println!(
        "plan     {} over {}: video {} kb/s + audio {} kb/s{changes}",
        DecimalBytes(target),
        display::duration(plan.duration_secs),
        plan.video_kbps,
        plan.audio_kbps
    );

    let compress = ops::compress(input, output, plan, codec);
    let mut outcome = None;
    finish(input, output, || {
        let bar = progress_bar("pass 1/2");
        let result = compress.run(ctx.cancel, ctx.overwrite, |stage, p| {
            if bar.prefix() != stage {
                bar.set_prefix(stage.to_string());
                bar.set_position(0);
            }
            show_progress(&bar, p);
        });
        bar.finish_and_clear();
        outcome = Some(result?);
        Ok(())
    })?;
    let outcome = outcome.expect("set on success");
    let retries = match outcome.attempts {
        1 => String::new(),
        n => format!(", after {} retr{} at {} kb/s", n - 1, if n == 2 { "y" } else { "ies" }, outcome.final_video_kbps),
    };
    println!(
        "
target   {} -> got {} ({:.1}% of target{retries})",
        DecimalBytes(target),
        DecimalBytes(outcome.bytes),
        outcome.bytes as f64 / target as f64 * 100.0
    );
    if outcome.over_target {
        eprintln!("warning: still over the target; try a slightly smaller --size");
    }
    Ok(())
}
