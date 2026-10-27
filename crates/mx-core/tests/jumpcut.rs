//! Silence detection and jump cuts on generated input. Skipped when ffmpeg
//! isn't installed.

use std::path::PathBuf;

use mx_core::CancelToken;
use mx_core::analysis::{SilenceOptions, find_silences, measure_loudness};
use mx_core::ffmpeg::encoders::Encoder;
use mx_core::ffmpeg::ops::{VideoEncode, jumpcut};
use mx_core::ffmpeg::{Job, probe, version};
use mx_core::timeline::keep_ranges;

#[test]
fn finds_the_pause_and_cuts_it_without_desync() {
    if version("ffmpeg").is_none() {
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("jumpcut");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // 6 s of tone with a 1 s gap at 2-3 s, over faint noise (about -50 dB)
    // so the pause isn't digital silence, like a real room.
    let source = dir.join("talk.mp4");
    let audio = "sine=frequency=440:duration=6,volume='if(between(t,2,3),0,0.5)':eval=frame[s];\
                 anoisesrc=duration=6:amplitude=0.003[n];[s][n]amix=inputs=2:normalize=0[a]";
    Job::new(&source)
        .args(["-f", "lavfi", "-i", "testsrc2=duration=6:size=320x240:rate=30"])
        .args(["-filter_complex", audio, "-map", "0:v", "-map", "[a]"])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest"])
        .overwrite(true)
        .run(&CancelToken::new(), |_| {})
        .unwrap();

    let envelope = measure_loudness(&source, &CancelToken::new(), |_| {}).unwrap();
    let report = find_silences(&envelope, &SilenceOptions::default());
    assert_eq!(report.silences.len(), 1, "{report:?}");
    let pause = report.silences[0];
    assert!((pause.start - 2.0).abs() < 0.1 && (pause.end - 3.0).abs() < 0.1, "{pause:?}");

    let keep = keep_ranges(6.0, &report.silences, 0.1);
    let video = VideoEncode { encoder: Encoder::lookup("libx264").unwrap(), quality: None };
    let output = dir.join("cut.mp4");
    let cut = jumpcut(&source, &output, &keep, Some(video)).unwrap();
    // 1 s pause minus 0.1 s padding each side = 0.8 s removed.
    assert!((cut.output_duration() - 5.2).abs() < 0.1, "{}", cut.output_duration());
    cut.job.clone().overwrite(true).run(&CancelToken::new(), |_| {}).unwrap();

    let streams = probe(&output).unwrap().streams;
    let duration = |kind: &str| {
        streams
            .iter()
            .find(|s| s.codec_type.as_deref() == Some(kind))
            .and_then(|s| s.duration_secs())
            .unwrap()
    };
    let (v, a) = (duration("video"), duration("audio"));
    assert!((v - 5.2).abs() < 0.1, "video {v}");
    // AAC adds a constant ~20-60 ms of priming/padding; drift would be more.
    assert!((a - v).abs() < 0.1, "audio {a} vs video {v}");
}
