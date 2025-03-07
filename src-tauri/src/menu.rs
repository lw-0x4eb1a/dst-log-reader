// popup menu for this app

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{self, Manager, Wry};
use tauri::menu::*;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_store::Store;

use crate::ds_log::LogPath;
use crate::window::open_log_impl;

#[derive(Debug, Clone)]
enum MenuEvent {
    Open,
    OpenRecent(LogPath),
    Settings,
    About,
}

#[derive(Default)]
pub struct MenuRef {
    menu: Mutex<Option<Menu<Wry>>>,
    menu_event_map: Mutex<HashMap<String, MenuEvent>>,
    locale: Mutex<String>,
}

const MENU_TEXT_ZH: [&'static str; 5] = [
    "打开",
    "打开最近的文件",
    "------------",
    "设置",
    "关于",
];

const MENU_TEXT_EN: [&'static str; 5] = [
    "Open",
    "Open Recent File",
    "------------",
    "Settings",
    "About",
];

impl MenuRef {
    pub fn update_locale(&self, locale: String) -> Result<(), tauri::Error> {
        if locale != self.locale.lock().unwrap().clone() {
            let is_zh = locale.as_str() == "zh";
            *self.locale.lock().unwrap() = locale;
            // change text
            let menu = self.menu.lock().unwrap();
            if let Some(menu) = menu.as_ref() {
                let items = menu.items()?;
                let text = if is_zh { MENU_TEXT_ZH } else { MENU_TEXT_EN };
                items[0].as_menuitem().unwrap().set_text(text[0])?;
                items[1].as_submenu().unwrap().set_text(text[1])?;
                // [2] is separator
                items[3].as_menuitem().unwrap().set_text(text[3])?;
                items[4].as_menuitem().unwrap().set_text(text[4])?;
            }
        }
        Ok(())
    }

    pub fn update_recent_files(&self, files: Vec<LogPath>) -> Result<(), tauri::Error> {
        let menu = self.menu.lock().unwrap();
        if let Some(menu) = menu.as_ref() {
            let items = menu.items()?;
            let open_recent = items[1].as_submenu().unwrap();
            // clear all items
            let max = open_recent.items()?.len();
            for _ in 0..max {
                open_recent.remove_at(0).ok();
            }
            // add new items
            for file in files {
                if !file.exists() {
                    continue;
                }
                let item = MenuItem::new(
                    open_recent.app_handle(),
                    file.get_menu_path(),
                    true,
                    None::<&str>,
                )?;
                open_recent.append(&item)?;
                // add to event map
                let mut map = self.menu_event_map.lock().unwrap();
                map.insert(item.id().0.to_string(), MenuEvent::OpenRecent(file.clone()));
            }
        }
        Ok(())
    }

    pub fn update_event_map(&self) -> Result<(), tauri::Error> {
        let menu = self.menu.lock().unwrap();
        if let Some(menu) = menu.as_ref() {
            let items = menu.items()?;
            let mut map = self.menu_event_map.lock().unwrap();
            map.insert(items[0].id().0.to_string(), MenuEvent::Open);
            map.insert(items[1].id().0.to_string(), MenuEvent::OpenRecent(LogPath::External("".into())));
            map.insert(items[3].id().0.to_string(), MenuEvent::Settings);
            map.insert(items[4].id().0.to_string(), MenuEvent::About);
        }
        Ok(())
    }
}

pub fn setup_menu(app: &tauri::App) -> Result<(), tauri::Error> {
    let handle = app.handle();
    let state = app.state::<MenuRef>();

    let menu = MenuBuilder::new(handle)
        .item(&MenuItem::new(handle, "Open", true, None::<&str>)?)
        .item(&SubmenuBuilder::new(handle, "Open Recent File").build()?)
        .separator()
        .item(&MenuItem::new(handle, "Settings", true, None::<&str>)?)
        .item(&MenuItem::new(handle, "About", true, None::<&str>)?)
        .build()?;

    state.menu.lock().unwrap().replace(menu);

    // register global listener
    app.on_menu_event(move |handle, event| {
        let state = handle.state::<MenuRef>();
        let map = state.menu_event_map.lock().unwrap();
        let id = event.id().0.to_string();
        if let Some(menu_event) = map.get(&id) {
            handle_menu_event(handle, menu_event.clone());
        }
    });

    Ok(())
}

#[tauri::command]
pub fn open_tool_menu(app: tauri::AppHandle, window: tauri::Window, locale: String) -> Result<(), tauri::Error> {    
    let state = app.state::<MenuRef>();
    let recent = app.state::<RecentFileList>();
    let files = recent.files.lock().unwrap().clone();
    state.update_locale(locale)?;
    state.update_recent_files(files)?;
    state.update_event_map()?;
    let menu = state.menu.lock().unwrap();
    // popup context menu
    if let Some(menu) = menu.as_ref() {
        menu.popup(window).ok();
    }
    Ok(())
}

fn handle_menu_event(handle: &tauri::AppHandle, event: MenuEvent) {
    use crate::window::open_unique_window;
    match event {
        MenuEvent::Open => {
            let handle2 = handle.clone();
            handle.dialog()
                .file()
                .add_filter("Log file", &["txt"])
                .pick_file(move |path| {
                    if let Some(path) = path {
                        open_log_impl(
                            &handle2,
                            handle2.get_webview_window("main").unwrap(),
                            LogPath::External(path.into_path().unwrap())
                        ).ok();
                    }
                });
        }
        MenuEvent::OpenRecent(path) => {
            open_log_impl(handle,
                handle.get_webview_window("main").unwrap(),
                path
            ).ok();
        }
        MenuEvent::Settings => {
            open_unique_window(handle, "settings", None).ok();
        }
        MenuEvent::About => {
            open_unique_window(handle, "about", None).ok();
        }
    }
}

#[derive(Default)]
pub struct RecentFileList {
    files: Mutex<Vec<LogPath>>,
}

const MAX_RECENT_FILES: usize = 10;

impl RecentFileList {
    pub fn load_from_store(&self, store: Arc<Store<Wry>>) {
        use tauri_plugin_store::JsonValue as Value;
        let files = store.get("files").unwrap_or_default();
        #[allow(clippy::redundant_closure)]
        let convert = |value| {
            match value {
                Value::Array(arr) => arr.iter()
                    .filter_map(|v| v.as_str())
                    .flat_map(LogPath::deserialize)
                    .collect(),
                _ => Vec::new(),
            }
        };
        *self.files.lock().unwrap() = convert(files);
    }

    /// add a new file to recent list
    pub fn on_open_file(&self, path: &LogPath) {
        let mut files = self.files.lock().unwrap();
        files.retain(|f| f != path);
        files.insert(0, path.clone());
        files.truncate(MAX_RECENT_FILES);
    }

    pub fn dump_to_store(&self, store: &Store<Wry>) {
        use tauri_plugin_store::JsonValue as Value;
        let files = self.files.lock().unwrap();
        let content = Value::Array(
            files.iter().map(|f| Value::String(f.serialize())).collect::<Vec<_>>()
        );
        store.set("files", content);
    }
}