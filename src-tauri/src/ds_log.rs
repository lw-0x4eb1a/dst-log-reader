// DST Log file iter and parser
// TODO: 大文件懒解析
// TODO: 多线程解析
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::io::{Read, Seek};
use std::fs;
use std::sync::{Arc, Mutex};
use lines::{read_lines, linereader::LineReader};
use regex::Regex;
use once_cell::sync::Lazy;
use tauri::Manager;
use uuid::Uuid;

use crate::bootstrap::PathExt;
use crate::reader::LogReader;
use crate::steam_workshop::SteamWorkshopIconManager;

/// Max length of a line in log file.
/// Too long line will be skipped parsing.
static MAX_LINE_LEN: usize = 2000;

#[derive(Debug, Clone, PartialEq)]
pub enum LogPath {
    Ds(PathBuf),
    DstLocal(PathBuf),
    DstCloud(PathBuf, String),
    External(PathBuf),
}

impl Default for LogPath {
    fn default() -> Self {
        LogPath::External(PathBuf::new())
    }
}

impl LogPath {
    pub fn get_game_type(&self) -> String {
        match self {
            LogPath::Ds(_) => "ds".to_string(),
            LogPath::DstLocal(_) => "dst".to_string(),
            LogPath::DstCloud(_, _) => "dst".to_string(),
            LogPath::External(_) => "dyn".to_string(),
        }
    }

    #[inline]
    pub fn is_cloud(&self) -> bool {
        matches!(self, LogPath::DstCloud(_, _))
    }

    #[inline]
    pub fn is_zip(&self) -> bool {
        self.is_cloud()
    }

    pub fn get_path(&self) -> &Path {
        match self {
            LogPath::Ds(p) |
            LogPath::DstLocal(p) |
            LogPath::External(p)   => p.as_path(),
            LogPath::DstCloud(p, _) => p.as_path(),
        }
    }

    pub fn get_name(&self) -> String {
        match self {
            LogPath::Ds(p) |
            LogPath::DstLocal(p) |
            LogPath::External(p) => p.file_name_utf8(),
            LogPath::DstCloud(_, name) => name.clone(),
        }
    }

    /// convert to tauri window label
    pub fn to_label(&self) -> String {
        let calc_v5 = |path: String| {
            let uuid = Uuid::new_v5(&Uuid::NAMESPACE_URL, path.as_bytes());
            let uuid = uuid.as_braced().to_string();
            uuid[1..uuid.len()-1].to_string()
        };
        match self {
            LogPath::Ds(p) => {
                format!("ds-{}", calc_v5(p.to_string_lossy().to_string()))
            },
            LogPath::DstLocal(p) |
            LogPath::External(p) => {
                format!("dst-{}", calc_v5(p.to_string_lossy().to_string()))
            },
            LogPath::DstCloud(p, name) => {
                format!("dstcloud-{}-{}", 
                    calc_v5(p.to_string_lossy().to_string()),
                    calc_v5(name.clone())
                )
            },
        }
    }

    pub fn to_ipc(&self) -> String {
        json::object! {
            "game": self.get_game_type(),
            "filename": self.get_name(),
            // TODO: 这里是否会导致信息损失？
            "filepath": self.get_path().to_string_lossy().to_string(),
            "mtime": self.get_path().mtime_f64(),
            "filesize": self.get_path().file_size(),
            "is_zip": self.is_zip(),
        }.dump()
    }

    pub fn serialize(&self) -> String {
        match self {
            LogPath::Ds(p)=> json::object! {
                "type": "ds",
                "path": p.to_string_lossy().to_string(),
            },
            LogPath::DstLocal(p) => json::object! {
                "type": "dst",
                "path": p.to_string_lossy().to_string(),
            },
            LogPath::DstCloud(p, name) => json::object! {
                "type": "dstcloud",
                "path": p.to_string_lossy().to_string(),
                "name": name.clone(),
            },
            LogPath::External(p) => json::object! {
                "type": "external",
                "path": p.to_string_lossy().to_string(),
            },
        }.dump()
    }

