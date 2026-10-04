//! Tools handed to the model: read more of the log, read game scripts and
//! read mod sources. Results are plain text and always bounded in size.

use serde_json::{json, Value};

use super::gamefs::{self, GameFs};
use super::llm::ToolDef;

/// Short description of a loaded mod, used in the prompt and in results.
#[derive(Debug, Clone)]
pub struct ModBrief {
    pub moddir: String,
    pub name: String,
    pub version: Option<String>,
    pub workshop_id: Option<String>,
}

pub struct ToolCtx<'a> {
    pub lines: &'a [String],
    pub gamefs: &'a GameFs,
    pub mods: &'a [ModBrief],
}

pub const SUBMIT_TOOL: &str = "submit_analysis";

pub fn tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "read_log_lines".into(),
            description:
                "读取当前日志中指定行号的原文（1 起始，含首尾）。用于查看报错片段前后的完整上下文。\
                 每次最多 200 行。"
                    .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "start": {"type": "integer", "description": "起始行号（从 1 开始）"},
                    "end": {"type": "integer", "description": "结束行号（含）"}
                },
                "required": ["start", "end"]
            }),
        },
        ToolDef {
            name: "search_log".into(),
            description: "在整份日志里按关键字搜索（不区分大小写），返回匹配行及其行号。\
                 可用于查找某个模组、某个函数或某个文件在日志中出现的所有位置。"
                .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "keyword": {"type": "string", "description": "要搜索的关键字，例如 'workshop-123456'"},
                    "context": {"type": "integer", "description": "每条匹配前后附带的行数，默认 2"},
                    "max_results": {"type": "integer", "description": "最多返回多少条匹配，默认 15"}
                },
                "required": ["keyword"]
            }),
        },
        ToolDef {
            name: "read_game_file".into(),
            description:
                "读取游戏自带脚本（只读）。path 用报错堆栈里的路径，例如 'scripts/widgets/craftslot.lua'。\
                 可用 line 指定行号，只返回该行附近的内容，便于定位。"
                    .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "脚本路径，例如 scripts/modutil.lua"},
                    "line": {"type": "integer", "description": "可选，想查看的行号"},
                    "context": {"type": "integer", "description": "可选，line 上下各显示多少行，默认 25"}
                },
                "required": ["path"]
            }),
        },
        ToolDef {
            name: "list_game_files".into(),
            description: "列出游戏脚本目录下的文件，用于确认路径是否正确。例如 dir='scripts/widgets'。"
                .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "dir": {"type": "string", "description": "相对 scripts 的目录，留空表示根目录"}
                }
            }),
        },
        ToolDef {
            name: "read_mod_file".into(),
            description:
                "读取模组文件（只读）。mod 传日志里的模组目录名，例如 'workshop-1234567' 或 'lw-richtext'；\
                 path 相对于模组根目录，例如 'modmain.lua'。可用 line 指定行号。"
                    .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "mod": {"type": "string", "description": "模组目录名（moddir），例如 workshop-1234567"},
                    "path": {"type": "string", "description": "模组内相对路径，例如 scripts/widgets/foo.lua"},
                    "line": {"type": "integer", "description": "可选，想查看的行号"},
                    "context": {"type": "integer", "description": "可选，line 上下各显示多少行，默认 25"}
                },
                "required": ["mod", "path"]
            }),
        },
        ToolDef {
            name: "list_mod_files".into(),
            description: "列出某个模组目录下的文件与子目录（已过滤图片/音频等资源）。"
                .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "mod": {"type": "string", "description": "模组目录名（moddir）"},
                    "dir": {"type": "string", "description": "可选，模组内的子目录"}
                },
                "required": ["mod"]
            }),
        },
        ToolDef {
            name: "search_mod_code".into(),
            description: "在某个模组的 Lua 源码中按关键字搜索（不区分大小写），返回 文件:行号: 内容。\
                 当堆栈里只有文件名或函数名时非常有用。"
                .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "mod": {"type": "string", "description": "模组目录名（moddir）"},
                    "keyword": {"type": "string", "description": "要搜索的关键字，例如函数名或变量名"},
                    "max_results": {"type": "integer", "description": "最多返回多少条，默认 30"}
                },
                "required": ["mod", "keyword"]
            }),
        },
        ToolDef {
            name: SUBMIT_TOOL.into(),
            description:
                "分析完成后提交最终结论。必须在结束前调用一次，不要在普通回复里输出结论。"
                    .into(),
            schema: json!({
                "type": "object",
                "properties": {
                    "summary": {"type": "string", "description": "一句话概括报错原因"},
                    "causes": {
                        "type": "array",
                        "description": "可能的原因，按可能性从高到低排序",
                        "items": {
                            "type": "object",
                            "properties": {
                                "title": {"type": "string", "description": "原因简述"},
                                "detail": {"type": "string", "description": "依据与解释，可引用日志行号或代码"},
                                "mods": {
                                    "type": "array",
                                    "description": "相关的模组 moddir 或名称",
                                    "items": {"type": "string"}
                                }
                            },
                            "required": ["title", "detail"]
                        }
                    },
                    "mods": {
                        "type": "array",
                        "description": "怀疑涉及的模组",
                        "items": {
                            "type": "object",
                            "properties": {
                                "moddir": {"type": "string", "description": "模组目录名，例如 workshop-1234567"},
                                "name": {"type": "string", "description": "模组名称"},
                                "reason": {"type": "string", "description": "为什么怀疑它"}
                            },
                            "required": ["name", "reason"]
                        }
                    },
                    "suggestions": {
                        "type": "array",
                        "description": "给玩家的处理建议",
                        "items": {"type": "string"}
                    }
                },
                "required": ["summary", "causes"]
            }),
        },
    ]
}

