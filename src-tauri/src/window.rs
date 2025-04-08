// multiply window manager

use tauri::{self, Listener, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_store::StoreExt;

use crate::{ds_log::{LogModelState, LogPath}, menu::RecentFileList};

pub fn open_unique_window(handle: &tauri::AppHandle, label: &str, _url: Option<String>) -> Result<(), tauri::Error> {
    match handle.get_webview_window(label) {
        Some(window) => {
            window.set_focus()?;
        },
        None => {
            // let url = WebviewUrl::App("index.html".into());
            match label {
                "about"=> {
                    WebviewWindowBuilder::new(handle, label, WebviewUrl::App("about_static.html".into()))
                        .title("About")
                        .resizable(false)
                        .maximizable(false)
                        .inner_size(300.0, 350.0)
                        .build()?;
                },
                "settings"=> {
                    WebviewWindowBuilder::new(handle, label, WebviewUrl::App("".into()))
                        .title("Settings")
                        .resizable(false)
                        .maximizable(false)
                        .inner_size(300.0, 250.0)
                        .build()?;
                },
                s => {
                    println!("unknown window label: {}", s);
                }
            }
        }
    }
    Ok(())
}

pub fn open_log_impl(handle: &tauri::AppHandle, path: LogPath) -> Result<(), tauri::Error>
{
    let label = path.to_label();
    let window = handle.get_webview_window("main").unwrap();
    match handle.get_webview_window(&label) {
        Some(window) => {
            window.set_focus()?;
        },
        None => {
            // register model on new window
            let state = handle.state::<LogModelState>();
            let max_count = if cfg!(debug_assertions) { 2 } else { 64 };
            if state.clear_inactive(max_count / 2) >= max_count {
                // pop an alert, but dont raise an Error
                handle.dialog()
                    .message("Too many windows are open. Please close some windows before opening a new one.")
                    .kind(MessageDialogKind::Warning)
                    .buttons(MessageDialogButtons::Ok)
                    .parent(&window)
                    .show(|_| {});
                return Ok(());
            }
            state.register(&path);
            let state = handle.state::<RecentFileList>();
            let store = handle.store("recent").unwrap();
            state.on_open_file(&path);
            state.dump_to_store(&store);
            let url = WebviewUrl::App("index.html".into());
            let window = WebviewWindowBuilder::new(handle, &label, url)
                .title(path.get_name())
                .resizable(true)
                .inner_size(1000.0, 750.0)
                .min_inner_size(400.0, 300.0)
                .initialization_script(&format!("window.logPath = JSON.parse({});", 
                    json::JsonValue::String(path.to_ipc()).dump()))
                .build()
                .unwrap();
            let handle = handle.clone();
            window.listen("tauri://destroyed", move |_| {
                let state = handle.state::<LogModelState>();
                state.set_inactive(&label);
            });
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn open_log(handle: tauri::AppHandle, filepath: String) -> Result<(), tauri::Error> {
    open_log_impl(&handle, LogPath::External(filepath.into()))
}