    pub fn deserialize(s: &str) -> Result<LogPath, String> {
        let v = json::parse(s).map_err(|e| e.to_string())?;
        let path = v["path"].as_str().ok_or("path not found")?;
        match v["type"].as_str().ok_or("type not found")? {
            "ds" => Ok(LogPath::Ds(PathBuf::from(path))),
            "dst" => Ok(LogPath::DstLocal(PathBuf::from(path))),
            "dstcloud" => {
                let name = v["name"].as_str().ok_or("name not found")?;
                Ok(LogPath::DstCloud(PathBuf::from(path), name.to_string()))
            },
            "external" => Ok(LogPath::External(PathBuf::from(path))),
            _=> Err("unknown type".to_string()),
        }
    }

    pub fn get_menu_path(&self) -> String {
        let strip_home = |s: String| {
            match std::env::home_dir().map(|p| p.to_string_lossy().to_string()) {
                Some(dir) => {
                    if s.starts_with(&dir) {
                        format!("~{}", &s[dir.len()..])
                    } else {
                        s
                    }
                },
                None=> s
            }
        };
        match self {
            LogPath::Ds(p) |
            LogPath::DstLocal(p) |
            LogPath::External(p) => strip_home(p.to_string_lossy().to_string()),
            LogPath::DstCloud(p, name) => strip_home(format!("{}:{}", p.to_string_lossy(), name)),
        }
    }

    #[inline]
    pub fn exists(&self) -> bool {
        self.get_path().is_file()
    }

    pub fn open(&self) -> Result<LogReader, String> {
        LogReader::new(self).map_err(|e| e.to_string())
    }
}

/// self held loading state for LogComment
#[derive(Debug, Clone, Default)]
pub struct LogState {
    is_launching: bool,
    current_field_name: String,
    current_field_line: usize,
    current_line: usize,
}

