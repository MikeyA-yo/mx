//! Transcription and caption burn-in on synthesized speech. Skipped unless
//! ffmpeg has the `flite` (speech synthesis) and `whisper` filters and a
//! whisper model is already installed: tests never download models.

use std::path::PathBuf;

use mx_core::CancelToken;
use mx_core::ffmpeg::encoders::Encoder;
use mx_core::ffmpeg::ops::{VideoEncode, burn_subtitles};
use mx_core::ffmpeg::{Job, has_filter, probe};
use mx_core::speech::{CaptionStyle, Model, TranscribeOptions, cues, to_ass, transcribe};
use mx_core::temp::TempFile;

#[test]
fn transcribes_speech_and_burns_it_in() {
    let Some(model) = ["tiny.en", "base.en", "tiny", "base"]
        .iter()
        .filter_map(|n| Model::lookup(n))
        .find(Model::is_installed)
    else {
        return;
    };
    if !has_filter("flite") || !has_filter("whisper") {
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("speech");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Two sentences with two seconds of silence between them, over video.
    let source = dir.join("talk.mp4");
    let graph = "flite=text='The quick brown fox jumps over the lazy dog.'[a1];\
                 flite=text='Seven days of free lessons.'[a2];\
                 [a1]apad=pad_dur=2[p1];[p1][a2]concat=n=2:v=0:a=1,aresample=44100[a]";
    Job::new(&source)
        .args(["-f", "lavfi", "-i", "testsrc2=size=320x240:rate=25:duration=9"])
        .args(["-filter_complex", graph, "-map", "0:v", "-map", "[a]"])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-shortest"])
        .overwrite(true)
        .run(&CancelToken::new(), |_| {})
        .unwrap();
    let duration = probe(&source).unwrap().duration_secs().unwrap();

    let opts = TranscribeOptions { language: "en".into(), use_gpu: true };
    let transcript = transcribe(&source, &model.path(), &opts, &CancelToken::new(), |_, _| {}).unwrap();
    let text = transcript.segments.iter().map(|s| s.text.to_lowercase()).collect::<Vec<_>>().join(" ");
    assert!(text.contains("brown fox") && text.contains("lazy dog"), "{transcript:?}");
    assert!(text.contains("free lessons"), "{transcript:?}");
    for s in &transcript.segments {
        assert!(0.0 <= s.start && s.start <= s.end && s.end <= duration + 0.1, "{s:?}");
    }
    // Every segment has its words, in order and inside it.
    for s in &transcript.segments {
        assert_eq!(s.words.len(), s.text.split_whitespace().count(), "{s:?}");
        for pair in s.words.windows(2) {
            assert!(pair[0].start <= pair[1].start, "{s:?}");
        }
        assert!(s.words.iter().all(|w| w.start >= s.start - 1e-6 && w.end <= s.end + 1e-6), "{s:?}");
    }
    // The second sentence starts after the pause.
    let second = transcript.segments.iter().find(|s| s.text.to_lowercase().contains("lessons")).unwrap();
    assert!(second.start > 2.0, "{second:?}");

    let style = CaptionStyle::for_frame(320, 240);
    let ass = TempFile::with_contents("test-captions", "ass", &to_ass(&cues(&transcript, &style), 320, 240, &style)).unwrap();
    let output = dir.join("captioned.mp4");
    let video = VideoEncode { encoder: Encoder::lookup("libx264").unwrap(), quality: None };
    burn_subtitles(&source, &output, ass.path(), video)
        .unwrap()
        .overwrite(true)
        .run(&CancelToken::new(), |_| {})
        .unwrap();
    let out = probe(&output).unwrap();
    assert!((out.duration_secs().unwrap() - duration).abs() < 0.1);
    assert!(out.has_audio());
}
