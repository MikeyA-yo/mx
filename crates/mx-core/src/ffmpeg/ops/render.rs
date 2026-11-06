//! Rendering in a single encode: one source with cuts, a new frame shape
//! and burned-in captions ([`render`]), or a whole [`Project`] of clips
//! with music ([`render_project`]). One pass is faster and better than
//! chaining `jumpcut`, `reframe` and `captions`: the video is decoded and
//! encoded once, so it loses quality once.
//!
//! Each clip is cut with the jump-cut machinery (see [`super::jumpcut`]):
//! video is made constant frame rate and `select`ed frame-exactly, audio
//! is `atrim`med at the matching sample. Clips are then fitted into one
//! frame and joined with `concat`, which needs every piece to share size,
//! frame rate and audio format. Subtitles and titles are drawn on the
//! joined result, so they're timed on the output; music is mixed in last,
//! ducked under the clips' own sound.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use super::clip::Frame;
use super::jumpcut::{audio_chain, parse_rate, select_expr, snap};
use super::reframe::{Fit, filtergraph};
use super::{VideoEncode, aac, audio_codec, finish};
use crate::ffmpeg::{Job, Probe, filter_path, probe};
use super::denoise::{Denoiser, quiet_stretch};
use crate::project::{Clip, Denoise, Keyframe, Placement, Project, length, output_transcript, titles_ass};
use crate::speech::{cues, to_ass};
use crate::temp::TempFile;
use crate::timeline::{Range, total};

/// What to make from one source.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Edit {
    /// Source ranges to keep. Empty keeps everything. They're played in
    /// time order, and overlapping ranges are merged.
    #[serde(default)]
    pub keep: Vec<Range>,
    /// A new frame shape, e.g. 9:16.
    #[serde(default)]
    pub frame: Option<Frame>,
    /// Subtitles (ASS keeps its styling, SRT works too) drawn into the
    /// picture. They're timed on the output, after cuts.
    #[serde(default)]
    pub subtitles: Option<PathBuf>,
}

/// A render ready to run. Holds the filter script, which is deleted when
/// this is dropped.
#[derive(Debug)]
pub struct Render {
    pub job: Job,
    /// Where each clip landed: its kept ranges after snapping to video
    /// frames, and its start on the output timeline.
    pub layout: Vec<Placement>,
    _script: Option<TempFile>,
    /// Captions and titles drawn by the job.
    _overlays: Vec<TempFile>,
}

impl Render {
    pub fn output_duration(&self) -> f64 {
        length(&self.layout)
    }

    /// The first (for [`render`], the only) clip's kept ranges.
    pub fn kept(&self) -> &[Range] {
        &self.layout[0].keep
    }
}