impl LogState {
    pub fn default() -> Self {
        Self { 
            is_launching: true,
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Mod {
    moddir: String,
    name: String,
    version: Option<String>,
    workshop_id: Option<String>,
}

fn utf8_first(s: &str, n: usize) -> &str {
    for (i, (j, _)) in s.char_indices().enumerate() {
        if i == n {
            return &s[..j];
        }
    }
    s
}

/// annotation on the important parts of log content
#[derive(Debug, Clone, Default)]
pub struct LogComment {
    /// a content block with some information, 0:start, 1:end, 2:type, 3:extra
    fields: Vec<(usize, usize, String, String)>,
    /// if this log contains Lua stacktrace (printed from `StackTraceToLog()`)
    has_stacktrace: bool,
    /// if this log contains Lua Error
    has_lua_crash: bool,
    /// if this log contains force crash (eg. Assertion)
    has_c_crash: bool,
    /// eg: 654321
    build_version: String,
    /// eg: WIN32_STEAM
    build_platform: String,
    /// eg: 64-bit
    build_arch: String,
    /// eg: Mounting file system databundles/klump.zip successful.
    /// true: using *.zip, false: using files (debug)
    databundles_mounting_state: HashMap<String, bool>,
    /// registed mod by ModIndex, only in DST log
    mods_registed: HashMap<String, ()>,
    /// actual mod loaded
    /// eg: Loading mod: workshop-727774324 (Craft Pot) Version:0.15.0	
    mods: HashMap<String, Mod>,
    /// total runtime of the log, usually get from the last line
    total_time: Vec<u32>,

    state: LogState,
}

impl LogComment {
    pub fn parse_line_u8(&mut self, mut line: &[u8]) {
        self.state.current_line += 1;
        // println!("line: {}", self.state.current_line);
        if line.len() > MAX_LINE_LEN {
            return;
        }
        for end in [b'\n', b'\r', b'\t'].iter() {
            if line.ends_with(&[*end]) {
                line = &line[..line.len() - 1];
            }
        }
        let line = String::from_utf8_lossy(line).to_string();
        self.parse_line_impl(line.as_str());
    }

    fn parse_line_impl(&mut self, mut line: &str) {
        // strip [00:00:00]
        static RE: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r"^\[(\d+):(\d+):(\d+)\]:\s").unwrap()
        });
        // has_time is true if we found time prefix
        let (has_time, mut line) = match RE.captures(utf8_first(line, 20)) {
            Some(m)=> {
                let hour = m.get(1).unwrap().as_str().parse::<u32>().unwrap();
                let minute = m.get(2).unwrap().as_str().parse::<u32>().unwrap();
                let second = m.get(3).unwrap().as_str().parse::<u32>().unwrap();
                self.total_time.copy_from_slice(&[hour, minute, second]);
                (true, &line[m.get(0).unwrap().as_str().len()..])
            },
            None=> {
                (false, line)
            },
        };
        if self.build_version.is_empty() && line.starts_with("Don't Starve") {
            // Don't Starve Together: 654321 WIN32_STEAM
            // Don't Starve: 578406 OSX_STEAM
            static RE: Lazy<Regex> = Lazy::new(|| {
                Regex::new(r"^Don't Starve( Together)?: (\d+) ([A-Z0-9_]+)").unwrap()
            });
            if let Some(m) = RE.captures(line) {
                self.build_version.push_str(m.get(2).unwrap().as_str());
                self.build_platform.push_str(m.get(3).unwrap().as_str());
                return;
            }
        }

        if self.build_arch.is_empty() {
            // Mode: 64-bit
            static RE: Lazy<Regex> = Lazy::new(|| {
                Regex::new(r"^Mode: ([\w-]+)").unwrap()
            });
            if let Some(m) = RE.captures(line) {
                self.build_arch.push_str(m.get(1).unwrap().as_str());
                return;
            }
        }

        // Mounting file system databundles/klump.zip successful.
        static BUNDLE_RE: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r"^Mounting file system databundles/([\w_]+\.zip) (successful|skipped)\.$").unwrap()
        });
        if let Some(m) = BUNDLE_RE.captures(line) {
            let file = m.get(1).unwrap().as_str().to_string();
            let is_zip = m.get(2).unwrap().as_str() == "successful";
            self.databundles_mounting_state.insert(file, is_zip);
            return;
        }

        if line == "cGame::StartPlaying" {
            self.on_exit_launching_info();
            return;
        }

        // strip Lua debug print prefix
        // eg: scripts/widgets/craftslot.lua(99,1) 
        static LUA_DEBUG_RE: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r"^scripts/([\w/]+\.lua)\(\d+,\d+\)\s").unwrap()
        });
        // let mut lua_src = None;
        if let Some(m) = LUA_DEBUG_RE.captures(line) {
            // lua_src = Some(m.get(1).unwrap().as_str());
            line = &line[m.get(0).unwrap().end()..];
        }

        /// ModIndex:GetModsToLoad inserting moddir, \tworkshop-2771766820
        const MODDIR_PREFIX: &str = "ModIndex:GetModsToLoad inserting moddir, \t";
        if let Some(n) = line.find(MODDIR_PREFIX) {
            let moddir = &line[n + MODDIR_PREFIX.len()..];
            self.mods_registed.insert(moddir.to_string(), ());
            return;
        }
        
        // scripts/mods.lua(179,1)\s?
        // Loading mod: workshop-351325790 (Geometric Placement) Version:3.2.0	
        static LOADING_MOD_PREFIX_RE: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r"^(Fontend\-|Frontend\-)?Loading mod:\s").unwrap()
        });
        if let Some(n) = LOADING_MOD_PREFIX_RE.shortest_match(line) {
            let mut line = &line[n..];
            // strip final version if we found
            let version = match line.rfind(" Version:") {
                Some(n)=> {
                    let version = &line[n + " Version:".len()..];
                    line = &line[..n];
                    Some(version.to_string())
                },
                None=> None,
            };
            // match workshop
            static WORKSHOP_RE: Lazy<Regex> = Lazy::new(|| {
                Regex::new(r"(workshop\-\d+)").unwrap()
            });
            if let Some(m) = WORKSHOP_RE.captures(line) {
                let name_index = m.get(0).unwrap().end() + 2;
                let name = &line[name_index..line.len() - 1];
                let moddir = m.get(1).unwrap().as_str();
                let workshop_id = &moddir["workshop-".len()..];
                self.mods.entry(moddir.to_string()).or_insert(Mod{
                    moddir: moddir.to_string(),
                    name: name.to_string(),
                    version,
                    workshop_id: Some(workshop_id.to_string()),
                });
                return;
            }
            // match local mods
            for moddir in self.mods_registed.keys() {
                if line.starts_with(format!("{} (", moddir).as_str()) {
                    let name = &line[moddir.len() + 2..line.len() - 1];
                    self.mods.entry(moddir.to_string()).or_insert(Mod{
                        moddir: moddir.to_string(),
                        name: name.to_string(),
                        version,
                        workshop_id: None,
                    });
                    return;
                }
            }
        }

        if line == "stack traceback:" {
            self.has_stacktrace = true;
        }

        // LUA ERROR stack traceback:
        if let Some(n) = line.find("LUA ERROR stack traceback:") {
            self.has_lua_crash = true;
            return;
        }


        return;
    }

    /// insert default values after launching info
    fn on_exit_launching_info(&mut self) {
        if self.build_version.is_empty() {
            self.build_version.push_str("unknown");
        }
        if self.build_platform.is_empty() {
            self.build_platform.push_str("unknown");
        }
        if self.build_arch.is_empty() {
            self.build_arch.push_str("unknown");
        }
    }

    pub fn to_json(&self) -> json::JsonValue {
        json::object! {
            "fields": self.fields.iter().map(|(start, end, t, e)| {
                json::object! {
                    "start": *start,
                    "end": *end,
                    "type": t.to_string(),
                    "extra": e.to_string(),
                }
            }).collect::<Vec<_>>(),
            "has_stacktrace": self.has_stacktrace,
            "has_lua_crash": self.has_lua_crash,
            "has_c_crash": self.has_c_crash,
            "build_version": self.build_version.clone(),
            "build_platform": self.build_platform.clone(),
            "build_arch": self.build_arch.clone(),
            "databundles_mounting_state": self.databundles_mounting_state.clone(),
            // "mods_registed": self.mods_registed.keys().cloned().collect::<Vec<_>>(),
            "mods": self.mods.iter().map(|(k, v)| {
                json::object! {
                    "moddir": v.moddir.clone(),
                    "name": v.name.clone(),
                    "version": v.version.clone(),
                    "workshop_id": v.workshop_id.clone(),
                }
            }).collect::<Vec<_>>(),
            "total_time": self.total_time.clone(),
        }
    }

    pub fn to_ipc(&self) -> String {
        self.to_json().dump()
    }
}

