pub mod pbix;

use pbix::{
    parse_report, read_entry, read_table_rows as decode_table_rows, EntryContent, ReportFile,
    TableRows,
};
use std::sync::Mutex;
use tauri::{Emitter, Manager};

struct PendingPaths(Mutex<Vec<String>>);

#[tauri::command]
async fn choose_report() -> Result<Option<ReportFile>, String> {
    let path = rfd::AsyncFileDialog::new()
        .add_filter("Power BI reports", &["pbix", "pbit"])
        .set_title("Open a Power BI report")
        .pick_file()
        .await;
    match path {
        Some(file) => {
            let path = file.path().to_path_buf();
            tauri::async_runtime::spawn_blocking(move || parse_report(&path))
                .await
                .map_err(|e| format!("Parser task failed: {e}"))?
                .map(Some)
        }
        None => Ok(None),
    }
}

#[tauri::command]
async fn open_report_path(path: String) -> Result<ReportFile, String> {
    tauri::async_runtime::spawn_blocking(move || parse_report(std::path::Path::new(&path)))
        .await
        .map_err(|e| format!("Parser task failed: {e}"))?
}

#[tauri::command]
fn take_pending_paths(paths: tauri::State<'_, PendingPaths>) -> Vec<String> {
    std::mem::take(&mut *paths.0.lock().expect("pending path lock poisoned"))
}

#[tauri::command]
async fn read_package_entry(path: String, entry_name: String) -> Result<EntryContent, String> {
    tauri::async_runtime::spawn_blocking(move || {
        read_entry(std::path::Path::new(&path), &entry_name)
    })
    .await
    .map_err(|e| format!("Reader task failed: {e}"))?
}

#[tauri::command]
async fn read_table_rows(
    path: String,
    table_name: String,
    offset: u64,
    limit: u64,
) -> Result<TableRows, String> {
    tauri::async_runtime::spawn_blocking(move || {
        decode_table_rows(std::path::Path::new(&path), &table_name, offset, limit)
    })
    .await
    .map_err(|error| format!("Table reader task failed: {error}"))?
}

#[tauri::command]
fn start_window_drag(window: tauri::Window) -> Result<(), String> {
    window
        .start_dragging()
        .map_err(|error| format!("Could not move window: {error}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(PendingPaths(Mutex::new(Vec::new())))
        .invoke_handler(tauri::generate_handler![
            choose_report,
            open_report_path,
            take_pending_paths,
            read_package_entry,
            read_table_rows,
            start_window_drag
        ])
        .build(tauri::generate_context!())
        .expect("error while building PBI Lens")
        .run(|app, event| {
            if let tauri::RunEvent::Opened { urls } = event {
                let incoming: Vec<String> = urls
                    .into_iter()
                    .filter_map(|url| url.to_file_path().ok())
                    .filter(|path| {
                        matches!(
                            path.extension()
                                .and_then(|s| s.to_str())
                                .map(str::to_ascii_lowercase)
                                .as_deref(),
                            Some("pbix" | "pbit")
                        )
                    })
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect();
                if !incoming.is_empty() {
                    app.state::<PendingPaths>()
                        .0
                        .lock()
                        .expect("pending path lock poisoned")
                        .extend(incoming.clone());
                    let _ = app.emit("pbi-lens://open", incoming);
                }
            }
        });
}
