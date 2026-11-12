//! Long-running work the UI can watch and stop.
//!
//! Every slow command runs as a task: on a worker thread, with a
//! [`CancelToken`] registered under an id the UI chose, and reporting
//! `task` events (one stream for downloads, exports and analysis alike)
//! that the task list and the panels render.

use std::collections::{HashMap, HashSet};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use mx_core::ffmpeg::{Job, Progress};
use mx_core::{CancelToken, Cancelled};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Default)]
pub struct Tasks(Mutex<HashMap<String, CancelToken>>);

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskUpdate {
    pub id: String,
    /// "download", "export", "analyse", "transcribe", "preview"
    pub kind: &'static str,
    pub title: String,
    pub status: Status,
    pub stage: Option<String>,
    /// 0-100, when known.
    pub percent: Option<f64>,
    /// Speed, size, time left.
    pub detail: Option<String>,
    /// The file a finished task produced.
    pub output: Option<String>,
    pub error: Option<String>,
}

/// The running task, handed to its work closure.
pub struct Task {
    app: AppHandle,
    update: TaskUpdate,
    pub cancel: CancelToken,
    last_sent: Instant,
}

/// Progress events more often than this are dropped: ffmpeg reports
/// several times a second per job, and the UI only needs to look smooth.
const THROTTLE: Duration = Duration::from_millis(150);

impl Task {
    fn send(&mut self) {
        self.last_sent = Instant::now();
        let _ = self.app.emit("task", &self.update);
    }

    /// Report progress. A new stage is always sent; updates within a
    /// stage are throttled.
    pub fn progress(&mut self, stage: &str, percent: Option<f64>, detail: Option<String>) {
        let new_stage = self.update.stage.as_deref() != Some(stage);
        self.update.stage = Some(stage.to_string());
        self.update.percent = percent.map(|p| p.clamp(0.0, 100.0));
        self.update.detail = detail;
        if new_stage || self.last_sent.elapsed() >= THROTTLE {
            self.send();
        }
    }

    /// Report an ffmpeg job's progress under `stage`.
    pub fn ffmpeg(&mut self, stage: &str, p: &Progress) {
        let mut detail = Vec::new();
        if let Some(speed) = p.speed.filter(|s| *s > 0.0) {
            detail.push(format!("{speed:.1}x"));
            if let (Some(pct), Some(t)) = (p.percent, p.out_time_secs)
                && pct > 0.5
            {
                detail.push(format!("{} left", clock(t * (100.0 / pct - 1.0) / speed)));
            }
        }
        self.progress(stage, p.percent, (!detail.is_empty()).then(|| detail.join(" · ")));
    }

    /// Run an ffmpeg job as this task (or one stage of it).
    pub fn run_job(&mut self, stage: &str, job: Job) -> Result<()> {
        let cancel = self.cancel.clone();
        job.overwrite(true).run(&cancel, |p| self.ffmpeg(stage, p))
    }

    pub fn set_output(&mut self, path: impl Into<String>) {
        self.update.output = Some(path.into());
    }
}

pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    match s {
        0..3600 => format!("{}:{:02}", s / 60, s % 60),
        _ => format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60),
    }
}

/// Run `work` on a worker thread as task `id`. Errors come back as text
/// for the UI; a cancelled task returns the error "cancelled".
pub async fn run<T: Send + 'static>(
    app: AppHandle,
    id: String,
    kind: &'static str,
    title: String,
    work: impl FnOnce(&mut Task) -> Result<T> + Send + 'static,
) -> Result<T, String> {
    run_task(app, id, kind, title, false, work).await
}

/// Like [`run`], but waits its turn: what every clip asks for as it
/// loads (its preview, thumbnails and waveform). A project of many clips
/// asks for all of them at once, and dozens of ffmpegs sharing the CPU
/// would each crawl.
pub async fn run_queued<T: Send + 'static>(
    app: AppHandle,
    id: String,
    kind: &'static str,
    title: String,
    work: impl FnOnce(&mut Task) -> Result<T> + Send + 'static,
) -> Result<T, String> {
    run_task(app, id, kind, title, true, work).await
}