fn iter_ds_logs(app: &tauri::AppHandle) -> Vec<LogPath> {
    let dir = app.path().document_dir().unwrap();
    let ds = dir.join("Klei/DoNotStarve/");
    let mut result = vec![];
    for name in ["log.txt", "backup_log.txt"] {
        let path = ds.join(name);
        if path.is_file() {
            if let Ok(f) = fs::OpenOptions::new().read(true).open(&path) {
                result.push(LogPath::Ds(path));
            }
        }
    }
    result
}

fn iter_dst_logs(app: &tauri::AppHandle, identifier: &str) -> Vec<LogPath> {
    let dir = app.path().document_dir().unwrap();
    let dst = dir.join("Klei").join(identifier);
    let mut result = vec![];
    if !dst.is_dir() {
        return result;
    }
    if dst.join("client_log.txt").is_file() {
        result.push(LogPath::DstLocal(dst.join("client_log.txt")));
    }
    if let Ok(read) = fs::read_dir(dst.join("backup/client_log")) {
        for entry in read.flatten() {
            let path = entry.path();
            // name like client_log_2025-01-12-20-35-47.txt
            static NAME_RE: Lazy<Regex> = Lazy::new(|| {
                Regex::new(r"^client_log_(\d{4}-\d{2}-\d{2}-\d{2}-\d{2}-\d{2})\.txt$").unwrap()
            });
            let name = path.file_name_utf8();
            if path.is_file() && NAME_RE.is_match(name.as_str()) {
                result.push(LogPath::DstLocal(path));
            }
            
        }
    }

    for entry in fs::read_dir(dst).unwrap().flatten() {
        static UID_RE: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r"^\d+$").unwrap()
        });
        let path = entry.path();
        let name = path.file_name_utf8();
        if path.is_dir() && UID_RE.is_match(name.as_str()) {
            result.extend(iter_dst_cluster_logs(&path));
        }
        
            
    }
    result
}

