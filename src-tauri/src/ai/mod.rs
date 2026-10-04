//! Lightweight AI log analysis.
//!
//! The whole log is never sent to the model. Only the parts around an error
//! marker are extracted (with their real line numbers). The model can then
//! pull whatever else it needs — more log lines, game scripts, mod sources —
//! through tools. The result is a small structured object so the UI can stay
//! clean and never shows tool calls or chain of thought.

mod gamefs;
mod llm;
mod tools;

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::ds_log::{LogModelState, LogPath};

use self::gamefs::GameFs;
use self::llm::{Call, Msg, ToolDef};
use self::tools::{ModBrief, ToolCtx};

pub const AI_EVENT: &str = "ai-analysis";

/// Round trips the model may spend exploring with tools.
const MAX_TOOL_STEPS: usize = 16;
/// Extra round trips granted after we ask it to wrap up.
const WRAP_UP_STEPS: usize = 3;
/// Hard limit of provider round trips.
const MAX_STEPS: usize = MAX_TOOL_STEPS + WRAP_UP_STEPS;
/// Char budget of the log excerpt sent to the model.
const EXCERPT_BUDGET: usize = 22_000;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfig {
    /// deepseek | claude | openai | custom
    pub provider: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    /// Manually configured game installation folder.
    #[serde(default)]
    pub game_dir: Option<String>,
    /// UI language, used for the answer.
    #[serde(default)]
    pub lang: Option<String>,
}

/// Analyze the log shown in `label`, emitting progress/result events to it.
#[tauri::command(rename_all = "camelCase")]
pub async fn ai_analyze(
    app: AppHandle,
    label: String,
    request_id: String,
    config: AiConfig,
) -> Result<(), String> {
    if config.api_key.trim().is_empty() {
        return Err("请先配置 API Key".to_string());
    }
    if config.model.trim().is_empty() {
        return Err("请先配置模型名称".to_string());
    }
    let log_path = app
        .state::<LogModelState>()
        .get_log_path(&label)
        .ok_or_else(|| "找不到该日志".to_string())?;
    let mods: Vec<ModBrief> = app
        .state::<LogModelState>()
        .get_mods(&label)
        .into_iter()
        .map(|m| ModBrief {
            moddir: m.moddir,
            name: m.name,
            version: m.version,
            workshop_id: m.workshop_id,
        })
        .collect();
    let documents_dir = app.path().document_dir().ok();

    std::thread::spawn(move || {
        let reporter = Reporter {
            app: &app,
            label: &label,
            request_id: &request_id,
            last_chunk: std::cell::Cell::new(None),
            usage: std::cell::RefCell::new(None),
        };
        reporter.send(json!({"state": "start"}));
        match analyze(
            &log_path,
            &config,
            mods,
            documents_dir,
            &reporter,
        ) {
            Ok(result) => reporter.send(json!({
                "state": "done",
                "result": result,
                "usage": reporter.usage_json(),
            })),
            Err(message) => reporter.send(json!({"state": "error", "message": message})),
        }
    });
    Ok(())
}

/// Progress sink, so the agent loop does not depend on Tauri.
trait Progress {
    fn status(&self, text: &str);
    /// tail of the text the model is currently streaming
    fn delta(&self, _tail: &str) {}
    /// the model is thinking, nothing to display yet
    fn thinking(&self) {}
    /// token accounting, emitted after every turn
    fn usage(&self, _usage: &llm::Usage, _model: &str) {}
}

struct Reporter<'a> {
    app: &'a AppHandle,
    label: &'a str,
    request_id: &'a str,
    last_chunk: std::cell::Cell<Option<std::time::Instant>>,
    usage: std::cell::RefCell<Option<Value>>,
}

impl Reporter<'_> {
    /// Token accounting of the last turn, if any.
    fn usage_json(&self) -> Value {
        self.usage.borrow().clone().unwrap_or(Value::Null)
    }

    fn send(&self, payload: Value) {
        let mut payload = payload;
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("id".to_string(), Value::String(self.request_id.to_string()));
        }
        let _ = self.app.emit_to(self.label, AI_EVENT, payload.to_string());
    }
}

/// Last `max` characters of a string, cut on a char boundary.
fn tail_of(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    text.chars().skip(count - max).collect()
}

impl Progress for Reporter<'_> {
    fn status(&self, text: &str) {
        self.send(json!({"state": "progress", "text": text}));
    }

    fn delta(&self, tail: &str) {
        // throttle: at most ~10 updates per second
        let now = std::time::Instant::now();
        if let Some(last) = self.last_chunk.get() {
            if now.duration_since(last).as_millis() < 100 {
                return;
            }
        }
        self.last_chunk.set(Some(now));
        self.send(json!({"state": "chunk", "text": tail}));
    }

    fn thinking(&self) {
        self.send(json!({"state": "thinking"}));
    }

    fn usage(&self, usage: &llm::Usage, model: &str) {
        let json = usage.to_json(model);
        *self.usage.borrow_mut() = Some(json.clone());
        self.send(json!({"state": "usage", "usage": json}));
    }
}

