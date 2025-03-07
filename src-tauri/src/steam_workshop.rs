use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use reqwest;
use tauri::Manager;
use tauri_plugin_store::Store;

const WORKSHOP_API: &str = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";

#[derive(Default)]
pub struct SteamWorkshopIconManager {
    /// 2657513551 -> time  https://steamuserimages-a.akamaihd.net/ugc/2011457496870171575/8979725976D1564830B5C7A635AF2A29C28706A1/
    data: Arc<Mutex<HashMap<String, (u64, String)>>>,

    queue: Arc<Mutex<HashSet<String>>>,
    last_update: Arc<Mutex<u64>>,
    is_updating: Arc<Mutex<bool>>,
}

fn timestamp_u64() -> u64 {
    chrono::Utc::now().timestamp() as u64
}

impl SteamWorkshopIconManager {
    pub fn load_from_store<R: tauri::Runtime>(&self, store: Arc<Store<R>>) {
        let data = store.get("data").unwrap_or_default();
        let mut lock = self.data.lock().unwrap();
        if let Some(data) = data.as_array() {
            data.iter().for_each(|item| {
                let id = item["id"].as_str().unwrap_or_default();
                let time = item["mtime"].as_u64().unwrap_or_default();
                let icon = item["icon"].as_str().unwrap_or_default();
                if !id.is_empty() && !icon.is_empty() {
                    lock.insert(id.to_string(), (time, icon.to_string()));
                }
            });
        };
    }

    pub fn get_icon_url(&self, id: &str) -> Option<String> {
        let data = self.data.lock().unwrap();
        let (icon, should_update) = match data.get(id) {
            Some((mtime, icon)) => {
                let should_update = timestamp_u64() - *mtime > 3600;
                (Some(icon.clone()), should_update)
            },
            None => {
                (None, true)
            },
        };
        if should_update {
            self.enqueue(id.to_string());
        };
        icon
    }

    pub fn enqueue(&self, id: String) {
        self.queue.lock().unwrap().insert(id);
        // try update
        let time = timestamp_u64();
        if *self.last_update.lock().unwrap() + 10 < time && 
            !*self.is_updating.lock().unwrap() {
            *self.is_updating.lock().unwrap() = true;
            self.update();
        } 
    }

    pub fn enqueue_list(&self, list: Vec<String>) {
        let data = self.data.lock().unwrap();
        let list = list.iter()
            .filter(|id| id.len() < 16 && id.parse::<u64>().is_ok())
            .filter(|id| match data.get(*id) {
                Some((mtime, _)) => timestamp_u64() - *mtime > 3600,
                None => true,
            })
            .cloned()
            .collect::<Vec<String>>();
        if !list.is_empty() {
            let ele = list[0].clone();
            self.queue.lock().unwrap().extend(list);
            self.enqueue(ele);
        }
    }

    pub fn update(&self) {
        let id_list = self.queue.lock().unwrap()
            .iter()
            .take(50)
            .cloned()
            .collect::<Vec<_>>();
        if id_list.is_empty() {
            return;
        }
        let data = self.data.clone();
        let queue = self.queue.clone();
        let last_update = self.last_update.clone();
        let is_updating = self.is_updating.clone();
        std::thread::spawn(move|| {
            println!("Updating workshop icons [START]");
            let time = timestamp_u64();
            if let Ok(result) = query(id_list) {
                let mut data = data.lock().unwrap();
                let mut keys = vec![];
                for (id, icon) in result {
                    data.insert(id.clone(), (time, icon));
                    keys.push(id);
                }
                let mut queue = queue.lock().unwrap();
                for id in keys {
                    queue.remove(&id);
                }
            }
            *last_update.lock().unwrap() = time;
            *is_updating.lock().unwrap() = false;
            println!("Updating workshop icons [DONE]");
        });
    }
    
    pub fn dump_to_store<R: tauri::Runtime>(&self, store: Arc<Store<R>>) {
        use tauri_plugin_store::JsonValue as Value;
        use serde_json::json;
        let data = self.data.lock().unwrap();
        let data = data.iter().map(|(id, (mtime, icon))| {
            json! ({
                "id": id,
                "mtime": mtime,
                "icon": icon,
            })
        }).collect::<Vec<_>>();
        store.set("data", Value::Array(data));
    }
}

fn query(id_list: Vec<String>) -> Result<HashMap<String, String>, String>{
    let itemcount = id_list.len();
    let mut args = vec![format!("itemcount={}", itemcount)];
    for (i, id) in id_list.iter().enumerate() {
        args.push(format!("publishedfileids[{}]={}", i, id));
    }
    let args = args.join("&");
    // POST
    let client = reqwest::blocking::Client::new();
    let res = client.post(WORKSHOP_API)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(args)
        .send()
        .map_err(|e| e.to_string())?;
    if res.status().is_success() {
        let body = res.text().map_err(|e| e.to_string())?;
        let data = json::parse(&body).map_err(|e| e.to_string())?;
        let data = parse_response(data)?;
        Ok(data)
    }
    else {
        Err(format!("{:?}", res))
    }
}

fn parse_response(data: json::JsonValue) -> Result<HashMap<String, String>, String> {
    let items = &data["response"]["publishedfiledetails"];
    if !items.is_array() {
        return Err("`response.publishedfiledetails` is not an array".to_string());
    }
    let data = items.members().map(|item| {
        let id = item["publishedfileid"].as_str().unwrap_or_default();
        let icon = item["preview_url"].as_str().unwrap_or_default();
        (id.to_string(), icon.to_string())
    }).collect::<HashMap<_, _>>();
    Ok(data)
}

#[tauri::command]
pub fn get_steam_workshop_icon(id: String, handle: tauri::AppHandle) -> String {
    let state = handle.state::<SteamWorkshopIconManager>();
    state.get_icon_url(&id).unwrap_or_default()
}

#[allow(unused)]
pub fn debug_get_info() {
    let ids = [1837053004_usize, 2657513551_usize];
    let itemcount = ids.len();
    let mut args = vec![format!("itemcount={}", itemcount)];
    for (i, id) in ids.iter().enumerate() {
        args.push(format!("publishedfileids[{}]={}", i, id));
    }
    let args = args.join("&");
    // POST
    let client = reqwest::blocking::Client::new();
    let res = client.post(WORKSHOP_API)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(args)
        .send()
        .unwrap();
    let body = res.text().unwrap();
    println!("{}", body);
}