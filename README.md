# mx — media extractor & processor

A Rust CLI that drives **yt-dlp** for extraction and **ffmpeg** for processing.

yt-dlp already has ~1800 site extractors (YouTube, TikTok, Instagram, X, Facebook,
Reddit, Twitch, Vimeo, SoundCloud, ...), so `mx` doesn't reimplement them. It runs
yt-dlp as a subprocess and adds a typed layer on top: structured metadata, format
selection, machine-readable progress, error classification, and an ffprobe check
of every file it saves. Running yt-dlp as a separate process means
`pip install -U yt-dlp` fixes broken extractors without rebuilding `mx`.

## Requirements

- `yt-dlp`: `pip install -U "yt-dlp[default]"`. Found on PATH, through `YTDLP_PATH`, or as `python -m yt_dlp`.
- `ffmpeg` + `ffprobe` on PATH (or `FFMPEG_PATH` / `FFPROBE_PATH`).
- `deno`: yt-dlp needs a JS runtime for YouTube.

Run `mx doctor` to check all of them.

## Usage

```sh
mx info <url>                    # title, uploader, duration, best resolution, subtitle languages
mx info <url> --json             # raw yt-dlp JSON
mx info --playlist <url>         # list playlist/channel entries (flat, fast)
mx formats <url>                 # every format: resolution, codec, bitrate, size, protocol

mx download <url>                # best quality -> mp4, or mkv if codecs need it
mx download <url> -q 720         # cap at 720p
mx download <url> --compat       # prefer H.264/AAC mp4 (editor-friendly)
mx download <url> -x --audio-format mp3
mx download <url> -f 137+140     # exact formats from `mx formats`
mx download <url> --playlist --info-json

mx probe <file>                  # ffprobe summary

mx convert in.mkv out.mp4        # H.264/AAC, GPU encoder if one works
mx convert in.mkv out.mkv -c hevc --quality 26
mx convert in.mkv out.mp4 --cpu  # force software (smaller files, slower)
mx convert in.mkv out.mp4 -e libx264 -y

mx trim in.mp4 clip.mp4 --from 1:00 --to 1:30         # frame-exact (re-encodes)
mx trim in.mp4 clip.mp4 --from 1:00 --length 30 --fast # instant, starts on a keyframe
mx reframe in.mp4 short.mp4                            # 9:16, blurred background
mx reframe in.mp4 square.mp4 -a 1:1 --fit crop --position 0.3
mx compress in.mp4 small.mp4 -s 10MB                   # two-pass, guaranteed under 10 MB
mx speed in.mp4 fast.mp4 2                             # 0.1x to 10x, audio keeps pitch
mx gif in.mp4 loop.gif --from 5 --to 9 --fps 15 --width 480
```

Every command that re-encodes takes `-c h264|hevc|av1`, `-e <encoder>`, `--cpu`
and `--quality`. `-y` replaces existing outputs.

**Trim modes.** Without re-encoding, a cut can only start on a keyframe.
`--fast` looks up that keyframe, cuts from it and tells you where it is. The
default re-encodes and starts on the exact frame.

**Compress.** Plans a bitrate from size ÷ duration. When the budget is tight it
drops to 30 fps first and then lowers the resolution; surround audio becomes
stereo. If x264's two-pass encode overshoots the target (it does by a few
percent on short clips), pass 2 is re-run at a corrected bitrate.

`mx doctor` test-encodes with every known encoder (NVENC, QuickSync, AMF,
x264/x265/SVT-AV1) and lists the ones that actually work here. ffmpeg builds
include GPU encoders whether or not the hardware is present.

Ctrl+C stops any download or encode cleanly and removes the partial file.

Files are saved as `downloads/<Extractor>/[<playlist>/]<title> [<id>].<ext>`.

### Logins and bot checks

Instagram, Facebook and Vimeo usually need a logged-in session. YouTube asks
IPs it has flagged to prove they aren't bots. `mx` recognises these errors and
tells you which fix applies:

```sh
mx download <url> --cookies-from-browser firefox
mx download <url> --cookies cookies.txt
```

YouTube's bot check is about the IP, not the account. A shared network like office
Wi-Fi or a VPN can trigger it when mobile data doesn't.

On Windows, `--cookies-from-browser` with Chrome, Brave or Edge usually fails:
these browsers lock the cookie file while running, and Chrome 127+ encrypts
cookies in a way yt-dlp can't read. Use Firefox, or export a `cookies.txt` with an
extension such as "Get cookies.txt LOCALLY" and pass `--cookies`.

## Layout

This is a Cargo workspace. `mx-core` is the engine. Every front end (the CLI
now, a desktop UI later) only builds requests and renders the events it sends
back. Long-running calls take a `CancelToken` and a progress callback, and
their data types implement `Serialize`, so a UI can forward them as JSON.

| Path | Role |
|---|---|
| `crates/mx-core/src/ytdlp/` | locate and run yt-dlp; typed metadata; progress templates |
| `crates/mx-core/src/ffmpeg/job.rs` | run any ffmpeg command with `-progress` parsing and cancellation |
| `crates/mx-core/src/ffmpeg/encoders.rs` | encoder catalogue, runtime detection, quality flags per backend |
| `crates/mx-core/src/ffmpeg/ops/` | ready-made jobs: transcode, trim, reframe, compress, speed, gif |
| `crates/mx-core/src/ffmpeg/probe.rs` | ffprobe |
| `crates/mx-core/src/process.rs` | shared subprocess streaming, cancellable |
| `crates/mx-core/src/{platform,error}.rs` | site detection; yt-dlp errors → fixes |
| `crates/mx-core/src/units.rs` | parsing times (1:30) and sizes (10MB) |
| `crates/mx-cli/` | the `mx` binary: arguments (`cli.rs`), processing commands (`edit.rs`), output |

## Roadmap

1. **Adaptive streaming**: encode a 1080/720/480/240 ladder with aligned keyframes, package as HLS/DASH.
2. **Auto-highlights**: scene detection, loudness and motion scoring, then cut clips and reframe them vertically.
3. **Subtitles**: Whisper → SRT, optional translation, burned-in styled captions.
4. **Live recorder**: watch a channel, record when it goes live, handle stream drops.
5. **Enhancement**: Real-ESRGAN upscaling, RIFE interpolation, Demucs stem separation.