fn iter_dst_cluster_logs(dir: &Path) -> Vec<LogPath> {
    let mut result = vec![];
    // local saves
    if let Ok(read) = fs::read_dir(dir) {
        for entry in read.flatten() {
            let path = entry.path();
            let name = path.file_name_utf8();
            if path.is_dir() && name.starts_with("Cluster_") {
                for shard in ["Master", "Caves"] {
                    let path = path.join(shard);
                    if path.is_dir() {
                        result.extend(iter_local_cluster_logs(&path));
                    }
                }
            }
            
        }
    }
    // cloud saves
    if let Ok(read) = fs::read_dir(dir.join("CloudSaves")) {
        for entry in read.flatten() {
            let path = entry.path();
            let name = path.file_name_utf8();
            // 00CFB14F0C009004
            static CLOUD_HASH: Lazy<Regex> = Lazy::new(|| {
                Regex::new(r"^[0-9A-F]{16}$").unwrap()
            });
            if path.is_dir() && CLOUD_HASH.is_match(name.as_str()) {
                for shard in ["Master", "Caves"] {
                    let path = path.join(format!("{}.zip", shard));
                    if path.is_file() {
                        result.extend(iter_cloud_cluster_logs(&path));
                    }
                }
            }
            
        }
    }
    result
}

fn iter_local_cluster_logs(dir: &Path) -> Vec<LogPath> {
    let mut result = vec![];
    if dir.join("server_log.txt").is_file() {
        result.push(LogPath::DstLocal(dir.join("server_log.txt")));
    }
    if let Ok(read) = fs::read_dir(dir.join("backup/server_log")) {
        for entry in read.flatten() {
            let path = entry.path();
            // name like server_log_2023-04-01-21-43-15.txt
            static NAME_RE: Lazy<Regex> = Lazy::new(|| {
                Regex::new(r"^server_log_(\d{4}-\d{2}-\d{2}-\d{2}-\d{2}-\d{2})\.txt$").unwrap()
            });
            let name = path.file_name_utf8();
            if path.is_file() && NAME_RE.is_match(name.as_str()) {
                result.push(LogPath::DstLocal(path));
            }
            
        }
    }
    result
}

fn iter_cloud_cluster_logs(dir: &Path) -> Vec<LogPath> {
    use zip;
    let mut result = vec![];
    // iterate all files in the zip
    let f = fs::OpenOptions::new().read(true).open(dir).unwrap();
    let mut archive = zip::ZipArchive::new(f).unwrap();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).unwrap();
        if !file.is_file() { continue; }
        let name = file.name();
        // server_log.txt
        // backup/server_log/server_log_2023-03-28-21-10-49.txt
        static NAME_RE: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r"^backup/server_log/server_log_(\d{4}-\d{2}-\d{2}-\d{2}-\d{2}-\d{2})\.txt$").unwrap()
        });
        if name == "server_log.txt" || NAME_RE.is_match(name) {
            let name = name.to_string();
            if file.read(&mut [0; 1]).is_ok() {
                result.push(LogPath::DstCloud(dir.to_path_buf(), name));
            }
        }
    }
    result
}

fn parse_log_content(f: impl Read) -> Result<LogComment, String> {
    // TODO: 这里似乎丢失了所有权，导致无法追踪f.tell()
    let mut reader = LineReader::new(f);
    let mut comment = LogComment{ 
        total_time: vec![0, 0, 0],
        state: LogState::default(),
        ..Default::default()
    };
    read_lines!(line in reader, {
        match line {
            Ok(line) => {
                comment.parse_line_u8(line);
            },
            Err(e) => return Err(e.to_string())
        }
    });
    Ok(comment)
}

/// List all logs for DS/DST and sort by mtime.
/// No file io in this handler.
#[tauri::command]
pub async fn list_all_logs(app: tauri::AppHandle) -> Result<Vec<String>, String>{
    // check document dir accessbility
    match app.path().document_dir() {
        Ok(dir) => {
            if !dir.is_dir() {
                return Err("document dir not exists".to_string())
            }
            else if let Err(e) = dir.read_dir() {
                return Err(format!("document dir access error: {}", e))
            }
        },
        Err(e) => return Err(format!("document dir resolve error: {}", e)),
        _=> {}
    }
    let mut result = vec![];
    result.extend(iter_ds_logs(&app));
    result.extend(iter_dst_logs(&app, "DoNotStarveTogether"));
    result.extend(iter_dst_logs(&app, "DoNotStarveTogetherBetaBranch"));
    result.extend(iter_dst_logs(&app, "DoNotStarveTogetherRail"));
    // sort
    result.sort_by(|a, b| {
        let a = a.get_path().mtime_f64();
        let b = b.get_path().mtime_f64();
        b.partial_cmp(&a).unwrap()
    });
    Ok(result.into_iter().map(|log| log.to_ipc()).collect())
}

