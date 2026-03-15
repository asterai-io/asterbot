use crate::bindings::asterai::host::api;
use crate::bindings::asterai::llm::llm::{
    chat, ChatMessage, ChatRole, ToolCall, ToolDefinition,
};
use crate::bindings::exports::asterbot::types::core::Guest;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_SUGGESTIONS: usize = 3;
const DEFAULT_SYSTEM_PROMPT: &str = "\
You are a personal AI assistant running inside Asterbot.

Be direct, concise, and genuinely helpful. Don't narrate routine \
actions — just do them. Be resourceful: check your memory, read \
files, and search for context before asking the user. Admit when \
you don't know something rather than guessing. Have a point of \
view — you're not a generic search engine.";
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

/// WIT JSON encoding of ChatMessage for the dynamic call boundary.
/// Uses kebab-case field names to match the WIT component model.
#[derive(Serialize, Deserialize)]
struct WitChatMessage {
    role: String,
    content: String,
    #[serde(rename = "tool-calls", default, skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<WitToolCall>,
    #[serde(rename = "tool-call-id", default, skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct WitToolCall {
    id: String,
    name: String,
    #[serde(rename = "arguments-json")]
    arguments_json: String,
}

impl WitChatMessage {
    fn from_chat_message(msg: &ChatMessage) -> Self {
        let role = match msg.role {
            ChatRole::System => "system",
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
            ChatRole::Tool => "tool",
        };
        WitChatMessage {
            role: role.to_string(),
            content: msg.content.clone(),
            tool_calls: msg
                .tool_calls
                .iter()
                .map(|tc| WitToolCall {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments_json: tc.arguments_json.clone(),
                })
                .collect(),
            tool_call_id: msg.tool_call_id.clone(),
        }
    }

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
        let mut history = load_history();
        // TODO: Run compaction asynchronously after response
        // delivery to avoid blocking the user. Consider
        // asterai:host-cron for deferred execution.
        if should_compact_history(history.len()) {
            history = compact_history(history);
        }
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
                save_history(&history);
                return response.content;
            }
            if response.tool_calls.is_empty() {
                history.push(ChatMessage {
                    role: ChatRole::Assistant,
                    content: response.content.clone(),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                });
                save_history(&history);
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
            if rounds_remaining >= 1 && rounds_remaining <= 2 {
                let note = match rounds_remaining {
                    1 => "\n\n[System: final tool round. \
                    Provide your response to the user now.]".to_owned(),
                    x => format!("\n\n[System: {x} tool rounds remaining. \
                    Begin wrapping up.]"),
                };
                if let Some(last) = history.last_mut() {
                    last.content.push_str(&note);
                }
            }
            if rounds_remaining == 0 {
                let msg = "max tool rounds reached".to_string();
                history.push(ChatMessage {
                    role: ChatRole::Assistant,
                    content: msg.clone(),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                });
                save_history(&history);
                return msg;
            }
        }
    }
}

