use crate::bindings::asterai::host::api;
use crate::bindings::asterai::llm::llm::{
    chat, ChatMessage, ChatRole, ToolCall, ToolDefinition,
};
use crate::bindings::exports::asterbot::types::core::Guest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_SUGGESTIONS: usize = 3;
const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant.";
const DEFAULT_MAX_TOOL_ROUNDS: usize = 10;
const TOOL_RESULT_TRUNCATE_CHARS: usize = 10_000;
// ~120k tokens at ~4 chars/token, leaving room for model response.
const DEFAULT_MAX_PROMPT_CHARS: usize = 500_000;

#[allow(warnings)]
mod bindings {
    wit_bindgen::generate!({
        path: "wit/package.wasm",
        world: "component",
        generate_all,
    });
}

struct Component;

/// Serializable version of ChatMessage for conversation.json persistence.
#[derive(Serialize, Deserialize, Clone)]
struct PersistedMessage {
    role: String,
    content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<PersistedToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct PersistedToolCall {
    id: String,
    name: String,
    arguments_json: String,
}

impl PersistedMessage {
    fn to_chat_message(&self) -> ChatMessage {
        let role = match self.role.as_str() {
            "system" => ChatRole::System,
            "user" => ChatRole::User,
            "assistant" => ChatRole::Assistant,
            "tool" => ChatRole::Tool,
            _ => ChatRole::User,
        };
        ChatMessage {
            role,
            content: self.content.clone(),
            tool_calls: self
                .tool_calls
                .iter()
                .map(|tc| ToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments_json: tc.arguments_json.clone(),
                })
                .collect(),
            tool_call_id: self.tool_call_id.clone(),
        }
    }

    fn from_chat_message(msg: &ChatMessage) -> Self {
        let role = match msg.role {
            ChatRole::System => "system",
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
            ChatRole::Tool => "tool",
        };
        PersistedMessage {
            role: role.to_string(),
            content: msg.content.clone(),
            tool_calls: msg
                .tool_calls
                .iter()
                .map(|tc| PersistedToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments_json: tc.arguments_json.clone(),
                })
                .collect(),
            tool_call_id: msg.tool_call_id.clone(),
        }
    }
}

struct ToolEntry {
    name: String,
    component: String,
    function: String,
    definition: ToolDefinition,
}

#[derive(Deserialize)]
struct ToolInfoJson {
    #[serde(rename = "component-name")]
    component_name: String,
    #[serde(rename = "function-name")]
    function_name: String,
    description: String,
    params: Vec<ToolParamJson>,
    #[serde(rename = "return-type")]
    _return_type: String,
}

#[derive(Deserialize)]
struct ToolParamJson {
    name: String,
    #[serde(rename = "type-name")]
    type_name: String,
}