#[tauri::command(rename_all = "snake_case")]
pub async fn load_log_abstract(filepath: String, filename: String, is_zip: bool) -> Result<String, String> {
    let path = Path::new(&filepath);
    if !path.is_file() {
        return Err("file not exists".to_string());
    }
    let mut f = fs::OpenOptions::new().read(true).open(path).unwrap();
    use std::io::Cursor;
    let f = if is_zip {
        // extract zip file
        let mut archive = zip::ZipArchive::new(f)
            .map_err(|e| format!("failed to load zip archive: {} {}", &filepath, e))?;
        let mut child_file = archive.by_name(&filename)
            .map_err(|e| format!("failed to load zip file: {} {}", &filename, e))?;
        let size = child_file.size();
        // TODO: skip parsing if size too big
        let mut buf = Vec::with_capacity(size as usize);
        child_file.read_to_end(&mut buf).unwrap();
        Cursor::new(buf)
    }
    else {
        // skip some bytes if file too big
        let size = path.file_size();
        const LOADING_SIZE: u64 = 10 * 1024 * 1024;
        if size > LOADING_SIZE {
            let skip = size - LOADING_SIZE;
            f.seek(std::io::SeekFrom::Start(skip)).unwrap();
        }
        let mut buf = Vec::with_capacity(LOADING_SIZE as usize);
        f.read_to_end(&mut buf)
            .map_err(|e| format!("failed to read file: {} {}",&filepath, e))?;
        Cursor::new(buf)
    };

    let mut has_lua_crash = false;
    let mut total_time = ["".to_string(), "".to_string(), "".to_string()];
    let mut reader = LineReader::new(f);
    read_lines!(line in reader, {
        match line {
            Ok(line) => {
                let line = String::from_utf8_lossy(line).to_string();
                if line.starts_with("LUA ERROR stack traceback:") {
                    has_lua_crash = true;
                }
                // grap time
                static RE: Lazy<Regex> = Lazy::new(|| {
                    Regex::new(r"^\[(\d+):(\d+):(\d+)\]:\s").unwrap()
                });
                if let Some(m) = RE.captures(line.as_str()) {
                    total_time[0] = m.get(1).unwrap().as_str().to_string();
                    total_time[1] = m.get(2).unwrap().as_str().to_string();
                    total_time[2] = m.get(3).unwrap().as_str().to_string();
                }
            },
            Err(e) => return Err(e.to_string())
        }
    });
    Ok(json::object! {
        "filename": filename,
        "filepath": filepath,
        "is_zip": is_zip,
        "total_time": total_time.to_vec(),
        "has_lua_crash": has_lua_crash,
    }.dump())
}

#[derive(Default)]
pub struct LogModel {
    pub path: LogPath,

    active: Arc<Mutex<bool>>,
    comment: Arc<Mutex<LogComment>>,
    exists: Arc<Mutex<bool>>,
    mtime: Arc<Mutex<f64>>,
    /// first 4096 bytes for diff
    head: Arc<Mutex<Vec<u8>>>,

    debug_content: Arc<Mutex<String>>,
}

