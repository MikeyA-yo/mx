//! Highlights and clip export on generated input. Skipped when ffmpeg
//! isn't installed.

use std::path::PathBuf;

use mx_core::CancelToken;
use mx_core::analysis::{HighlightOptions, find_highlights, measure_signals};
use mx_core::ffmpeg::encoders::Encoder;
use mx_core::ffmpeg::ops::{Aspect, Fit, Frame, VideoEncode, clip};
use mx_core::ffmpeg::{Job, probe, version};

#[test]
fn finds_the_loud_moving_part_and_exports_it_vertically() {
    if version("ffmpeg").is_none() {
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("highlights");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // 12 s: a box that only flies across the frame at 6-9 s, and a tone
    // that is loud at 6-9 s and quiet elsewhere.
    let source = dir.join("match.mp4");
    let graph = "color=c=gray:s=320x240:r=30:d=12[bg];testsrc2=s=80x80:r=30:d=12[box];\
                 [bg][box]overlay=x='if(between(t,6,9),mod((t-6)*300,240),0)':y=80[v];\
                 sine=frequency=440:duration=12,volume='if(between(t,6,9),0.8,0.05)':eval=frame[s];\
                 anoisesrc=duration=12:amplitude=0.003[n];[s][n]amix=inputs=2:normalize=0[a]";
    Job::new(&source)
        .args(["-filter_complex", graph, "-map", "[v]", "-map", "[a]"])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest"])
        .overwrite(true)
        .run(&CancelToken::new(), |_| {})
        .unwrap();

    let signals = measure_signals(&source, &CancelToken::new(), |_| {}).unwrap();
    assert!(signals.loudness.is_some() && signals.motion.is_some());
    let found = find_highlights(&signals, &HighlightOptions { count: 1, length: 3.0, within: None });
    let h = &found[0];
    assert!((h.range.start - 6.0).abs() <= 0.5 && (h.range.end - 9.0).abs() <= 0.5, "{h:?}");
    assert!(h.breakdown.loudness.unwrap() > 0.9 && h.breakdown.motion.unwrap() > 0.9, "{h:?}");

    let output = dir.join("short.mp4");
    let frame = Frame { aspect: Aspect { w: 9, h: 16 }, fit: Fit::Blur, short_edge: 180 };
    let video = VideoEncode { encoder: Encoder::lookup("libx264").unwrap(), quality: None };
    clip(&source, &output, h.range, Some(frame), Some(video))
        .unwrap()
        .overwrite(true)
        .run(&CancelToken::new(), |_| {})
        .unwrap();

    let out = probe(&output).unwrap();
    let v = out.video().unwrap();
    assert_eq!((v.width, v.height), (Some(180), Some(320)));
    let len = h.range.len();
    for s in &out.streams {
        let d = s.duration_secs().unwrap();
        // AAC rounds up to whole 1024-sample frames.
        assert!((d - len).abs() < 0.05, "{:?} lasts {d}, expected {len}", s.codec_type);
    }
}