fn analyze(
    log_path: &LogPath,
    config: &AiConfig,
    mods: Vec<ModBrief>,
    documents_dir: Option<std::path::PathBuf>,
    progress: &dyn Progress,
) -> Result<Value, String> {
    let lines = gamefs::read_log_lines(log_path)?;
    if lines.is_empty() {
        return Err("日志内容为空".to_string());
    }
    let (excerpt, found) = build_excerpt(&lines);
    let game_type = log_path.get_game_type();
    let game_fs = GameFs::detect(documents_dir, config.game_dir.as_deref(), &game_type);

    let ctx = ToolCtx {
        lines: &lines,
        gamefs: &game_fs,
        mods: &mods,
    };
    let tool_defs = tools::tool_defs();
    let system = system_prompt(config);
    let user = user_prompt(log_path, &mods, &game_fs, &excerpt, found, &lines);

    let provider = llm::build_provider(config)?;
    let mut history: Vec<Msg> = vec![Msg::User(user)];
    let mut nudged = false;
    let mut used_tools = false;
    // how many times we asked the model to fix a broken `submit_analysis`
    let mut repairs = 0u8;

    let mut total_usage = llm::Usage::default();
    let mut streamed = String::new();
    // identical calls are answered from here instead of paying for them twice
    let mut tool_cache: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut wrapping_up = false;

    for step in 0..MAX_STEPS {
        if step >= MAX_TOOL_STEPS && !wrapping_up {
            wrapping_up = true;
            progress.status("正在整理结论…");
        }
        let turn = {
            // `streamed` accumulates the visible answer of the current turn
            let mut on_stream = |event: llm::Stream| match event {
                llm::Stream::Delta(part) => {
                    streamed.push_str(&part);
                    let tail = tail_of(&streamed, 600);
                    progress.delta(&tail);
                },
                llm::Stream::Thinking => progress.thinking(),
            };
            provider.chat(&system, &history, &tool_defs, &mut on_stream)?
        };
        total_usage.merge(&turn.usage);
        progress.usage(&total_usage, config.model.trim());
        let reply = turn.reply;

        streamed.clear();

        if let Some(call) = reply.calls.iter().find(|c| c.name == tools::SUBMIT_TOOL) {
            eprintln!(
                "[ai] submit_analysis arguments: {}",
                gamefs::clip(&call.args.to_string(), 600)
            );
            // a truncated / malformed tool call must not end as an empty panel
            if let Some(error) = &call.args_error {
                if repairs < 2 {
                    repairs += 1;
                    eprintln!("[ai] asking the model to resubmit: {}", error);
                    history.push(Msg::Assistant {
                        text: reply.text.clone(),
                        reasoning: reply.reasoning.clone(),
                        calls: reply.calls.clone(),
                    });
                    let message = format!(
                        "错误: submit_analysis 的参数不是合法 JSON（{}）。请重新调用 submit_analysis，并确保字符串里的换行写成 \\n、引号正确转义。",
                        error
                    );
                    let results = results_around_submit(&ctx, &reply.calls, &call.id, &message);
                    history.push(Msg::Results(results));
                    continue;
                }
                return Err(format!(
                    "模型返回的工具参数无法解析（{}）。请重试或更换模型。",
                    error
                ));
            }
            let result = normalize_result(&call.args);
            if is_empty_result(&result) {
                let text = reply.text.clone().unwrap_or_default();
                if !text.trim().is_empty() {
                    // the model wrote the answer in text form, keep it
                    return Ok(fallback_result(&text));
                }
                if repairs < 1 {
                    repairs += 1;
                    eprintln!("[ai] submitting an empty analysis, asking again");
                    history.push(Msg::Assistant {
                        text: reply.text.clone(),
                        reasoning: reply.reasoning.clone(),
                        calls: reply.calls.clone(),
                    });
                    let results = results_around_submit(
                        &ctx,
                        &reply.calls,
                        &call.id,
                        "错误: submit_analysis 的参数为空。请重新调用并填写 summary 与 causes。",
                    );
                    history.push(Msg::Results(results));
                    continue;
                }
                return Err(format!(
                    "模型没有返回有效的分析结论，请重试或更换模型。（原始参数: {}）",
                    gamefs::clip(&call.args.to_string(), 300)
                ));
            }
            return Ok(result);
        }

        if reply.calls.is_empty() {
            let text = reply.text.clone().unwrap_or_default();
            if text.trim().is_empty() {
                return Err("模型没有返回内容，请检查模型名称是否支持工具调用".to_string());
            }
            if !nudged {
                // one more chance to produce the structured result
                nudged = true;
                progress.status("正在整理结论…");
                history.push(Msg::Assistant {
                    text: Some(text),
                    reasoning: reply.reasoning.clone(),
                    calls: vec![],
                });
                history.push(Msg::User(
                    "请调用 submit_analysis 工具提交结论（summary / causes / mods / suggestions）。"
                        .to_string(),
                ));
                continue;
            }
            return Ok(fallback_result(&text));
        }

        if !used_tools {
            used_tools = true;
            progress.status("正在查阅相关代码…");
        }
        history.push(Msg::Assistant {
            text: reply.text.clone(),
            reasoning: reply.reasoning.clone(),
            calls: reply.calls.clone(),
        });
        let mut results = vec![];
        for call in &reply.calls {
            eprintln!(
                "[ai] tool call: {} {}",
                call.name,
                gamefs::clip(&call.args.to_string(), 200)
            );
            let key = format!("{}\n{}", call.name, call.args);
            let output = if wrapping_up {
                // out of budget: no more exploring, ask for the conclusion
                "错误: 分析步数已达上限，请立即调用 submit_analysis 提交目前已有的结论，不要再调用其它工具。"
                    .to_string()
            } else if let Some(cached) = tool_cache.get(&key) {
                format!(
                    "（这次调用与之前完全相同，直接复用上次结果；请勿重复调用相同参数）\n{}",
                    cached
                )
            } else {
                let output = tools::execute(&ctx, &call.name, &call.args);
                tool_cache.insert(key, output.clone());
                output
            };
            results.push((call.id.clone(), output));
        }
        history.push(Msg::Results(results));
    }

    // still no structured answer: keep whatever the model wrote
    if !streamed.trim().is_empty() {
        eprintln!("[ai] step budget exhausted, falling back to the streamed text");
        return Ok(fallback_result(&streamed));
    }
    Err(format!(
        "模型在 {} 步内一直没有给出结论（可能反复读取文件），已中止。请重试，或换用更果断的模型。",
        MAX_STEPS
    ))
}

/// Every tool call of a round must be answered, otherwise providers reject
/// the next request. Used when we ask the model to redo `submit_analysis`.
fn results_around_submit(
    ctx: &ToolCtx,
    calls: &[Call],
    submit_id: &str,
    message: &str,
) -> Vec<(String, String)> {
    calls
        .iter()
        .map(|call| {
            if call.id == submit_id {
                (call.id.clone(), message.to_string())
            } else {
                (call.id.clone(), tools::execute(ctx, &call.name, &call.args))
            }
        })
        .collect()
}

fn system_prompt(config: &AiConfig) -> String {
    let lang = config.lang.clone().unwrap_or_else(|| "zh".to_string());
    let lang_line = if lang.starts_with("zh") {
        "使用简体中文回答，专有名词（函数名、文件名）保持原文。"
    } else {
        "Answer in English, keep identifiers (function names, file paths) as-is."
    };
    format!(
        r#"你是《饥荒联机版》(Don't Starve Together) 与《饥荒》(Don't Starve) 的日志诊断专家，擅长结合日志堆栈、游戏源码与模组源码定位崩溃原因。

工作方式：
- 用户只给了日志中的报错片段（带真实行号）和模组列表，不要假设日志其余内容；需要时用工具读取。
- 工具：read_log_lines / search_log 读取日志其余部分；read_game_file / list_game_files 读取游戏自带脚本；read_mod_file / list_mod_files / search_mod_code 读取模组源码。
- 堆栈中 `scripts/xxx.lua` 是游戏脚本；`../mods/<moddir>/xxx.lua` 是模组脚本，<moddir> 形如 workshop-1234567 或本地模组目录名。
- 结论必须有证据：引用具体的日志行号或代码内容，不要编造不存在的文件、函数或变量。
- 证据不足时明确说明不确定，并给出最可能的方向，不要强行下结论。
- 不要输出工具调用过程、思考过程或客套话。
- 高效使用工具：优先读取堆栈里直接出现的文件，不要反复列出目录、也不要重复读取相同内容；证据足够时立刻调用 submit_analysis 提交结论，不要追求穷尽所有细节。
- 调用工具时参数必须是合法 JSON：字符串内部不要直接换行（写成 \\n），不要输出多余的解释文字。
- 分析结束前必须调用 submit_analysis 提交结论：summary 一句话说明报错原因；causes 按可能性从高到低排列，说明依据；mods 只列出有依据怀疑的模组（给出 moddir）；suggestions 给出可操作的排查/解决办法。

{lang_line}
"#
    )
}

/// Version / build / mode lines from the log header, they help the model
/// reason about version specific bugs.
fn head_info(lines: &[String]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for line in lines.iter().take(400) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("Don't Starve") || line.starts_with("Mode:") {
            if !out.iter().any(|item| item == line) {
                out.push(gamefs::clip(line, 200));
            }
        }
        if out.len() >= 3 {
            break;
        }
    }
    out
}

