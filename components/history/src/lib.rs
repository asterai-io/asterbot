use crate::bindings::asterai::fs::fs as fs;
use crate::bindings::asterai::llm::llm::{ChatMessage, ChatRole, ToolCall};
use crate::bindings::exports::asterbot::types::history::Guest;
use serde::{Deserialize, Serialize};

const HISTORY_FILE: &str = "conversation.json";

#[allow(warnings)]
mod bindings {
    wit_bindgen::generate!({
        path: "wit/package.wasm",
        world: "component",
        generate_all,
    });
}

struct Component;

/// JSON-serializable representation of a chat message.
#[derive(Serialize, Deserialize)]
struct PersistedMessage {
    role: String,
    content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<PersistedToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct PersistedToolCall {
    id: String,
    name: String,
    arguments_json: String,
}

impl PersistedMessage {
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

impl Guest for Component {
    fn load() -> Vec<ChatMessage> {
        let bytes = match fs::read(HISTORY_FILE) {
            Ok(b) => b,
            Err(_) => return Vec::new(),
        };
        let contents = match String::from_utf8(bytes) {
            Ok(s) if !s.trim().is_empty() => s,
            _ => return Vec::new(),
        };
        let persisted: Vec<PersistedMessage> =
            serde_json::from_str(&contents).unwrap_or_else(|e| {
                eprintln!("error: failed to parse {HISTORY_FILE}: {e}");
                Vec::new()
            });
        persisted.iter().map(|m| m.to_chat_message()).collect()
    }

    fn save(messages: Vec<ChatMessage>) {
        let persisted: Vec<PersistedMessage> =
            messages.iter().map(PersistedMessage::from_chat_message).collect();
        match serde_json::to_string_pretty(&persisted) {
            Ok(json) => {
                if let Err(e) = fs::write(HISTORY_FILE, json.as_bytes()) {
                    eprintln!("error: failed to write {HISTORY_FILE}: {e}");
                }
            }
            Err(e) => eprintln!("error: failed to serialize history: {e}"),
        }
    }

    fn clear() {
        let _ = fs::rm(HISTORY_FILE, false);
    }
}

bindings::export!(Component with_types_in bindings);
