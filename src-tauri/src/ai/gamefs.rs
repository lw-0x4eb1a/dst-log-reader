//! Locate the Don't Starve / Don't Starve Together installation and mod
//! folders on the current machine, and read files out of them.
//!
//! The layout differs between platforms, eg:
//! - Windows (Steam): `.../steamapps/common/Don't Starve Together/data/databundles/scripts.zip`
//! - macOS (Steam):   `.../steamapps/common/Don't Starve Together/dontstarve_steam.app/Contents/data/databundles/scripts.zip`
//!
//! Nothing here is fatal: when the game folder can not be found the tools
//! simply report that to the model.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use crate::ds_log::LogPath;
use crate::reader::LogReader;

/// Max lines returned by a single read.
pub const MAX_READ_LINES: usize = 600;
/// Max characters returned by a single read.
pub const MAX_READ_CHARS: usize = 24_000;
/// Max entries returned by a listing.
const MAX_LIST: usize = 400;
/// Max file size we are willing to read.
const MAX_FILE_SIZE: u64 = 8 * 1024 * 1024;
/// Max lines of a log file we keep in memory.
const MAX_LOG_LINES: usize = 400_000;
/// Max bytes of a log file we keep in memory.
const MAX_LOG_BYTES: u64 = 128 * 1024 * 1024;

const GAME_DIRS: [&str; 3] = [
    "Don't Starve Together",
    "Don't Starve Together Dedicated Server",
    "Don't Starve",
];

/// Steam app ids for workshop content.
const DST_APP_ID: &str = "322330";
const DS_APP_ID: &str = "219740";

/// Where a set of scripts lives.
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptSource {
    /// A plain directory which is the root of `scripts/` (eg `data/scripts`).
    Dir(PathBuf),
    /// A zip archive whose entries are named `scripts/xxx.lua`.
    Zip(PathBuf),
}

#[derive(Debug, Clone)]
pub struct Install {
    pub name: String,
    pub root: PathBuf,
    pub sources: Vec<ScriptSource>,
}

#[derive(Debug, Default, Clone)]
pub struct GameFs {
    pub installs: Vec<Install>,
    pub workshop_dirs: Vec<PathBuf>,
    pub local_mod_dirs: Vec<PathBuf>,
}

fn push_path(v: &mut Vec<PathBuf>, p: PathBuf) {
    if p.is_dir() && !v.contains(&p) {
        v.push(p);
    }
}

fn push_dir(out: &mut Vec<ScriptSource>, p: PathBuf) {
    if p.is_dir() && !out.iter().any(|s| matches!(s, ScriptSource::Dir(d) if d == &p)) {
        out.push(ScriptSource::Dir(p));
    }
}

fn push_zip(out: &mut Vec<ScriptSource>, p: PathBuf) {
    if p.is_file() && !out.iter().any(|s| matches!(s, ScriptSource::Zip(d) if d == &p)) {
        out.push(ScriptSource::Zip(p));
    }
}

/// Truncate a string on a char boundary.
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

fn home_dir() -> Option<PathBuf> {
    #[allow(deprecated)]
    std::env::home_dir()
}

