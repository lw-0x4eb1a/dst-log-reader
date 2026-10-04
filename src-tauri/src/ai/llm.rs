//! Minimal chat providers.
//!
//! Two wire formats are supported: the OpenAI `/chat/completions` style
//! (DeepSeek, ChatGPT and any compatible endpoint) and the Anthropic
//! `/v1/messages` style (Claude).
//!
//! Both are requested in streaming mode so the UI can show live progress, with
//! a transparent fall back to a blocking request when streaming is not
//! available. Token usage is collected for every turn.

use std::time::Duration;

use serde_json::{json, Value};

use super::AiConfig;

const REQUEST_TIMEOUT: u64 = 300;

#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

#[derive(Debug, Clone)]
pub struct Call {
    pub id: String,
    pub name: String,
    pub args: Value,
    /// Set when the provider sent `arguments` we could not parse, so the
    /// agent loop can ask for a correction instead of silently using `{}`.
    pub args_error: Option<String>,
}

impl Call {
    fn new(id: String, name: String, args: Value, args_error: Option<String>) -> Self {
        Self { id, name, args, args_error }
    }
}

/// A message in the conversation, provider independent.
#[derive(Debug, Clone)]
pub enum Msg {
    User(String),
    Assistant {
        text: Option<String>,
        /// Chain of thought that has to be replayed on the next turn:
        /// a string for `reasoning_content` (DeepSeek thinking mode with
        /// tools) or the raw blocks for Anthropic style thinking.
        reasoning: Option<Value>,
        calls: Vec<Call>,
    },
    Results(Vec<(String, String)>),
}

#[derive(Debug, Clone)]
pub struct Reply {
    pub text: Option<String>,
    pub reasoning: Option<Value>,
    pub calls: Vec<Call>,
}

/// Token accounting of one or more turns.
#[derive(Debug, Clone, Default)]
pub struct Usage {
    /// input tokens billed over the whole conversation
    pub prompt_tokens: u64,
    /// output tokens billed over the whole conversation
    pub completion_tokens: u64,
    /// input tokens served from the provider cache
    pub cached_tokens: u64,
    /// input tokens of the latest request, ie the current context size
    pub context_tokens: u64,
    pub turns: u32,
}

impl Usage {
    /// Fold a single turn into the running total.
    pub fn merge(&mut self, turn: &Usage) {
        self.prompt_tokens += turn.prompt_tokens;
        self.completion_tokens += turn.completion_tokens;
        self.cached_tokens += turn.cached_tokens;
        if turn.context_tokens > 0 {
            self.context_tokens = turn.context_tokens;
        }
        self.turns += 1;
    }

    pub fn to_json(&self, model: &str) -> Value {
        json!({
            "prompt_tokens": self.prompt_tokens,
            "completion_tokens": self.completion_tokens,
            "total_tokens": self.prompt_tokens + self.completion_tokens,
            "cached_tokens": self.cached_tokens,
            "context_tokens": self.context_tokens,
            "context_limit": context_limit(model),
            "turns": self.turns,
        })
    }
}

/// Known context window sizes, used to show how full the context is.
pub fn context_limit(model: &str) -> Option<u64> {
    let model = model.to_lowercase();
    if model.contains("deepseek") {
        Some(1_000_000)
    } else if model.contains("claude") {
        Some(200_000)
    } else if model.contains("gpt") {
        Some(128_000)
    } else {
        None
    }
}

/// Incremental output of a streaming turn.
#[derive(Debug, Clone)]
pub enum Stream {
    /// final answer text
    Delta(String),
    /// the model is producing chain of thought, nothing to show yet
    Thinking,
}

pub struct Turn {
    pub reply: Reply,
    pub usage: Usage,
}

#[derive(Debug, Clone)]
pub enum Provider {
    OpenAi {
        url: String,
        api_key: String,
        model: String,
    },
    Claude {
        url: String,
        api_key: String,
        model: String,
    },
}

#[derive(Debug, Clone, Default)]
struct PartialCall {
    id: String,
    name: String,
    args: String,
}

