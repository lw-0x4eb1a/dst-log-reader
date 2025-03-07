use std::path::Path;
use tauri::Manager;

use crate::ds_log::LogModelState;

pub trait PathExt {
    fn file_name_utf8(&self) -> String;
    fn mtime_f64(&self) -> f64;
    fn file_size(&self) -> u64;
}

impl PathExt for Path {
    fn file_name_utf8(&self) -> String {
        self.file_name().unwrap_or_default().to_string_lossy().to_string()
    }

    fn mtime_f64(&self) -> f64 {
        match self.metadata() {
            Ok(meta) => {
                match meta.modified() {
                    Ok(time) => {
                        time.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64()
                    }
                    Err(_) => -1.0,
                }
            }
            Err(_) => -1.0,
        }
    }

    fn file_size(&self) -> u64 {
        match self.metadata() {
            Ok(meta) => meta.len(),
            Err(_) => 0,
        }
    }
}

#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    webbrowser::open(&url).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn show_file(handle: tauri::AppHandle, path: String) {
    use tauri_plugin_opener::OpenerExt;
    handle.opener().reveal_item_in_dir(path).ok();
}

#[tauri::command]
pub fn show_file_by_label(handle: tauri::AppHandle, label: String) {
    use tauri_plugin_opener::OpenerExt;
    if let Some(path) = handle.state::<LogModelState>().get_path(&label) {
        handle.opener().reveal_item_in_dir(path).ok();
    }
}

#[tauri::command]
pub async fn save_file(handle: tauri::AppHandle, window: tauri::Window, default_path: String, content: String) -> Result<(), String> {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let handle2 = handle.clone();
    handle.dialog()
        .file()
        .set_file_name(default_path)
        .add_filter("Log file", &["txt"])
        .set_parent(&window)
        .save_file(move |path| {
            if let Some(path) = path {
                if let Err(e) = std::fs::write(path.as_path().unwrap(), content) {
                    handle2.dialog()
                        .message(format!("Failed to save file: {}", e))
                        .kind(MessageDialogKind::Error)
                        .buttons(MessageDialogButtons::Ok)
                        .parent(&window)
                        .show(|_| {});
                }
            }
        });
    Ok(())
}