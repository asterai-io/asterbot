use crate::bindings::asterai::host::api;
use crate::bindings::exports::asterbot::types::core::Guest;
use serde::{Deserialize, Serialize};

#[allow(warnings)]
mod bindings {
    wit_bindgen::generate!({
        path: "wit/package.wasm",
        world: "component",
        generate_all,
    });
}

struct Component;

#[derive(Serialize, Deserialize, Clone)]
struct Message {
    role: String,
    content: String,
}

const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant.";
const DEFAULT_MAX_TOOL_ROUNDS: usize = 10;
// ~120k tokens at ~4 chars/token, leaving room for model response.
const DEFAULT_MAX_PROMPT_CHARS: usize = 500_000;
const TOOL_RESULT_TRUNCATE_CHARS: usize = 2000;

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
        history.push(Message {
            role: "user".to_string(),
            content: input,
        });
        let context = build_context(&host_dir);
        let mut rounds_remaining = max_tool_rounds;
        let mut accumulated_text: Vec<String> = Vec::new();
        loop {
            let prompt = build_prompt(&context, &history);
            let response = match call_llm(&prompt, &model) {
                Ok(r) => r,
                Err(e) => {
                    let msg = format!("error: LLM call failed: {e}");
                    save_history(&host_dir, &history);
                    return msg;
                }
            };
            let Some(parsed) = parse_tool_calls(&response) else {
                let mut final_response = response.clone();
                if !accumulated_text.is_empty() {
                    final_response =
                        format!("{}\n\n{}", accumulated_text.join("\n\n"), final_response);
                }
                history.push(Message {
                    role: "assistant".to_string(),
                    content: final_response.clone(),
                });
                save_history(&host_dir, &history);
                return final_response;
            };
            if let Some(text) = &parsed.surrounding_text {
                accumulated_text.push(text.clone());
                history.push(Message {
                    role: "assistant".to_string(),
                    content: text.clone(),
                });
            }
            for tc in &parsed.tool_calls {
                history.push(Message {
                    role: "assistant".to_string(),
                    content: format!(
                        "<tool_call>\n<component>{}</component>\n<function>{}</function>\n<args>{}</args>\n</tool_call>",
                        tc.component, tc.function, tc.args
                    ),
                });
                let tool_result = call_tool(&tc.component, &tc.function, &tc.args);
                history.push(Message {
                    role: "tool_result".to_string(),
                    content: tool_result.clone(),
                });
            }
            rounds_remaining -= 1;
            if rounds_remaining == 0 {
                let mut msg = "max tool rounds reached".to_string();
                if !accumulated_text.is_empty() {
                    msg = format!("{}\n\n{}", accumulated_text.join("\n\n"), msg);
                }
                history.push(Message {
                    role: "assistant".to_string(),
                    content: msg.clone(),
                });
                save_history(&host_dir, &history);
                return msg;
            }
        }
    }
}

fn build_context(host_dir: &str) -> String {
    let system_prompt = resolve_system_prompt(host_dir);
    let tool_descriptions = get_tool_descriptions();
    let soul = fetch_soul();
    let skills = fetch_skills();
    let memory_mention = format_memory_mention();
    let mut context = system_prompt;
    if !tool_descriptions.is_empty() && tool_descriptions != "No tools available." {
        context.push_str(
            "\n\n\
            You have access to tools. To call a tool, use XML blocks:\n\
            \n\
            <tool_call>\n\
            <component>component-name</component>\n\
            <function>interface/function</function>\n\
            <args>{\"key\": \"value\"}</args>\n\
            </tool_call>\n\
            \n\
            You can make multiple tool calls in a single response.\n\
            After tool calls, you will receive the results and can\n\
            then respond to the user or call more tools.\n\
            \n",
        );
        context.push_str(&tool_descriptions);
    }
    if !soul.is_empty() {
        context.push_str("\n\nYour soul (personality & self-knowledge):\n");
        context.push_str(&soul);
    }
    if !skills.is_empty() {
        context.push_str("\n\nYour skills:\n");
        context.push_str(&skills);
    }
    if !memory_mention.is_empty() {
        context.push_str("\n\n");
        context.push_str(&memory_mention);
    }
    context
}

