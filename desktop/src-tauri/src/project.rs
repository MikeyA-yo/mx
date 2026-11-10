//! Saving and opening projects (.mxp).
//!
//! A project file is mx-core's [`mx_core::project::Project`] as JSON,
//! which `mx render` reads, plus the editor's own state under `editor`
//! keys (pause detection settings, deleted transcript lines, ...) that
//! the renderer ignores. The editor builds the JSON; this side only
//! stores it.
//!
//! Every project being edited is also autosaved as a *session*, one file
//! each, so closing the window loses nothing and one project never
//! overwrites another's autosave. The last [`KEEP_SESSIONS`] are kept.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::tasks::blocking;

/// Autosaved sessions kept, newest first.
const KEEP_SESSIONS: usize = 10;

fn data_dir(app: &AppHandle) -> Result<PathBuf> {
    app.path().app_data_dir().context("no app data directory")
}

fn sessions_dir(app: &AppHandle) -> Result<PathBuf> {
    Ok(data_dir(app)?.join("sessions"))
}

/// The one autosave older versions kept.
fn legacy_autosave(app: &AppHandle) -> Result<PathBuf> {
    Ok(data_dir(app)?.join("autosave.mxp"))
}

fn session_path(app: &AppHandle, session: &str) -> Result<PathBuf> {
    if session.is_empty() || !session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!("not a session id: {session}");
    }
    Ok(sessions_dir(app)?.join(format!("{session}.mxp")))
}

/// Written to a temporary file first, so a crash mid-write can't leave a
/// half-written project behind.
fn write(target: &Path, project: &Value) -> Result<()> {
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let partial = target.with_extension("mxp.part");
    std::fs::write(&partial, serde_json::to_vec_pretty(project)?)?;
    std::fs::rename(&partial, target)?;
    Ok(())
}

fn modified(path: &Path) -> Option<u64> {
    let time = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    Some(time.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
}

/// Write `project` to `path`, or autosave it as `session` when `path` is
/// `None`.
#[tauri::command]
pub async fn save_project(app: AppHandle, path: Option<String>, project: Value, session: Option<String>) -> Result<String, String> {
    let target = match (path, session) {
        (Some(p), _) => Ok(PathBuf::from(p)),
        (None, Some(s)) => session_path(&app, &s),
        (None, None) => legacy_autosave(&app),
    };
    let sessions = sessions_dir(&app);
    blocking(move || {
        let target = target?;
        write(&target, &project)?;
        // Only the newest sessions are worth keeping.
        if let Ok(dir) = sessions {
            let mut all: Vec<(u64, PathBuf)> = std::fs::read_dir(&dir)
                .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "mxp")).collect())
                .unwrap_or_else(|_| Vec::new())
                .into_iter()
                .map(|p| (modified(&p).unwrap_or(0), p))
                .collect();
            all.sort_by_key(|p| std::cmp::Reverse(p.0));
            for (_, old) in all.into_iter().skip(KEEP_SESSIONS) {
                let _ = std::fs::remove_file(old);
            }
        }
        Ok(target.display().to_string())
    })
    .await
}

/// Read a project from `path`, or the old single autosave when `path` is
/// `None` (which may not exist: then `null`).
#[tauri::command]
pub async fn load_project(app: AppHandle, path: Option<String>) -> Result<Value, String> {
    let autosave = path.is_none();
    let source = match path {
        Some(p) => Ok(PathBuf::from(p)),
        None => legacy_autosave(&app),
    };
    blocking(move || {
        let source = source?;
        if autosave && !source.is_file() {
            return Ok(Value::Null);
        }
        let text = std::fs::read_to_string(&source).with_context(|| format!("couldn't read {}", source.display()))?;
        serde_json::from_str(&text).with_context(|| format!("{} isn't an mx project", source.display()))
    })
    .await
}

/// Forget a session's autosave: its project was saved to a file, or
/// closed with nothing unsaved.
#[tauri::command]
pub async fn discard_session(app: AppHandle, session: String) -> Result<(), String> {
    let path = session_path(&app, &session);
    blocking(move || {
        let _ = std::fs::remove_file(path?);
        Ok(())
    })
    .await
}