/// Like [`run_queued`], but asks `ready` first for a result made before
/// (by an earlier run, kept on disk): one comes straight back, with no
/// task to show and no turn to wait for. Reopening a project of many
/// clips is mostly this.
pub async fn run_queued_unless<T: Send + 'static>(
    app: AppHandle,
    id: String,
    kind: &'static str,
    title: String,
    ready: impl FnOnce() -> Option<T> + Send + 'static,
    work: impl FnOnce(&mut Task) -> Result<T> + Send + 'static,
) -> Result<T, String> {
    if let Ok(Some(done)) = tauri::async_runtime::spawn_blocking(ready).await {
        if let Some(h) = HURRIED.lock().unwrap().as_mut() {
            h.remove(&id);
        }
        return Ok(done);
    }
    run_queued(app, id, kind, title, work).await
}

/// Queued tasks running now, and a signal when one finishes.
static SLOTS: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());

/// Queued tasks that go now, waiting or not: the clip on screen's.
static HURRIED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn hurried(id: &str) -> bool {
    HURRIED.lock().unwrap().as_ref().is_some_and(|h| h.contains(id))
}

/// Let a queued task skip the queue: it's for what's on screen.
#[tauri::command]
pub fn hurry_task(id: String) {
    HURRIED.lock().unwrap().get_or_insert_with(HashSet::new).insert(id);
}

/// A queued task's turn; the next one's starts when it's dropped.
struct Slot;

impl Slot {
    fn take(task: &mut Task) -> Result<Slot> {
        let at_once = std::thread::available_parallelism().map_or(2, |n| n.get() / 2).clamp(2, 4);
        let (busy, freed) = &SLOTS;
        let mut busy = busy.lock().unwrap();
        if *busy >= at_once {
            task.progress("waiting for other tasks", None, None);
        }
        while *busy >= at_once && !hurried(&task.update.id) {
            if task.cancel.is_cancelled() {
                return Err(Cancelled.into());
            }
            busy = freed.wait_timeout(busy, Duration::from_millis(200)).unwrap().0;
        }
        *busy += 1;
        Ok(Slot)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let (busy, freed) = &SLOTS;
        *busy.lock().unwrap() -= 1;
        freed.notify_one();
    }
}

async fn run_task<T: Send + 'static>(
    app: AppHandle,
    id: String,
    kind: &'static str,
    title: String,
    queued: bool,
    work: impl FnOnce(&mut Task) -> Result<T> + Send + 'static,
) -> Result<T, String> {
    let cancel = CancelToken::new();
    app.state::<Tasks>().0.lock().unwrap().insert(id.clone(), cancel.clone());
    let update = TaskUpdate {
        id: id.clone(),
        kind,
        title,
        status: Status::Running,
        stage: None,
        percent: None,
        detail: None,
        output: None,
        error: None,
    };
    let mut task = Task { app: app.clone(), update, cancel, last_sent: Instant::now() };
    task.send();

    let joined = tauri::async_runtime::spawn_blocking(move || {
        let result = (|| {
            let _turn = if queued { Some(Slot::take(&mut task)?) } else { None };
            work(&mut task)
        })();
        (task, result)
    })
    .await;
    app.state::<Tasks>().0.lock().unwrap().remove(&id);
    if let Some(h) = HURRIED.lock().unwrap().as_mut() {
        h.remove(&id);
    }
    let (mut task, result) = joined.map_err(|e| format!("task crashed: {e}"))?;

    task.update.detail = None;
    match &result {
        Ok(_) => {
            task.update.status = Status::Done;
            task.update.percent = Some(100.0);
        }
        Err(e) if e.is::<Cancelled>() => task.update.status = Status::Cancelled,
        Err(e) => {
            task.update.status = Status::Failed;
            task.update.error = Some(format!("{e:#}"));
        }
    }
    task.send();
    result.map_err(|e| if e.is::<Cancelled>() { "cancelled".to_string() } else { format!("{e:#}") })
}

/// Stop every task and give them a moment to clean up after themselves
/// (killing ffmpeg, removing half-written files): the app is closing.
pub fn stop_all(app: &AppHandle) {
    let tasks = app.state::<Tasks>();
    tasks.0.lock().unwrap().values().for_each(CancelToken::cancel);
    let until = Instant::now() + Duration::from_secs(3);
    while !tasks.0.lock().unwrap().is_empty() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[tauri::command]
pub fn cancel_task(app: AppHandle, id: String) {
    if let Some(token) = app.state::<Tasks>().0.lock().unwrap().get(&id) {
        token.cancel();
    }
}

/// Run blocking `work` off the async runtime, with errors as text.
pub async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || work().map_err(|e| format!("{e:#}")))
        .await
        .map_err(|e| format!("crashed: {e}"))?
}