/// Steam installation roots (the folder containing `steamapps`).
fn steam_roots() -> Vec<PathBuf> {
    let mut out = vec![];
    if let Some(home) = home_dir() {
        if cfg!(target_os = "macos") {
            push_path(&mut out, home.join("Library/Application Support/Steam"));
        }
        if cfg!(target_os = "linux") {
            push_path(&mut out, home.join(".steam/steam"));
            push_path(&mut out, home.join(".local/share/Steam"));
            push_path(&mut out, home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
        }
        if cfg!(target_os = "windows") {
            push_path(&mut out, home.join("AppData/Local/Steam"));
        }
    }
    if cfg!(target_os = "windows") {
        for key in ["ProgramFiles(x86)", "ProgramFiles", "ProgramW6432"] {
            if let Ok(value) = std::env::var(key) {
                let value = PathBuf::from(value);
                push_path(&mut out, value.join("Steam"));
                push_path(&mut out, value.join("Steam/steamapps/common"));
            }
        }
        for drive in ["C", "D", "E", "F", "G", "H"] {
            push_path(&mut out, PathBuf::from(format!("{}:/Steam", drive)));
            push_path(&mut out, PathBuf::from(format!("{}:/SteamLibrary", drive)));
            push_path(&mut out, PathBuf::from(format!("{}:/Program Files (x86)/Steam", drive)));
            push_path(&mut out, PathBuf::from(format!("{}:/Program Files/Steam", drive)));
        }
    }
    out
}

/// Steam roots plus every additional library folder found in
/// `steamapps/libraryfolders.vdf`.
fn library_roots() -> Vec<PathBuf> {
    let mut out = vec![];
    for root in steam_roots() {
        push_path(&mut out, root.clone());
        let vdf = root.join("steamapps/libraryfolders.vdf");
        if let Ok(text) = fs::read_to_string(&vdf) {
            for line in text.lines() {
                let line = line.trim();
                let Some(rest) = line.strip_prefix("\"path\"") else { continue };
                let mut parts = rest.split('"').filter(|s| !s.trim().is_empty());
                let value = parts.next().unwrap_or_default();
                // vdf escapes backslashes
                let path = value.replace("\\\\", "\\");
                push_path(&mut out, PathBuf::from(path));
            }
        }
    }
    // library folders keep their own `steamapps` marker
    out.retain(|p| p.is_dir());
    out
}

/// Find script sources below an installation folder.
fn find_sources(root: &Path) -> Vec<ScriptSource> {
    let mut out: Vec<ScriptSource> = vec![];
    const DIR_RELS: [&str; 4] = [
        "data/scripts",
        "data/databundles/scripts",
        "Contents/data/scripts",
        "Contents/data/databundles/scripts",
    ];
    const ZIP_RELS: [&str; 2] = [
        "data/databundles/scripts.zip",
        "Contents/data/databundles/scripts.zip",
    ];
    for rel in DIR_RELS {
        push_dir(&mut out, root.join(rel));
    }
    for rel in ZIP_RELS {
        push_zip(&mut out, root.join(rel));
    }
    // macOS ships the game inside an .app bundle
    if let Ok(rd) = fs::read_dir(root) {
        for entry in rd.flatten() {
            let path = entry.path();
            let is_bundle = path.is_dir()
                && path
                    .file_name()
                    .map(|n| n.to_string_lossy().ends_with(".app"))
                    .unwrap_or(false);
            if !is_bundle {
                continue;
            }
            for rel in ZIP_RELS {
                push_zip(&mut out, path.join(rel));
            }
            for rel in DIR_RELS {
                push_dir(&mut out, path.join(rel));
            }
        }
    }
    if out.is_empty() {
        // unknown layout: shallow search
        walk_sources(root, 0, 4, &mut out);
    }
    out
}

fn walk_sources(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<ScriptSource>) {
    if depth > max_depth || out.len() > 8 {
        return;
    }
    const SKIP: [&str; 14] = [
        "mods", "workshop", "cache", "userdata", "steamapps", "anim", "images",
        "sound", "fonts", "levels", "bigportraits", "minimap", "shaders", "movies",
    ];
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if SKIP.contains(&name.as_str()) {
            continue;
        }
        if name == "databundles" {
            push_zip(out, path.join("scripts.zip"));
            push_dir(out, path.join("scripts"));
        }
        let in_data = dir
            .file_name()
            .map(|n| n.to_string_lossy() == "data")
            .unwrap_or(false);
        if name == "scripts" && in_data {
            push_dir(out, path.clone());
        }
        walk_sources(&path, depth + 1, max_depth, out);
    }
}

