// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod reader;
mod ds_log;
mod steam_workshop;
mod bootstrap;
mod menu;
mod window;

use ds_log::{list_all_logs, load_log_abstract, load_log_init, load_log_handshake};
use steam_workshop::{get_steam_workshop_icon, SteamWorkshopIconManager};
use window::{open_log};
use menu::{open_tool_menu, setup_menu, MenuRef, RecentFileList};
use tauri::Manager;
use bootstrap::{open_url, show_file, show_file_by_label, save_file};
use ds_log::LogModelState;

#[macro_use]
extern crate rental; 

fn main() {
    // ds_log::debug_parse_log("/Users/wzh/Downloads/client_log (1).txt".into());  
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(MenuRef::default())
        .manage(RecentFileList::default())
        .manage(LogModelState::default())
        .manage(SteamWorkshopIconManager::default())
        .setup(|app| {
            setup_store(app)?;
            setup_menu(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_all_logs,
            load_log_abstract,
            open_log,
            open_tool_menu,
            load_log_init,
            load_log_handshake,
            open_url,
            show_file,
            show_file_by_label,
            save_file,
            get_steam_workshop_icon,
            shutdown,
        ])
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                if window.label() == "main" {
                    #[cfg(target_os = "macos")]
                    {
                        tauri::AppHandle::hide(window.app_handle()).unwrap();
                        api.prevent_close();
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        window.destroy().unwrap();
                    }
                }
            },
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn setup_store(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let workshop = tauri_plugin_store::StoreBuilder::new(app, "steam_workshop").build()?;
    let state = app.state::<SteamWorkshopIconManager>();
    state.load_from_store(workshop);
    // auto save
    let handle = app.handle().clone();
    std::thread::spawn(move|| {
        loop {
            let workshop = tauri_plugin_store::StoreBuilder::new(&handle, "steam_workshop").build().unwrap();
            let state = handle.state::<SteamWorkshopIconManager>();
            state.dump_to_store(workshop);
            std::thread::sleep(std::time::Duration::from_secs(10));
        }
    });
    let recent = tauri_plugin_store::StoreBuilder::new(app, "recent").build()?;
    let state = app.state::<RecentFileList>();
    state.load_from_store(recent);
    Ok(())
}

#[tauri::command]
fn shutdown() {
    std::process::exit(0);
}