impl LogModel {
    pub fn new(path: LogPath)-> Self {
        let filepath = path.get_path().to_owned();
        let filename = path.get_name();
        let model = Self {
            path,
            active: Arc::new(Mutex::new(true)),
            ..Default::default()
        };
        let path = model.path.clone();
        // clone thread variables
        let active = model.active.clone();
        let comment = model.comment.clone();
        let exists = model.exists.clone();
        let mtime = model.mtime.clone();
        let head = model.head.clone();
        let debug_content = model.debug_content.clone();
        // spawn child thread for log parsing
        std::thread::spawn(move || {
            // sleep macro
            macro_rules! sleep {
                ($sec:expr) => {
                    std::thread::sleep(std::time::Duration::from_secs($sec));
                    continue;
                };
                ()=> {
                    sleep!(1);
                }
            }
            loop {
                if !*active.lock().unwrap() {
                    sleep!();
                }
                if !filepath.is_file() {
                    *exists.lock().unwrap() = false;
                    sleep!();
                }

                *exists.lock().unwrap() = true;
                let last_mtime = *mtime.lock().unwrap();
                let current_mtime = filepath.mtime_f64();
                if current_mtime == last_mtime {
                    sleep!();
                }
                *mtime.lock().unwrap() = current_mtime;
                
                let mut f = match path.open() {
                    Ok(f)=> f,
                    Err(e)=> {
                        eprintln!("failed to open file: {}", e);
                        sleep!();
                    }
                };
                let mut buffer = vec![0; 4096];
                f.read(&mut buffer).unwrap();
                // TODO: 需要进行diff
                *head.lock().unwrap() = buffer.clone();
                f.read_to_end(&mut buffer).unwrap();
                f.rewind().unwrap();
                let new_comment = parse_log_content(f).unwrap_or_default();
                *comment.lock().unwrap() = new_comment;
                debug_content.lock().unwrap().clear();
                debug_content.lock().unwrap().push_str(String::from_utf8_lossy(buffer.as_slice()).as_ref());
                sleep!();
            }
        });
        model
    }

    pub fn label(&self) -> String {
        self.path.to_label()
    }

    pub fn get_mod_id_list(&self) -> Vec<String> {
        self.comment.lock().unwrap().mods.keys().cloned().collect()
    }

    pub fn to_json(&self) -> json::JsonValue {
        json::object! {
            "label": self.label(),
            "active": *self.active.lock().unwrap(),
            "exists": *self.exists.lock().unwrap(),
            "mtime": *self.mtime.lock().unwrap(),
            "comment": self.comment.lock().unwrap().to_json(),
            "debug_content": self.debug_content.lock().unwrap().clone(),
        }
    }

    pub fn to_ipc(&self) -> String {
        self.to_json().dump()
    }
}

#[derive(Default)]
pub struct LogModelState {
    logs: Mutex<HashMap<String, LogModel>>,
}

impl LogModelState {
    pub fn register(&self, path: &LogPath) {
        let label = path.to_label();
        let mut logs = self.logs.lock().unwrap();
        logs.entry(label).or_insert(LogModel::new(path.clone()));

        println!("register log: {:?} / current: {}", path, logs.len());
    }

    pub fn to_ipc(&self, id: &str) -> String {
        let logs = self.logs.lock().unwrap();
        match logs.get(id) {
            Some(log)=> log.to_ipc(),
            None=> "".to_string(),
        }
    }

    pub fn get_mod_id_list(&self, id: &str) -> Vec<String> {
        let logs = self.logs.lock().unwrap();
        match logs.get(id) {
            Some(log)=> log.get_mod_id_list(),
            None=> vec![],
        }
    }

    pub fn len(&self) -> usize {
        self.logs.lock().unwrap().len()
    }

    /// remove all inactive logs and return current count
    pub fn clear_inactive(&self, threshold: usize) -> usize {
        let mut logs = self.logs.lock().unwrap();
        if logs.len() > threshold {
            logs.retain(|_, log| *log.active.lock().unwrap());
        }
        logs.len()
    }

    pub fn set_inactive(&self, label: &str) {
        let mut logs = self.logs.lock().unwrap();
        logs.entry(label.to_string()).and_modify(|log| {
            *log.active.lock().unwrap() = false;
        });
    }

    pub fn get_path(&self, label: &str) -> Option<PathBuf> {
        let logs = self.logs.lock().unwrap();
        logs.get(label).map(|log| log.path.get_path().to_path_buf())
    }
}

#[tauri::command]
pub async fn load_log_init(app: tauri::AppHandle, id: String) -> Result<String, String> {
    let state = app.state::<LogModelState>();
    let mod_id_list = state.get_mod_id_list(&id);
    app.state::<SteamWorkshopIconManager>().enqueue_list(mod_id_list);
    Ok(state.to_ipc(&id))
}

#[tauri::command]
pub async fn load_log_handshake(id: String) -> Result<String, String> {
    unimplemented!()
    
}

#[cfg(debug_assertions)]
#[allow(unused)]
pub fn debug_parse_log(path: String) {
    let f = fs::OpenOptions::new().read(true).open(path).unwrap();
    let comment = parse_log_content(f).unwrap();
    println!("{:?}", comment);
}