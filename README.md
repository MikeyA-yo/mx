# mx — media extractor & processor

A Rust CLI and desktop editor that drive **yt-dlp** for extraction and
**ffmpeg** for processing.

yt-dlp already has ~1800 site extractors (YouTube, TikTok, Instagram, X, Facebook,
Reddit, Twitch, Vimeo, SoundCloud, ...), so `mx` doesn't reimplement them. It runs
yt-dlp as a subprocess and adds a typed layer on top: structured metadata, format
selection, machine-readable progress, error classification, and an ffprobe check
of every file it saves. Running yt-dlp as a separate process means
`pip install -U yt-dlp` fixes broken extractors without rebuilding `mx`.

## Requirements

- `yt-dlp`: `pip install -U "yt-dlp[default,curl-cffi]"`. Found on PATH, through `YTDLP_PATH`, or as `python -m yt_dlp`.
- `ffmpeg` + `ffprobe` on PATH (or `FFMPEG_PATH` / `FFPROBE_PATH`).
- `deno`: yt-dlp needs a JS runtime for YouTube.
- For `transcribe`/`captions`: an ffmpeg 8 build with the `whisper` filter (the gyan.dev
  "full" build has it). The speech model downloads on first use, to
  `%LOCALAPPDATA%\mx\models` (or `~/.cache/mx/models`; override with `MX_MODELS_DIR`).

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
mx download <url> --container mp4  # same streams, mp4 file (mp4 | mkv | webm)
mx download <url> -x --audio-format mp3
mx download <url> -f 137+140     # exact formats from `mx formats`
mx download <url> --playlist --info-json

mx probe <file>                  # ffprobe summary

mx convert in.mkv out.mp4        # H.264/AAC, GPU encoder if one works
mx convert in.mkv out.mkv -c hevc --quality 26
mx convert in.mkv out.mp4 --cpu  # force software (smaller files, slower)
mx convert in.mkv out.mp4 -e libx264 -y

mx remux in.mkv                  # -> in.mp4, lossless, about a second
mx remux in.mp4 --to mkv
mx remux in.mkv out.webm

mx trim in.mp4 clip.mp4 --from 1:00 --to 1:30         # frame-exact (re-encodes)
mx trim in.mp4 clip.mp4 --from 1:00 --length 30 --fast # instant, starts on a keyframe
mx reframe in.mp4 short.mp4                            # 9:16, blurred background
mx reframe in.mp4 square.mp4 -a 1:1 --fit crop --position 0.3
mx compress in.mp4 small.mp4 -s 10MB                   # two-pass, guaranteed under 10 MB
mx speed in.mp4 fast.mp4 2                             # 0.1x to 10x, audio keeps pitch
mx silence talk.mp4                                    # list pauses, preview what a jump cut removes
mx jumpcut talk.mp4 tight.mp4                          # remove pauses (video or audio-only)
mx jumpcut talk.mp4 tight.mp4 --min-silence 0.25 --padding 0.08 -t -38dB
mx highlights film.mp4                                 # the 3 most eventful 30 s stretches
mx highlights film.mp4 -n 5 -l 20 --to 7:15            # search only before the credits
mx highlights film.mp4 --export clips --vertical       # render them as 9:16 Shorts
mx gif in.mp4 loop.gif --from 5 --to 9 --fps 15 --width 480
mx join intro.mp4 talk.mp4 outro.mp4 -o video.mp4      # one video, clips fitted into the first one's frame
mx join a.mp4 b.mp4 -o short.mp4 -a 9:16 --music song.mp3 --music-volume 0.2
mx render my.mxp out.mp4                               # a project saved by the desktop editor