/// Sorted, clamped to the source, overlaps merged.
fn normalize(keep: &[Range], duration: f64) -> Vec<Range> {
    let mut ranges: Vec<Range> = keep
        .iter()
        .map(|r| Range::new(r.start.max(0.0), r.end.min(duration)))
        .filter(|r| !r.is_empty())
        .collect();
    ranges.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut merged: Vec<Range> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// One clip, as the graph sees it.
#[derive(Debug, Clone)]
struct ClipPlan {
    /// Kept ranges relative to the (possibly seeked) input; `None` keeps
    /// the whole input.
    cuts: Option<Vec<Range>>,
    /// Source frame size, when there's video.
    size: Option<(u32, u32)>,
    audio: bool,
    /// Output length of this clip.
    duration: f64,
    volume: f64,
    /// Noise reduction for its sound, run before anything is cut, and the
    /// input holding the pause it learns from, when it learns from one.
    denoise: Option<(Denoiser, Option<usize>)>,
    /// `xfade` transition from the clip before, and how long it overlaps.
    transition: Option<(&'static str, f64)>,
    /// Zoom/pan filters, run first on the clip's picture.
    motion: Option<String>,
}

#[derive(Debug, Clone)]
struct MusicPlan {
    input: usize,
    volume: f64,
    duck: bool,
    fade_out: f64,
}

/// An overlay, already trimmed by its input options.
#[derive(Debug, Clone)]
struct OverlayPlan {
    input: usize,
    /// Output time it starts at.
    start: f64,
    length: f64,
    fade: f64,
    /// Box width in pixels and centre (0..1), or `None` to cover the frame.
    pip: Option<(u32, f64, f64)>,
}

/// A sound, already trimmed by its input options.
#[derive(Debug, Clone)]
struct SoundPlan {
    input: usize,
    /// Output time it starts at.
    start: f64,
    /// How long it plays.
    length: f64,
    volume: f64,
    fade_in: f64,
    fade_out: f64,
}

#[derive(Debug)]
struct Plan {
    clips: Vec<ClipPlan>,
    /// Frame rate as ffmpeg wrote it and as a number; `None` = audio only.
    rate: Option<(String, f64)>,
    /// Output frame and how clips are fitted into it.
    size: (u32, u32),
    fit: Fit,
    /// A frame was asked for, so even a matching clip is passed through
    /// the fit (it also sets the pixel aspect).
    reframe: bool,
    /// Escaped filter paths, drawn in order.
    subtitles: Vec<String>,
    music: Option<MusicPlan>,
    sounds: Vec<SoundPlan>,
    overlays: Vec<OverlayPlan>,
}

/// The filter graph pieces, and the pads to map.
#[derive(Debug, Default, PartialEq)]
struct Graph {
    parts: Vec<String>,
    video: Option<String>,
    audio: Option<String>,
}

const AUDIO_FORMAT: &str = "aformat=sample_rates=48000:channel_layouts=stereo";

/// Join clips one pair at a time: `xfade` + `acrossfade` where a clip has
/// a transition in, a two-piece `concat` where it doesn't. `xfade` also
/// needs both sides on the same time base and pixel format.
fn join_with_transitions(g: &mut Graph, plan: &Plan, has_video: bool, any_audio: bool) -> (Option<String>, Option<String>) {
    let pad = |p: &Option<String>| p.as_ref().map(|p| format!("[{p}]")).unwrap_or_default();
    let (mut v, mut a) = (None::<String>, None::<String>);
    let mut length = 0.0;
    for (i, c) in plan.clips.iter().enumerate() {
        let vi = has_video.then(|| {
            g.parts.push(format!("[v{i}]settb=AVTB,format=yuv420p[t{i}]"));
            format!("t{i}")
        });
        let ai = any_audio.then(|| format!("a{i}"));
        if i == 0 {
            (v, a, length) = (vi, ai, c.duration);
            continue;
        }
        match c.transition {
            Some((kind, d)) => {
                if let (Some(prev), Some(next)) = (&v, &vi) {
                    let at = length - d;
                    g.parts.push(format!("[{prev}][{next}]xfade=transition={kind}:duration={d:.6}:offset={at:.6}[xv{i}]"));
                    v = Some(format!("xv{i}"));
                }
                if let (Some(prev), Some(next)) = (&a, &ai) {
                    g.parts.push(format!("[{prev}][{next}]acrossfade=d={d:.6}:c1=tri:c2=tri[xa{i}]"));
                    a = Some(format!("xa{i}"));
                }
                length += c.duration - d;
            }
            None => {
                let ins = format!("{}{}{}{}", pad(&v), pad(&a), pad(&vi), pad(&ai));
                v = has_video.then(|| format!("xv{i}"));
                a = any_audio.then(|| format!("xa{i}"));
                let outs = format!("{}{}", pad(&v), pad(&a));
                g.parts.push(format!("{ins}concat=n=2:v={}:a={}{outs}", usize::from(has_video), usize::from(any_audio)));
                length += c.duration;
            }
        }
    }
    (v, a)
}

fn graph(plan: &Plan) -> Graph {
    let mut g = Graph::default();
    // One clip, no music: pass streams through untouched where possible.
    let single = plan.clips.len() == 1 && plan.music.is_none() && plan.sounds.is_empty() && plan.overlays.is_empty();
    // Sounds are mixed into the clips' audio, so silent clips still need
    // a track for them to be mixed into.
    let any_audio = plan.clips.iter().any(|c| c.audio) || !plan.sounds.is_empty();
    let mut joined = Vec::new();

    for (i, c) in plan.clips.iter().enumerate() {
        let mut pads = String::new();
        if let Some((rate, fps)) = &plan.rate {
            let mut filters: Vec<String> = c.motion.iter().cloned().collect();
            match &c.cuts {
                Some(cuts) => {
                    filters.push(format!("fps={rate}"));
                    filters.push(format!("select='{}'", select_expr(cuts, 0.5 / fps)));
                    filters.push("setpts=N/FRAME_RATE/TB".into());
                }
                // Joined clips must share a frame rate and start at zero.
                None if !single => {
                    filters.push(format!("fps={rate}"));
                    filters.push("setpts=PTS-STARTPTS".into());
                }
                None => {}
            }
            let fits = plan.reframe || !single || c.size != Some(plan.size);
            let mut pad = format!("{i}:v:0");
            if !filters.is_empty() {
                g.parts.push(format!("[{pad}]{}[vc{i}]", filters.join(",")));
                pad = format!("vc{i}");
            }
            if fits {
                g.parts.push(filtergraph(&pad, &format!("v{i}"), plan.fit, plan.size.0, plan.size.1));
                pad = format!("v{i}");
            }
            pads += &format!("[{pad}]");
        }
        if any_audio {
            let gain = if (c.volume - 1.0).abs() > 1e-6 { format!(",volume={:.3}", c.volume) } else { String::new() };
            let mut source = format!("{i}:a:0");
            if let (Some((d, pause)), true) = (&c.denoise, c.audio) {
                let pause = pause.map(|n| format!("{n}:a:0"));
                g.parts.push(d.graph(&source, pause.as_deref(), &format!("dn{i}")));
                source = format!("dn{i}");
            }
            let pad = match (&c.cuts, c.audio) {
                (Some(cuts), true) => {
                    g.parts.push(audio_chain(&source, &format!("c{i}"), cuts, &format!("ac{i}")));
                    if single && gain.is_empty() {
                        format!("ac{i}")
                    } else {
                        g.parts.push(format!("[ac{i}]{AUDIO_FORMAT}{gain}[a{i}]"));
                        format!("a{i}")
                    }
                }
                (None, true) if single && gain.is_empty() => source,
                (None, true) => {
                    g.parts.push(format!("[{source}]{AUDIO_FORMAT},asetpts=PTS-STARTPTS{gain}[a{i}]"));
                    format!("a{i}")
                }
                // A silent clip between ones with sound still needs its
                // stretch of audio, or concat would misalign the rest.
                (_, false) => {
                    g.parts.push(format!("anullsrc=r=48000:cl=stereo,atrim=duration={:.6}[a{i}]", c.duration));
                    format!("a{i}")
                }
            };
            pads += &format!("[{pad}]");
        }
        joined.push(pads);
    }

    let has_video = plan.rate.is_some();
    let (mut video, mut audio) = if plan.clips.len() == 1 {
        let pads = &joined[0];
        // Split "[v][a]" back into its pads.
        let mut it = pads.trim_start_matches('[').trim_end_matches(']').split("][");
        let v = has_video.then(|| it.next().unwrap_or_default().to_string());
        let a = any_audio.then(|| it.next().unwrap_or_default().to_string());
        (v, a)
    } else if plan.clips.iter().all(|c| c.transition.is_none()) {
        let n = plan.clips.len();
        let (v, a) = (usize::from(has_video), usize::from(any_audio));
        let outs = format!("{}{}", if has_video { "[vcat]" } else { "" }, if any_audio { "[acat]" } else { "" });
        g.parts.push(format!("{}concat=n={n}:v={v}:a={a}{outs}", joined.concat()));
        (has_video.then(|| "vcat".to_string()), any_audio.then(|| "acat".to_string()))
    } else {
        join_with_transitions(&mut g, plan, has_video, any_audio)
    };

    // Overlays over the joined picture, under captions and titles. Each
    // is brought to the frame rate and frame (or box), faded through its
    // alpha, and shifted to its start; `overlay` shows it while it has
    // frames and passes the picture through before and after.
    if let (Some(v), Some((rate, _))) = (&mut video, &plan.rate) {
        let (w, h) = plan.size;
        for (k, o) in plan.overlays.iter().enumerate() {
            g.parts.push(format!("[{}:v:0]fps={rate},setpts=PTS-STARTPTS[ovs{k}]", o.input));
            let (x, y) = match o.pip {
                None => {
                    g.parts.push(filtergraph(&format!("ovs{k}"), &format!("ovf{k}"), plan.fit, w, h));
                    ("0".to_string(), "0".to_string())
                }
                Some((pw, cx, cy)) => {
                    g.parts.push(format!("[ovs{k}]scale={pw}:-2,setsar=1[ovf{k}]"));
                    (format!("W*{cx:.4}-w/2"), format!("H*{cy:.4}-h/2"))
                }
            };
            let fade_out = (o.length - o.fade).max(0.0);
            g.parts.push(format!(
                "[ovf{k}]format=yuva420p,fade=t=in:d={:.3}:alpha=1,fade=t=out:st={fade_out:.6}:d={:.3}:alpha=1,\
                 setpts=PTS+{:.6}/TB[ov{k}]",
                o.fade, o.fade, o.start
            ));
            g.parts.push(format!("[{v}][ov{k}]overlay=x={x}:y={y}:eof_action=pass:repeatlast=0[vo{k}]"));
            *v = format!("vo{k}");
        }
    }

    if let Some(v) = &mut video {
        for (k, subs) in plan.subtitles.iter().enumerate() {
            g.parts.push(format!("[{v}]subtitles=filename={subs}[vs{k}]"));
            *v = format!("vs{k}");
        }
    }

    let total: f64 = plan.clips.iter().map(|c| c.duration - c.transition.map_or(0.0, |t| t.1)).sum();
    if !plan.sounds.is_empty()
        && let Some(a) = &audio
    {
        // Each sound faded, delayed to its start, and mixed over the clips'
        // sound; the clips decide the length.
        let mut ins = format!("[{a}]");
        for (k, s) in plan.sounds.iter().enumerate() {
            let fade_out_at = (s.length - s.fade_out).max(0.0);
            let delay = (s.start * 1000.0).round().max(0.0) as u64;
            g.parts.push(format!(
                "[{}:a:0]{AUDIO_FORMAT},asetpts=PTS-STARTPTS,volume={:.3},afade=t=in:d={:.3},\
                 afade=t=out:st={fade_out_at:.6}:d={:.3},adelay=delays={delay}:all=1[snd{k}]",
                s.input, s.volume, s.fade_in, s.fade_out
            ));
            ins += &format!("[snd{k}]");
        }
        g.parts.push(format!("{ins}amix=inputs={}:duration=first:normalize=0[asnd]", plan.sounds.len() + 1));
        audio = Some("asnd".to_string());
    }

    if let Some(m) = &plan.music {
        let fade_at = (total - m.fade_out).max(0.0);
        g.parts.push(format!(
            "[{}:a:0]{AUDIO_FORMAT},atrim=duration={total:.6},asetpts=PTS-STARTPTS,volume={:.3},\
             afade=t=out:st={fade_at:.6}:d={:.3}[mus]",
            m.input, m.volume, m.fade_out
        ));
        audio = Some(match audio {
            Some(a) if m.duck => {
                // The clips' sound (and any sounds over them) is the
                // sidechain: while it's loud the music is pushed down, and
                // it comes back in the pauses.
                g.parts.push(format!("[{a}]asplit=2[amain][akey]"));
                g.parts.push("[mus][akey]sidechaincompress=threshold=0.02:ratio=10:attack=15:release=350[mduck]".into());
                g.parts.push("[amain][mduck]amix=inputs=2:duration=first:normalize=0[amix]".into());
                "amix".to_string()
            }
            Some(a) => {
                g.parts.push(format!("[{a}][mus]amix=inputs=2:duration=first:normalize=0[amix]"));
                "amix".to_string()
            }
            None => "mus".to_string(),
        });
    }

    // Graph outputs are mapped as "[pad]", input streams as "0:v:0".
    let map = |pad: String| if pad.contains(':') { pad } else { format!("[{pad}]") };
    g.video = video.map(map);
    g.audio = audio.map(map);
    g
}

/// An ffmpeg expression for one keyframe value at time `t` (an
/// expression too): eased with a half cosine between keyframes, held
/// outside them. Keyframes must be sorted by time.
fn eased(keys: &[Keyframe], value: impl Fn(&Keyframe) -> f64, t: &str) -> String {
    let v = |k: &Keyframe| format!("{:.5}", value(k));
    let mut expr = v(&keys[keys.len() - 1]);
    for pair in keys.windows(2).rev() {
        let (a, b) = (&pair[0], &pair[1]);
        let span = (b.time - a.time).max(1e-3);
        expr = format!(
            "if(lt({t},{:.4}),{}+({}-{})*(0.5-0.5*cos(PI*({t}-{:.4})/{span:.4})),{expr})",
            b.time,
            v(a),
            v(b),
            v(a),
            a.time
        );
    }
    format!("if(lt({t},{:.4}),{},{expr})", keys[0].time, v(&keys[0]))
}

/// Filters that zoom and pan a `w`x`h` picture along `keys`, keeping its
/// size: scale up by the zoom (re-evaluated every frame), then crop a
/// `w`x`h` window centred on the keyframed point, held inside the
/// picture. `seek` is where the input starts in source time, since
/// keyframes are in source time and a seeked input's clock starts at 0.
/// `None` when the path never zooms in.
fn motion_filters(keys: &[Keyframe], (w, h): (u32, u32), seek: f64) -> Option<String> {
    let mut keys: Vec<Keyframe> = keys
        .iter()
        .map(|k| Keyframe { time: k.time, zoom: k.zoom.clamp(1.0, 8.0), x: k.x.clamp(0.0, 1.0), y: k.y.clamp(0.0, 1.0) })
        .collect();
    keys.sort_by(|a, b| a.time.total_cmp(&b.time));
    if keys.iter().all(|k| k.zoom < 1.001) {
        return None;
    }
    let t = format!("(t+{seek:.6})");
    let z = eased(&keys, |k| k.zoom, &t);
    let x = eased(&keys, |k| k.x, &t);
    let y = eased(&keys, |k| k.y, &t);
    let (w, h) = ((w / 2).max(1) * 2, (h / 2).max(1) * 2);
    Some(format!(
        "scale=w='trunc(iw*{z}/2)*2':h='trunc(ih*{z}/2)*2':eval=frame:flags=bicubic,\
         crop={w}:{h}:x='max(0,min(iw-ow,iw*{x}-ow/2))':y='max(0,min(ih-oh,ih*{y}-oh/2))'"
    ))
}

/// Render `edit` of `input` to `output`. `video` is required when the
/// input has video.
pub fn render(input: &Path, output: &Path, edit: &Edit, video: Option<VideoEncode>) -> Result<Render> {
    let mut clip = Clip::new(input);
    clip.keep = edit.keep.clone();
    let project = Project { clips: vec![clip], frame: edit.frame, ..Project::default() };
    let subtitles: Vec<PathBuf> = edit.subtitles.iter().cloned().collect();
    render_project(&project, &subtitles, output, video)
}

/// Render `project` to `output` with its captions and titles. Other
/// `subtitles` (ASS or SRT files timed on the output) are drawn first.
/// `video` is required when the clips have video.
pub fn render_project(project: &Project, subtitles: &[PathBuf], output: &Path, video: Option<VideoEncode>) -> Result<Render> {
    if project.clips.is_empty() {
        bail!("there are no clips to render");
    }
    let sources: Vec<Probe> = project.clips.iter().map(|c| probe(&c.source)).collect::<Result<_>>()?;
    let durations: Vec<f64> = sources
        .iter()
        .zip(&project.clips)
        .map(|(s, c)| s.duration_secs().ok_or_else(|| anyhow!("couldn't read the duration of {}", c.source.display())))
        .collect::<Result<_>>()?;

    let has_video = sources.iter().any(|s| s.video().is_some());
    if has_video {
        if let Some(i) = sources.iter().position(|s| s.video().is_none()) {
            bail!("{} has no video; every clip in a video needs some", project.clips[i].source.display());
        }
    } else if project.frame.is_some() || !subtitles.is_empty() {
        bail!("there's no video to reframe or caption");
    }

    // Every clip is converted to the first clip's frame rate and fitted
    // into one frame: the requested one, else the first clip's.
    let rate_text = sources[0].video().and_then(|v| v.avg_frame_rate.clone());
    let rate = match (has_video, rate_text.as_deref().and_then(|r| Some((r.to_string(), parse_rate(r)?)))) {
        (false, _) => None,
        (true, Some(rate)) => Some(rate),
        (true, None) => bail!("couldn't read the frame rate of {}", project.clips[0].source.display()),
    };
    let even = |x: u32| (x / 2).max(1) * 2;
    let size = match (project.frame, sources[0].video()) {
        (Some(f), _) => f.aspect.frame(f.short_edge),
        (None, Some(v)) => (even(v.width.unwrap_or(1920)), even(v.height.unwrap_or(1080))),
        (None, None) => (0, 0),
    };
    let fit = project.frame.map_or(Fit::Blur, |f| f.fit);

    let mut job = Job::new(output);
    let mut keeps = Vec::new();
    let mut clips = Vec::new();
    // Pauses to learn background noise from, by clip.
    let mut pauses = Vec::new();
    for (i, (clip, source)) in project.clips.iter().zip(&sources).enumerate() {
        let d = durations[i];
        let mut kept = if clip.keep.is_empty() { vec![Range::new(0.0, d)] } else { normalize(&clip.keep, d) };
        if let Some((_, fps)) = &rate {
            kept = snap(&kept, *fps);
        }
        if kept.is_empty() {
            bail!("nothing is kept of {}", clip.source.display());
        }
        let whole = kept.len() == 1 && kept[0].start < 0.001 && kept[0].end > d - 0.001;
        // Start decoding where the first kept range begins, and stop
        // after the last: a 30 s clip from the end of an hour-long file
        // shouldn't decode the hour. Seeking restarts timestamps at 0.
        let seek = if whole { 0.0 } else { kept[0].start };
        if !whole {
            if seek > 0.0 {
                job = job.args(["-ss", &format!("{seek:.6}")]);
            }
            job = job.args(["-t", &format!("{:.6}", kept[kept.len() - 1].end - seek)]);
        }
        job = job.input(&clip.source);

        let length = total(&kept);
        clips.push(ClipPlan {
            cuts: (!whole).then(|| kept.iter().map(|r| Range::new(r.start - seek, r.end - seek)).collect()),
            size: source.video().and_then(|v| v.width.zip(v.height)),
            audio: source.has_audio(),
            duration: length,
            volume: clip.volume.max(0.0),
            denoise: match (clip.denoise, source.has_audio()) {
                (Denoise::Off, _) | (_, false) => None,
                (d, true) => {
                    // Light learns from a pause anywhere in the file, cut
                    // or not: it's fed in as an input of its own, below.
                    let quiet = if d == Denoise::Light { quiet_stretch(&clip.source)? } else { None };
                    let denoiser = Denoiser::new(d, quiet.map(|q| q.end - q.start))?;
                    if let (Some(q), Some(true)) = (quiet, denoiser.as_ref().map(Denoiser::wants_pause)) {
                        pauses.push((clips.len(), q));
                    }
                    denoiser.map(|d| (d, None))
                }
            },
            transition: None,
            motion: match (source.video().and_then(|v| v.width.zip(v.height)), clip.motion.is_empty()) {
                (Some(size), false) => motion_filters(&clip.motion, size, seek),
                _ => None,
            },
        });
        keeps.push(kept);
    }
    let layout = project.arrange(keeps);
    for ((plan, place), clip) in clips.iter_mut().zip(&layout).zip(&project.clips) {
        if place.overlap > 0.0 {
            plan.transition = clip.transition.map(|t| (t.kind.xfade(), place.overlap));
        }
    }
    let duration = length(&layout);

    // Captions and titles are drawn on the output, through the same
    // placement as the clips.
    let mut overlays = Vec::new();
    if has_video {
        let (w, h) = size;
        if let Some(style) = &project.captions {
            let transcripts: Vec<_> = project.clips.iter().map(|c| c.transcript.clone()).collect();
            let timed = output_transcript(&layout, &transcripts);
            if !timed.segments.is_empty() {
                overlays.push(TempFile::with_contents("captions", "ass", &to_ass(&cues(&timed, style), w, h, style))?);
            }
        }
        if project.clips.iter().any(|c| !c.titles.is_empty()) {
            overlays.push(TempFile::with_contents("titles", "ass", &titles_ass(&project.clips, &layout, w, h))?);
        }
    }

    let music = match &project.music {
        Some(m) => {
            // Loop the song if the video outlasts it.
            job = job.args(["-stream_loop", "-1"]);
            if m.offset > 0.0 {
                job = job.args(["-ss", &format!("{:.6}", m.offset)]);
            }
            job = job.input(&m.source);
            Some(MusicPlan {
                input: project.clips.len(),
                volume: m.volume.max(0.0),
                duck: m.duck,
                fade_out: m.fade_out.max(0.0),
            })
        }
        None => None,
    };

    // Sounds and overlays: each trimmed by input options, then placed at
    // its clip's moment. One that would outlast the video is cut at its
    // end.
    let mut next_input = project.clips.len() + usize::from(music.is_some());
    let mut add_input = |job: Job, source: &Path, offset: f64, length: f64| {
        let job = if offset > 0.0 { job.args(["-ss", &format!("{offset:.6}")]) } else { job };
        next_input += 1;
        (job.args(["-t", &format!("{length:.6}")]).input(source), next_input - 1)
    };
    let length_of = |source: &Path, offset: f64, wanted: Option<f64>, start: f64| -> Result<f64> {
        let available = match wanted {
            Some(d) => d,
            None => probe(source)?.duration_secs().ok_or_else(|| anyhow!("couldn't read the duration of {}", source.display()))? - offset,
        };
        Ok(available.min(duration - start))
    };
    let mut sounds = Vec::new();
    for (clip, place) in project.clips.iter().zip(&layout) {
        for s in &clip.sounds {
            let start = place.at(s.at);
            let length = length_of(&s.source, s.offset, s.duration, start)?;
            if length < 0.05 {
                continue;
            }
            let input;
            (job, input) = add_input(job, &s.source, s.offset, length);
            sounds.push(SoundPlan {
                input,
                start,
                length,
                volume: s.volume.max(0.0),
                fade_in: s.fade_in.clamp(0.0, length / 2.0),
                fade_out: s.fade_out.clamp(0.0, length / 2.0),
            });
        }
    }
    let mut layers = Vec::new();
    for (clip, place) in project.clips.iter().zip(&layout) {
        for o in &clip.overlays {
            if !has_video {
                bail!("overlays need a video to go over");
            }
            let source = probe(&o.source)?;
            if source.video().is_none() {
                bail!("{} has no video to overlay", o.source.display());
            }
            let start = place.at(o.at);
            let length = length_of(&o.source, o.offset, o.duration, start)?;
            if length < 0.05 {
                continue;
            }
            let input;
            (job, input) = add_input(job, &o.source, o.offset, length);
            let fade = o.fade.clamp(0.0, length / 2.0);
            let pip = o.pip.map(|p| ((((p.size.clamp(0.05, 1.0) * size.0 as f64) / 2.0).round() as u32).max(1) * 2, p.x.clamp(0.0, 1.0), p.y.clamp(0.0, 1.0)));
            layers.push(OverlayPlan { input, start, length, fade, pip });
            // Its own sound, when it's turned up, mixes in like a sound.
            if o.volume > 0.0 && source.has_audio() {
                sounds.push(SoundPlan { input, start, length, volume: o.volume, fade_in: fade, fade_out: fade });
            }
        }
    }

    // The pauses noise reduction learns from, each a short input of its own.
    for (i, q) in pauses {
        let input;
        (job, input) = add_input(job, &project.clips[i].source, q.start, q.end - q.start);
        if let Some((_, pause)) = &mut clips[i].denoise {
            *pause = Some(input);
        }
    }

    let subtitles = subtitles
        .iter()
        .map(PathBuf::as_path)
        .chain(overlays.iter().map(TempFile::path))
        .map(filter_path)
        .collect::<Result<Vec<_>>>()?;
    let plan = Plan { clips, rate, size, fit, reframe: project.frame.is_some(), subtitles, music, sounds, overlays: layers };
    let g = graph(&plan);

    // Hundreds of ranges overflow Windows' 32k command-line limit, so the
    // graph goes in a file, loaded with ffmpeg 7+'s `-/option file` syntax.
    let script = match g.parts.is_empty() {
        true => None,
        false => Some(TempFile::with_contents("render", "txt", &g.parts.join(";\n"))?),
    };
    if let Some(s) = &script {
        job = job.arg("-/filter_complex").arg(s.path());
    }
    if let Some(pad) = &g.video {
        let video = video.ok_or_else(|| anyhow!("the clips have video, so a video encoder is needed"))?;
        job = job.args(["-map", pad]).args(video.args());
    }
    if let Some(pad) = &g.audio {
        job = job.args(["-map", pad]);
        let untouched = pad.contains(':');
        let codec = sources[0].streams.iter().find(|s| s.codec_type.as_deref() == Some("audio")).and_then(|s| s.codec_name.as_deref());
        let ext = output.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        job = if untouched && codec == Some("aac") && ["mp4", "m4v", "mov", "mkv", "m4a"].contains(&ext.as_str()) {
            // Untouched audio is copied: re-encoding AAC is the slow part
            // of an otherwise GPU-encoded render.
            job.args(["-c:a", "copy"])
        } else if g.video.is_some() {
            job.args(aac(192))
        } else {
            job.args(audio_codec(output))
        };
    }
    // Looping music never ends by itself; the clips decide the length.
    if plan.music.is_some() {
        job = job.args(["-t", &format!("{duration:.6}")]);
    }
    Ok(Render { job: finish(job.duration(duration), output), layout, _script: script, _overlays: overlays })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::ops::Aspect;

    fn r(start: f64, end: f64) -> Range {
        Range::new(start, end)
    }

    fn clip(cuts: Option<Vec<Range>>, size: (u32, u32), audio: bool) -> ClipPlan {
        let duration = cuts.as_ref().map_or(10.0, |c| total(c));
        ClipPlan { cuts, size: Some(size), audio, duration, volume: 1.0, denoise: None, transition: None, motion: None }
    }

    fn plan(clips: Vec<ClipPlan>) -> Plan {
        Plan {
            clips,
            rate: Some(("25/1".into(), 25.0)),
            size: (320, 240),
            fit: Fit::Blur,
            reframe: false,
            subtitles: vec![],
            music: None,
            sounds: vec![],
            overlays: vec![],
        }
    }

    #[test]
    fn keep_ranges_are_sorted_clamped_and_merged() {
        let n = normalize(&[r(5.0, 7.0), r(-1.0, 2.0), r(1.5, 3.0), r(9.0, 12.0), r(4.0, 4.0)], 10.0);
        assert_eq!(n, [r(0.0, 3.0), r(5.0, 7.0), r(9.0, 10.0)]);
    }

    #[test]
    fn cuts_shift_video_boundaries_by_half_a_frame() {
        let g = graph(&plan(vec![clip(Some(vec![r(1.0, 2.0), r(3.0, 4.0)]), (320, 240), true)]));
        let s = g.parts.join(";\n");
        assert!(s.contains("[0:v:0]fps=25/1,select='gte(t,0.980000)*lt(t,1.980000)+gte(t,2.980000)*lt(t,3.980000)'"), "{s}");
        assert!(s.contains("[0:a:0]asplit=2[c0s0][c0s1];"), "{s}");
        assert!(s.contains("[c0s1]atrim=start=3.000000:end=4.000000,asetpts=PTS-STARTPTS[c0t1];"), "{s}");
        assert!(s.contains("[c0t0][c0t1]concat=n=2:v=0:a=1[ac0]"), "{s}");
        // Same size, no frame asked for: no scaling.
        assert!(!s.contains("scale"), "{s}");
        assert_eq!((g.video.as_deref(), g.audio.as_deref()), (Some("[vc0]"), Some("[ac0]")));
    }

    #[test]
    fn untouched_single_clip_maps_streams_directly() {
        let g = graph(&plan(vec![clip(None, (320, 240), true)]));
        assert_eq!(g, Graph { parts: vec![], video: Some("0:v:0".into()), audio: Some("0:a:0".into()) });
    }

    #[test]
    fn frame_and_subtitles_chain_after_the_cuts() {
        let mut p = plan(vec![clip(Some(vec![r(0.0, 1.0)]), (320, 240), false)]);
        p.size = Aspect { w: 9, h: 16 }.frame(720);
        p.fit = Fit::Pad;
        p.reframe = true;
        p.subtitles = vec!["'c.ass'".into(), "'t.ass'".into()];
        let g = graph(&p);
        assert!(g.parts[1].starts_with("[vc0]scale=720:1280") && g.parts[1].ends_with("[v0]"), "{:?}", g.parts);
        assert_eq!(g.parts[2], "[v0]subtitles=filename='c.ass'[vs0]");
        assert_eq!(g.parts[3], "[vs0]subtitles=filename='t.ass'[vs1]");
        assert_eq!((g.video.as_deref(), g.audio), (Some("[vs1]"), None));
    }

    #[test]
    fn clips_are_fitted_normalized_and_joined() {
        // The second clip is a different size and has no sound.
        let g = graph(&plan(vec![clip(None, (320, 240), true), clip(Some(vec![r(2.0, 3.0)]), (1920, 1080), false)]));
        let s = g.parts.join(";\n");
        assert!(s.contains("[0:v:0]fps=25/1,setpts=PTS-STARTPTS[vc0]"), "{s}");
        assert!(s.contains("[vc1]split=2[bg][fg]") || s.contains("[vc1]split"), "{s}");
        assert!(s.contains("[0:a:0]aformat=sample_rates=48000:channel_layouts=stereo,asetpts=PTS-STARTPTS[a0]"), "{s}");
        assert!(s.contains("anullsrc=r=48000:cl=stereo,atrim=duration=1.000000[a1]"), "{s}");
        assert!(s.contains("[v0][a0][v1][a1]concat=n=2:v=1:a=1[vcat][acat]"), "{s}");
        assert_eq!((g.video.as_deref(), g.audio.as_deref()), (Some("[vcat]"), Some("[acat]")));
    }

    #[test]
    fn transitions_crossfade_pairs_and_plain_cuts_concat() {
        let mut b = clip(None, (320, 240), true);
        b.transition = Some(("fadeblack", 0.5));
        let c = clip(None, (320, 240), true);
        let g = graph(&plan(vec![clip(None, (320, 240), true), b, c]));
        let s = g.parts.join(";\n");
        assert!(s.contains("[v0]settb=AVTB,format=yuv420p[t0]"), "{s}");
        // The second clip starts half a second before the first one's 10 s end.
        assert!(s.contains("[t0][t1]xfade=transition=fadeblack:duration=0.500000:offset=9.500000[xv1]"), "{s}");
        assert!(s.contains("[a0][a1]acrossfade=d=0.500000:c1=tri:c2=tri[xa1]"), "{s}");
        assert!(s.contains("[xv1][xa1][t2][a2]concat=n=2:v=1:a=1[xv2][xa2]"), "{s}");
        assert_eq!((g.video.as_deref(), g.audio.as_deref()), (Some("[xv2]"), Some("[xa2]")));
    }

    #[test]
    fn music_is_trimmed_faded_and_ducked_under_the_clips() {
        let mut p = plan(vec![clip(None, (320, 240), true), clip(None, (320, 240), true)]);
        p.music = Some(MusicPlan { input: 2, volume: 0.25, duck: true, fade_out: 2.0 });
        let s = graph(&p);
        let text = s.parts.join(";\n");
        assert!(text.contains("[2:a:0]aformat=sample_rates=48000:channel_layouts=stereo,atrim=duration=20.000000"), "{text}");
        assert!(text.contains("volume=0.250,afade=t=out:st=18.000000:d=2.000[mus]"), "{text}");
        assert!(text.contains("[acat]asplit=2[amain][akey]"), "{text}");
        assert!(text.contains("[mus][akey]sidechaincompress"), "{text}");
        assert_eq!(s.audio.as_deref(), Some("[amix]"));
    }

    #[test]
    fn motion_eases_between_keyframes_and_holds_outside_them() {
        let k = |time, zoom| Keyframe { time, zoom, x: 0.5, y: 0.5 };
        let e = eased(&[k(1.0, 1.0), k(3.0, 2.0)], |k| k.zoom, "T");
        assert_eq!(e, "if(lt(T,1.0000),1.00000,if(lt(T,3.0000),1.00000+(2.00000-1.00000)*(0.5-0.5*cos(PI*(T-1.0000)/2.0000)),2.00000))");
        let f = motion_filters(&[k(0.0, 1.0), k(2.0, 1.5)], (321, 240), 4.0).unwrap();
        assert!(f.starts_with("scale=w='trunc(iw*if(lt((t+4.000000),0.0000)"), "{f}");
        assert!(f.contains("eval=frame") && f.contains("crop=320:240:"), "{f}");
        // Never zooming in is no motion at all.
        assert!(motion_filters(&[k(0.0, 1.0), k(2.0, 0.5)], (320, 240), 0.0).is_none());
    }

    #[test]
    fn sounds_are_delayed_faded_and_mixed_over_silent_clips() {
        let mut p = plan(vec![clip(None, (320, 240), false), clip(None, (320, 240), false)]);
        p.sounds = vec![SoundPlan { input: 2, start: 3.25, length: 4.0, volume: 0.8, fade_in: 0.5, fade_out: 1.0 }];
        let g = graph(&p);
        let s = g.parts.join(";\n");
        // No clip has sound, so both get silence for the voice to sit on.
        assert!(s.contains("anullsrc=r=48000:cl=stereo,atrim=duration=10.000000[a0]"), "{s}");
        assert!(s.contains("[2:a:0]aformat=sample_rates=48000:channel_layouts=stereo,asetpts=PTS-STARTPTS,volume=0.800,afade=t=in:d=0.500,"), "{s}");
        assert!(s.contains("afade=t=out:st=3.000000:d=1.000,adelay=delays=3250:all=1[snd0]"), "{s}");
        assert!(s.contains("[acat][snd0]amix=inputs=2:duration=first:normalize=0[asnd]"), "{s}");
        assert_eq!(g.audio.as_deref(), Some("[asnd]"));
    }

    #[test]
    fn overlays_are_fitted_faded_shifted_and_drawn_under_the_text() {
        let mut p = plan(vec![clip(None, (320, 240), true)]);
        p.subtitles = vec!["'c.ass'".into()];
        p.overlays = vec![
            OverlayPlan { input: 1, start: 2.5, length: 3.0, fade: 0.2, pip: None },
            OverlayPlan { input: 2, start: 4.0, length: 2.0, fade: 0.0, pip: Some((96, 0.75, 0.25)) },
        ];
        let g = graph(&p);
        let s = g.parts.join(";\n");
        assert!(s.contains("[1:v:0]fps=25/1,setpts=PTS-STARTPTS[ovs0]"), "{s}");
        assert!(s.contains("[ovs0]split=2[bg][fg]"), "{s}");
        assert!(s.contains("[ovf0]format=yuva420p,fade=t=in:d=0.200:alpha=1,fade=t=out:st=2.800000:d=0.200:alpha=1,setpts=PTS+2.500000/TB[ov0]"), "{s}");
        assert!(s.contains("[ovs1]scale=96:-2,setsar=1[ovf1]"), "{s}");
        assert!(s.contains("[vo0][ov1]overlay=x=W*0.7500-w/2:y=H*0.2500-h/2:eof_action=pass:repeatlast=0[vo1]"), "{s}");
        // Captions go on top of the overlays.
        assert!(s.contains("[vo1]subtitles=filename='c.ass'[vs0]"), "{s}");
        assert_eq!(g.video.as_deref(), Some("[vs0]"));
    }

    #[test]
    fn audio_only_projects_have_no_video_chain() {
        let mut p = plan(vec![clip(Some(vec![r(1.0, 2.0)]), (0, 0), true)]);
        p.rate = None;
        let g = graph(&p);
        assert!(g.video.is_none() && !g.parts.iter().any(|p| p.contains("0:v")), "{:?}", g.parts);
    }
}
