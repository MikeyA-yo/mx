//! Real two-pass encodes on generated input. Skipped when ffmpeg isn't
//! installed.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use mx_core::CancelToken;
use mx_core::ffmpeg::ops::{CompressCodec, compress, plan_compression};
use mx_core::ffmpeg::{Job, probe, version};

fn files_in(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// 4 s of 640x360 test pattern with a tone, in its own directory.
fn source(dir: &Path) -> PathBuf {
    let path = dir.join("source.mp4");
    Job::new(&path)
        .args(["-f", "lavfi", "-i", "testsrc2=duration=4:size=640x360:rate=30"])
        .args(["-f", "lavfi", "-i", "sine=duration=4"])
        .args(["-c:v", "libx264", "-preset", "ultrafast", "-crf", "18", "-c:a", "aac", "-shortest"])
        .overwrite(true)
        .run(&CancelToken::new(), |_| {})
        .unwrap();
    path
}

fn check_codec(codec: CompressCodec, name: &str) {
    if version("ffmpeg").is_none() {
        return;
    }
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("compress-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let input = source(&dir);
    let output = dir.join("small.mp4");
    let cwd = std::env::current_dir().unwrap();
    let cwd_before = files_in(&cwd);

    let target = 400_000;
    let plan = plan_compression(target, &probe(&input).unwrap(), codec).unwrap();
    let job = compress(&input, &output, plan, codec);
    let outcome = job.run(&CancelToken::new(), false, |_, _| {}).unwrap();
    assert!(outcome.bytes <= target, "{outcome:?}");
    drop(job);

    let left: BTreeSet<_> = files_in(&dir);
    assert_eq!(
        left,
        ["small.mp4", "source.mp4"].map(String::from).into(),
        "pass-1 stats not cleaned up"
    );
    // Regression: x265 once wrote its stats to `C` / `C.cutree` in the
    // working directory because the drive-letter colon split its options.
    assert_eq!(files_in(&cwd), cwd_before, "stray files in the working directory");
}

#[test]
fn h264_two_pass_stays_under_target_and_cleans_up() {
    check_codec(CompressCodec::H264, "h264");
}

#[test]
fn hevc_two_pass_stays_under_target_and_cleans_up() {
    check_codec(CompressCodec::Hevc, "hevc");
}