fn ensure_scheme(url: &str) -> String {
    let url = url.trim().trim_end_matches('/').to_string();
    if url.starts_with("http://") || url.starts_with("https://") {
        url
    } else {
        format!("https://{}", url)
    }
}

/// DeepSeek / OpenAI / custom (OpenAI compatible) endpoint.
fn openai_url(provider: &str, base_url: Option<&str>) -> Result<String, String> {
    let base = match provider {
        // the documented OpenAI-format base url; `/chat/completions` is appended
        "deepseek" => "https://api.deepseek.com".to_string(),
        "openai" | "chatgpt" => "https://api.openai.com/v1".to_string(),
        "custom" => {
            let url = base_url.unwrap_or("").trim();
            if url.is_empty() {
                return Err("请填写自定义 API 域名".to_string());
            }
            ensure_scheme(url)
        },
        other => return Err(format!("不支持的服务商: {}", other)),
    };
    if base.ends_with("/chat/completions") {
        Ok(base)
    } else {
        Ok(format!("{}/chat/completions", base.trim_end_matches('/')))
    }
}

/// Anthropic `/v1/messages` endpoint, optionally behind a proxy/gateway.
fn claude_url(base_url: Option<&str>) -> String {
    let Some(base) = base_url.map(|s| s.trim()).filter(|s| !s.is_empty()) else {
        return "https://api.anthropic.com/v1/messages".to_string();
    };
    let base = ensure_scheme(base);
    if base.ends_with("/messages") {
        base
    } else if base.ends_with("/v1") {
        format!("{}/messages", base)
    } else {
        format!("{}/v1/messages", base)
    }
}

pub fn build_provider(config: &AiConfig) -> Result<Provider, String> {
    let api_key = config.api_key.trim().to_string();
    if api_key.is_empty() {
        return Err("请先配置 API Key".to_string());
    }
    let model = config.model.trim().to_string();
    match config.provider.as_str() {
        "claude" | "anthropic" => {
            if model.is_empty() {
                return Err("请先配置模型名称".to_string());
            }
            Ok(Provider::Claude {
                url: claude_url(config.base_url.as_deref()),
                api_key,
                model,
            })
        },
        other => {
            let url = openai_url(other, config.base_url.as_deref())?;
            if model.is_empty() {
                return Err("请先配置模型名称".to_string());
            }
            Ok(Provider::OpenAi { url, api_key, model })
        },
    }
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(REQUEST_TIMEOUT))
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

fn friendly_net_error(e: reqwest::Error) -> String {
    if e.is_timeout() {
        "请求超时，请稍后重试或更换更快的模型".to_string()
    } else if e.is_connect() {
        format!("无法连接到 API 服务器，请检查网络或域名配置 ({})", e)
    } else {
        e.to_string()
    }
}

fn http_error(status: reqwest::StatusCode, text: &str) -> String {
    let hint = match status.as_u16() {
        401 | 403 => "（API Key 无效或没有权限）",
        404 => "（接口地址或模型名不正确）",
        429 => "（请求过于频繁或余额不足）",
        _ => "",
    };
    format!(
        "接口返回 {} {}: {}",
        status.as_u16(),
        hint,
        clip(text.trim(), 600)
    )
}

fn new_call_id(index: usize) -> String {
    format!("call_{}", index)
}

/// Truncate on a char boundary.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

/// Decode a tool `arguments` payload. Tolerates plain objects, JSON strings
/// and double encoded strings.
fn parse_arguments(raw: &Value) -> (Value, Option<String>) {
    match raw {
        Value::Object(_) => (raw.clone(), None),
        Value::Null => (json!({}), None),
        Value::String(text) => {
            if text.trim().is_empty() {
                return (json!({}), None);
            }
            match serde_json::from_str::<Value>(text) {
                Ok(Value::String(inner)) => match serde_json::from_str::<Value>(&inner) {
                    // double encoded payload
                    Ok(value) => (value, None),
                    Err(e) => (json!({}), Some(format!("{} ({})", e, clip(text, 200)))),
                },
                Ok(value) => (value, None),
                Err(e) => (json!({}), Some(format!("{} ({})", e, clip(text, 200)))),
            }
        },
        other => (
            json!({}),
            Some(format!("arguments 类型异常: {}", clip(&other.to_string(), 120))),
        ),
    }
}