impl GameFs {
    /// Best effort detection of the game and mod folders.
    pub fn detect(
        documents_dir: Option<PathBuf>,
        game_dir: Option<&str>,
        game_type: &str,
    ) -> GameFs {
        let mut fs_ = GameFs::default();
        let mut roots: Vec<PathBuf> = vec![];
        let user_root = game_dir
            .map(|d| PathBuf::from(d.trim()))
            .filter(|p| p.is_dir());

        let libs = library_roots();
        if let Some(root) = &user_root {
            push_path(&mut roots, root.clone());
            for name in GAME_DIRS {
                push_path(&mut roots, root.join(name));
                push_path(&mut roots, root.join("steamapps/common").join(name));
            }
        }
        for lib in &libs {
            let common = lib.join("steamapps/common");
            for name in GAME_DIRS {
                push_path(&mut roots, common.join(name));
            }
        }
        for root in roots {
            let sources = find_sources(&root);
            if sources.is_empty() {
                continue;
            }
            let name = root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            fs_.installs.push(Install { name, root, sources });
        }
        // put the game the log belongs to first
        let prefer_ds = game_type == "ds";
        fs_.installs.sort_by_key(|i| {
            let is_ds = i.name.contains("Don't Starve") && !i.name.contains("Together");
            if is_ds == prefer_ds { 0 } else { 1 }
        });

        for lib in &libs {
            for app_id in [DST_APP_ID, DS_APP_ID] {
                push_path(
                    &mut fs_.workshop_dirs,
                    lib.join("steamapps/workshop/content").join(app_id),
                );
            }
        }
        if let Some(root) = &user_root {
            for anc in root.ancestors().take(5) {
                for app_id in [DST_APP_ID, DS_APP_ID] {
                    push_path(
                        &mut fs_.workshop_dirs,
                        anc.join("steamapps/workshop/content").join(app_id),
                    );
                    push_path(&mut fs_.workshop_dirs, anc.join("workshop/content").join(app_id));
                }
            }
        }

        for install in &fs_.installs {
            push_path(&mut fs_.local_mod_dirs, install.root.join("mods"));
            push_path(&mut fs_.local_mod_dirs, install.root.join("Contents/mods"));
            if let Ok(rd) = fs::read_dir(&install.root) {
                for entry in rd.flatten() {
                    let path = entry.path();
                    let is_bundle = path.is_dir()
                        && path
                            .file_name()
                            .map(|n| n.to_string_lossy().ends_with(".app"))
                            .unwrap_or(false);
                    if is_bundle {
                        push_path(&mut fs_.local_mod_dirs, path.join("Contents/mods"));
                    }
                }
            }
        }

        // user data: local mods and per-cluster mods
        if let Some(doc) = documents_dir {
            let klei = doc.join("Klei");
            for ident in [
                "DoNotStarveTogether",
                "DoNotStarveTogetherBetaBranch",
                "DoNotStarveTogetherRail",
                "DoNotStarve",
            ] {
                let base = klei.join(ident);
                push_path(&mut fs_.local_mod_dirs, base.join("mods"));
                let Ok(rd) = fs::read_dir(&base) else { continue };
                for uid in rd.flatten() {
                    let uid_path = uid.path();
                    if !uid_path.is_dir() {
                        continue;
                    }
                    let Ok(rd) = fs::read_dir(&uid_path) else { continue };
                    for cluster in rd.flatten() {
                        let cluster_path = cluster.path();
                        let name = cluster_path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        if !name.starts_with("Cluster_") {
                            continue;
                        }
                        for shard in ["Master", "Caves"] {
                            push_path(&mut fs_.local_mod_dirs, cluster_path.join(shard).join("mods"));
                        }
                    }
                }
            }
        }
        fs_
    }

    pub fn has_scripts(&self) -> bool {
        self.installs.iter().any(|i| !i.sources.is_empty())
    }

