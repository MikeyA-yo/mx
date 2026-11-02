mod cli;
mod display;
mod edit;
mod speech;

use std::process::{Command as Process, ExitCode};
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use indicatif::{HumanBytes, ProgressBar, ProgressStyle};

use cli::{Cli, Command, DownloadArgs, GlobalArgs};
use mx_core::ffmpeg::{self, encoders, encoders::Codec};
use mx_core::platform::Platform;
use mx_core::ytdlp::progress::Event;
use mx_core::ytdlp::{Auth, DownloadRequest, Selection, YtDlp};
use mx_core::{CancelToken, Cancelled};

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Ctrl+C reaches yt-dlp/ffmpeg too (same console), but flagging the
    // token lets the job runner clean up partial output before we exit.
    let cancel = CancelToken::new();
    let on_ctrl_c = cancel.clone();
    let _ = ctrlc::set_handler(move || on_ctrl_c.cancel());

    match run(cli, &cancel) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e.is::<Cancelled>() => {
            eprintln!("cancelled");
            ExitCode::from(130)
        }
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli, cancel: &CancelToken) -> Result<()> {
    match cli.command {
        Command::Info { url, json, playlist } => {
            let yt = ytdlp_from(&cli.global, &url)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&yt.dump_json(&url, playlist)?)?);
            } else {
                display::info(&yt.info(&url, playlist)?, Platform::detect(&url));
            }
        }
        Command::Formats { url } => {
            let yt = ytdlp_from(&cli.global, &url)?;
            display::formats(&yt.info(&url, false)?.formats);
        }
        Command::Download(args) => {
            let yt = ytdlp_from(&cli.global, &args.url)?;
            download(&yt, args, cancel)?;
        }
        Command::Probe { file } => display::probe(&ffmpeg::probe(&file)?),
        Command::Doctor => doctor(&cli.global),
        processing => {
            let ctx = edit::Ctx { cancel, overwrite: cli.global.overwrite, verbose: cli.global.verbose };
            match processing {
                speech @ (Command::Transcribe { .. } | Command::Captions { .. } | Command::Models { .. }) => {
                    speech::run(speech, &ctx)?
                }
                other => edit::run(other, &ctx)?,
            }
        }
    }
    Ok(())
}

fn ytdlp_from(global: &GlobalArgs, url: &str) -> Result<YtDlp> {
    let auth = Auth {
        cookies_file: global.cookies.clone(),
        cookies_from_browser: global.cookies_from_browser.clone(),
    };
    let platform = Platform::detect(url);
    if platform.usually_needs_login() && !auth.is_set() {
        eprintln!(
            "note: {} usually needs a login; if this fails, retry with --cookies-from-browser <browser>",
            platform.name()
        );
    }
    YtDlp::locate(global.yt_dlp.clone(), auth, global.verbose)
}

pub(crate) fn bar_style(template: &str) -> ProgressStyle {
    ProgressStyle::with_template(template)
        .expect("valid template")
        .progress_chars("=> ")
}

fn download(yt: &YtDlp, args: DownloadArgs, cancel: &CancelToken) -> Result<()> {
    let selection = if let Some(expr) = args.format {
        Selection::Raw(expr)
    } else if args.audio_only {
        Selection::Audio { codec: args.audio_format }
    } else {
        Selection::Best { max_height: args.max_height }
    };
    let req = DownloadRequest {
        output_dir: args.output,
        selection,
        playlist: args.playlist,
        compat: args.compat,
        container: args.container.map(|c| c.core()),
        info_json: args.info_json,
    };

    let bar = ProgressBar::new_spinner();
    bar.set_style(bar_style("{spinner} {prefix:<12} [{bar:30}] {bytes:>10}/{total_bytes:<10} {msg}"));
    bar.enable_steady_tick(Duration::from_millis(120));
    bar.set_prefix("resolving");

    let outcome = yt.download(&args.url, &req, cancel, |event| match event {
        Event::Download(p) => {
            bar.set_prefix(format!("fmt {}", p.format_id));
            if let Some(total) = p.total {
                bar.set_length(total);
            }
            bar.set_position(p.downloaded);
            let speed = p.speed.map(|s| format!("{}/s", HumanBytes(s as u64))).unwrap_or_default();
            let eta = p.eta_secs.map(|e| format!("eta {}", display::duration(e as f64))).unwrap_or_default();
            bar.set_message(format!("{speed} {eta}"));
        }
        Event::PostProcess { status, name } if status == "started" => {
            bar.set_prefix(name);
            bar.set_message("post-processing");
        }
        // Finished files are reported (and probed) below once yt-dlp exits.
        Event::PostProcess { .. } | Event::Done(_) => {}
    });
    bar.finish_and_clear();
    let outcome = outcome?;

    for w in &outcome.warnings {
        eprintln!("warning: {w}");
    }
    // Close the loop with ffprobe: what actually landed on disk, which is
    // what the processing stages will see.
    for file in &outcome.files {
        println!("\n{}", file.display());
        match ffmpeg::probe(file) {
            Ok(p) => display::probe(&p),
            Err(e) => eprintln!("  (probe failed: {e:#})"),
        }
    }
    if outcome.files.is_empty() {
        println!("nothing new downloaded (already present?)");
    }
    Ok(())
}

fn doctor(global: &GlobalArgs) {
    let check = |name: &str, found: Option<String>, why: &str| match found {
        Some(v) => println!("  ok       {name:<8} {v}"),
        None => println!("  MISSING  {name:<8} {why}"),
    };

    let yt = YtDlp::locate(global.yt_dlp.clone(), Auth::default(), false).ok();
    check(
        "yt-dlp",
        yt.as_ref().and_then(|y| Some(format!("{} (via `{}`)", y.version().ok()?, y.invocation()))),
        "pip install -U \"yt-dlp[default,curl-cffi]\"",
    );
    if let Some(yt) = &yt {
        check(
            "browser",
            yt.can_impersonate().ok().filter(|&ok| ok).map(|_| "impersonation available (curl_cffi)".to_string()),
            "TikTok and others send bot challenges without it: pip install -U \"yt-dlp[default,curl-cffi]\"",
        );
    }
    check("ffmpeg", ffmpeg::version("ffmpeg"), "needed to merge video+audio and for all processing");
    check("ffprobe", ffmpeg::version("ffprobe"), "ships with ffmpeg");
    let deno = Process::new("deno")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().next().map(str::to_string));
    check("deno", deno, "yt-dlp needs a JS runtime to solve YouTube's player challenges");

    println!("\n  working encoders (* = hardware, tried first):");
    let working = encoders::detect();
    for codec in [Codec::H264, Codec::Hevc, Codec::Av1] {
        let names: Vec<_> = working
            .iter()
            .filter(|e| e.codec == codec)
            .map(|e| if e.is_hardware() { format!("{}*", e.name) } else { e.name.to_string() })
            .collect();
        let list = if names.is_empty() { "none".to_string() } else { names.join(", ") };
        println!("    {:<5} {list}", format!("{codec:?}").to_lowercase());
    }
}