/// True when the server answered with plain JSON although we asked for a
/// stream (some OpenAI compatible gateways do that).
fn is_json_response(response: &reqwest::blocking::Response) -> bool {
    response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.contains("json"))
        .unwrap_or(false)
}

/// Read an SSE body line by line and hand every `data:` payload over.
fn read_sse(
    response: reqwest::blocking::Response,
    mut on_data: impl FnMut(&str),
) -> Result<(), String> {
    use std::io::BufRead;
    let reader = std::io::BufReader::new(response);
    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let line = line.trim();
        let Some(data) = line.strip_prefix("data:") else { continue };
        let data = data.trim();
        if data.is_empty() {
            continue;
        }
        if data == "[DONE]" {
            break;
        }
        on_data(data);
    }
    Ok(())
}

// ---------------------------------------------------------------- request body

fn openai_messages(system: &str, msgs: &[Msg]) -> Vec<Value> {
    let mut messages = vec![json!({"role": "system", "content": system})];
    for msg in msgs {
        match msg {
            Msg::User(text) => messages.push(json!({"role": "user", "content": text})),
            Msg::Assistant { text, reasoning, calls } => {
                let mut message = json!({"role": "assistant", "content": text});
                // DeepSeek thinking mode hands out `reasoning_content` and
                // requires it back on every later turn while `tools` is set.
                if let Some(reasoning @ Value::String(_)) = reasoning {
                    message["reasoning_content"] = reasoning.clone();
                }
                if !calls.is_empty() {
                    message["tool_calls"] = json!(calls
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "type": "function",
                                "function": {"name": c.name, "arguments": c.args.to_string()}
                            })
                        })
                        .collect::<Vec<_>>());
                }
                messages.push(message);
            },
            Msg::Results(results) => {
                for (id, content) in results {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": id,
                        "content": content,
                    }));
                }
            },
        }
    }
    messages
}

fn openai_tools(tools: &[ToolDef]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.schema,
                }
            })
        })
        .collect()
}

fn openai_body(
    model: &str,
    system: &str,
    msgs: &[Msg],
    tools: &[ToolDef],
    stream: bool,
    include_usage: bool,
) -> Value {
    let mut body = json!({
        "model": model,
        "messages": openai_messages(system, msgs),
        "tools": openai_tools(tools),
        "tool_choice": "auto",
        "stream": stream,
    });
    if stream && include_usage {
        // ask for the usage block in the final chunk
        body["stream_options"] = json!({"include_usage": true});
    }
    body
}

fn claude_messages(msgs: &[Msg]) -> Vec<Value> {
    let mut messages: Vec<Value> = vec![];
    for msg in msgs {
        match msg {
            Msg::User(text) => messages.push(json!({
                "role": "user",
                "content": [{"type": "text", "text": text}],
            })),
            Msg::Assistant { text, reasoning, calls } => {
                let mut content: Vec<Value> = vec![];
                // extended thinking blocks must be replayed unmodified
                if let Some(Value::Array(blocks)) = reasoning {
                    content.extend(blocks.iter().cloned());
                }
                if let Some(text) = text {
                    if !text.trim().is_empty() {
                        content.push(json!({"type": "text", "text": text}));
                    }
                }
                for call in calls {
                    content.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.args,
                    }));
                }
                if content.is_empty() {
                    continue;
                }
                messages.push(json!({"role": "assistant", "content": content}));
            },
            Msg::Results(results) => {
                let content = results
                    .iter()
                    .map(|(id, text)| {
                        json!({
                            "type": "tool_result",
                            "tool_use_id": id,
                            "content": text,
                        })
                    })
                    .collect::<Vec<_>>();
                messages.push(json!({"role": "user", "content": content}));
            },
        }
    }
    messages
}

fn claude_tools(tools: &[ToolDef]) -> Vec<Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.schema,
            })
        })
        .collect()
}