    /// Read a game script, eg `scripts/widgets/craftslot.lua`.
    pub fn read_script(
        &self,
        path: &str,
        line: Option<usize>,
        context: usize,
    ) -> Result<String, String> {
        let names = candidate_names(path);
        if names.is_empty() {
            return Err("路径为空".to_string());
        }
        let mut last_err = None;
        for install in &self.installs {
            for source in &install.sources {
                match read_source(source, &names) {
                    Ok(Some(text)) => {
                        let label = source_label(source, &names[0]);
                        return Ok(format!(
                            "// {}\n{}",
                            label,
                            slice_text(&text, line, context)
                        ));
                    },
                    Ok(None) => {},
                    Err(e) => last_err = Some(e),
                }
            }
        }
        let mut msg = format!("未找到游戏脚本 `{}`。", path);
        if !self.has_scripts() {
            msg.push_str(
                "\n没有检测到游戏安装目录。可以在“设置 - AI 分析”里手动填写游戏安装路径；\
                 Windows 一般位于 `C:/Program Files (x86)/Steam/steamapps/common/Don't Starve Together`，\
                 macOS 一般位于 `~/Library/Application Support/Steam/steamapps/common/Don't Starve Together`。",
            );
        } else {
            let roots = self
                .installs
                .iter()
                .map(|i| i.root.to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            msg.push_str(&format!("\n已找到的安装目录: {}", roots));
        }
        if let Some(e) = last_err {
            msg.push_str(&format!("\n读取失败: {}", e));
        }
        Err(msg)
    }

    /// List files under the game scripts root.
    pub fn list_scripts(&self, dir: &str) -> Result<String, String> {
        let prefix = normalize_rel(dir).unwrap_or_default();
        for install in &self.installs {
            for source in &install.sources {
                if let Ok(entries) = list_source(source, &prefix) {
                    if entries.is_empty() {
                        continue;
                    }
                    return Ok(format!(
                        "{} ({}):\n{}",
                        source_label(source, ""),
                        install.name,
                        entries.join("\n")
                    ));
                }
            }
        }
        if !self.has_scripts() {
            return Err("没有检测到游戏安装目录，无法列出游戏脚本。".to_string());
        }
        Err(format!("目录 `{}` 不存在或为空", dir))
    }

    /// Resolve a moddir (`workshop-123456` or a local folder name) to a folder.
    pub fn find_mod_root(&self, moddir: &str) -> Option<PathBuf> {
        let key = mod_key(moddir)?;
        let id = key
            .strip_prefix("workshop-")
            .map(|s| s.to_string())
            .unwrap_or_else(|| key.clone());
        if key.starts_with("workshop-") {
            for dir in &self.workshop_dirs {
                let path = dir.join(&id);
                if path.is_dir() {
                    return Some(path);
                }
            }
        }
        for dir in &self.local_mod_dirs {
            for name in [&key, &id] {
                let path = dir.join(name);
                if path.is_dir() {
                    return Some(path);
                }
            }
        }
        // case insensitive fallback
        for dir in self.local_mod_dirs.iter().chain(self.workshop_dirs.iter()) {
            let Ok(rd) = fs::read_dir(dir) else { continue };
            for entry in rd.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.eq_ignore_ascii_case(&key) || name.eq_ignore_ascii_case(&id) {
                    return Some(entry.path());
                }
            }
        }
        None
    }

    pub fn mod_dirs_hint(&self) -> String {
        let mut dirs = self
            .local_mod_dirs
            .iter()
            .chain(self.workshop_dirs.iter())
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        dirs.truncate(6);
        if dirs.is_empty() {
            "（未找到任何模组目录）".to_string()
        } else {
            dirs.join(" | ")
        }
    }

    pub fn read_mod_file(
        &self,
        moddir: &str,
        path: &str,
        line: Option<usize>,
        context: usize,
    ) -> Result<String, String> {
        let root = self.find_mod_root(moddir).ok_or_else(|| {
            format!(
                "未找到模组 `{}`。已知的模组目录: {}",
                moddir,
                self.mod_dirs_hint()
            )
        })?;
        let rel = normalize_rel(path)
            .filter(|p| !p.is_empty())
            .ok_or_else(|| format!("非法路径 `{}`", path))?;
        let full = root.join(&rel);
        let canon_root = root.canonicalize().unwrap_or_else(|_| root.clone());
        let canon = full
            .canonicalize()
            .map_err(|_| format!("模组 `{}` 中不存在文件 `{}`", moddir, path))?;
        if !canon.starts_with(&canon_root) || !canon.is_file() {
            return Err(format!("非法路径 `{}`", path));
        }
        let text = read_text(&canon)?;
        Ok(format!(
            "// {}/{}\n{}",
            root.to_string_lossy(),
            rel,
            slice_text(&text, line, context)
        ))
    }