mx transcribe talk.mp4                                 # -> talk.srt (also .vtt, .txt, .json with -o)
mx transcribe talk.mp4 -l en -m small.en               # English-only model, more accurate
mx captions short.mp4 captioned.mp4                    # transcribe + burn in Shorts-style captions
mx captions short.mp4 captioned.mp4 --upper --words 3 --save-srt
mx captions in.mp4 out.mp4 --subs fixed.srt            # burn in subtitles you've corrected
mx captions in.mp4 out.mp4 --highlight '#ffd400'       # colour each word as it's spoken
mx models                                              # speech models: sizes, what's installed
```

Every command that re-encodes takes `-c h264|hevc|av1`, `-e <encoder>`, `--cpu`
and `--quality`. `-y` replaces existing outputs.

**Convert or remux?** `convert` re-encodes. It can change the codec, size and
quality, but it's slow and loses a little quality each time. `remux` copies
the same streams into a different container: lossless and near-instant, but
the codecs must fit. mkv holds anything; mp4 holds almost everything; webm
only holds VP8/VP9/AV1 with Opus/Vorbis. Every audio track is kept. Text
subtitles are converted to the target's subtitle format, and image subtitles
(Blu-ray PGS) are left out with a note, because only mkv can hold them.

**Jump cuts.** ffmpeg measures loudness every 20 ms, and the pauses are found
in Rust. A fixed threshold like -30 dB fails on real recordings: in a noisy
room the background never drops that low. So by default the threshold sits a
third of the way from the recording's noise floor to its speech level.
`--min-silence` skips gaps between words, and `--padding` keeps a little air
around each cut so words aren't clipped. Every cut is snapped to a video frame
and audio is cut at the exact sample, so sync holds: 246 cuts on a 25-minute
video ended with audio and video within the same 66 ms (AAC's fixed overhead)
as a 5-cut clip.

**Highlights.** Every one-second step of the timeline is a candidate window,
described by four signals: loudness, how little of it is silence, how much
the picture moves, and how many scene cuts it has. Each signal is ranked
against the rest of the same recording, because a podcast's "loud" is a
concert's "quiet". A signal that barely varies is left out instead of
ranking noise: a levelled voiceover has the same loudness everywhere, and a
static podcast shot never moves. The best windows are picked with space
between them, and their edges are moved onto a nearby pause or scene cut,
so clips don't start mid-word.

These signals measure activity, not meaning. They found the key scenes of
Big Buck Bunny, but scored its rolling credits (loud music, fast-moving
text) as action too, so `--to` keeps the search before them. On a talk or
tutorial, what makes a moment good is what's said; transcripts will cover
that. Measuring takes about a minute per 10 minutes of 720p (two ffmpeg
processes, audio and video in parallel) and is cached, so re-running with
different options is instant.

**Transcription.** ffmpeg's `whisper` filter runs whisper.cpp, which
always processes a 30 s window however little audio it gets. The filter's
default 3 s queue therefore costs a full window per 3 s (about realtime), and
transcribes each 3 s piece without context, so words break at the seams.
Its voice detection fixes the seams but ran at 0.05x realtime. So `mx` cuts
the audio at pauses into chunks of up to 29 s, pads each to exactly 30 s, and
transcribes them all in one ffmpeg run, mapping times back per chunk. With
base.en on an Intel laptop GPU that's about 3x realtime (the CPU alone is 4x
slower). Use `-m small.en` or larger when accuracy matters more: base heard
"epoll" as "E-Pul".

**Captions** are short phrases (up to `--words`, breaking at commas and full
stops) in big outlined text that pops in, higher up on vertical video to
stay clear of Shorts/TikTok buttons. whisper only gives sentence-level times,
so word times are estimated while transcribing: each word gets a share of
its sentence's *voiced* time by length, skipping the pauses the loudness
envelope found inside it, so a mid-sentence pause doesn't drag the words
after it early. Phrases are timed from their words, and `--highlight`
colours the word being spoken. Audio is
copied, not re-encoded. To fix mistakes, `--save-srt`, edit the .srt, and
burn it with `--subs`.

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

The GPU isn't always faster. GPU encoders pay a fixed cost per frame, so on
small video the CPU wins. On an Intel iGPU, QuickSync ran ~200-240 fps at both
360p and 1080p, while libx264 ran ~440 fps at 360p but was 2.8x slower at
1080p60. So auto-pick uses the CPU below 720p and says so.

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

## Desktop editor

`desktop/` is a Tauri app on the same engine: download from a link, then
edit on a timeline and export once.

A project is a row of clips. Add files with **+ Add clip** (or drop
several on the window), reorder them in the clip bar, and pick one to
edit it. Each clip keeps its own cuts, transcript and titles in its own
time, so reordering or recutting never strands a title; the frame,
caption style and music belong to the whole project. Playback runs on
from one clip into the next, with the music in step. Between clips, the
clip bar picks a transition: crossfade, through black, wipe, slide,
circle or zoom (`xfade`, with the sound crossfaded too); it overlaps the
two clips, so the video gets shorter by its length. ▶ beside it renders
the few seconds around the join at 360p with the export's own filters and
plays them.

**Projects** save as `.mxp` (<kbd>Ctrl+S</kbd>, <kbd>Ctrl+Shift+S</kbd> to
save as): mx-core's project JSON, which `mx render` reads, plus each
clip's editor state (pause settings, deleted transcript lines, picks).
Save suggests `Videos\mx\Projects`, so projects
are in one place. Every unsaved change is also autosaved, one file per
project (the last ten are kept), so closing the window loses nothing and
one project never overwrites another's autosave.

The start screen lists **recent projects**: saved files (folder, when,
which clips, Show in folder) and autosaved sessions, including unsaved
changes to a saved file, to pick up where they left off. "Find projects
on this computer" searches Videos, Documents, Desktop, Downloads and the
recent videos' folders for `.mxp` files. A project whose video has been
moved says where it was and offers **Find the file...** to point it at
the new place, its edits kept, or to remove that clip.

- **Cut**: find pauses and jump-cut them (sliders for the shortest pause,
  the breathing room kept and the threshold), or drag on the timeline and
  press <kbd>X</kbd>.
- **Text**: transcribe, then edit the video by deleting lines from the
  transcript. Burn the rest in as styled captions; the preview shows them
  where and how they'll be drawn.
- **Layers**: video or an image over the picture, started at a moment of a clip and
  shown for as long as you set, across later clips: full-frame b-roll
  (fitted like the clips) while the clip's voice carries on, or a
  picture-in-picture box dragged into place on the preview and resized by its corner. Muted unless you
  turn its sound up, faded in and out. An image (PNG, JPEG, WebP...)
  holds still for as long as it's set to show, and a PNG's transparency
  is kept, so a logo or sticker sits on the picture; images dropped on
  the window land on the clip on screen at the playhead. And text titles (colour, font,
  size, an optional box), dragged into place on the preview. Captions and
  titles draw over the video layers.
- **Picks**: the liveliest stretches (highlights), marked on the
  timeline, exported as clips.
- **Frame**: 9:16, 1:1, 4:5 or 16:9 with a blurred background, a crop
  (with position) or bars, previewed live. Below it, **zoom & pan** for
  the clip on screen: presets (slow zoom in/out, punch-in, pan across) or
  keyframes of zoom and position, eased between (a `scale` evaluated per
  frame, then a `crop` that follows the keyframed point).
- **Audio**: each clip's own sound from muted to 200% (turn it down
  under a voiceover, mute it for b-roll). **Background noise** reduction per
  clip: *Light* plays a pause from the recording (found by the loudness
  analysis) to an FFT denoiser (`afftdn`) first, so it knows the room's
  hum before the clip starts, and takes about 10 dB off it with the voice
  untouched; *Strong* runs RNNoise (`arnndn`, with the speech model
  `sh.rnnn` built into mx, from GregorR/rnnoise-models), about 25 dB off,
  made for talking (music in the clip goes too). Both run before cuts and
  start with an 80 Hz high-pass. "Listen" renders six seconds before and
  after to compare. **Sounds** (a voiceover, an
  effect) start at a moment of a clip and play to their own end, over the
  clips after it, with their own volume, fades and in/out points in the
  file; anchored to a clip, they stay in sync when clips move or are
  recut. **Music** under everything loops if it's short, starts where you
  choose in the song, fades out, and ducks under the clips and sounds
  (`sidechaincompress` keyed on them).
- **Export**: every clip, cut, the frame, captions, titles and music in
  **one encode** (`ops::render_project`), optionally two-passed under a
  file size. GIFs from the selection.

**The timeline** shows the **whole video** by default: every clip end to
end, as long as it plays after its cuts (a transition overlaps two), one
playhead across them all, and play runs on from clip to clip. Click
anywhere to jump there, in whichever clip plays it. Drag a clip's block to
reorder it, or its edges to trim its start or end (the cut Q or W makes,
moved). A selection can span clips: <kbd>X</kbd> cuts it from each one.
Layers drag between clips too: text, b-roll or a sound dropped on another
clip moves into it, at the moment under it. Any layer can also run on
across cuts and into the clips after its own: drag its end edge past the
clip's end (text holds on screen for the rest, sounds and videos keep
playing). **This clip** switches to one
clip in its own time, with what's cut still shown in red, for precise
trimming; edits there are the same edits.

Both show a filmstrip and waveform, and under them a lane each for text,
video layers and sounds.
Drag a layer to move it and its edges to trim it (a video or sound's start
edge starts the file later; it can't stretch past the file's own length);
edges snap to the playhead, the clip's ends, cuts and other layers, and
<kbd>Alt</kbd> drags freely. Clicking a layer opens it in its panel, and
<kbd>Delete</kbd> removes it. A selection's edges drag too.
<kbd>Ctrl</kbd>+wheel zooms around the pointer; the wheel scrolls a zoomed
timeline.

The side panel and the timeline resize like VS Code's panels: drag the
divider at the panel's edge or the timeline's top (a taller timeline has a
taller filmstrip and waveform); double-click it to reset. The sizes are kept.

Each clip's preview, thumbnails and waveform are made a few at a time
(the rest wait their turn in **Tasks**), so opening a project of many
clips doesn't start dozens of ffmpegs at once; they're kept, so the next
opening is quick. Closing the window stops whatever is still running,
and a preview cut short is never mistaken for a finished one.

**Undo** covers every edit in the project: cuts, transcript lines, layers,
zoom and pan, volumes, the frame, captions, music, transitions, and adding,
removing or reordering clips (a removed clip comes back with its edits).
A slider or drag is one step, however long it's held.

Keys: <kbd>Space</kbd> play, <kbd>←</kbd>/<kbd>→</kbd> 1 s (<kbd>Shift</kbd> 5 s),
<kbd>,</kbd>/<kbd>.</kbd> one frame, <kbd>I</kbd>/<kbd>O</kbd> selection start/end,
<kbd>X</kbd> cut the selection, <kbd>Q</kbd>/<kbd>W</kbd> cut everything
before/after the playhead, <kbd>Delete</kbd> remove the picked layer,
<kbd>Ctrl+Z</kbd> undo, <kbd>Ctrl+Shift+Z</kbd> or <kbd>Ctrl+Y</kbd> redo.

**Getting started.** The first launch walks through the app with a short
guided tour, and the first video opened gets a tour of the editor (a
spotlight on each part with a card saying what it's for). A quick-start
checklist above the tabs ticks off the usual steps (trim, captions, text,
shape, music, export) and opens the tab for each. **Help**, on the left,
replays either tour, shows the keyboard shortcuts (or press <kbd>?</kbd>)
and hides or brings back the checklist. Closing a project with unsaved
changes asks first.

Files the web view can't play (HEVC, 10-bit, mkv) get a preview: mkv is
remuxed to mp4 in seconds; anything else becomes a 540p proxy with a
keyframe every half second for quick scrubbing. Previews, thumbnails,
loudness, highlight measurements and transcripts are cached per file in
`%TEMP%\mx-cache`, so reopening a file is instant.

```sh
cd desktop
npm install
npm run tauri dev       # run it
npm run tauri build     # installer in target/release/bundle/nsis
```

It builds with either Rust toolchain on Windows. On the GNU toolchain it
needs MinGW's `windres` (MSYS2's mingw64 has it), and `WebView2Loader.dll`
from `target/<profile>/` must sit next to the exe.

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
| `crates/mx-core/src/ffmpeg/ops/` | ready-made jobs: transcode, remux, trim, reframe, clip, jumpcut, compress, speed, gif |
| `crates/mx-core/src/analysis/` | measurements and decisions: loudness, motion and scene cuts, silences, highlights |
| `crates/mx-core/src/speech/` | whisper models, transcription, subtitle files, caption styling |
| `crates/mx-core/src/timeline.rs` | time ranges, the building block of edit lists; mapping source time to output time |
| `crates/mx-core/src/ffmpeg/probe.rs` | ffprobe |
| `crates/mx-core/src/process.rs` | shared subprocess streaming, cancellable |
| `crates/mx-core/src/{platform,error}.rs` | site detection; yt-dlp errors → fixes |
| `crates/mx-core/src/units.rs` | parsing times (1:30) and sizes (10MB) |
| `crates/mx-core/src/project.rs` | projects: clips in order, music, titles; laying clips out on the output timeline |
| `crates/mx-core/src/ffmpeg/ops/render.rs` | one-pass render of a project or a single edit: cuts, fitting clips into one frame, joining, subtitles, music |
| `crates/mx-core/src/cache.rs` | per-file results kept between runs (measurements, previews, transcripts) |
| `crates/mx-cli/` | the `mx` binary: arguments (`cli.rs`), processing commands (`edit.rs`), output |
| `desktop/src-tauri/` | the desktop app's Rust side: commands over mx-core, and a task registry for progress and cancel |
| `desktop/src/` | the desktop UI (React + TypeScript): download, editor, timeline, panels |

## Roadmap

1. **Adaptive streaming**: encode a 1080/720/480/240 ladder with aligned keyframes, package as HLS/DASH.
2. **Auto-highlights**: done for activity (loudness, motion, cuts); next, transcript-aware picks.
3. **Subtitles**: done (Whisper → SRT/VTT, styled burn-in, editing by transcript); next, word-level timing.
4. **Desktop editor**: clips, video layers (b-roll, picture-in-picture), titles, music, sounds and clip volume, transitions (with previews), zoom & pan, saved projects, word highlighting, layer lanes with drag and trim, and project-wide undo done, and a whole-video timeline across clips done; next, a Windows installer, beat-synced cuts, stickers and images, speed ramps.
5. **Live recorder**: watch a channel, record when it goes live, handle stream drops.
6. **Enhancement**: Real-ESRGAN upscaling, RIFE interpolation, Demucs stem separation.
