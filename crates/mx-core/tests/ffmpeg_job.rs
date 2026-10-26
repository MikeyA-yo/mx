//! Runs the real ffmpeg on generated (lavfi) input. Skipped when ffmpeg
//! isn't installed.

use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use mx_core::ffmpeg::{Job, version};
use mx_core::{CancelToken, Cancelled};

fn ffmpeg_available() -> bool {
    version("ffmpeg").is_some()
}

fn temp_file(name: &str) -> PathBuf {
    // Cargo's scratch dir for integration tests, inside target/.
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// `seconds` of 720p test pattern encoded with x264.
fn test_pattern(output: &PathBuf, seconds: u32) -> Job {
    Job::new(output)
        .args(["-f", "lavfi", "-i", &format!("testsrc2=duration={seconds}:size=1280x720:rate=30")])
        .args(["-c:v", "libx264", "-preset", "veryfast"])
        .duration(seconds as f64)
}

#[test]
fn reports_progress_through_to_done() {
    if !ffmpeg_available() {
        return;
    }
    let out = temp_file("progress.mp4");
    let mut snapshots = Vec::new();
    test_pattern(&out, 2).run(&CancelToken::new(), |p| snapshots.push(p.clone())).unwrap();

    let last = snapshots.last().expect("at least one progress block");
    assert!(last.done);
    assert_eq!(last.percent, Some(100.0));
    assert!(out.metadata().unwrap().len() > 0);
}

#[test]
fn cancel_kills_ffmpeg_and_removes_partial_output() {
    if !ffmpeg_available() {
        return;
    }
    let out = temp_file("cancelled.mp4");
    let cancel = CancelToken::new();
    let trigger = cancel.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(700));
        trigger.cancel();
    });

    let started = Instant::now();
    // Long enough that it can only finish by being cancelled.
    let err = test_pattern(&out, 600).run(&cancel, |_| {}).unwrap_err();

    assert!(err.is::<Cancelled>(), "{err:#}");
    assert!(started.elapsed() < Duration::from_secs(5), "cancel took {:?}", started.elapsed());
    assert!(!out.exists(), "partial output left behind");
}

#[test]
fn refuses_to_overwrite_and_leaves_existing_file_alone() {
    if !ffmpeg_available() {
        return;
    }
    let out = temp_file("existing.mp4");
    std::fs::write(&out, b"keep me").unwrap();

    let err = test_pattern(&out, 1).run(&CancelToken::new(), |_| {}).unwrap_err();
    assert!(err.to_string().contains("already exists"), "{err:#}");
    assert_eq!(std::fs::read(&out).unwrap(), b"keep me");

    test_pattern(&out, 1).overwrite(true).run(&CancelToken::new(), |_| {}).unwrap();
    assert_ne!(std::fs::read(&out).unwrap(), b"keep me");
}
