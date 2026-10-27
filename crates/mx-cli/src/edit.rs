//! Local processing commands: convert, remux, trim, reframe, compress, speed, gif.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use indicatif::{DecimalBytes, HumanBytes, ProgressBar};

use crate::cli::{CodecArg, Command, CompressCodecArg, EncodeArgs, FitArg};
use crate::{bar_style, display};
use mx_core::{CancelToken, Cancelled};
use mx_core::ffmpeg::ops::{self, CompressCodec, Cut, Fit, GifOptions, VideoEncode};
use mx_core::ffmpeg::{self, Job, Progress, encoders, encoders::Codec, encoders::Encoder};
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
            let video = resolve_encoder(&encode)?;
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
            let cut = if fast { Cut::Fast } else { Cut::Exact(resolve_encoder(&encode)?) };
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
            let video = resolve_encoder(&encode)?;
            let fit = match fit {
                FitArg::Blur => Fit::Blur,
                FitArg::Crop => Fit::Crop { position },
                FitArg::Pad => Fit::Pad,
            };
            let (w, h) = aspect.frame(short_edge);
            println!("frame    {}:{} -> {w}x{h}", aspect.w, aspect.h);
            let job = ops::reframe(&input, &output, aspect, fit, short_edge, video);
            finish(&input, &output, || run_job(ctx, "reframe", job))
        }
        Command::Compress { input, output, size, codec } => {
            check_output(&output, ctx)?;
            compress(ctx, &input, &output, size, codec)
        }
        Command::Speed { input, output, factor, encode } => {
            check_output(&output, ctx)?;
            let video = resolve_encoder(&encode)?;
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

/// Fail before any slow work (encoder detection, pass 1) rather than after.
fn check_output(output: &Path, ctx: &Ctx) -> Result<()> {
    if output.exists() && !ctx.overwrite {
        bail!("{} already exists (pass -y to replace it)", output.display());
    }
    Ok(())
}

fn resolve_encoder(args: &EncodeArgs) -> Result<VideoEncode> {
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
            encoders::pick(&encoders::detect(), codec, !args.cpu)
                .ok_or_else(|| anyhow!("no working {codec:?} encoder found (see `mx doctor`)"))?
        }
    };
    let quality = args.quality.unwrap_or_else(|| encoder.default_quality());
    println!("encoder  {} ({:?}), quality {quality}", encoder.name, encoder.backend);
    Ok(VideoEncode { encoder, quality: Some(quality) })
}

fn run_job(ctx: &Ctx, label: &str, job: Job) -> Result<()> {
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

fn progress_bar(label: &str) -> ProgressBar {
    let bar = ProgressBar::new(1000);
    bar.set_style(bar_style("{spinner} {prefix:<10} [{bar:30}] {percent:>3}% {msg}"));
    bar.set_prefix(label.to_string());
    bar.enable_steady_tick(Duration::from_millis(120));
    bar
}

fn show_progress(bar: &ProgressBar, p: &Progress) {
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
fn finish(input: &Path, output: &Path, work: impl FnOnce() -> Result<()>) -> Result<()> {
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
    if let Some(h) = plan.height {
        changes.push(format!("{h}p"));
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