fn claude_body(model: &str, system: &str, msgs: &[Msg], tools: &[ToolDef], stream: bool) -> Value {
    json!({
        "model": model,
        "max_tokens": 8192,
        "system": system,
        "messages": claude_messages(msgs),
        "tools": claude_tools(tools),
        "stream": stream,
    })
}

// ------------------------------------------------------------------- responses

fn parse_usage(value: &Value) -> Usage {
    let prompt = value["prompt_tokens"].as_u64().unwrap_or_else(|| {
        // Anthropic style: cached tokens are reported separately
        let input = value["input_tokens"].as_u64().unwrap_or(0);
        let read = value["cache_read_input_tokens"].as_u64().unwrap_or(0);
        let creation = value["cache_creation_input_tokens"].as_u64().unwrap_or(0);
        input + read + creation
    });
    let completion = value["completion_tokens"]
        .as_u64()
        .or_else(|| value["output_tokens"].as_u64())
        .unwrap_or(0);
    let cached = value["prompt_cache_hit_tokens"]
        .as_u64()
        .or_else(|| value["prompt_tokens_details"]["cached_tokens"].as_u64())
        .or_else(|| value["cache_read_input_tokens"].as_u64())
        .unwrap_or(0);
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        cached_tokens: cached,
        context_tokens: prompt,
        turns: 0,
    }
}

/// Streaming events report usage piecewise (input first, output later).
fn absorb_usage(target: &mut Usage, value: &Value) {
    let incoming = parse_usage(value);
    if incoming.prompt_tokens > 0 {
        target.prompt_tokens = incoming.prompt_tokens;
        target.context_tokens = incoming.prompt_tokens;
    }
    if incoming.completion_tokens > 0 {
        target.completion_tokens = incoming.completion_tokens;
    }
    if incoming.cached_tokens > 0 {
        target.cached_tokens = incoming.cached_tokens;
    }
}

/// Build a turn out of a plain (non streaming) chat completion body.
fn openai_turn_from_body(data: &Value) -> Result<Turn, String> {
    let message = &data["choices"][0]["message"];
    if message.is_null() {
        if !data["error"].is_null() {
            return Err(format!("接口返回错误: {}", data["error"]));
        }
        return Err(format!("接口返回格式异常: {}", clip(&data.to_string(), 300)));
    }
    Ok(Turn {
        reply: parse_openai_message(message),
        usage: parse_usage(&data["usage"]),
    })
}

/// Build a turn out of a plain (non streaming) Anthropic messages body.
fn claude_turn_from_body(data: &Value) -> Result<Turn, String> {
    if !data["error"].is_null() {
        return Err(format!("接口返回错误: {}", data["error"]));
    }
    let blocks = data["content"].as_array().cloned().unwrap_or_default();
    Ok(Turn {
        reply: parse_claude_blocks(&blocks),
        usage: parse_usage(&data["usage"]),
    })
}

fn parse_json_body(text: &str) -> Result<Value, String> {
    serde_json::from_str(text)
        .map_err(|e| format!("无法解析接口返回内容: {} - {}", e, clip(text.trim(), 300)))
}