fn s(args: &Value, key: &str) -> String {
    args[key].as_str().unwrap_or_default().trim().to_string()
}

fn n(args: &Value, key: &str) -> Option<usize> {
    match &args[key] {
        Value::Number(num) => num.as_u64().map(|v| v as usize),
        Value::String(text) => text.trim().parse::<usize>().ok(),
        _ => None,
    }
}

fn n_or(args: &Value, keys: &[&str], default: usize) -> usize {
    for key in keys {
        if let Some(v) = n(args, key) {
            return v;
        }
    }
    default
}

fn render(lines: &[String], start: usize, end: usize) -> String {
    let mut out = String::new();
    let mut chars = 0usize;
    for i in start..=end {
        let Some(line) = lines.get(i - 1) else { break };
        let line = gamefs::clip(line.trim_end(), 600);
        let row = format!("{}: {}\n", i, line);
        chars += row.len();
        if chars > gamefs::MAX_READ_CHARS {
            out.push_str("…（输出已截断）\n");
            break;
        }
        out.push_str(&row);
    }
    if out.is_empty() {
        out.push_str("（没有内容）");
    }
    out
}

fn read_log_lines(ctx: &ToolCtx, args: &Value) -> String {
    let total = ctx.lines.len();
    if total == 0 {
        return "日志为空".to_string();
    }
    let start = n_or(args, &["start", "start_line", "from"], 1).max(1);
    let end = n_or(args, &["end", "end_line", "to"], start + 60);
    let end = end.max(start).min(start + 199);
    if start > total {
        return format!("起始行号 {} 超出日志范围（共 {} 行）", start, total);
    }
    let end = end.min(total);
    render(ctx.lines, start, end)
}

fn search_log(ctx: &ToolCtx, args: &Value) -> String {
    let keyword = s(args, "keyword");
    if keyword.is_empty() {
        return "错误: keyword 不能为空".to_string();
    }
    let context = n_or(args, &["context"], 2).min(20);
    let max = n_or(args, &["max_results", "max"], 15).clamp(1, 40);
    let needle = keyword.to_lowercase();
    let mut blocks: Vec<String> = vec![];
    let mut covered_to = 0usize;
    let mut chars = 0usize;
    for (i, line) in ctx.lines.iter().enumerate() {
        if !line.to_lowercase().contains(&needle) {
            continue;
        }
        let start = (i + 1).saturating_sub(context).max(covered_to + 1).max(1);
        let end = (i + 1 + context).min(ctx.lines.len());
        if end < start {
            continue;
        }
        let block = render(ctx.lines, start, end);
        chars += block.len();
        blocks.push(format!("--- 匹配行 {} ---\n{}", i + 1, block));
        covered_to = end;
        if blocks.len() >= max || chars > gamefs::MAX_READ_CHARS {
            break;
        }
    }
    if blocks.is_empty() {
        format!("日志中没有找到 `{}`", keyword)
    } else {
        format!("找到以下匹配：\n{}", blocks.join("\n"))
    }
}

