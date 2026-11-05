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
            Segment { start: 0.5, end: 1.5, text: "first".into() },
            Segment { start: 2.0, end: 4.0, text: "cut".into() },
            Segment { start: 5.0, end: 6.0, text: "second".into() },
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