    pub fn list_mod_files(&self, moddir: &str, dir: &str) -> Result<String, String> {
        let root = self.find_mod_root(moddir).ok_or_else(|| {
            format!(
                "未找到模组 `{}`。已知的模组目录: {}",
                moddir,
                self.mod_dirs_hint()
            )
        })?;
        let rel = normalize_rel(dir).unwrap_or_default();
        let target = root.join(&rel);
        let canon_root = root.canonicalize().unwrap_or_else(|_| root.clone());
        let target = target
            .canonicalize()
            .map_err(|_| format!("目录 `{}` 不存在", dir))?;
        if !target.starts_with(&canon_root) || !target.is_dir() {
            return Err(format!("非法目录 `{}`", dir));
        }
        let mut entries: Vec<String> = vec![];
        for entry in fs::read_dir(&target).map_err(|e| e.to_string())?.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if path.is_dir() {
                if is_noise_dir(&name) {
                    continue;
                }
                entries.push(format!("{}/", name));
            } else if is_text_file(&name) {
                entries.push(name);
            }
            if entries.len() > MAX_LIST {
                break;
            }
        }
        entries.sort();
        Ok(format!(
            "{} ({}):\n{}",
            root.to_string_lossy(),
            if rel.is_empty() { "." } else { rel.as_str() },
            entries.join("\n")
        ))
    }

    /// Search a mod for a keyword, returning `file:line: text` matches.
    pub fn search_mod(&self, moddir: &str, keyword: &str, max: usize) -> Result<String, String> {
        let root = self.find_mod_root(moddir).ok_or_else(|| {
            format!(
                "未找到模组 `{}`。已知的模组目录: {}",
                moddir,
                self.mod_dirs_hint()
            )
        })?;
        let keyword = keyword.trim();
        if keyword.is_empty() {
            return Err("关键字为空".to_string());
        }
        let needle = keyword.to_lowercase();
        let mut hits: Vec<String> = vec![];
        let mut visited = 0usize;
        let mut queue = vec![(root.clone(), 0usize)];
        while let Some((dir, depth)) = queue.pop() {
            if depth > 6 || hits.len() >= max || visited > 4000 {
                break;
            }
            let Ok(rd) = fs::read_dir(&dir) else { continue };
            for entry in rd.flatten() {
                visited += 1;
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                if path.is_dir() {
                    if !is_noise_dir(&name) {
                        queue.push((path, depth + 1));
                    }
                    continue;
                }
                if !is_text_file(&name) {
                    continue;
                }
                if path.metadata().map(|m| m.len()).unwrap_or(0) > 2 * 1024 * 1024 {
                    continue;
                }
                let Ok(text) = read_text(&path) else { continue };
                let rel = path
                    .strip_prefix(&root)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or(name);
                for (i, l) in text.lines().enumerate() {
                    if l.to_lowercase().contains(&needle) {
                        hits.push(format!("{}:{}: {}", rel, i + 1, clip(l.trim(), 200)));
                        if hits.len() >= max {
                            break;
                        }
                    }
                }
                if hits.len() >= max {
                    break;
                }
            }
        }
        if hits.is_empty() {
            Ok(format!("在模组 `{}` 中没有找到 `{}`", moddir, keyword))
        } else {
            Ok(hits.join("\n"))
        }
    }
}

/// Normalize the moddir found in a stacktrace, eg
/// `../mods/workshop-1234567/scripts/x.lua` -> `workshop-1234567`.
fn mod_key(moddir: &str) -> Option<String> {
    let mut key = moddir.trim().replace('\\', "/");
    key = key.trim_end_matches('/').to_string();
    let mut changed = true;
    while changed {
        changed = false;
        for prefix in ["../mods/", "mods/", "./"] {
            if let Some(rest) = key.strip_prefix(prefix) {
                key = rest.to_string();
                changed = true;
            }
        }
    }
    if let Some(index) = key.find('/') {
        key = key[..index].to_string();
    }
    if key.is_empty() || key.contains(':') {
        None
    } else {
        Some(key)
    }
}

fn is_noise_dir(name: &str) -> bool {    matches!(
        name,
        "anim"
            | "images"
            | "sound"
            | "fonts"
            | "bigportraits"
            | "minimap"
            | "shaders"
            | "movies"
            | "levels"
            | "haptics"
            | "unsafedata"
            | "node_modules"
    )
}

fn is_text_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or_default();
    matches!(
        ext,
        "lua" | "txt" | "xml" | "json" | "md" | "ini" | "manifest" | "po" | "csv" | "yml" | "yaml"
    ) || lower == "modinfo"
}

/// Normalize a relative path given by the model. Returns `None` for absolute
/// paths or paths escaping their root.
pub fn normalize_rel(path: &str) -> Option<String> {
    let mut path = path.trim().replace('\\', "/");
    while let Some(rest) = path.strip_prefix("./") {
        path = rest.to_string();
    }
    let path = path.trim_start_matches('/').to_string();
    if path.is_empty() {
        return Some(String::new());
    }
    let mut parts: Vec<&str> = vec![];
    for part in path.split('/') {
        match part {
            "" | "." => {},
            ".." => return None,
            p => {
                if p.contains(':') {
                    return None;
                }
                parts.push(p)
            },
        }
    }
    Some(parts.join("/"))
}