fn build_prompt(context: &str, history: &[Message]) -> String {
    let max_chars = std::env::var("ASTERBOT_MAX_PROMPT_CHARS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_PROMPT_CHARS);
    let mut prompt = context.to_string();
    prompt.push_str("\n\nConversation:\n");
    let remaining = max_chars.saturating_sub(prompt.len());
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0;
    for msg in history.iter().rev() {
        let content = if msg.role == "tool_result" && msg.content.len() > TOOL_RESULT_TRUNCATE_CHARS
        {
            format!(
                "{}... (truncated)",
                &msg.content[..TOOL_RESULT_TRUNCATE_CHARS]
            )
        } else {
            msg.content.clone()
        };
        let line = format!("{}: {}\n", msg.role, content);
        if used + line.len() > remaining && !lines.is_empty() {
            break;
        }
        used += line.len();
        lines.push(line);
    }
    lines.reverse();
    for line in &lines {
        prompt.push_str(line);
    }
    prompt
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

fn fetch_skills() -> String {
    let names: Vec<String> =
        match api::call_component_function("asterbot:skills", "skills/list-all", "[]") {
            Ok(result) => serde_json::from_str(&result).unwrap_or_default(),
            Err(_) => return String::new(),
        };
    if names.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for name in &names {
        let arg = serde_json::to_string(name).unwrap_or_default();
        let args = format!("[{arg}]");
        if let Ok(result) = api::call_component_function("asterbot:skills", "skills/get", &args) {
            let content = decode_json_string(&result);
            if !content.trim().is_empty() {
                out.push_str(&format!("\n### {name}\n{content}\n"));
            }
        }
    }
    out
}

fn format_memory_mention() -> String {
    let names: Vec<String> =
        match api::call_component_function("asterbot:memory", "memory/list-all", "[]") {
            Ok(result) => serde_json::from_str(&result).unwrap_or_default(),
            Err(_) => return String::new(),
        };
    if names.is_empty() {
        return "You have persistent memory available. Use the memory tools to store and retrieve information across conversations.".to_string();
    }
    let list = names.join(", ");
    format!("You have persistent memory files: {list}. Use memory/get to retrieve them or memory/set to update them.")
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
                // TODO: check this is the correct dir:
                // prioritise by dirs that include known files
                // rather than returning first one.
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

fn get_tool_descriptions() -> String {
    match api::call_component_function("asterbot:toolkit", "toolkit/format-tools-for-prompt", "[]")
    {
        Ok(result) => decode_json_string(&result),
        Err(_) => String::new(),
    }
}

fn call_llm(prompt: &str, model: &str) -> Result<String, String> {
    let prompt_json = serde_json::to_string(prompt).map_err(|e| e.to_string())?;
    let model_json = serde_json::to_string(model).map_err(|e| e.to_string())?;
    let args = format!("[{prompt_json}, {model_json}]");
    api::call_component_function("asterai:llm", "llm/prompt", &args)
        .map(|r| decode_json_string(&r))
        .map_err(|e| format!("{:?}: {}", e.kind, e.message))
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

struct ParsedResponse {
    tool_calls: Vec<ToolCall>,
    surrounding_text: Option<String>,
}

struct ToolCall {
    component: String,
    function: String,
    args: String,
}

fn parse_tool_calls(response: &str) -> Option<ParsedResponse> {
    let mut tool_calls = Vec::new();
    let mut text_parts: Vec<String> = Vec::new();
    let mut remaining = response;
    loop {
        let Some(open_start) = remaining.find("<tool_call>") else {
            let trimmed = remaining.trim();
            if !trimmed.is_empty() {
                text_parts.push(trimmed.to_string());
            }
            break;
        };
        let before = remaining[..open_start].trim();
        if !before.is_empty() {
            text_parts.push(before.to_string());
        }
        let content_start = open_start + "<tool_call>".len();
        let Some(close_offset) = remaining[content_start..].find("</tool_call>") else {
            let trimmed = remaining.trim();
            if !trimmed.is_empty() {
                text_parts.push(trimmed.to_string());
            }
            break;
        };
        let close_start = content_start + close_offset;
        let block = remaining[content_start..close_start].trim();
        if let Some(tc) = parse_single_tool_call(block) {
            tool_calls.push(tc);
        }
        remaining = &remaining[close_start + "</tool_call>".len()..];
    }
    if tool_calls.is_empty() {
        return None;
    }
    let surrounding_text = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join("\n"))
    };
    Some(ParsedResponse {
        tool_calls,
        surrounding_text,
    })
}

fn parse_single_tool_call(block: &str) -> Option<ToolCall> {
    Some(ToolCall {
        component: extract_tag(block, "component")?,
        function: extract_tag(block, "function")?,
        args: extract_tag(block, "args").unwrap_or_else(|| "{}".to_string()),
    })
}

fn extract_tag(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(text[start..end].trim().to_string())
}

fn load_history(host_dir: &str) -> Vec<Message> {
    let path = format!("{host_dir}/conversation.json");
    match std::fs::read_to_string(&path) {
        Ok(contents) if !contents.trim().is_empty() => serde_json::from_str(&contents)
            .unwrap_or_else(|e| {
                eprintln!("error: failed to parse conversation.json: {e}");
                Vec::new()
            }),
        Ok(_) => Vec::new(),
        Err(_) => Vec::new(),
    }
}

fn save_history(host_dir: &str, history: &[Message]) {
    let path = format!("{host_dir}/conversation.json");
    match serde_json::to_string_pretty(history) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                eprintln!("error: failed to write conversation.json: {e}");
            }
        }
        Err(e) => eprintln!("error: failed to serialize history: {e}"),
    }
}

bindings::export!(Component with_types_in bindings);