fn parse_openai_message(message: &Value) -> Reply {
    let text = message["content"]
        .as_str()
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty());
    let reasoning = message["reasoning_content"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(|s| json!(s));
    let mut calls = vec![];
    if let Some(list) = message["tool_calls"].as_array() {
        for (i, call) in list.iter().enumerate() {
            let name = call["function"]["name"].as_str().unwrap_or_default().to_string();
            if name.is_empty() {
                continue;
            }
            let id = call["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| new_call_id(i));
            let (args, args_error) = parse_arguments(&call["function"]["arguments"]);
            if let Some(error) = &args_error {
                eprintln!("[ai] cannot parse arguments of `{}`: {}", name, error);
            }
            calls.push(Call::new(id, name, args, args_error));
        }
    }
    Reply { text, reasoning, calls }
}

fn parse_claude_blocks(blocks: &[Value]) -> Reply {
    let mut text = String::new();
    let mut thinking: Vec<Value> = vec![];
    let mut calls = vec![];
    for (i, block) in blocks.iter().enumerate() {
        match block["type"].as_str().unwrap_or_default() {
            "text" => {
                if let Some(t) = block["text"].as_str() {
                    text.push_str(t);
                }
            },
            "thinking" | "redacted_thinking" => thinking.push(block.clone()),
            "tool_use" => {
                let name = block["name"].as_str().unwrap_or_default().to_string();
                if name.is_empty() {
                    continue;
                }
                let id = block["id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| new_call_id(i));
                let (args, args_error) = parse_arguments(&block["input"]);
                if let Some(error) = &args_error {
                    eprintln!("[ai] cannot parse input of `{}`: {}", name, error);
                }
                calls.push(Call::new(id, name, args, args_error));
            },
            _ => {},
        }
    }
    Reply {
        text: if text.trim().is_empty() { None } else { Some(text) },
        reasoning: if thinking.is_empty() {
            None
        } else {
            Some(Value::Array(thinking))
        },
        calls,
    }
}

// --------------------------------------------------------------------- chat

impl Provider {
    /// One round trip. Streams when the provider supports it and falls back to
    /// a blocking request when it does not.
    pub fn chat(
        &self,
        system: &str,
        msgs: &[Msg],
        tools: &[ToolDef],
        on_stream: &mut dyn FnMut(Stream),
    ) -> Result<Turn, String> {
        let got_output = std::cell::Cell::new(false);
        match self {
            Provider::OpenAi { url, api_key, model } => {
                // `stream_options` is not understood by every gateway, so try
                // with it, then without it, then without streaming at all.
                let mut last_error = String::new();
                for include_usage in [true, false] {
                    got_output.set(false);
                    let result = {
                        let mut forward = |event: Stream| {
                            got_output.set(true);
                            on_stream(event);
                        };
                        self.openai_stream(
                            url, api_key, model, system, msgs, tools, include_usage,
                            &mut forward,
                        )
                    };
                    match result {
                        Ok(turn) => return Ok(turn),
                        // a partial answer must not be requested twice
                        Err(error) if got_output.get() => return Err(error),
                        Err(error) => last_error = error,
                    }
                }
                eprintln!("[ai] streaming unavailable ({}), retrying without it", last_error);
                self.openai_blocking(url, api_key, model, system, msgs, tools)
            },
            Provider::Claude { url, api_key, model } => {
                let result = {
                    let mut forward = |event: Stream| {
                        got_output.set(true);
                        on_stream(event);
                    };
                    self.claude_stream(url, api_key, model, system, msgs, tools, &mut forward)
                };
                match result {
                    Ok(turn) => Ok(turn),
                    Err(error) if !got_output.get() => {
                        eprintln!("[ai] streaming unavailable ({}), retrying without it", error);
                        self.claude_blocking(url, api_key, model, system, msgs, tools)
                    },
                    Err(error) => Err(error),
                }
            },
        }
    }

    fn openai_blocking(
        &self,
        url: &str,
        api_key: &str,
        model: &str,
        system: &str,
        msgs: &[Msg],
        tools: &[ToolDef],
    ) -> Result<Turn, String> {
        let body = openai_body(model, system, msgs, tools, false, false);
        let response = http_client()?
            .post(url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .map_err(friendly_net_error)?;
        let status = response.status();
        let text = response.text().map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(http_error(status, &text));
        }
        openai_turn_from_body(&parse_json_body(&text)?)
    }

    fn openai_stream(
        &self,
        url: &str,
        api_key: &str,
        model: &str,
        system: &str,
        msgs: &[Msg],
        tools: &[ToolDef],
        include_usage: bool,
        on_stream: &mut dyn FnMut(Stream),
    ) -> Result<Turn, String> {
        let body = openai_body(model, system, msgs, tools, true, include_usage);
        let response = http_client()?
            .post(url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .body(body.to_string())
            .send()
            .map_err(friendly_net_error)?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().unwrap_or_default();
            return Err(http_error(status, &text));
        }

        if is_json_response(&response) {
            // the provider ignored `stream`, reuse the body as a blocking answer
            let body = response.text().map_err(|e| e.to_string())?;
            return openai_turn_from_body(&parse_json_body(&body)?);
        }

        let mut text = String::new();
        let mut reasoning = String::new();
        let mut calls: Vec<PartialCall> = vec![];
        let mut usage = Usage::default();
        let mut stream_error: Option<String> = None;

        read_sse(response, |data| {
            let Ok(value) = serde_json::from_str::<Value>(data) else {
                return;
            };
            if !value["error"].is_null() {
                stream_error = Some(format!("接口返回错误: {}", value["error"]));
                return;
            }
            if !value["usage"].is_null() {
                absorb_usage(&mut usage, &value["usage"]);
            }
            let Some(choices) = value["choices"].as_array() else { return };
            for choice in choices {
                let delta = &choice["delta"];
                if let Some(part) = delta["content"].as_str() {
                    if !part.is_empty() {
                        text.push_str(part);
                        on_stream(Stream::Delta(part.to_string()));
                    }
                }
                if let Some(part) = delta["reasoning_content"].as_str() {
                    if !part.is_empty() {
                        reasoning.push_str(part);
                        on_stream(Stream::Thinking);
                    }
                }
                let Some(list) = delta["tool_calls"].as_array() else { continue };
                for call in list {
                    let index = match call["index"].as_u64() {
                        Some(index) => index as usize,
                        None if !calls.is_empty() => calls.len() - 1,
                        None => 0,
                    };
                    while calls.len() <= index {
                        calls.push(PartialCall::default());
                    }
                    let slot = &mut calls[index];
                    if let Some(id) = call["id"].as_str() {
                        if !id.is_empty() {
                            slot.id = id.to_string();
                        }
                    }
                    if let Some(name) = call["function"]["name"].as_str() {
                        slot.name.push_str(name);
                    }
                    if let Some(args) = call["function"]["arguments"].as_str() {
                        slot.args.push_str(args);
                    }
                }
            }
        })?;

        if let Some(error) = stream_error {
            return Err(error);
        }
        let calls = calls
            .into_iter()
            .filter(|call| !call.name.is_empty())
            .enumerate()
            .map(|(i, call)| {
                let (args, args_error) = parse_arguments(&Value::String(call.args));
                if let Some(error) = &args_error {
                    eprintln!("[ai] cannot parse arguments of `{}`: {}", call.name, error);
                }
                let id = if call.id.is_empty() { new_call_id(i) } else { call.id };
                Call::new(id, call.name, args, args_error)
            })
            .collect();
        Ok(Turn {
            reply: Reply {
                text: if text.trim().is_empty() { None } else { Some(text) },
                reasoning: if reasoning.trim().is_empty() {
                    None
                } else {
                    Some(json!(reasoning))
                },
                calls,
            },
            usage,
        })
    }

    fn claude_blocking(
        &self,
        url: &str,
        api_key: &str,
        model: &str,
        system: &str,
        msgs: &[Msg],
        tools: &[ToolDef],
    ) -> Result<Turn, String> {
        let body = claude_body(model, system, msgs, tools, false);
        let response = http_client()?
            .post(url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .map_err(friendly_net_error)?;
        let status = response.status();
        let text = response.text().map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(http_error(status, &text));
        }
        claude_turn_from_body(&parse_json_body(&text)?)
    }

    fn claude_stream(
        &self,
        url: &str,
        api_key: &str,
        model: &str,
        system: &str,
        msgs: &[Msg],
        tools: &[ToolDef],
        on_stream: &mut dyn FnMut(Stream),
    ) -> Result<Turn, String> {
        let body = claude_body(model, system, msgs, tools, true);
        let response = http_client()?
            .post(url)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .body(body.to_string())
            .send()
            .map_err(friendly_net_error)?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().unwrap_or_default();
            return Err(http_error(status, &text));
        }

        if is_json_response(&response) {
            // the provider ignored `stream`, reuse the body as a blocking answer
            let body = response.text().map_err(|e| e.to_string())?;
            return claude_turn_from_body(&parse_json_body(&body)?);
        }

        #[derive(Default, Clone)]
        struct Block {
            kind: String,
            raw: Value,
            text: String,
            thinking: String,
            signature: String,
            id: String,
            name: String,
            json: String,
        }

        let mut blocks: Vec<Block> = vec![];
        let mut usage = Usage::default();
        let mut stream_error: Option<String> = None;

        read_sse(response, |data| {
            let Ok(value) = serde_json::from_str::<Value>(data) else {
                return;
            };
            match value["type"].as_str().unwrap_or_default() {
                "error" => {
                    stream_error = Some(format!("接口返回错误: {}", value["error"]));
                },
                "message_start" => {
                    if !value["message"]["usage"].is_null() {
                        absorb_usage(&mut usage, &value["message"]["usage"]);
                    }
                },
                "message_delta" => {
                    if !value["usage"].is_null() {
                        absorb_usage(&mut usage, &value["usage"]);
                    }
                },
                "content_block_start" => {
                    let index = value["index"].as_u64().unwrap_or(0) as usize;
                    while blocks.len() <= index {
                        blocks.push(Block::default());
                    }
                    let block = &value["content_block"];
                    let slot = &mut blocks[index];
                    slot.kind = block["type"].as_str().unwrap_or_default().to_string();
                    slot.raw = block.clone();
                    slot.id = block["id"].as_str().unwrap_or_default().to_string();
                    slot.name = block["name"].as_str().unwrap_or_default().to_string();
                },
                "content_block_delta" => {
                    let index = value["index"].as_u64().unwrap_or(0) as usize;
                    while blocks.len() <= index {
                        blocks.push(Block::default());
                    }
                    let delta = &value["delta"];
                    let slot = &mut blocks[index];
                    match delta["type"].as_str().unwrap_or_default() {
                        "text_delta" => {
                            if let Some(part) = delta["text"].as_str() {
                                slot.text.push_str(part);
                                on_stream(Stream::Delta(part.to_string()));
                            }
                        },
                        "thinking_delta" => {
                            if let Some(part) = delta["thinking"].as_str() {
                                slot.thinking.push_str(part);
                                on_stream(Stream::Thinking);
                            }
                        },
                        "signature_delta" => {
                            if let Some(part) = delta["signature"].as_str() {
                                slot.signature.push_str(part);
                            }
                        },
                        "input_json_delta" => {
                            if let Some(part) = delta["partial_json"].as_str() {
                                slot.json.push_str(part);
                            }
                        },
                        _ => {},
                    }
                },
                _ => {},
            }
        })?;

        if let Some(error) = stream_error {
            return Err(error);
        }

        let mut content: Vec<Value> = vec![];
        let mut calls: Vec<Call> = vec![];
        for (i, block) in blocks.iter().enumerate() {
            match block.kind.as_str() {
                "text" => {
                    if !block.text.trim().is_empty() {
                        content.push(json!({"type": "text", "text": block.text}));
                    }
                },
                "thinking" | "redacted_thinking" => {
                    // keep the original block shape (redacted blocks carry `data`)
                    let mut raw = block.raw.clone();
                    if !raw.is_object() {
                        raw = json!({"type": "thinking", "thinking": ""});
                    }
                    raw["thinking"] = json!(block.thinking);
                    if !block.signature.is_empty() {
                        raw["signature"] = json!(block.signature);
                    }
                    content.push(raw);
                },
                "tool_use" => {
                    if block.name.is_empty() {
                        continue;
                    }
                    let raw = if block.json.trim().is_empty() {
                        Value::Object(Default::default())
                    } else {
                        Value::String(block.json.clone())
                    };
                    let (args, args_error) = parse_arguments(&raw);
                    if let Some(error) = &args_error {
                        eprintln!("[ai] cannot parse input of `{}`: {}", block.name, error);
                    }
                    let id = if block.id.is_empty() {
                        new_call_id(i)
                    } else {
                        block.id.clone()
                    };
                    calls.push(Call::new(id, block.name.clone(), args, args_error));
                },
                _ => {},
            }
        }
        let reply = parse_claude_blocks(&content);
        Ok(Turn {
            reply: Reply { calls, ..reply },
            usage,
        })
    }
}
