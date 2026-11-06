//! Saving and opening projects (.mxp).
//!
//! A project file is mx-core's [`mx_core::project::Project`] as JSON,
//! which `mx render` reads, plus the editor's own state under `editor`
//! keys (pause detection settings, deleted transcript lines, ...) that
//! the renderer ignores. The editor builds the JSON; this side only
//! stores it, and keeps an autosave so a closed window loses nothing.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::tasks::blocking;

fn autosave_path(app: &AppHandle) -> Result<PathBuf> {
    let dir = app.path().app_data_dir().context("no app data directory")?;
    Ok(dir.join("autosave.mxp"))
}

/// Write `project` to `path`, or to the autosave when `path` is `None`.
/// Written to a temporary file first, so a crash mid-write can't leave a
/// half-written project behind.
#[tauri::command]
pub async fn save_project(app: AppHandle, path: Option<String>, project: Value) -> Result<String, String> {
    let target = match path {
        Some(p) => Ok(PathBuf::from(p)),
        None => autosave_path(&app),
    };
    blocking(move || {
        let target = target?;
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let partial = target.with_extension("mxp.part");
        std::fs::write(&partial, serde_json::to_vec_pretty(&project)?)?;
        std::fs::rename(&partial, &target)?;
        Ok(target.display().to_string())
    })
    .await
}

/// Read a project from `path`, or the autosave when `path` is `None`
/// (which may not exist: then `null`).
#[tauri::command]
pub async fn load_project(app: AppHandle, path: Option<String>) -> Result<Value, String> {
    let autosave = path.is_none();
    let source = match path {
        Some(p) => Ok(PathBuf::from(p)),
        None => autosave_path(&app),
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

/// Forget the autosave (the project was closed on purpose).
#[tauri::command]
pub async fn clear_autosave(app: AppHandle) -> Result<(), String> {
    let path = autosave_path(&app);
    blocking(move || {
        let _ = std::fs::remove_file(path?);
        Ok(())
    })
    .await
}
