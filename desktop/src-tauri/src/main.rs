//! The mx desktop editor. The window is a web UI (`desktop/src`); this
//! side only translates its calls into mx-core requests and reports
//! progress back. Nothing media-related is decided here.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod download;
mod export;
mod media;
mod speech;
mod system;
mod tasks;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(tasks::Tasks::default())
        .setup(|_| {
            // Test-encode in the background now, so the first export
            // doesn't wait a second for it.
            std::thread::spawn(export::working_encoders);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            tasks::cancel_task,
            media::probe_media,
            media::prepare_preview,
            media::filmstrip,
            media::waveform,
            media::detect_silences,
            media::highlights,
            media::plan_edit,
            media::caption_defaults,
            media::caption_cues,
            speech::list_models,
            speech::transcribe,
            speech::save_subtitles,
            speech::load_subtitles,
            download::fetch_info,
            download::download,
            export::export,
            export::export_gif,
            export::list_encoders,
            system::doctor,
            system::startup_files,
        ])
        .run(tauri::generate_context!())
        .expect("error while running mx");
}