fn user_prompt(
    log_path: &LogPath,
    mods: &[ModBrief],
    game_fs: &GameFs,
    excerpt: &str,
    found: bool,
    lines: &[String],
) -> String {
    let total_lines = lines.len();
    let game_name = match log_path.get_game_type().as_str() {
        "ds" => "饥荒 (Don't Starve)",
        _ => "饥荒联机版 (Don't Starve Together)",
    };
    let mut out = String::new();
    out.push_str("## 日志信息\n");
    out.push_str(&format!("- 文件名: {}\n", log_path.get_name()));
    out.push_str(&format!("- 游戏: {}\n", game_name));
    out.push_str(&format!("- 路径: {}\n", log_path.get_path().to_string_lossy()));
    out.push_str(&format!("- 总行数: {}\n", total_lines));
    for info in head_info(lines) {
        out.push_str(&format!("- {}", info));
        out.push('\n');
    }
    if game_fs.has_scripts() {
        let roots = game_fs
            .installs
            .iter()
            .map(|i| i.root.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("- 游戏安装目录: {}\n", roots));
    } else {
        out.push_str("- 游戏安装目录: 未检测到（read_game_file 可能不可用）\n");
    }
    out.push_str("\n## 已加载的模组（moddir / 名称 / 版本）\n");
    if mods.is_empty() {
        out.push_str("（未解析到模组，可能未安装模组或日志未包含模组信息）\n");
    } else {
        for (i, m) in mods.iter().take(80).enumerate() {
            let version = m
                .version
                .as_deref()
                .map(|v| format!(" v{}", v))
                .unwrap_or_default();
            out.push_str(&format!(
                "{}. {}{}{}\n",
                i + 1,
                m.moddir,
                if m.name.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", m.name)
                },
                version
            ));
        }
    }
    out.push_str("\n## 日志报错片段（前缀数字为日志真实行号）\n");
    if !found {
        out.push_str("（没有匹配到明显的报错标记，下面是日志末尾的内容）\n");
    }
    out.push_str(excerpt);
    out.push_str("\n请定位报错原因，并判断是否由模组引起。需要更多信息时请使用工具。\n");
    out
}

/// Error markers, ordered by importance: `(regex, lines before, lines after)`.
static ANCHORS: Lazy<Vec<(Regex, usize, usize)>> = Lazy::new(|| {
    vec![
        (
            Regex::new(
                r"(?i)lua error|assertion failed|segmentation fault|fatal error|\bfatal\b|out of memory|stack traceback",
            )
            .unwrap(),
            3,
            45,
        ),
        (
            Regex::new(
                r#"(?i)\[string "|attempt to (index|call|perform|concatenate|compare|arithmetic|get length|upvalue)|error loading|no field package\.preload|invalid (argument|option|value)|cannot open file|could not (open|load|find)"#,
            )
            .unwrap(),
            2,
            18,
        ),
        (
            Regex::new(r"(?i)(:\s*error:|^\s*error:|\[error\]|failed to|unable to)").unwrap(),
            2,
            10,
        ),
        (Regex::new(r"(?i)(warning:|warn:)").unwrap(), 1, 1),
    ]
});

/// Extract the interesting part of a log, with real line numbers.
/// Returns the rendered excerpt and whether an error marker was found.
fn build_excerpt(lines: &[String]) -> (String, bool) {
    let mut ranges: Vec<(usize, usize)> = vec![];
    let mut found = false;
    for (re, before, after) in ANCHORS.iter() {
        if ranges.len() >= 14 {
            break;
        }
        let mut taken = 0;
        for (i, line) in lines.iter().enumerate() {
            if line.len() > 2000 || !re.is_match(line) {
                continue;
            }
            if ranges.iter().any(|(s, e)| i >= *s && i <= *e) {
                continue;
            }
            let start = i.saturating_sub(*before);
            let end = (i + *after).min(lines.len().saturating_sub(1));
            ranges.push((start, end));
            found = true;
            taken += 1;
            if taken >= 5 {
                break;
            }
        }
    }
    ranges.sort();
    let mut merged: Vec<(usize, usize)> = vec![];
    for (start, end) in ranges {
        match merged.last_mut() {
            Some((_, last_end)) if start <= *last_end + 1 => *last_end = (*last_end).max(end),
            _ => merged.push((start, end)),
        }
    }

    let mut out = String::new();
    let mut chars = 0usize;
    if found {
        for (start, end) in &merged {
            let mut block = format!("[行 {}-{}]\n", start + 1, end + 1);
            for i in *start..=*end {
                let Some(line) = lines.get(i) else { break };
                block.push_str(&format!("{}: {}\n", i + 1, gamefs::clip(line.trim_end(), 600)));
            }
            chars += block.len();
            out.push_str(&block);
            if chars >= EXCERPT_BUDGET {
                out.push_str("…（报错片段过多，已截断）\n");
                break;
            }
        }
    } else {
        // nothing matched: hand over the tail of the log
        let start = lines.len().saturating_sub(80);
        for i in start..lines.len() {
            out.push_str(&format!("{}: {}\n", i + 1, gamefs::clip(lines[i].trim_end(), 600)));
        }
    }
    (out, found)
}

fn clip_str(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

/// Render any JSON value as displayable text.
fn as_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Bool(_) | Value::Number(_) => value.to_string(),
        Value::Array(items) => items
            .iter()
            .map(as_text)
            .filter(|text| !text.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Object(_) => {
            serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
        },
    }
}

/// Key comparison that ignores case, underscores and dashes, so `relatedMods`,
/// `related_mods` and `RelatedMods` all match.
fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

/// First non empty value among several possible key names.
fn pick<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    let object = value.as_object()?;
    for key in keys {
        let wanted = normalize_key(key);
        for (actual, found) in object {
            if normalize_key(actual) != wanted || found.is_null() {
                continue;
            }
            let empty = found
                .as_str()
                .map(|text| text.trim().is_empty())
                .unwrap_or(false);
            if !empty {
                return Some(found);
            }
        }
    }
    None
}

fn text_at(value: &Value, keys: &[&str]) -> String {
    pick(value, keys).map(as_text).unwrap_or_default()
}

const KEY_SUMMARY: [&str; 4] = ["summary", "总结", "overview", "conclusion"];
const KEY_CAUSES: [&str; 5] = ["causes", "reasons", "possible_causes", "原因", "分析"];
const KEY_MODS: [&str; 4] = ["mods", "related_mods", "mod_list", "模组"];
const KEY_SUGGESTIONS: [&str; 5] = ["suggestions", "advice", "recommendations", "建议", "解决方案"];

/// Models sometimes wrap the payload, eg `{"analysis": {...}}` or a JSON
/// string inside a field. Unwrap until real fields show up.
fn unwrap_args(args: &Value) -> Value {
    let mut current = args.clone();
    for _ in 0..3 {
        let Some(object) = current.as_object() else { break };
        let known = object.keys().any(|key| {
            let key = normalize_key(key);
            KEY_SUMMARY.iter().chain(KEY_CAUSES.iter()).chain(KEY_MODS.iter()).chain(KEY_SUGGESTIONS.iter())
                .any(|known| normalize_key(known) == key)
        });
        if known || object.len() != 1 {
            break;
        }
        let Some(inner) = object.values().next() else { break };
        match inner {
            Value::Object(_) => current = inner.clone(),
            Value::String(text) => match serde_json::from_str::<Value>(text) {
                Ok(value) if value.is_object() => current = value,
                _ => break,
            },
            _ => break,
        }
    }
    current
}

