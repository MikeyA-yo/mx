//! Shared subprocess plumbing for tools that stream line-based status on
//! stdout (yt-dlp progress templates, ffmpeg `-progress pipe:1`).

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::cancel::{CancelToken, Cancelled};

/// How often to check the cancel token when the child is quiet.
const POLL: Duration = Duration::from_millis(100);

/// A command for `program`. Every tool is spawned through here.
///
/// On Windows, a console program started from a GUI app (the desktop
/// editor) opens a console window of its own, so a busy job flashes
/// dozens of them. When this process has no console, children are told
/// not to create one. A console process (the CLI) is left alone, so its
/// children keep sharing the console and still receive Ctrl+C.
pub(crate) fn command(program: impl AsRef<OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        unsafe extern "system" {
            fn GetConsoleWindow() -> *mut std::ffi::c_void;
        }
        // SAFETY: takes no arguments and only reads process state.
        if unsafe { GetConsoleWindow() }.is_null() {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
    }
    cmd
}

pub struct Finished {
    pub status: ExitStatus,
    pub stderr: String,
}

/// Run `cmd`, calling `on_line` for each stdout line, until it exits or
/// `cancel` fires. On cancel the child is killed and `Cancelled` returned.
///
/// stdout is read on a worker thread and handed over a channel so the cancel
/// check never blocks behind a child that has gone silent (e.g. stalled on
/// the network). stderr is drained on another thread so it can't fill its
/// pipe and deadlock the child.
pub fn run_streaming(
    mut cmd: Command,
    cancel: &CancelToken,
    mut on_line: impl FnMut(&str),
) -> Result<Finished> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start {program}"))?;

    let mut stderr = child.stderr.take().expect("stderr is piped");
    let stderr_reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    });

    let stdout = child.stdout.take().expect("stdout is piped");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).split(b'\n') {
            let Ok(line) = line else { break };
            if tx.send(String::from_utf8_lossy(&line).into_owned()).is_err() {
                break;
            }
        }
    });

    loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Cancelled.into());
        }
        match rx.recv_timeout(POLL) {
            Ok(line) => on_line(line.trim_end_matches('\r')),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    let status = child.wait()?;
    let stderr = stderr_reader.join().unwrap_or_default();
    Ok(Finished { status, stderr })
}