fn read_game_file(ctx: &ToolCtx, args: &Value) -> String {
    let path = s(args, "path");
    if path.is_empty() {
        return "错误: path 不能为空".to_string();
    }
    let line = n(args, "line");
    let context = n_or(args, &["context"], 25);
    match ctx.gamefs.read_script(&path, line, context) {
        Ok(text) => text,
        Err(e) => format!("错误: {}", e),
    }
}

fn list_game_files(ctx: &ToolCtx, args: &Value) -> String {
    let dir = s(args, "dir");
    match ctx.gamefs.list_scripts(&dir) {
        Ok(text) => text,
        Err(e) => format!("错误: {}", e),
    }
}

fn read_mod_file(ctx: &ToolCtx, args: &Value) -> Result<String, String> {
    let moddir = s(args, "mod");
    let path = s(args, "path");
    if moddir.is_empty() || path.is_empty() {
        return Err("mod 与 path 都必须提供".to_string());
    }
    let line = n(args, "line");
    let context = n_or(args, &["context"], 25);
    ctx.gamefs.read_mod_file(&moddir, &path, line, context)
}

fn list_mod_files(ctx: &ToolCtx, args: &Value) -> Result<String, String> {
    let moddir = s(args, "mod");
    if moddir.is_empty() {
        return Err("mod 不能为空".to_string());
    }
    let dir = s(args, "dir");
    ctx.gamefs.list_mod_files(&moddir, &dir)
}

fn search_mod_code(ctx: &ToolCtx, args: &Value) -> Result<String, String> {
    let moddir = s(args, "mod");
    let keyword = s(args, "keyword");
    if moddir.is_empty() || keyword.is_empty() {
        return Err("mod 与 keyword 都必须提供".to_string());
    }
    let max = n_or(args, &["max_results", "max"], 30).clamp(1, 60);
    ctx.gamefs.search_mod(&moddir, &keyword, max)
}

/// Known mods, as a hint for the model when a mod can not be resolved.
pub fn mod_hint(ctx: &ToolCtx) -> String {
    if ctx.mods.is_empty() {
        return String::new();
    }
    let list = ctx
        .mods
        .iter()
        .take(40)
        .map(|m| {
            let mut item = m.moddir.clone();
            if !m.name.is_empty() {
                item.push_str(&format!(" ({})", m.name));
            }
            if let Some(id) = &m.workshop_id {
                item.push_str(&format!(" [id:{}]", id));
            }
            item
        })
        .collect::<Vec<_>>()
        .join(" | ");
    format!("\n日志中已加载的模组: {}", list)
}

/// Append the loaded-mod list to a tool error, so the model can retry with a
/// valid moddir instead of guessing.
fn with_mod_hint(ctx: &ToolCtx, message: String) -> String {
    format!("错误: {}{}", message, mod_hint(ctx))
}

pub fn execute(ctx: &ToolCtx, name: &str, args: &Value) -> String {
    let result = match name {
        "read_log_lines" => read_log_lines(ctx, args),
        "search_log" => search_log(ctx, args),
        "read_game_file" => read_game_file(ctx, args),
        "list_game_files" => list_game_files(ctx, args),
        "read_mod_file" => match read_mod_file(ctx, args) {
            Ok(text) => text,
            Err(e) => with_mod_hint(ctx, e),
        },
        "list_mod_files" => match list_mod_files(ctx, args) {
            Ok(text) => text,
            Err(e) => with_mod_hint(ctx, e),
        },
        "search_mod_code" => match search_mod_code(ctx, args) {
            Ok(text) => text,
            Err(e) => with_mod_hint(ctx, e),
        },
        other => format!("未知工具: {}", other),
    };
    // hard cap, keeps the context (and the bill) predictable
    let limit = 3 * gamefs::MAX_READ_CHARS;
    if result.chars().count() > limit {
        let mut result = gamefs::clip(&result, limit);
        result.push_str("\n…（输出过长已截断）");
        return result;
    }
    result
}