impl Guest for Component {
    fn converse(input: String) -> String {
        let model = std::env::var("ASTERBOT_MODEL").unwrap_or_default();
        if model.is_empty() {
            return "error: ASTERBOT_MODEL env var is required".to_string();
        }
        let max_tool_rounds = std::env::var("ASTERBOT_MAX_TOOL_ROUNDS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_TOOL_ROUNDS);
        let host_dir = match resolve_host_dir() {
            Ok(d) => d,
            Err(e) => return e,
        };
        let mut history = load_history(&host_dir);
        let system_message = build_system_message(&host_dir, &input);
        let tools = get_tool_entries();
        history.push(ChatMessage {
            role: ChatRole::User,
            content: input,
            tool_calls: Vec::new(),
            tool_call_id: None,
        });
        let tool_defs: Vec<ToolDefinition> =
            tools.iter().map(|t| t.definition.clone()).collect();
        let mut rounds_remaining = max_tool_rounds;
        loop {
            let mut messages = vec![system_message.clone()];
            messages.extend(trim_history(&history).iter().cloned());
            let response = chat(&messages, &tool_defs, &model);
            if response.content.starts_with("error: ") && response.tool_calls.is_empty() {
                save_history(&host_dir, &history);
                return response.content;
            }
            if response.tool_calls.is_empty() {
                history.push(ChatMessage {
                    role: ChatRole::Assistant,
                    content: response.content.clone(),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                });
                save_history(&host_dir, &history);
                return response.content;
            }
            history.push(ChatMessage {
                role: ChatRole::Assistant,
                content: response.content.clone(),
                tool_calls: response.tool_calls.clone(),
                tool_call_id: None,
            });
            for tc in &response.tool_calls {
                let (component, function) = match resolve_tool_name(&tc.name, &tools) {
                    Some(cf) => cf,
                    None => {
                        history.push(ChatMessage {
                            role: ChatRole::Tool,
                            content: format!("error: unknown tool '{}'", tc.name),
                            tool_calls: Vec::new(),
                            tool_call_id: Some(tc.id.clone()),
                        });
                        continue;
                    }
                };
                let result = call_tool(&component, &function, &tc.arguments_json);
                let truncated = truncate_result(&result);
                history.push(ChatMessage {
                    role: ChatRole::Tool,
                    content: truncated,
                    tool_calls: Vec::new(),
                    tool_call_id: Some(tc.id.clone()),
                });
            }
            rounds_remaining -= 1;
            if rounds_remaining == 0 {
                let msg = "max tool rounds reached".to_string();
                history.push(ChatMessage {
                    role: ChatRole::Assistant,
                    content: msg.clone(),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                });
                save_history(&host_dir, &history);
                return msg;
            }
        }
    }
}

fn build_system_message(host_dir: &str, input: &str) -> ChatMessage {
    let mut content = resolve_system_prompt(host_dir);
    let soul = fetch_soul();
    let skill_hints = suggest_files("asterbot:skills", "skills/list-all", input);
    let memory_hints = suggest_files("asterbot:memory", "memory/list-all", input);
    if !soul.is_empty() {
        content.push_str("\n\nYour soul (personality & self-knowledge):\n");
        content.push_str(&soul);
    }
    if !skill_hints.is_empty() {
        content.push_str(
            "\n\nThe following skills may be relevant \
            (stored as .md files in skills/). \
            You can read, create, edit, or delete them using the tools.\n",
        );
        for name in &skill_hints {
            content.push_str(&format!("- {name}.md\n"));
        }
        content.push_str(
            "These are just the top matches. \
            You may list all available using the tools.",
        );
    }
    if !memory_hints.is_empty() {
        content.push_str(
            "\n\nThe following memories may be relevant \
            (stored as .md files in memory/). \
            You can read, create, edit, or delete them using the tools.\n",
        );
        for name in &memory_hints {
            content.push_str(&format!("- {name}.md\n"));
        }
        content.push_str(
            "These are just the top matches. \
            You may list all available using the tools.",
        );
    }
    ChatMessage {
        role: ChatRole::System,
        content,
        tool_calls: Vec::new(),
        tool_call_id: None,
    }
}

fn get_tool_entries() -> Vec<ToolEntry> {
    let tools_json =
        match api::call_component_function("asterbot:toolkit", "toolkit/list-tools", "[]") {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
    let tool_infos: Vec<ToolInfoJson> = serde_json::from_str(&tools_json).unwrap_or_default();
    tool_infos
        .into_iter()
        .map(|info| {
            let tool_name = encode_tool_name(&info.component_name, &info.function_name);
            let params_schema = build_params_schema(&info.params);
            ToolEntry {
                name: tool_name.clone(),
                component: info.component_name,
                function: info.function_name,
                definition: ToolDefinition {
                    name: tool_name,
                    description: info.description,
                    parameters_json_schema: params_schema,
                },
            }
        })
        .collect()
}

/// Encodes a component name and function name into a tool name
/// that is safe for LLM tool calling APIs.
/// e.g. "asterbot:memory" + "memory/get" → "asterbot-memory--memory-get"
fn encode_tool_name(component: &str, function: &str) -> String {
    let c = component.replace(':', "-");
    let f = function.replace('/', "-");
    format!("{c}--{f}")
}

fn resolve_tool_name(tool_name: &str, tools: &[ToolEntry]) -> Option<(String, String)> {
    tools
        .iter()
        .find(|t| t.name == tool_name)
        .map(|t| (t.component.clone(), t.function.clone()))
}

fn build_params_schema(params: &[ToolParamJson]) -> String {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for p in params {
        let is_option = p.type_name.starts_with("option<");
        let json_type = wit_type_to_json_type(&p.type_name);
        properties.insert(p.name.clone(), json_type);
        if !is_option {
            required.push(Value::String(p.name.clone()));
        }
    }
    let schema = serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
    });
    serde_json::to_string(&schema).unwrap_or_else(|_| r#"{"type":"object"}"#.to_string())
}

fn wit_type_to_json_type(wit_type: &str) -> Value {
    match wit_type {
        "string" => serde_json::json!({"type": "string"}),
        "bool" => serde_json::json!({"type": "boolean"}),
        "u8" | "u16" | "u32" | "u64" | "s8" | "s16" | "s32" | "s64" => {
            serde_json::json!({"type": "integer"})
        }
        "f32" | "f64" | "float32" | "float64" => {
            serde_json::json!({"type": "number"})
        }
        t if t.starts_with("list<") && t.ends_with('>') => {
            let inner = &t[5..t.len() - 1];
            serde_json::json!({
                "type": "array",
                "items": wit_type_to_json_type(inner)
            })
        }
        t if t.starts_with("option<") && t.ends_with('>') => {
            let inner = &t[7..t.len() - 1];
            wit_type_to_json_type(inner)
        }
        _ => serde_json::json!({"type": "string"}),
    }
}

fn truncate_result(result: &str) -> String {
    if result.len() <= TOOL_RESULT_TRUNCATE_CHARS {
        result.to_string()
    } else {
        format!(
            "{}... (truncated)",
            &result[..TOOL_RESULT_TRUNCATE_CHARS]
        )
    }
}

fn trim_history(history: &[ChatMessage]) -> &[ChatMessage] {
    let max_chars = std::env::var("ASTERBOT_MAX_PROMPT_CHARS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_PROMPT_CHARS);
    let max_user_messages_opt: Option<usize> = std::env::var("ASTERBOT_MAX_PROMPT_USER_MESSAGES")
        .ok()
        .and_then(|v| v.parse().ok());
    let mut start = 0;
    if let Some(max) = max_user_messages_opt {
        let mut user_count = 0;
        for (i, msg) in history.iter().enumerate().rev() {
            if matches!(msg.role, ChatRole::User) {
                user_count += 1;
                if user_count > max {
                    start = i + 1;
                    break;
                }
            }
        }
    }
    let mut total_chars = 0;
    for (i, msg) in history[start..].iter().enumerate().rev() {
        total_chars += msg.content.len();
        if total_chars > max_chars && i > 0 {
            start += i + 1;
            break;
        }
    }
    &history[start..]
}

fn fetch_soul() -> String {
    match api::call_component_function("asterbot:soul", "soul/get", "[]") {
        Ok(result) => {
            let content = decode_json_string(&result);
            if content.trim().is_empty() {
                String::new()
            } else {
                content
            }
        }
        Err(_) => String::new(),
    }
}

fn suggest_files(component: &str, list_fn: &str, input: &str) -> Vec<String> {
    let names: Vec<String> = match api::call_component_function(component, list_fn, "[]") {
        Ok(result) => serde_json::from_str(&result).unwrap_or_default(),
        Err(_) => return Vec::new(),
    };
    if names.is_empty() {
        return Vec::new();
    }
    let input_words: Vec<String> = input
        .split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() > 2)
        .collect();
    let mut scored: Vec<(usize, &String)> = names
        .iter()
        .map(|name| {
            let name_lower = name.to_lowercase();
            let score = input_words
                .iter()
                .filter(|w| name_lower.contains(w.as_str()))
                .count();
            (score, name)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, name)| name.clone())
        .collect()
}

fn resolve_host_dir() -> Result<String, String> {
    if let Ok(v) = std::env::var("ASTERBOT_HOST_DIR") {
        if !v.is_empty() {
            return Ok(v);
        }
    }
    if let Ok(dirs) = std::env::var("ASTERAI_ALLOWED_DIRS") {
        if let Some(first) = dirs.split(':').next() {
            if !first.is_empty() {
                return Ok(first.to_string());
            }
        }
    }
    Err(
        "error: no state directory available — pass --allow-dir to grant filesystem access"
            .to_string(),
    )
}

fn resolve_system_prompt(host_dir: &str) -> String {
    let path = format!("{host_dir}/SYSTEM_PROMPT.md");
    if let Ok(contents) = std::fs::read_to_string(&path) {
        if !contents.trim().is_empty() {
            return contents;
        }
    }
    std::env::var("ASTERBOT_SYSTEM_PROMPT").unwrap_or_else(|_| DEFAULT_SYSTEM_PROMPT.to_string())
}

fn decode_json_string(json: &str) -> String {
    serde_json::from_str::<String>(json).unwrap_or_else(|_| json.to_string())
}

fn call_tool(component: &str, function: &str, args: &str) -> String {
    let component_json = serde_json::to_string(component).unwrap_or_default();
    let function_json = serde_json::to_string(function).unwrap_or_default();
    let args_json = serde_json::to_string(args).unwrap_or_default();
    let call_args = format!("[{component_json}, {function_json}, {args_json}]");
    match api::call_component_function("asterbot:toolkit", "toolkit/call-tool", &call_args) {
        Ok(result) => decode_json_string(&result),
        Err(e) => format!("error: tool call failed: {:?}: {}", e.kind, e.message),
    }
}

fn load_history(host_dir: &str) -> Vec<ChatMessage> {
    let path = format!("{host_dir}/conversation.json");
    match std::fs::read_to_string(&path) {
        Ok(contents) if !contents.trim().is_empty() => {
            let persisted: Vec<PersistedMessage> =
                serde_json::from_str(&contents).unwrap_or_else(|e| {
                    eprintln!("error: failed to parse conversation.json: {e}");
                    Vec::new()
                });
            persisted.iter().map(|m| m.to_chat_message()).collect()
        }
        Ok(_) => Vec::new(),
        Err(_) => Vec::new(),
    }
}

fn save_history(host_dir: &str, history: &[ChatMessage]) {
    let path = format!("{host_dir}/conversation.json");
    let persisted: Vec<PersistedMessage> =
        history.iter().map(PersistedMessage::from_chat_message).collect();
    match serde_json::to_string_pretty(&persisted) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                eprintln!("error: failed to write conversation.json: {e}");
            }
        }
        Err(e) => eprintln!("error: failed to serialize history: {e}"),
    }
}

bindings::export!(Component with_types_in bindings);
