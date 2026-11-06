//! One-pass renders (cuts + reframe + captions), previews and filmstrips
//! on generated input. Skipped when ffmpeg isn't installed.

use std::path::{Path, PathBuf};

use mx_core::CancelToken;
use mx_core::ffmpeg::encoders::Encoder;
use mx_core::ffmpeg::ops::{Aspect, Edit, Fit, Frame, PreviewKind, VideoEncode, filmstrip, preview, render};
use mx_core::ffmpeg::{Job, probe, version};
use mx_core::speech::{CaptionStyle, Segment, Transcript, cues, to_ass};
use mx_core::temp::TempFile;
use mx_core::timeline::{Range, keep_ranges};

fn dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn source(path: &Path, container_args: &[&str]) {
    Job::new(path)
        .args(["-f", "lavfi", "-i", "testsrc2=duration=10:size=320x240:rate=30"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=10"])
        .args(["-c:v", "libx264", "-preset", "ultrafast"])
        .args(container_args)
        .args(["-shortest"])
        .overwrite(true)
        .run(&CancelToken::new(), |_| {})
        .unwrap();
}

fn durations(path: &Path) -> (f64, f64) {
    let streams = probe(path).unwrap().streams;
    let d = |kind: &str| {
        streams.iter().find(|s| s.codec_type.as_deref() == Some(kind)).and_then(|s| s.duration_secs()).unwrap()
    };
    (d("video"), d("audio"))
}

#[test]
fn renders_cuts_frame_and_captions_in_one_pass() {
    if version("ffmpeg").is_none() {
        return;
    }
    let dir = dir("render");
    let input = dir.join("in.mp4");
    source(&input, &["-c:a", "aac"]);

    // Cut 2-4 s and 7-10 s: 5 s left, starting at 0.
    let transcript = Transcript {
        segments: vec![
            Segment { start: 0.5, end: 1.5, text: "first".into(), words: Vec::new() },
            Segment { start: 2.0, end: 4.0, text: "cut".into(), words: Vec::new() },
            Segment { start: 5.0, end: 6.0, text: "second".into(), words: Vec::new() },
        ],
    };
    let keep = keep_ranges(10.0, &[Range::new(2.0, 4.0), Range::new(7.0, 10.0)], 0.0);
    let retimed = transcript.retimed(&keep);
    assert_eq!(retimed.segments.len(), 2);
    assert!((retimed.segments[1].start - 3.0).abs() < 1e-9);

    let frame = Frame { aspect: Aspect { w: 9, h: 16 }, fit: Fit::Blur, short_edge: 240 };
    let (w, h) = frame.aspect.frame(frame.short_edge);
    let style = CaptionStyle::for_frame(w, h);
    let ass = TempFile::with_contents("test-render", "ass", &to_ass(&cues(&retimed, &style), w, h, &style)).unwrap();

    let output = dir.join("out.mp4");
    let edit = Edit { keep, frame: Some(frame), subtitles: Some(ass.path().to_path_buf()) };
    let video = VideoEncode { encoder: Encoder::lookup("libx264").unwrap(), quality: None };
    let job = render(&input, &output, &edit, Some(video)).unwrap();
    assert!((job.output_duration() - 5.0).abs() < 0.05);
    job.job.clone().overwrite(true).run(&CancelToken::new(), |_| {}).unwrap();

    let out = probe(&output).unwrap();
    let v = out.video().unwrap();
    assert_eq!((v.width, v.height), (Some(240), Some(426)));
    let (vd, ad) = durations(&output);
    assert!((vd - 5.0).abs() < 0.1, "video {vd}");
    assert!((ad - vd).abs() < 0.1, "audio {ad} vs video {vd}");
}

#[test]
fn a_single_late_range_seeks_instead_of_decoding_from_the_start() {
    if version("ffmpeg").is_none() {
        return;
    }
    let dir = dir("render-seek");
    let input = dir.join("in.mp4");
    source(&input, &["-c:a", "aac"]);
    let output = dir.join("clip.mp4");
    let video = VideoEncode { encoder: Encoder::lookup("libx264").unwrap(), quality: None };
    let edit = Edit { keep: vec![Range::new(6.0, 8.5)], ..Edit::default() };
    let job = render(&input, &output, &edit, Some(video)).unwrap();
    let line: Vec<String> = job.job.command_line().iter().map(|a| a.to_string_lossy().into_owned()).collect();
    assert_eq!(line[line.iter().position(|a| a == "-ss").unwrap() + 1], "6.000000");
    job.job.clone().overwrite(true).run(&CancelToken::new(), |_| {}).unwrap();
    let (vd, ad) = durations(&output);
    assert!((vd - 2.5).abs() < 0.1 && (ad - vd).abs() < 0.1, "video {vd}, audio {ad}");
}

#[test]
fn previews_mkv_by_remuxing_and_makes_a_filmstrip() {
    if version("ffmpeg").is_none() {
        return;
    }
    let dir = dir("preview");
    let input = dir.join("in.mkv");
    source(&input, &["-c:a", "aac"]);
    let src = probe(&input).unwrap();

    let out = dir.join("preview.mp4");
    let (kind, job) = preview(&input, &src, &out).unwrap();
    assert_eq!(kind, PreviewKind::Remux);
    job.unwrap().overwrite(true).run(&CancelToken::new(), |_| {}).unwrap();
    let (vd, _) = durations(&out);
    assert!((vd - 10.0).abs() < 0.1);

    let strip_path = dir.join("strip.jpg");
    let (strip, job) = filmstrip(&input, &src, &strip_path, 10, 36).unwrap();
    job.overwrite(true).run(&CancelToken::new(), |_| {}).unwrap();
    let img = probe(&strip_path).unwrap();
    let v = img.video().unwrap();
    assert_eq!((v.width, v.height), (Some(strip.thumb_width * 10), Some(36)));
}

#[test]
fn renders_a_project_of_mismatched_clips_with_music_and_a_title() {
    use mx_core::project::{Clip, Music, Project, Title};
    if version("ffmpeg").is_none() {
        return;
    }
    let dir = dir("project");
    // Clip a: 320x240 at 30 fps with a tone. Clip b: 640x360 at 25 fps, silent.
    let a = dir.join("a.mp4");
    source(&a, &["-c:a", "aac"]);
    let b = dir.join("b.mp4");
    Job::new(&b)
        .args(["-f", "lavfi", "-i", "smptebars=duration=4:size=640x360:rate=25"])
        .args(["-c:v", "libx264", "-preset", "ultrafast"])
        .overwrite(true)
        .run(&CancelToken::new(), |_| {})
        .unwrap();
    let song = dir.join("song.m4a");
    Job::new(&song)
        .args(["-f", "lavfi", "-i", "sine=frequency=220:duration=3", "-c:a", "aac"])
        .overwrite(true)
        .run(&CancelToken::new(), |_| {})
        .unwrap();

    let mut first = Clip::new(&a);
    first.keep = vec![Range::new(1.0, 3.0), Range::new(5.0, 6.0)];
    let mut second = Clip::new(&b);
    second.titles = vec![Title {
        text: "Part two".into(),
        start: 0.0,
        end: 3.0,
        x: 0.5,
        y: 0.3,
        size: 0.1,
        color: "#ffffff".into(),
        font: "Arial".into(),
        boxed: true,
    }];
    let project = Project {
        clips: vec![first, second],
        frame: Some(Frame { aspect: Aspect { w: 9, h: 16 }, fit: Fit::Blur, short_edge: 240 }),
        // Shorter than the video: it loops.
        music: Some(Music { source: song, offset: 0.0, volume: 0.3, duck: true, fade_out: 1.0 }),
        captions: None,
    };

    let output = dir.join("project.mp4");
    let video = VideoEncode { encoder: Encoder::lookup("libx264").unwrap(), quality: None };
    let job = mx_core::ffmpeg::ops::render_project(&project, &[], &output, Some(video)).unwrap();
    assert!((job.output_duration() - 7.0).abs() < 0.05, "{}", job.output_duration());
    assert!((job.layout[1].offset - 3.0).abs() < 0.05);
    job.job.clone().overwrite(true).run(&CancelToken::new(), |_| {}).unwrap();

    let out = probe(&output).unwrap();
    let v = out.video().unwrap();
    assert_eq!((v.width, v.height, v.fps().map(|f| f.round())), (Some(240), Some(426), Some(30.0)));
    let (vd, ad) = durations(&output);
    assert!((vd - 7.0).abs() < 0.1, "video {vd}");
    assert!((ad - vd).abs() < 0.1, "audio {ad} vs video {vd}");
}

#[test]
fn transitions_overlap_clips_and_captions_follow() {
    use mx_core::project::{Clip, Project, Transition, TransitionKind};
    use mx_core::speech::{CaptionStyle, Segment, Transcript};
    if version("ffmpeg").is_none() {
        return;
    }
    let dir = dir("transition");
    let a = dir.join("a.mp4");
    source(&a, &["-c:a", "aac"]);
    let b = dir.join("b.mp4");
    Job::new(&b)
        .args(["-f", "lavfi", "-i", "smptebars=duration=4:size=640x360:rate=25"])
        .args(["-f", "lavfi", "-i", "sine=frequency=880:duration=4"])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest"])
        .overwrite(true)
        .run(&CancelToken::new(), |_| {})
        .unwrap();

    let mut first = Clip::new(&a);
    first.keep = vec![Range::new(0.0, 3.0)];
    let mut second = Clip::new(&b);
    second.transition = Some(Transition { kind: TransitionKind::Fade, duration: 0.5 });
    second.transcript = Some(Transcript { segments: vec![Segment { start: 1.0, end: 2.0, text: "after the fade".into(), words: Vec::new() }] });
    let project = Project {
        clips: vec![first, second],
        captions: Some(CaptionStyle::for_frame(320, 240)),
        ..Project::default()
    };
    let output = dir.join("out.mp4");
    let video = VideoEncode { encoder: Encoder::lookup("libx264").unwrap(), quality: None };
    let job = mx_core::ffmpeg::ops::render_project(&project, &[], &output, Some(video)).unwrap();
    // 3 s + 4 s, overlapping by 0.5 s.
    assert!((job.output_duration() - 6.5).abs() < 0.05, "{}", job.output_duration());
    assert!((job.layout[1].offset - 2.5).abs() < 0.05);
    job.job.clone().overwrite(true).run(&CancelToken::new(), |_| {}).unwrap();
    let (vd, ad) = durations(&output);
    assert!((vd - 6.5).abs() < 0.1, "video {vd}");
    assert!((ad - vd).abs() < 0.1, "audio {ad} vs video {vd}");
}

#[test]
fn motion_zooms_along_keyframes_in_source_time() {
    use mx_core::project::{Clip, Keyframe, Project};
    if version("ffmpeg").is_none() {
        return;
    }
    let dir = dir("motion");
    let input = dir.join("in.mp4");
    source(&input, &["-c:a", "aac"]);
    // Keep 4-8 s (so the input is seeked to 4) and zoom in 2x from 5 s to 7 s.
    let mut clip = Clip::new(&input);
    clip.keep = vec![Range::new(4.0, 8.0)];
    clip.motion = vec![Keyframe { time: 5.0, zoom: 1.0, x: 0.5, y: 0.5 }, Keyframe { time: 7.0, zoom: 2.0, x: 0.25, y: 0.25 }];
    let project = Project { clips: vec![clip], ..Project::default() };
    let output = dir.join("out.mp4");
    let video = VideoEncode { encoder: Encoder::lookup("libx264").unwrap(), quality: None };
    let job = mx_core::ffmpeg::ops::render_project(&project, &[], &output, Some(video)).unwrap();
    job.job.clone().overwrite(true).run(&CancelToken::new(), |_| {}).unwrap();
    let out = probe(&output).unwrap();
    assert_eq!((out.video().unwrap().width, out.video().unwrap().height), (Some(320), Some(240)));
    let (vd, ad) = durations(&output);
    assert!((vd - 4.0).abs() < 0.1 && (ad - vd).abs() < 0.1, "video {vd}, audio {ad}");
}

/// RMS level (dBFS) of `path`'s audio between `from` and `to`.
fn level(path: &Path, from: f64, to: f64) -> f64 {
    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-ss", &from.to_string(), "-t", &(to - from).to_string(), "-i"])
        .arg(path)
        .args(["-af", "astats=metadata=1:reset=0,ametadata=print:key=lavfi.astats.Overall.RMS_level:file=-", "-f", "null", "-"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("lavfi.astats.Overall.RMS_level="))
        .next_back()
        .map_or(-120.0, |v| v.parse().unwrap_or(-120.0))
}

#[test]
fn sounds_play_over_muted_and_silent_clips_across_the_join() {
    use mx_core::project::{Clip, Project, Sound};
    if version("ffmpeg").is_none() {
        return;
    }
    let dir = dir("sounds");
    let a = dir.join("a.mp4");
    source(&a, &["-c:a", "aac"]);
    let b = dir.join("b.mp4");
    Job::new(&b)
        .args(["-f", "lavfi", "-i", "smptebars=duration=4:size=320x240:rate=30", "-c:v", "libx264", "-preset", "ultrafast"])
        .overwrite(true)
        .run(&CancelToken::new(), |_| {})
        .unwrap();
    let voice = dir.join("voice.m4a");
    Job::new(&voice)
        .args(["-f", "lavfi", "-i", "sine=frequency=330:duration=6", "-c:a", "aac"])
        .overwrite(true)
        .run(&CancelToken::new(), |_| {})
        .unwrap();

    // Clip a (10 s, its own tone muted), then silent clip b (4 s). The
    // voice starts 7 s into a and runs 6 s: 3 s over a, 3 s over b.
    let mut first = Clip::new(&a);
    first.volume = 0.0;
    first.sounds = vec![Sound { source: voice, at: 7.0, offset: 0.0, duration: None, volume: 1.0, fade_in: 0.0, fade_out: 0.0 }];
    let project = Project { clips: vec![first, Clip::new(&b)], ..Project::default() };
    let output = dir.join("out.mp4");
    let video = VideoEncode { encoder: Encoder::lookup("libx264").unwrap(), quality: None };
    let job = mx_core::ffmpeg::ops::render_project(&project, &[], &output, Some(video)).unwrap();
    job.job.clone().overwrite(true).run(&CancelToken::new(), |_| {}).unwrap();

    let (vd, ad) = durations(&output);
    assert!((vd - 14.0).abs() < 0.1 && (ad - vd).abs() < 0.1, "video {vd}, audio {ad}");
    // Muted clip, before the voice: silence. Then the voice, on both sides of the join.
    assert!(level(&output, 1.0, 6.0) < -60.0, "{}", level(&output, 1.0, 6.0));
    assert!(level(&output, 7.5, 9.5) > -30.0, "{}", level(&output, 7.5, 9.5));
    assert!(level(&output, 10.5, 12.5) > -30.0, "{}", level(&output, 10.5, 12.5));
    assert!(level(&output, 13.3, 13.9) < -60.0, "{}", level(&output, 13.3, 13.9));
}

#[test]
fn overlays_cover_or_box_the_picture_while_the_voice_carries_on() {
    use mx_core::project::{Clip, Overlay, Pip, Project};
    if version("ffmpeg").is_none() {
        return;
    }
    let dir = dir("overlays");
    let main = dir.join("main.mp4");
    source(&main, &["-c:a", "aac"]);
    let broll = dir.join("broll.mp4");
    Job::new(&broll)
        .args(["-f", "lavfi", "-i", "smptebars=duration=6:size=640x360:rate=25", "-c:v", "libx264", "-preset", "ultrafast"])
        .overwrite(true)
        .run(&CancelToken::new(), |_| {})
        .unwrap();

    let mut clip = Clip::new(&main);
    let over = |at: f64, duration: f64, pip: Option<Pip>| Overlay { source: broll.clone(), at, offset: 1.0, duration: Some(duration), pip, volume: 0.0, fade: 0.0 };
    clip.overlays = vec![over(2.0, 3.0, None), over(6.0, 2.0, Some(Pip { x: 0.75, y: 0.75, size: 0.4 }))];
    let project = Project { clips: vec![clip], ..Project::default() };
    let output = dir.join("out.mp4");
    let video = VideoEncode { encoder: Encoder::lookup("libx264").unwrap(), quality: None };
    let job = mx_core::ffmpeg::ops::render_project(&project, &[], &output, Some(video)).unwrap();
    job.job.clone().overwrite(true).run(&CancelToken::new(), |_| {}).unwrap();

    let (vd, ad) = durations(&output);
    assert!((vd - 10.0).abs() < 0.1 && (ad - vd).abs() < 0.1, "video {vd}, audio {ad}");
    // The main clip's tone plays straight through the cutaway.
    assert!(level(&output, 2.5, 4.5) > -30.0, "{}", level(&output, 2.5, 4.5));
}

#[test]
fn noise_reduction_quiets_the_background_and_keeps_the_voice() {
    use mx_core::project::{Clip, Denoise, Project};
    if version("ffmpeg").is_none() || !mx_core::ffmpeg::has_filter("arnndn") {
        return;
    }
    let dir = dir("denoise");
    // 8 s of steady pink noise (a fan), with a loud tone from 3 to 6 s
    // standing in for a voice: the stretches either side are the pauses.
    let input = dir.join("noisy.mp4");
    Job::new(&input)
        .args(["-f", "lavfi", "-i", "smptebars=duration=8:size=320x240:rate=30"])
        .args(["-f", "lavfi", "-i", "anoisesrc=color=pink:amplitude=0.05:duration=8:sample_rate=48000"])
        .args(["-f", "lavfi", "-i", "sine=frequency=330:duration=8:sample_rate=48000,volume=0.3,volume=volume=0:enable='not(between(t,3,6))'"])
        .args(["-filter_complex", "[1:a][2:a]amix=inputs=2:normalize=0[a]", "-map", "0:v", "-map", "[a]"])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-b:a", "192k"])
        .overwrite(true)
        .run(&CancelToken::new(), |_| {})
        .unwrap();

    let render_with = |denoise: Denoise, name: &str| {
        // The first half second is cut, so the input is seeked: the pause
        // Light learns from has to be found in the seeked input's time.
        let mut clip = Clip::new(&input);
        clip.keep = vec![Range::new(0.5, 8.0)];
        clip.denoise = denoise;
        let project = Project { clips: vec![clip], ..Project::default() };
        let output = dir.join(name);
        let video = VideoEncode { encoder: Encoder::lookup("libx264").unwrap(), quality: None };
        let job = mx_core::ffmpeg::ops::render_project(&project, &[], &output, Some(video)).unwrap();
        job.job.clone().overwrite(true).run(&CancelToken::new(), |_| {}).unwrap();
        output
    };
    let off = render_with(Denoise::Off, "off.mp4");
    let light = render_with(Denoise::Light, "light.mp4");
    let strong = render_with(Denoise::Strong, "strong.mp4");

    // Output time: the noise alone until 2.5 s, the tone from 2.5 to 5.5 s.
    let noise = |p: &Path| level(p, 0.5, 2.0);
    let tone = |p: &Path| level(p, 3.0, 5.0);
    assert!(noise(&light) < noise(&off) - 5.0, "light {} vs off {}", noise(&light), noise(&off));
    assert!(noise(&strong) < noise(&off) - 10.0, "strong {} vs off {}", noise(&strong), noise(&off));
    assert!((tone(&light) - tone(&off)).abs() < 2.0, "light tone {} vs off {}", tone(&light), tone(&off));
    let (vd, ad) = durations(&strong);
    assert!((vd - 7.5).abs() < 0.1 && (ad - vd).abs() < 0.1, "video {vd}, audio {ad}");
}