/// A project on disk, for the start screen.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    path: String,
    /// Milliseconds since 1970, when it was last written.
    modified: Option<u64>,
    /// File names of its clips, in order.
    clips: Vec<String>,
    /// An autosaved session (its id), rather than a file the user saved.
    session: Option<String>,
    /// For a session: the file it was opened from or saved to, if any.
    saved_to: Option<String>,
    /// The file has gone: moved, renamed or deleted.
    missing: bool,
}

fn info(path: &Path, session: Option<String>) -> ProjectInfo {
    let json = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let saved_to = json.as_ref().and_then(|v| Some(v.get("editor")?.get("savedTo")?.as_str()?.to_string()));
    let clips = json
        .as_ref()
        .and_then(|v| v.get("clips")?.as_array().cloned())
        .map(|clips| {
            clips
                .iter()
                .filter_map(|c| c.get("source")?.as_str())
                .map(|s| Path::new(s).file_name().map_or(s.to_string(), |n| n.to_string_lossy().into_owned()))
                .collect()
        });
    ProjectInfo {
        path: path.display().to_string(),
        modified: modified(path),
        missing: clips.is_none(),
        clips: clips.unwrap_or_default(),
        session,
        saved_to,
    }
}

/// What the start screen lists: the saved projects at `paths` (the
/// editor remembers those), then every autosaved session. Newest first;
/// files that have gone come last.
#[tauri::command]
pub async fn recent_projects(app: AppHandle, paths: Vec<String>) -> Result<Vec<ProjectInfo>, String> {
    let sessions = sessions_dir(&app);
    let legacy = legacy_autosave(&app);
    blocking(move || {
        let mut all: Vec<ProjectInfo> = paths.iter().map(|p| info(Path::new(p), None)).collect();
        if let Ok(dir) = sessions?.read_dir() {
            for entry in dir.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "mxp") {
                    let id = path.file_stem().map(|s| s.to_string_lossy().into_owned());
                    all.push(info(&path, id));
                }
            }
        }
        if let Ok(old) = legacy
            && old.is_file()
        {
            all.push(info(&old, Some("autosave".into())));
        }
        all.retain(|p| !(p.session.is_some() && (p.missing || p.clips.is_empty())));
        all.sort_by(|a, b| a.missing.cmp(&b.missing).then(b.modified.cmp(&a.modified)));
        Ok(all)
    })
    .await
}

/// Project files under `dirs`, a few folders deep, newest first: for
/// projects saved somewhere and forgotten.
#[tauri::command]
pub async fn find_projects(dirs: Vec<String>) -> Result<Vec<ProjectInfo>, String> {
    blocking(move || {
        let mut found = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for dir in dirs {
            walk(Path::new(&dir), 4, &mut seen, &mut found);
        }
        found.sort_by_key(|p| std::cmp::Reverse(p.modified));
        Ok(found)
    })
    .await
}

fn walk(dir: &Path, depth: usize, seen: &mut std::collections::HashSet<PathBuf>, found: &mut Vec<ProjectInfo>) {
    // Each folder once (they overlap: Videos, and a video's folder in it),
    // and not forever: a few thousand folders is plenty.
    let Ok(real) = std::fs::canonicalize(dir) else { return };
    if seen.len() > 5000 || !seen.insert(real) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            if depth > 0 && !name.starts_with('.') && !["node_modules", "target", "AppData", "$RECYCLE.BIN"].contains(&name.as_str()) {
                walk(&path, depth - 1, seen, found);
            }
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("mxp")) {
            found.push(info(&path, None));
        }
    }
}

/// Where projects are saved unless chosen otherwise: Videos\mx\Projects,
/// made if it isn't there yet.
#[tauri::command]
pub async fn projects_dir(app: AppHandle) -> Result<String, String> {
    let videos = app.path().video_dir().or_else(|_| app.path().home_dir());
    blocking(move || {
        let dir = videos.context("no videos folder")?.join("mx").join("Projects");
        std::fs::create_dir_all(&dir)?;
        Ok(dir.display().to_string())
    })
    .await
}