/// Names a script may be referred to by: with or without the `scripts/`
/// prefix, and with `data/` stripped.
fn candidate_names(path: &str) -> Vec<String> {
    let Some(mut path) = normalize_rel(path) else { return vec![] };
    for prefix in ["data/scripts/", "data/databundles/scripts/", "data/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            path = format!("scripts/{}", rest);
            break;
        }
    }
    let mut out = vec![path.clone()];
    if let Some(rest) = path.strip_prefix("scripts/") {
        out.push(rest.to_string());
    } else {
        out.push(format!("scripts/{}", path));
    }
    out.dedup();
    out
}

fn source_label(source: &ScriptSource, name: &str) -> String {
    match source {
        ScriptSource::Dir(dir) => dir.join(name).to_string_lossy().to_string(),
        ScriptSource::Zip(zip) => format!("{}::{}", zip.to_string_lossy(), name),
    }
}

fn read_zip_entry(zip: &Path, names: &[String]) -> Result<Option<String>, String> {
    let file = fs::File::open(zip).map_err(|e| format!("{}: {}", zip.to_string_lossy(), e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    for name in names {
        if let Ok(mut entry) = archive.by_name(name) {
            let mut buf = Vec::new();
            let size = entry.size().min(MAX_FILE_SIZE);
            entry
                .by_ref()
                .take(size)
                .read_to_end(&mut buf)
                .map_err(|e| e.to_string())?;
            return Ok(Some(String::from_utf8_lossy(&buf).to_string()));
        }
    }
    // fall back to a suffix search over the archive index
    for i in 0..archive.len() {
        let Ok(entry) = archive.by_index_raw(i) else { continue };
        let entry_name = entry.name().replace('\\', "/");
        let matched = names.iter().any(|n| {
            entry_name == *n || entry_name.ends_with(&format!("/{}", n))
        });
        if matched {
            let name = entry_name.clone();
            drop(entry);
            if let Ok(mut entry) = archive.by_name(&name) {
                let mut buf = Vec::new();
                let size = entry.size().min(MAX_FILE_SIZE);
                entry
                    .by_ref()
                    .take(size)
                    .read_to_end(&mut buf)
                    .map_err(|e| e.to_string())?;
                return Ok(Some(String::from_utf8_lossy(&buf).to_string()));
            }
        }
    }
    Ok(None)
}

fn read_source(source: &ScriptSource, names: &[String]) -> Result<Option<String>, String> {
    match source {
        ScriptSource::Dir(base) => {
            for name in names {
                let path = base.join(name);
                if path.is_file() {
                    return read_text(&path).map(Some);
                }
            }
            Ok(None)
        },
        ScriptSource::Zip(zip) => read_zip_entry(zip, names),
    }
}

/// Build `N: line` output, optionally around a specific line.
pub fn slice_text(text: &str, line: Option<usize>, context: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let (start, end) = match line {
        Some(n) if n > 0 => {
            let n = n.min(total.max(1));
            let ctx = context.clamp(1, 200);
            (n.saturating_sub(ctx).max(1), (n + ctx).min(total))
        },
        _ => (1, total.min(MAX_READ_LINES)),
    };
    if start > end || total == 0 {
        return "（空文件）".to_string();
    }
    let mut out = String::new();
    let mut chars = 0usize;
    for i in start..=end {
        let Some(text) = lines.get(i - 1) else { break };
        let text = clip(text.trim_end(), 500);
        let row = format!("{}: {}\n", i, text);
        chars += row.len();
        if chars > MAX_READ_CHARS {
            out.push_str(&format!("…（已截断，文件共 {} 行）\n", total));
            break;
        }
        out.push_str(&row);
    }
    if line.is_none() && total > MAX_READ_LINES {
        out.push_str(&format!(
            "…（文件共 {} 行，已显示前 {} 行；可用 line 参数查看指定行附近的内容）\n",
            total, MAX_READ_LINES
        ));
    }
    out
}

fn read_text(path: &Path) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_FILE_SIZE {
        return Err(format!("文件过大（{} 字节）", meta.len()));
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn list_source(source: &ScriptSource, prefix: &str) -> Result<Vec<String>, String> {
    let prefix = prefix.trim_matches('/');
    match source {
        ScriptSource::Dir(base) => {
            let target = if prefix.is_empty() {
                base.clone()
            } else {
                base.join(prefix)
            };
            let target = match target.canonicalize() {
                Ok(p) => p,
                Err(_) => return Ok(vec![]),
            };
            let base_canon = base.canonicalize().unwrap_or_else(|_| base.clone());
            if !target.starts_with(&base_canon) {
                return Err("非法目录".to_string());
            }
            let mut entries = vec![];
            for entry in fs::read_dir(&target).map_err(|e| e.to_string())?.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                if entry.path().is_dir() {
                    entries.push(format!("{}/", name));
                } else {
                    entries.push(name);
                }
                if entries.len() > MAX_LIST {
                    break;
                }
            }
            entries.sort();
            Ok(entries)
        },
        ScriptSource::Zip(zip) => {
            let file = fs::File::open(zip).map_err(|e| e.to_string())?;
            let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
            // zip entries are usually named `scripts/xxx`, but be tolerant
            let mut prefixes = vec![if prefix.is_empty() {
                String::new()
            } else {
                format!("{}/", prefix)
            }];
            if !prefix.is_empty() && !prefix.starts_with("scripts/") {
                prefixes.push(format!("scripts/{}/", prefix));
            }
            let mut entries = vec![];
            for search in prefixes {
                for i in 0..archive.len() {
                    let Ok(entry) = archive.by_index_raw(i) else { continue };
                    let name = entry.name().replace('\\', "/");
                    let Some(rest) = name.strip_prefix(&search) else { continue };
                    if rest.is_empty() {
                        continue;
                    }
                    let item = match rest.find('/') {
                        Some(n) => format!("{}/", &rest[..n]),
                        None => rest.to_string(),
                    };
                    if !entries.contains(&item) {
                        entries.push(item);
                    }
                    if entries.len() > MAX_LIST {
                        break;
                    }
                }
                if !entries.is_empty() {
                    break;
                }
            }
            entries.sort();
            Ok(entries)
        },
    }
}

/// Read a whole log file (plain file or an entry inside a cloud save zip).
pub fn read_log_lines(path: &LogPath) -> Result<Vec<String>, String> {
    let reader: LogReader = path.open()?;
    read_all_lines(reader)
}

pub fn read_all_lines(reader: impl Read) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    let mut bytes = 0u64;
    loop {
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        let mut line = String::from_utf8_lossy(&buf).to_string();
        while line.ends_with('\n') || line.ends_with('\r') {
            line.pop();
        }
        out.push(line);
        if out.len() >= MAX_LOG_LINES || bytes >= MAX_LOG_BYTES {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "dst-log-reader-{}-{}",
                name,
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write(&self, rel: &str, content: &str) {
            let path = self.0.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn normalize_rel_blocks_escapes() {
        assert_eq!(normalize_rel("scripts/a.lua").unwrap(), "scripts/a.lua");
        assert_eq!(normalize_rel("./scripts\\a.lua").unwrap(), "scripts/a.lua");
        assert_eq!(normalize_rel("/scripts/a.lua").unwrap(), "scripts/a.lua");
        assert!(normalize_rel("../../etc/passwd").is_none());
        assert!(normalize_rel("C:/Windows/system32").is_none());
    }

    #[test]
    fn script_names_try_both_prefixes() {
        assert_eq!(
            candidate_names("scripts/widgets/wheel.lua"),
            vec![
                "scripts/widgets/wheel.lua".to_string(),
                "widgets/wheel.lua".to_string()
            ]
        );
        assert_eq!(
            candidate_names("data/scripts/widgets/wheel.lua"),
            vec![
                "scripts/widgets/wheel.lua".to_string(),
                "widgets/wheel.lua".to_string()
            ]
        );
        assert_eq!(
            candidate_names("widgets/wheel.lua"),
            vec![
                "widgets/wheel.lua".to_string(),
                "scripts/widgets/wheel.lua".to_string()
            ]
        );
    }

    #[test]
    fn mod_key_from_stacktrace_path() {
        assert_eq!(
            mod_key("../mods/workshop-1234567/scripts/x.lua").unwrap(),
            "workshop-1234567"
        );
        assert_eq!(mod_key("dsa-260628/scripts/x.lua").unwrap(), "dsa-260628");
        assert!(mod_key("   ").is_none());
    }

    #[test]
    fn slice_text_reports_real_line_numbers() {
        let text = (1..=10)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let slice = slice_text(&text, Some(5), 2);
        assert!(slice.contains("3: line 3"));
        assert!(slice.contains("5: line 5"));
        assert!(slice.contains("7: line 7"));
        assert!(!slice.contains("2: line 2"));
    }

    #[test]
    fn reads_game_scripts_and_mods_from_a_fake_install() {
        let tmp = TempDir::new("gamefs");
        // a release-ish layout: <root>/data/scripts/... (extracted bundle)
        tmp.write(
            "data/databundles/scripts/modutil.lua",
            "local function AddClassPostConstruct()\n  ERROR_MARKER\nend\n",
        );
        // local mods live in <root>/mods on every platform
        tmp.write(
            "mods/workshop-1234567/modinfo.lua",
            "name = \"Test Mod\"\n",
        );
        tmp.write(
            "mods/workshop-1234567/scripts/broken.lua",
            "local x = nil\nx.field = 1 -- UNIQUE_MARKER\n",
        );

        let game_fs = GameFs::detect(None, Some(tmp.path().to_str().unwrap()), "dst");
        assert!(game_fs.has_scripts(), "scripts should be detected");

        // read / list game scripts
        let script = game_fs
            .read_script("scripts/modutil.lua", Some(2), 1)
            .unwrap();
        assert!(script.contains("ERROR_MARKER"), "{}", script);
        let listing = game_fs.list_scripts("scripts").unwrap();
        assert!(listing.contains("modutil.lua"), "{}", listing);

        // resolve mods through the moddir found in a stacktrace
        let root = game_fs.find_mod_root("workshop-1234567").unwrap();
        assert!(root.ends_with("workshop-1234567"));
        let modfile = game_fs
            .read_mod_file("workshop-1234567", "scripts/broken.lua", None, 2)
            .unwrap();
        assert!(modfile.contains("UNIQUE_MARKER"), "{}", modfile);
        assert!(game_fs
            .read_mod_file("workshop-1234567", "../../etc/passwd", None, 2)
            .is_err());
        let listed = game_fs.list_mod_files("workshop-1234567", "").unwrap();
        assert!(listed.contains("modinfo.lua"), "{}", listed);
        let hits = game_fs
            .search_mod("workshop-1234567", "unique_marker", 10)
            .unwrap();
        assert!(hits.contains("scripts/broken.lua:2"), "{}", hits);
    }

    /// Manual check against the machine's real installation:
    /// `cargo test --bin log-reader -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn manual_detect_real_install() {
        let docs = home_dir().map(|h| h.join("Documents"));
        let game_fs = GameFs::detect(docs, None, "dst");
        for install in &game_fs.installs {
            println!("install {} at {:?}", install.name, install.root);
            for source in &install.sources {
                println!("  source: {}", source_label(source, ""));
            }
        }
        println!("workshop dirs: {:?}", game_fs.workshop_dirs);
        println!("local mod dirs: {:?}", game_fs.local_mod_dirs);
        if let Some(install) = game_fs.installs.first() {
            if let Some(source) = install.sources.first() {
                let listing = list_source(source, "widgets")
                    .map(|e| e.len())
                    .unwrap_or(0);
                println!("widgets entries: {}", listing);
            }
            let read = game_fs.read_script("scripts/modutil.lua", Some(162), 3);
            println!("read modutil.lua: {:?}", read.map(|s| s.lines().count()));
        }
        // resolve and read a real mod, the way the tools do it
        let mut checked = 0;
        for dir in &game_fs.local_mod_dirs {
            let Ok(rd) = fs::read_dir(dir) else { continue };
            for entry in rd.flatten() {
                if !entry.path().is_dir() || checked >= 2 {
                    continue;
                }
                let moddir = entry.file_name().to_string_lossy().to_string();
                let read = game_fs.read_mod_file(&moddir, "modinfo.lua", Some(1), 5);
                println!("mod {} -> {:?}", moddir, read.map(|s| s.lines().count()));
                checked += 1;
            }
        }
        if let Some(install) = game_fs.installs.first() {
            if let Some(source) = install.sources.iter().find(|s| matches!(s, ScriptSource::Dir(_))) {
                println!("dir source entries: {}", list_source(source, "").unwrap().len());
            }
        }
    }
}