/// Keep the structured result small and predictable.
fn normalize_result(args: &Value) -> Value {
    let args = unwrap_args(args);
    let causes = pick(&args, &KEY_CAUSES)
        .and_then(|value| value.as_array().cloned())
        .map(|list| {
            list.iter()
                .take(8)
                .map(|cause| match cause {
                    // a plain string is still a usable cause
                    Value::String(text) => json!({
                        "title": clip_str(text, 200),
                        "detail": "",
                        "mods": Vec::<String>::new(),
                    }),
                    other => {
                        let title = text_at(other, &["title", "name", "cause", "原因", "简述"]);
                        let detail = text_at(
                            other,
                            &["detail", "details", "description", "explanation", "说明", "依据"],
                        );
                        let mods = pick(other, &["mods", "related_mods", "模组"])
                            .map(|value| match value {
                                Value::Array(items) => items
                                    .iter()
                                    .map(as_text)
                                    .filter(|text| !text.trim().is_empty())
                                    .take(6)
                                    .map(|text| clip_str(&text, 80))
                                    .collect::<Vec<_>>(),
                                other => vec![clip_str(&as_text(other), 80)],
                            })
                            .unwrap_or_default();
                        json!({
                            "title": clip_str(&title, 200),
                            "detail": clip_str(&detail, 2000),
                            "mods": mods,
                        })
                    },
                })
                .filter(|cause| {
                    !cause["title"].as_str().unwrap_or("").is_empty()
                        || !cause["detail"].as_str().unwrap_or("").is_empty()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mods = pick(&args, &KEY_MODS)
        .and_then(|value| value.as_array().cloned())
        .map(|list| {
            list.iter()
                .take(8)
                .map(|item| match item {
                    Value::String(text) => json!({
                        "moddir": clip_str(text, 80),
                        "name": clip_str(text, 120),
                        "reason": "",
                    }),
                    other => json!({
                        "moddir": clip_str(
                            &text_at(other, &["moddir", "mod_dir", "dir", "directory", "id"]),
                            80,
                        ),
                        "name": clip_str(
                            &text_at(other, &["name", "mod", "title", "名称", "moddir"]),
                            120,
                        ),
                        "reason": clip_str(
                            &text_at(other, &["reason", "why", "detail", "说明", "原因"]),
                            800,
                        ),
                    }),
                })
                .filter(|item| {
                    !item["name"].as_str().unwrap_or("").is_empty()
                        || !item["moddir"].as_str().unwrap_or("").is_empty()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let suggestions = pick(&args, &KEY_SUGGESTIONS)
        .map(|value| match value {
            Value::Array(items) => items
                .iter()
                .map(as_text)
                .filter(|text| !text.trim().is_empty())
                .take(8)
                .map(|text| clip_str(&text, 600))
                .collect::<Vec<_>>(),
            other => vec![clip_str(&as_text(other), 600)]
                .into_iter()
                .filter(|text| !text.trim().is_empty())
                .collect(),
        })
        .unwrap_or_default();
    json!({
        "summary": clip_str(&text_at(&args, &KEY_SUMMARY), 600),
        "causes": causes,
        "mods": mods,
        "suggestions": suggestions,
        "raw": false,
    })
}

fn is_empty_result(result: &Value) -> bool {
    result["summary"].as_str().unwrap_or("").is_empty()
        && result["causes"].as_array().map(|list| list.is_empty()).unwrap_or(true)
        && result["mods"].as_array().map(|list| list.is_empty()).unwrap_or(true)
        && result["suggestions"]
            .as_array()
            .map(|list| list.is_empty())
            .unwrap_or(true)
}

/// Used when the model never calls `submit_analysis`.
fn fallback_result(text: &str) -> Value {
    json!({
        "summary": clip_str(text, 4000),
        "causes": [],
        "mods": [],
        "suggestions": [],
        "raw": true,
    })
}

/// Expose the tool list for debugging / docs.
#[allow(unused)]
pub fn tool_names() -> Vec<String> {
    tools::tool_defs().into_iter().map(|t: ToolDef| t.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpt_keeps_real_line_numbers() {
        let mut lines: Vec<String> = (1..=240)
            .map(|i| format!("[00:00:{:02}]: normal line {}", i % 60, i))
            .collect();
        lines[119] = "[00:02:00]: LUA ERROR stack traceback:".to_string();
        lines[120] =
            "        scripts/modutil.lua(162,1) in function 'AddClassPostConstruct'".to_string();
        lines[121] = "        ../mods/workshop-1234567/modmain.lua(11,1) in function 'result'"
            .to_string();

        let (excerpt, found) = build_excerpt(&lines);
        assert!(found);
        assert!(
            excerpt.contains("120: [00:02:00]: LUA ERROR stack traceback:"),
            "{}",
            excerpt
        );
        assert!(excerpt.contains("workshop-1234567"), "{}", excerpt);
        // the excerpt is bounded and does not drag the whole file along
        assert!(excerpt.lines().count() < 120, "{}", excerpt.lines().count());
        assert!(!excerpt.contains("100: "), "excerpt leaked unrelated lines");
    }

    #[test]
    fn excerpt_falls_back_to_the_log_tail() {
        let lines: Vec<String> = (1..=100).map(|i| format!("line {}", i)).collect();
        let (excerpt, found) = build_excerpt(&lines);
        assert!(!found);
        assert!(excerpt.contains("100: line 100"), "{}", excerpt);
        assert!(excerpt.lines().count() <= 80);
    }

    #[test]
    fn excerpt_collapses_warning_floods() {
        let lines: Vec<String> = (1..=5000)
            .map(|i| format!("[00:00:00]: WARNING: something noisy {}", i))
            .collect();
        let (excerpt, _) = build_excerpt(&lines);
        assert!(
            excerpt.lines().count() < 40,
            "warnings should not flood: {}",
            excerpt.lines().count()
        );
    }

    struct DummyProgress;

    impl Progress for DummyProgress {
        fn status(&self, _text: &str) {}
    }

    #[derive(Default)]
    struct RecordingProgress {
        tail: std::cell::RefCell<String>,
        usage: std::cell::RefCell<Vec<Value>>,
    }

    impl Progress for RecordingProgress {
        fn status(&self, _text: &str) {}
        fn delta(&self, tail: &str) {
            *self.tail.borrow_mut() = tail.to_string();
        }
        fn usage(&self, usage: &llm::Usage, model: &str) {
            self.usage.borrow_mut().push(usage.to_json(model));
        }
    }

    /// Serve `(status, body)` pairs, used to simulate API errors.
    fn mock_statuses(
        responses: Vec<(u16, String)>,
    ) -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = requests.clone();
        std::thread::spawn(move || {
            for (status, response) in responses {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut raw = String::new();
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some(value) = line.trim().to_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    raw.push_str(&line);
                }
                let mut body = vec![0u8; length];
                reader.read_exact(&mut body).ok();
                raw.push_str(&String::from_utf8_lossy(&body));
                sink.lock().unwrap().push(raw);
                let reason = if status == 200 { "OK" } else { "Error" };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    reason,
                    content_type(&response),
                    response.len(),
                    response
                );
                let _ = stream.flush();
            }
        });
        (port, requests)
    }

    /// Serve raw bodies without any conversion.
    fn mock_raw(bodies: Vec<String>) -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = requests.clone();
        std::thread::spawn(move || {
            for response in bodies {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut raw = String::new();
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some(value) = line.trim().to_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    raw.push_str(&line);
                }
                let mut body = vec![0u8; length];
                reader.read_exact(&mut body).ok();
                raw.push_str(&String::from_utf8_lossy(&body));
                sink.lock().unwrap().push(raw);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    content_type(&response),
                    response.len(),
                    response
                );
                let _ = stream.flush();
            }
        });
        (port, requests)
    }

    /// Serve a canned OpenAI compatible response per request, as an SSE stream.
    fn mock_openai(responses: Vec<Value>) -> (u16, std::sync::Arc<std::sync::Mutex<Vec<Value>>>) {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let bodies = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = bodies.clone();
        std::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    let trimmed = line.trim().to_lowercase();
                    if trimmed.is_empty() {
                        break;
                    }
                    if let Some(value) = trimmed.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; length];
                reader.read_exact(&mut body).ok();
                if let Ok(value) = serde_json::from_slice::<Value>(&body) {
                    sink.lock().unwrap().push(value);
                }
                let payload = to_sse(&response);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    content_type(&payload),
                    payload.len(),
                    payload
                );
                let _ = stream.flush();
            }
        });
        (port, bodies)
    }

    /// content type that matches the canned body
    fn content_type(body: &str) -> &'static str {
        let head = body.trim_start();
        if head.starts_with("data:") || head.starts_with("event:") {
            "text/event-stream"
        } else {
            "application/json"
        }
    }

    /// Convert an OpenAI chat completion body into an SSE stream, so the tests
    /// exercise the streaming parser.
    fn to_sse(response: &Value) -> String {
        let message = &response["choices"][0]["message"];
        let mut delta = json!({"role": "assistant"});
        if !message["content"].is_null() {
            delta["content"] = message["content"].clone();
        }
        if !message["reasoning_content"].is_null() {
            delta["reasoning_content"] = message["reasoning_content"].clone();
        }
        if let Some(calls) = message["tool_calls"].as_array() {
            let calls = calls
                .iter()
                .enumerate()
                .map(|(i, call)| {
                    let mut call = call.clone();
                    call["index"] = json!(i);
                    call
                })
                .collect::<Vec<_>>();
            delta["tool_calls"] = json!(calls);
        }
        let usage = json!({
            "prompt_tokens": 1200,
            "completion_tokens": 30,
            "prompt_cache_hit_tokens": 400,
        });
        format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices": [{"index": 0, "delta": delta}]}),
            json!({"choices": [], "usage": usage}),
        )
    }

    /// Convert an Anthropic messages body into its SSE event stream.
    fn claude_sse(response: &Value) -> String {
        let event = |name: &str, data: Value| format!("event: {}\ndata: {}\n\n", name, data);
        let mut out = String::new();
        out.push_str(&event(
            "message_start",
            json!({"type": "message_start", "message": {"usage": {"input_tokens": 1000, "cache_read_input_tokens": 250}}}),
        ));
        let blocks = response["content"].as_array().cloned().unwrap_or_default();
        for (i, block) in blocks.iter().enumerate() {
            match block["type"].as_str().unwrap_or_default() {
                "text" => {
                    out.push_str(&event("content_block_start", json!({
                        "type": "content_block_start", "index": i,
                        "content_block": {"type": "text", "text": ""}
                    })));
                    out.push_str(&event("content_block_delta", json!({
                        "type": "content_block_delta", "index": i,
                        "delta": {"type": "text_delta", "text": block["text"]}
                    })));
                },
                "thinking" => {
                    out.push_str(&event("content_block_start", json!({
                        "type": "content_block_start", "index": i,
                        "content_block": {"type": "thinking", "thinking": ""}
                    })));
                    out.push_str(&event("content_block_delta", json!({
                        "type": "content_block_delta", "index": i,
                        "delta": {"type": "thinking_delta", "thinking": block["thinking"]}
                    })));
                    if !block["signature"].is_null() {
                        out.push_str(&event("content_block_delta", json!({
                            "type": "content_block_delta", "index": i,
                            "delta": {"type": "signature_delta", "signature": block["signature"]}
                        })));
                    }
                },
                "tool_use" => {
                    out.push_str(&event("content_block_start", json!({
                        "type": "content_block_start", "index": i,
                        "content_block": {"type": "tool_use", "id": block["id"], "name": block["name"], "input": {}}
                    })));
                    out.push_str(&event("content_block_delta", json!({
                        "type": "content_block_delta", "index": i,
                        "delta": {"type": "input_json_delta", "partial_json": block["input"].to_string()}
                    })));
                },
                _ => {},
            }
            out.push_str(&event(
                "content_block_stop",
                json!({"type": "content_block_stop", "index": i}),
            ));
        }
        out.push_str(&event(
            "message_delta",
            json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 80}}),
        ));
        out.push_str(&event("message_stop", json!({"type": "message_stop"})));
        out
    }

    fn tool_reply(id: &str, name: &str, arguments: &str) -> Value {
        json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": id,
                        "type": "function",
                        "function": {"name": name, "arguments": arguments}
                    }]
                }
            }]
        })
    }

    /// Full loop against a fake OpenAI compatible endpoint: the model reads the
    /// game source, searches a mod and finally submits a structured answer.
    #[test]
    fn agent_loop_with_a_mock_provider() {
        use crate::ds_log::LogPath;
        use std::path::PathBuf;

        let dir = std::env::temp_dir().join(format!("dst-ai-agent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("game/data/scripts")).unwrap();
        std::fs::create_dir_all(dir.join("game/mods/workshop-1234567/scripts")).unwrap();

        std::fs::write(
            dir.join("game/data/scripts/modutil.lua"),
            "-- GAME_SOURCE_MARKER\nlocal M = {}\nfunction M.AddClassPostConstruct()\nend\nreturn M\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("game/mods/workshop-1234567/scripts/broken.lua"),
            "-- MOD_MARKER\nlocal x = nil\n",
        )
        .unwrap();

        // a long log with the error somewhere in the middle
        let mut log = String::new();
        for i in 1..=300 {
            log.push_str(&format!("[00:00:{:02}]: log line {}\n", i % 60, i));
        }
        let error_line = "129: [00:02:09]: LUA ERROR stack traceback:";
        let lines: Vec<&str> = log.lines().collect();
        let mut log_lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        log_lines[0] = "Don't Starve Together: 654321 WIN32_STEAM".to_string();
        log_lines[1] = "Mode: 64-bit".to_string();
        log_lines[128] = "[00:02:09]: LUA ERROR stack traceback:".to_string();
        log_lines[129] = "        scripts/modutil.lua(162,1) in function 'AddClassPostConstruct'".to_string();
        log_lines[130] = "        ../mods/workshop-1234567/modmain.lua(11,1) in main chunk".to_string();
        let log_text = log_lines.join("\n") + "\n";
        let log_path = dir.join("log.txt");
        std::fs::write(&log_path, &log_text).unwrap();

        let mut first_reply = tool_reply(
            "call_1",
            "read_game_file",
            "{\"path\":\"scripts/modutil.lua\",\"line\":162}",
        );
        // DeepSeek style thinking output, it must be echoed back
        first_reply["choices"][0]["message"]["reasoning_content"] =
            json!("先读一下游戏源码");

        let (port, requests) = mock_openai(vec![
            first_reply,
            tool_reply(
                "call_2",
                "search_mod_code",
                "{\"mod\":\"workshop-1234567\",\"keyword\":\"MOD_MARKER\"}",
            ),
            tool_reply(
                "call_3",
                "submit_analysis",
                "{\"summary\":\"模组缺少文件导致崩溃\",\"causes\":[{\"title\":\"缺少模块\",\"detail\":\"modutil 报错\",\"mods\":[\"workshop-1234567\"]}],\"mods\":[{\"moddir\":\"workshop-1234567\",\"name\":\"Test Mod\",\"reason\":\"堆栈中出现\"}],\"suggestions\":[\"更新模组\"]}",
            ),
        ]);

        let config = AiConfig {
            provider: "custom".to_string(),
            api_key: "test-key".to_string(),
            model: "mock-model".to_string(),
            base_url: Some(format!("http://127.0.0.1:{}/v1", port)),
            game_dir: Some(dir.join("game").to_string_lossy().to_string()),
            lang: Some("zh".to_string()),
        };
        let mods = vec![ModBrief {
            moddir: "workshop-1234567".to_string(),
            name: "Test Mod".to_string(),
            version: Some("1.0.0".to_string()),
            workshop_id: Some("1234567".to_string()),
        }];
        let result = analyze(
            &LogPath::External(PathBuf::from(&log_path)),
            &config,
            mods,
            None,
            &DummyProgress,
        )
        .unwrap();

        assert_eq!(result["summary"], "模组缺少文件导致崩溃");
        assert_eq!(result["causes"][0]["title"], "缺少模块");
        assert_eq!(result["mods"][0]["moddir"], "workshop-1234567");
        assert_eq!(result["suggestions"][0], "更新模组");

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 3, "one request per model turn");

        // the first prompt carries the error except with real line numbers,
        // but never the whole log
        let first = requests[0].to_string();
        assert!(first.contains("LUA ERROR stack traceback:"), "{}", first);
        assert!(first.contains(error_line), "line numbers should be kept");
        assert!(
            first.contains("Don't Starve Together: 654321 WIN32_STEAM"),
            "build info should be part of the prompt"
        );
        assert!(
            !first.contains("log line 299"),
            "the whole log must not be sent"
        );
        // tool results are fed back to the model
        assert!(
            !first.contains("先读一下游戏源码"),
            "the first request has no reasoning to replay"
        );
        let second = requests[1].to_string();
        assert!(second.contains("GAME_SOURCE_MARKER"), "game source missing");
        assert!(
            second.contains("先读一下游戏源码") && second.contains("reasoning_content"),
            "thinking output must be replayed when tools are used"
        );
        let third = requests[2].to_string();
        assert!(third.contains("MOD_MARKER"), "mod search result missing");
        assert!(third.contains("GAME_SOURCE_MARKER"), "history should be kept");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Same as [`mock_openai`] but speaks the Anthropic SSE dialect and keeps
    /// the raw request (headers + body).
    fn mock_http(responses: Vec<Value>) -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = requests.clone();
        std::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut raw = String::new();
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    if line.trim().is_empty() {
                        break;
                    }
                    let lower = line.trim().to_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    raw.push_str(&line);
                }
                let mut body = vec![0u8; length];
                reader.read_exact(&mut body).ok();
                raw.push_str(&String::from_utf8_lossy(&body));
                sink.lock().unwrap().push(raw);
                let payload = claude_sse(&response);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    content_type(&payload),
                    payload.len(),
                    payload
                );
                let _ = stream.flush();
            }
        });
        (port, requests)
    }

    /// Claude / Anthropic wire format, including the "please submit" nudge.
    #[test]
    fn claude_flow_and_nudge() {
        use crate::ds_log::LogPath;
        use std::path::PathBuf;

        let dir = std::env::temp_dir().join(format!("dst-ai-claude-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("log.txt");
        let mut lines: Vec<String> = (1..=120)
            .map(|i| format!("[00:00:{:02}]: line {}", i % 60, i))
            .collect();
        lines[59] = "[00:01:00]: LUA ERROR stack traceback:".to_string();
        lines[60] = "        scripts/modutil.lua(162,1) in function 'f'".to_string();
        std::fs::write(&log_path, lines.join("\n")).unwrap();

        let (port, requests) = mock_http(vec![
            json!({
                "content": [
                    {"type": "thinking", "thinking": "先看看日志上下文", "signature": "sig"},
                    {"type": "tool_use", "id": "toolu_1", "name": "read_log_lines",
                     "input": {"start": 55, "end": 65}}
                ],
                "stop_reason": "tool_use"
            }),
            json!({
                "content": [{"type": "text", "text": "初步判断是模组问题"}],
                "stop_reason": "end_turn"
            }),
            json!({
                "content": [{"type": "tool_use", "id": "toolu_2", "name": "submit_analysis",
                             "input": {"summary": "模组缺少模块", "causes": [],
                                       "mods": [], "suggestions": []}}],
                "stop_reason": "tool_use"
            }),
        ]);

        let config = AiConfig {
            provider: "claude".to_string(),
            api_key: "test-key".to_string(),
            model: "claude-sonnet-4-5".to_string(),
            base_url: Some(format!("http://127.0.0.1:{}", port)),
            game_dir: None,
            lang: Some("zh".to_string()),
        };
        let result = analyze(
            &LogPath::External(PathBuf::from(&log_path)),
            &config,
            vec![],
            None,
            &DummyProgress,
        )
        .unwrap();
        assert_eq!(result["summary"], "模组缺少模块");

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 3, "tool use, nudge, then submit");
        // anthropic auth headers instead of a bearer token
        assert!(requests[0].contains("x-api-key: test-key"));
        assert!(requests[0].contains("anthropic-version"));
        assert!(!requests[0].contains("Authorization: Bearer"));
        // the tool result of turn 1 is sent back as a tool_result block
        assert!(requests[1].contains("tool_result"), "{}", requests[1]);
        assert!(requests[1].contains("toolu_1"));
        assert!(requests[1].contains("LUA ERROR"), "{}", requests[1]);
        // extended thinking blocks have to be replayed unmodified
        assert!(
            requests[1].contains("先看看日志上下文"),
            "thinking block was dropped: {}",
            requests[1]
        );
        // the nudge reaches the model after a plain text answer
        assert!(requests[2].contains("初步判断是模组问题"), "{}", requests[2]);
        assert!(requests[2].contains("submit_analysis"), "{}", requests[2]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn provider_urls_and_models() {
        use self::llm::Provider;

        let cfg = |provider: &str, base: Option<&str>| AiConfig {
            provider: provider.to_string(),
            api_key: "k".to_string(),
            model: "m".to_string(),
            base_url: base.map(|s| s.to_string()),
            game_dir: None,
            lang: None,
        };
        let openai_url_of = |config: &AiConfig| match llm::build_provider(config).unwrap() {
            Provider::OpenAi { url, .. } => url,
            other => panic!("expected an openai style provider: {:?}", other),
        };
        let claude_url_of = |config: &AiConfig| match llm::build_provider(config).unwrap() {
            Provider::Claude { url, .. } => url,
            other => panic!("expected claude: {:?}", other),
        };

        assert_eq!(
            openai_url_of(&cfg("deepseek", None)),
            "https://api.deepseek.com/chat/completions"
        );
        assert_eq!(
            openai_url_of(&cfg("openai", None)),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            openai_url_of(&cfg("custom", Some("api.example.com/v1"))),
            "https://api.example.com/v1/chat/completions"
        );
        assert_eq!(
            openai_url_of(&cfg("custom", Some("https://x.example.com/v1/chat/completions"))),
            "https://x.example.com/v1/chat/completions"
        );
        assert!(llm::build_provider(&cfg("custom", None)).is_err());
        assert!(llm::build_provider(&cfg("custom", Some("  "))).is_err());

        assert_eq!(
            claude_url_of(&cfg("claude", None)),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            claude_url_of(&cfg("claude", Some("https://proxy.example.com"))),
            "https://proxy.example.com/v1/messages"
        );
        assert_eq!(
            claude_url_of(&cfg("claude", Some("https://proxy.example.com/anthropic"))),
            "https://proxy.example.com/anthropic/v1/messages"
        );
    }

    /// A broken `submit_analysis` call must be corrected, never rendered as
    /// an empty panel.
    #[test]
    fn malformed_submit_is_repaired() {
        use crate::ds_log::LogPath;
        use std::path::PathBuf;

        let dir = std::env::temp_dir().join(format!("dst-ai-repair-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("log.txt");
        std::fs::write(
            &log_path,
            "[00:00:01]: LUA ERROR stack traceback:\n    scripts/modutil.lua(1,1) in f\n",
        )
        .unwrap();

        // a trailing comma makes the arguments invalid JSON
        let fixed = json!({
            "summary": "缺少模块",
            "causes": [{"title": "缺少文件", "detail": "见第 1 行"}],
            "mods": [],
            "suggestions": ["重装模组"]
        })
        .to_string();
        let (port, requests) = mock_openai(vec![
            tool_reply("call_1", "submit_analysis", r#"{"summary": "x",}"#),
            tool_reply("call_2", "submit_analysis", &fixed),
        ]);

        let config = AiConfig {
            provider: "custom".to_string(),
            api_key: "k".to_string(),
            model: "m".to_string(),
            base_url: Some(format!("http://127.0.0.1:{}/v1", port)),
            game_dir: None,
            lang: None,
        };
        let result = analyze(
            &LogPath::External(PathBuf::from(&log_path)),
            &config,
            vec![],
            None,
            &DummyProgress,
        )
        .unwrap();
        assert_eq!(result["summary"], "缺少模块");
        assert_eq!(result["causes"][0]["title"], "缺少文件");

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let second = requests[1].to_string();
        assert!(
            second.contains("不是合法 JSON"),
            "the model should be told what was wrong: {}",
            &second[second.len().saturating_sub(600)..]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A round that mixes a real tool call with a broken `submit_analysis`
    /// must still answer every call id.
    #[test]
    fn repair_answers_every_tool_call() {
        use crate::ds_log::LogPath;
        use std::path::PathBuf;

        let dir = std::env::temp_dir().join(format!("dst-ai-multi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("log.txt");
        std::fs::write(&log_path, "[00:00:01]: line one\n[00:00:02]: line two\n").unwrap();

        let mixed = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {"id": "call_a", "type": "function",
                         "function": {"name": "read_log_lines", "arguments": "{\"start\":1,\"end\":2}"}},
                        {"id": "call_b", "type": "function",
                         "function": {"name": "submit_analysis", "arguments": "{}"}}
                    ]
                }
            }]
        });
        let fixed = json!({
            "summary": "结论",
            "causes": [{"title": "t", "detail": "d"}]
        })
        .to_string();
        let (port, requests) = mock_openai(vec![
            mixed,
            tool_reply("call_c", "submit_analysis", &fixed),
        ]);

        let config = AiConfig {
            provider: "custom".to_string(),
            api_key: "k".to_string(),
            model: "m".to_string(),
            base_url: Some(format!("http://127.0.0.1:{}/v1", port)),
            game_dir: None,
            lang: None,
        };
        let result = analyze(
            &LogPath::External(PathBuf::from(&log_path)),
            &config,
            vec![],
            None,
            &DummyProgress,
        )
        .unwrap();
        assert_eq!(result["summary"], "结论");

        let requests = requests.lock().unwrap();
        let second = requests[1].to_string();
        assert!(second.contains("call_a"), "missing result for the real tool call");
        assert!(second.contains("call_b"), "missing result for submit_analysis");
        assert!(second.contains("line one"), "the real tool should have run");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Models do not always follow the schema exactly.
    #[test]
    fn normalize_accepts_loose_shapes() {
        let args = json!({
            "analysis": {
                "总结": "模组报错",
                "原因": ["缺少文件", {"title": "版本不匹配", "说明": "第 120 行"}],
                "模组": [{"moddir": "workshop-1", "why": "堆栈中出现"}],
                "建议": "更新模组"
            }
        });
        let result = normalize_result(&args);
        assert_eq!(result["summary"], "模组报错");
        assert_eq!(result["causes"][0]["title"], "缺少文件");
        assert_eq!(result["causes"][1]["title"], "版本不匹配");
        assert_eq!(result["causes"][1]["detail"], "第 120 行");
        assert_eq!(result["mods"][0]["moddir"], "workshop-1");
        assert_eq!(result["mods"][0]["reason"], "堆栈中出现");
        assert_eq!(result["suggestions"][0], "更新模组");
        assert!(!is_empty_result(&result));

        // camelCase / PascalCase keys survive too
        let camel = json!({
            "Summary": "驼峰键",
            "RelatedMods": ["workshop-9"],
            "Suggestions": ["升级模组"]
        });
        let result = normalize_result(&camel);
        assert_eq!(result["summary"], "驼峰键");
        assert_eq!(result["mods"][0]["moddir"], "workshop-9");
        assert_eq!(result["suggestions"][0], "升级模组");

        // a payload that was encoded as a string inside a wrapper
        let inner = json!({"summary": "双重编码", "causes": []}).to_string();
        let wrapped = json!({"content": inner});
        assert_eq!(normalize_result(&wrapped)["summary"], "双重编码");

        // garbage in, empty (but well formed) result out
        assert!(is_empty_result(&normalize_result(&json!({}))));
        assert!(is_empty_result(&normalize_result(&json!("nope"))));
    }

    /// The UI gets live text plus token accounting.
    #[test]
    fn streams_text_and_reports_usage() {
        use crate::ds_log::LogPath;
        use std::path::PathBuf;

        let dir = std::env::temp_dir().join(format!("dst-ai-stream-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("log.txt");
        std::fs::write(&log_path, "[00:00:01]: LUA ERROR stack traceback:\n").unwrap();

        let answer = json!({"summary": "结论", "causes": [{"title": "t", "detail": "d"}]}).to_string();
        let (port, _requests) = mock_openai(vec![
            json!({"choices": [{"message": {
                "role": "assistant",
                "content": "我先看一下游戏源码",
                "tool_calls": [{"id": "c1", "type": "function",
                    "function": {"name": "read_game_file", "arguments": "{\"path\":\"scripts/modutil.lua\"}"}}]
            }}]}),
            tool_reply("c2", "submit_analysis", &answer),
        ]);

        let config = AiConfig {
            provider: "custom".to_string(),
            api_key: "k".to_string(),
            model: "deepseek-flash".to_string(),
            base_url: Some(format!("http://127.0.0.1:{}/v1", port)),
            game_dir: None,
            lang: None,
        };
        let progress = RecordingProgress::default();
        let result = analyze(
            &LogPath::External(PathBuf::from(&log_path)),
            &config,
            vec![],
            None,
            &progress,
        )
        .unwrap();
        assert_eq!(result["summary"], "结论");

        // the live tail carries the streamed answer text
        assert!(
            progress.tail.borrow().contains("我先看一下游戏源码"),
            "live tail: {:?}",
            progress.tail.borrow()
        );

        let usage = progress.usage.borrow();
        assert_eq!(usage.len(), 2, "one usage report per turn");
        let last = &usage[1];
        assert_eq!(last["prompt_tokens"], 2400, "input tokens accumulate: {}", last);
        assert_eq!(last["completion_tokens"], 60, "{}", last);
        assert_eq!(last["cached_tokens"], 800, "{}", last);
        assert_eq!(last["context_tokens"], 1200, "context is the latest request");
        assert_eq!(last["context_limit"], 1_000_000, "{}", last);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Providers that answer with plain JSON although `stream` was requested.
    #[test]
    fn plain_json_answer_is_used_as_a_fallback() {
        use crate::ds_log::LogPath;
        use std::path::PathBuf;

        let dir = std::env::temp_dir().join(format!("dst-ai-json-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("log.txt");
        std::fs::write(&log_path, "[00:00:01]: LUA ERROR stack traceback:\n").unwrap();

        let body = json!({"choices": [{"message": {
            "role": "assistant",
            "content": null,
            "tool_calls": [{"id": "c1", "type": "function",
                "function": {"name": "submit_analysis",
                    "arguments": "{\"summary\":\"非流式回答\",\"causes\":[]}"}}]
        }}], "usage": {"prompt_tokens": 500, "completion_tokens": 20}})
        .to_string();
        let (port, _requests) = mock_raw(vec![body]);

        let config = AiConfig {
            provider: "custom".to_string(),
            api_key: "k".to_string(),
            model: "some-model".to_string(),
            base_url: Some(format!("http://127.0.0.1:{}/v1", port)),
            game_dir: None,
            lang: None,
        };
        let result = analyze(
            &LogPath::External(PathBuf::from(&log_path)),
            &config,
            vec![],
            None,
            &DummyProgress,
        )
        .unwrap();
        assert_eq!(result["summary"], "非流式回答");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Gateways that reject `stream_options` must still stream.
    #[test]
    fn streaming_retries_without_stream_options() {
        use crate::ds_log::LogPath;
        use std::path::PathBuf;

        let dir = std::env::temp_dir().join(format!("dst-ai-opt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("log.txt");
        std::fs::write(&log_path, "[00:00:01]: LUA ERROR stack traceback:\n").unwrap();

        let answer = json!({"summary": "重试成功", "causes": []}).to_string();
        let (port, requests) = mock_statuses(vec![
            (
                400,
                json!({"error": {"message": "Unrecognized request argument: stream_options"}})
                    .to_string(),
            ),
            (200, to_sse(&tool_reply("c1", "submit_analysis", &answer))),
        ]);

        let config = AiConfig {
            provider: "custom".to_string(),
            api_key: "k".to_string(),
            model: "m".to_string(),
            base_url: Some(format!("http://127.0.0.1:{}/v1", port)),
            game_dir: None,
            lang: None,
        };
        let result = analyze(
            &LogPath::External(PathBuf::from(&log_path)),
            &config,
            vec![],
            None,
            &DummyProgress,
        )
        .unwrap();
        assert_eq!(result["summary"], "重试成功");

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2, "one failed attempt, one retry");
        assert!(requests[0].contains("stream_options"), "first attempt asks for usage");
        assert!(
            !requests[1].contains("stream_options"),
            "the retry must drop it: {}",
            &requests[1][requests[1].len().saturating_sub(300)..]
        );
        assert!(requests[1].contains("\"stream\":true"), "the retry still streams");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A model that keeps exploring must be asked to conclude, not killed.
    #[test]
    fn wrap_up_asks_for_a_conclusion() {
        use crate::ds_log::LogPath;
        use std::path::PathBuf;

        let dir = std::env::temp_dir().join(format!("dst-ai-wrap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("log.txt");
        std::fs::write(&log_path, "[00:00:01]: LUA ERROR stack traceback:\n").unwrap();

        // the model never stops reading files...
        let exploring = to_sse(&tool_reply(
            "c",
            "read_log_lines",
            "{\"start\":1,\"end\":2}",
        ));
        // it even ignores the first wrap-up round and asks for one more file
        let mut responses: Vec<(u16, String)> =
            (0..=MAX_TOOL_STEPS).map(|_| (200, exploring.clone())).collect();
        // ...and only then submits
        let answer = json!({"summary": "最终结论", "causes": []}).to_string();
        responses.push((200, to_sse(&tool_reply("s", "submit_analysis", &answer))));

        let (port, requests) = mock_statuses(responses);
        let config = AiConfig {
            provider: "custom".to_string(),
            api_key: "k".to_string(),
            model: "m".to_string(),
            base_url: Some(format!("http://127.0.0.1:{}/v1", port)),
            game_dir: None,
            lang: None,
        };
        let result = analyze(
            &LogPath::External(PathBuf::from(&log_path)),
            &config,
            vec![],
            None,
            &DummyProgress,
        )
        .unwrap();
        assert_eq!(result["summary"], "最终结论");

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), MAX_TOOL_STEPS + 2);
        assert!(
            requests[MAX_TOOL_STEPS + 1].contains("分析步数已达上限"),
            "once the budget is spent the model is asked for a conclusion: {}",
            &requests[MAX_TOOL_STEPS + 1][requests[MAX_TOOL_STEPS + 1].len().saturating_sub(600)..]
        );
        // duplicate calls are served from the cache instead of being executed again
        assert!(
            requests[2].contains("请勿重复调用相同参数"),
            "{}",
            &requests[2][requests[2].len().saturating_sub(600)..]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Giving up must explain itself and keep the step budget honest.
    #[test]
    fn step_limit_fails_gracefully() {
        use crate::ds_log::LogPath;
        use std::path::PathBuf;

        let dir = std::env::temp_dir().join(format!("dst-ai-limit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log_path = dir.join("log.txt");
        std::fs::write(&log_path, "[00:00:01]: LUA ERROR stack traceback:\n").unwrap();

        let exploring = to_sse(&tool_reply("c", "list_game_files", "{}"));
        let responses: Vec<(u16, String)> =
            (0..MAX_STEPS).map(|_| (200, exploring.clone())).collect();
        let (port, _requests) = mock_statuses(responses);
        let config = AiConfig {
            provider: "custom".to_string(),
            api_key: "k".to_string(),
            model: "m".to_string(),
            base_url: Some(format!("http://127.0.0.1:{}/v1", port)),
            game_dir: None,
            lang: None,
        };
        let error = analyze(
            &LogPath::External(PathBuf::from(&log_path)),
            &config,
            vec![],
            None,
            &DummyProgress,
        )
        .unwrap_err();
        assert!(error.contains("没有给出结论"), "{}", error);
        assert!(error.contains(&MAX_STEPS.to_string()), "{}", error);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn result_is_normalized() {
        let args = json!({
            "summary": "boom",
            "causes": [{"title": "t", "detail": "d", "mods": ["workshop-1"]}],
            "mods": [{"name": "Mod", "reason": "why"}],
            "suggestions": ["do it"],
        });
        let result = normalize_result(&args);
        assert_eq!(result["summary"], "boom");
        assert_eq!(result["causes"][0]["mods"][0], "workshop-1");
        assert_eq!(result["mods"][0]["name"], "Mod");
        assert_eq!(result["suggestions"][0], "do it");
        assert_eq!(result["raw"], false);
    }
}
