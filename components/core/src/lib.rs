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
const DEFAULT_MAX_MESSAGES: usize = 100;
const DEFAULT_MAX_PROMPT_CHARS: usize = 24000;
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
        let max_messages = std::env::var("ASTERBOT_MAX_MESSAGES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_MAX_MESSAGES);
        let host_dir = match resolve_host_dir() {
            Ok(d) => d,
            Err(e) => return e,
        };
        let mut history = load_history(&host_dir);
        history.push(Message {
            role: "user".to_string(),
            content: input,
        });
        trim_history(&mut history, max_messages);
        let tool_descriptions = get_tool_descriptions();
        let system_prompt = resolve_system_prompt(&host_dir);
        let mut rounds_remaining = max_tool_rounds;
        loop {
            let prompt = build_prompt(&system_prompt, &tool_descriptions, &history);
            let response = match call_llm(&prompt, &model) {
                Ok(r) => r,
                Err(e) => {
                    let msg = format!("error: LLM call failed: {e}");
                    save_history(&host_dir, &history);
                    return msg;
                }
            };
            let Some(tc) = parse_tool_call(&response) else {
                history.push(Message {
                    role: "assistant".to_string(),
                    content: response.clone(),
                });
                save_history(&host_dir, &history);
                return response;
            };
            history.push(Message {
                role: "tool_call".to_string(),
                content: format!("{} / {} {}", tc.component, tc.function, tc.args),
            });
            let tool_result = call_tool(&tc.component, &tc.function, &tc.args);
            history.push(Message {
                role: "tool_result".to_string(),
                content: tool_result.clone(),
            });
            rounds_remaining -= 1;
            if rounds_remaining == 0 {
                let msg = format!("{}\n\n(max tool rounds reached)", tool_result,);
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

fn build_prompt(system_prompt: &str, tool_descriptions: &str, history: &[Message]) -> String {
    let max_chars = std::env::var("ASTERBOT_MAX_PROMPT_CHARS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_PROMPT_CHARS);
    let mut preamble = system_prompt.to_string();
    if !tool_descriptions.is_empty() && tool_descriptions != "No tools available." {
        preamble.push_str(
            "\n\n\
            You have access to tools. To call a tool, respond with\n\
            exactly one XML block:\n\
            \n\
            <tool_call>\n\
            <component>component-name</component>\n\
            <function>interface/function</function>\n\
            <args>{\"key\": \"value\"}</args>\n\
            </tool_call>\n\
            \n\
            After a tool call, you will receive the result and can\n\
            then respond to the user or call another tool.\n\
            \n",
        );
        preamble.push_str(tool_descriptions);
    }
    preamble.push_str("\n\nConversation:\n");
    let remaining = max_chars.saturating_sub(preamble.len());
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
    let mut prompt = preamble;
    for line in &lines {
        prompt.push_str(line);
    }
    prompt
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
    let path = format!("{host_dir}/system-prompt.txt");
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

struct ToolCall {
    component: String,
    function: String,
    args: String,
}

fn parse_tool_call(response: &str) -> Option<ToolCall> {
    let block = extract_tag(response, "tool_call")?;
    Some(ToolCall {
        component: extract_tag(&block, "component")?,
        function: extract_tag(&block, "function")?,
        args: extract_tag(&block, "args").unwrap_or_else(|| "{}".to_string()),
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

fn trim_history(history: &mut Vec<Message>, max: usize) {
    if history.len() > max {
        let drop = history.len() - max;
        history.drain(..drop);
    }
}

bindings::export!(Component with_types_in bindings);
