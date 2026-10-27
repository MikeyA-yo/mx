//! Real remuxes on generated input. Skipped when ffmpeg isn't installed.

use std::path::{Path, PathBuf};

use mx_core::CancelToken;
use mx_core::ffmpeg::ops::remux;
use mx_core::ffmpeg::{Job, probe, version};

fn run(job: Job) {
    job.overwrite(true).run(&CancelToken::new(), |_| {}).unwrap();
}

/// MD5 of one stream's packets, without decoding. Equal hashes across
/// containers mean the stream was carried over bit for bit.
fn packet_md5(file: &Path, stream: &str, dir: &Path) -> String {
    let out = dir.join(format!("{}.{}.md5", file.file_name().unwrap().to_string_lossy(), &stream[0..1]));
    run(Job::new(&out).input(file).args(["-map", &format!("0:{stream}"), "-c", "copy", "-f", "md5"]));
    std::fs::read_to_string(out).unwrap().trim().to_string()
}

#[test]
fn remux_to_mp4_is_lossless_and_converts_text_subtitles() {
    if version("ffmpeg").is_none() {
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("remux");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let srt = dir.join("en.srt");
    std::fs::write(&srt, "1\n00:00:00,500 --> 00:00:01,500\nhello\n").unwrap();
    let source = dir.join("source.mkv");
    run(Job::new(&source)
        .args(["-f", "lavfi", "-i", "testsrc2=duration=2:size=320x240:rate=25"])
        .args(["-f", "lavfi", "-i", "sine=duration=2"])
        .input(&srt)
        .args(["-map", "0", "-map", "1", "-map", "2"])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac", "-c:s", "srt"]));

    let output = dir.join("out.mp4");
    let r = remux(&source, &output).unwrap();
    assert_eq!(r.subtitles.converted, 1);
    run(r.job);

    for stream in ["v", "a"] {
        assert_eq!(packet_md5(&source, stream, &dir), packet_md5(&output, stream, &dir), "{stream} changed");
    }
    let subs: Vec<_> = probe(&output)
        .unwrap()
        .streams
        .into_iter()
        .filter(|s| s.codec_type.as_deref() == Some("subtitle"))
        .filter_map(|s| s.codec_name)
        .collect();
    assert_eq!(subs, ["mov_text"]);

    assert!(remux(&output, &output).is_err(), "remuxing a file onto itself must be refused");
}