fn build_system_message(host_dir: &str, input: &str) -> ChatMessage {
    let mut content = resolve_system_prompt(host_dir);
    let model = std::env::var("ASTERBOT_MODEL").unwrap_or_default();
    let soul = fetch_soul();
    let memory_names = list_component_files("asterbot:memory", "memory/list-all");
    let skill_names = list_component_files("asterbot:skills", "skills/list-all");

    // Context awareness.
    content.push_str("\n\n## Context\n");
    if !model.is_empty() {
        content.push_str(&format!("Model: {model}\n"));
    }
    content.push_str(
        "Your conversation history is persisted across sessions. \
        However, older messages may be trimmed from context when \
        conversations get long — rely on your memory tools for \
        important information rather than assuming old messages \
        are still visible.",
    );

    // Soul.
    if let Some(soul_content) = &soul {
        content.push_str("\n\n## Soul\n");
        content.push_str(
            "Your soul defines your personality and self-knowledge. \
            Embody it. You may evolve it as you learn about yourself, \
            but do so thoughtfully.\n",
        );
        if !soul_content.is_empty() {
            content.push_str("\n");
            content.push_str(soul_content);
        }
    }
    // History context (user profile, conversation summary, bond).
    let history_context = get_history_context();
    if !history_context.is_empty() {
        content.push_str("\n\n");
        content.push_str(&history_context);
    }
    // Tool guidance.
    content.push_str(
        "\n\n## Tool usage\n\
        Use your tools proactively and efficiently:\n\
        - Verify before guessing. Read before writing. Search before asking.\n\
        - Prefer the simplest approach. Don't chain unnecessary tool calls.\n\
        - If a tool call fails, try a different approach rather than \
        retrying the same thing.",
    );

    // Memory (mandatory).
    if let Some(names) = &memory_names {
        content.push_str(
            "\n\n## Memory\n\
            You have persistent memory across conversations, stored as \
            named .md files you can read, create, update, and delete.\n\n\
            MANDATORY: Before answering questions about prior conversations, \
            user preferences, past decisions, or anything that might have \
            been discussed before, check your memories first.\n\n\
            Proactively save important things you learn — user preferences, \
            key decisions, useful context. Don't wait to be asked. Update \
            or remove stale memories rather than letting them accumulate.",
        );
        let hints = suggest_from_names(names, input);
        if !hints.is_empty() {
            content.push_str("\n\nMemories that may be relevant:\n");
            for name in &hints {
                content.push_str(&format!("- {name}\n"));
            }
            content.push_str(
                "These are the top matches. Use the list tool to see all.",
            );
        }
    }

    // Skills (mandatory).
    if let Some(names) = &skill_names {
        content.push_str(
            "\n\n## Skills\n\
            You have a persistent skill library, stored as named .md \
            files you can read, create, update, and delete.\n\n\
            MANDATORY: Before attempting a complex or multi-step task, \
            check if a relevant skill exists.\n\n\
            After completing a complex workflow (3+ tool calls), \
            consider saving the approach as a skill for future use.",
        );
        let hints = suggest_from_names(names, input);
        if !hints.is_empty() {
            content.push_str("\n\nSkills that may be relevant:\n");
            for name in &hints {
                content.push_str(&format!("- {name}\n"));
            }
            content.push_str(
                "These are the top matches. Use the list tool to see all.",
            );
        }
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

fn get_history_context() -> String {
    match api::call_component_function("asterbot:history", "history/get-context", "[]") {
        Ok(result) => decode_json_string(&result),
        Err(_) => String::new(),
    }
}

fn fetch_soul() -> Option<String> {
    match api::call_component_function("asterbot:soul", "soul/get", "[]") {
        Ok(result) => {
            let content = decode_json_string(&result);
            Some(content.trim().to_string())
        }
        Err(_) => None,
    }
}

/// Returns Some(names) if the component is available, None if not.
fn list_component_files(component: &str, list_fn: &str) -> Option<Vec<String>> {
    match api::call_component_function(component, list_fn, "[]") {
        Ok(result) => Some(serde_json::from_str(&result).unwrap_or_default()),
        Err(_) => None,
    }
}

/// Rank file names by keyword overlap with the user input.
fn suggest_from_names(names: &[String], input: &str) -> Vec<String> {
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

fn load_history() -> Vec<ChatMessage> {
    match api::call_component_function("asterbot:history", "history/load", "[]") {
        Ok(json) => {
            let msgs: Vec<WitChatMessage> =
                serde_json::from_str(&json).unwrap_or_else(|e| {
                    eprintln!("error: failed to parse history from component: {e}");
                    Vec::new()
                });
            msgs.iter().map(|m| m.to_chat_message()).collect()
        }
        Err(e) => {
            eprintln!("error: failed to load history: {:?}: {}", e.kind, e.message);
            Vec::new()
        }
    }
}

fn should_compact_history(count: usize) -> bool {
    let args = format!("[{}]", count);
    match api::call_component_function(
        "asterbot:history",
        "history/should-compact",
        &args,
    ) {
        Ok(result) => result.trim() == "true",
        Err(_) => false,
    }
}

fn compact_history(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let wit_msgs: Vec<WitChatMessage> = messages
        .iter()
        .map(WitChatMessage::from_chat_message)
        .collect();
    let json = serde_json::to_string(&wit_msgs).unwrap_or_default();
    let args = format!("[{json}]");
    match api::call_component_function(
        "asterbot:history",
        "history/compact",
        &args,
    ) {
        Ok(result) => {
            let msgs: Vec<WitChatMessage> =
                serde_json::from_str(&result).unwrap_or_default();
            msgs.iter().map(|m| m.to_chat_message()).collect()
        }
        Err(e) => {
            eprintln!(
                "error: compaction failed: {:?}: {}",
                e.kind, e.message,
            );
            messages
        }
    }
}

fn save_history(history: &[ChatMessage]) {
    let msgs: Vec<WitChatMessage> =
        history.iter().map(WitChatMessage::from_chat_message).collect();
    let json = serde_json::to_string(&msgs).unwrap_or_default();
    let args = format!("[{json}]");
    if let Err(e) = api::call_component_function("asterbot:history", "history/save", &args) {
        eprintln!("error: failed to save history: {:?}: {}", e.kind, e.message);
    }
}

bindings::export!(Component with_types_in bindings);
