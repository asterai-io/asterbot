use crate::bindings::asterai::fs::fs;
use crate::bindings::asterai::llm::llm::{
    chat, ChatMessage, ChatRole, ToolCall, ToolDefinition,
};
use crate::bindings::exports::asterbot::types::history::Guest;
use serde::{Deserialize, Serialize};

const HISTORY_FILE: &str = "conversation.json";
const DEFAULT_COMPACTION_THRESHOLD: usize = 50;
const DEFAULT_KEEP_RECENT_TURNS: usize = 10;
const TOOL_RESULT_PREVIEW_CHARS: usize = 200;

#[allow(warnings)]
mod bindings {
    wit_bindgen::generate!({
        path: "wit/package.wasm",
        world: "component",
        generate_all,
    });
}

struct Component;

/// The complete persisted state, stored as a single
/// conversation.json file. The `history` array is
/// append-only and never truncated.
#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ConversationState {
    /// The full conversation history (never truncated).
    history: Vec<PersistedMessage>,
    /// Index into `history`: messages before this index
    /// have been summarised into the context fields below.
    #[serde(default)]
    compacted_through: usize,
    /// Rolling summary of the conversation so far.
    #[serde(default)]
    conversation_summary: String,
    /// Observed profile of the user.
    #[serde(default)]
    user_summary: String,
    /// Notes on the user–assistant relationship.
    #[serde(default)]
    bond_summary: String,
}

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

/// Parsed output from the compaction tool call.
#[derive(Deserialize)]
struct CompactionResult {
    conversation_summary: String,
    user_profile: String,
    bond: String,
}

impl Guest for Component {
    fn load() -> Vec<ChatMessage> {
        let state = read_state();
        let start = state
            .compacted_through
            .min(state.history.len());
        state.history[start..]
            .iter()
            .map(|m| m.to_chat_message())
            .collect()
    }

    fn save(messages: Vec<ChatMessage>) {
        let mut state = read_state();
        let start = state
            .compacted_through
            .min(state.history.len());
        // Keep the archived portion, replace the working set.
        state.history.truncate(start);
        state.history.extend(
            messages
                .iter()
                .map(PersistedMessage::from_chat_message),
        );
        write_state(&state);
    }

    fn clear() {
        let _ = fs::rm(HISTORY_FILE, false);
    }

    fn get_context() -> String {
        let state = read_state();
        let mut parts = Vec::new();

        if !state.user_summary.is_empty() {
            parts.push(format!(
                "## User\n{}",
                state.user_summary,
            ));
        }
        if !state.conversation_summary.is_empty() {
            parts.push(format!(
                "## Conversation so far\n{}",
                state.conversation_summary,
            ));
        }
        if !state.bond_summary.is_empty() {
            parts.push(format!(
                "## Bond\n{}",
                state.bond_summary,
            ));
        }

        parts.join("\n\n")
    }

    fn should_compact(message_count: u32) -> bool {
        let threshold = std::env::var("ASTERBOT_COMPACTION_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_COMPACTION_THRESHOLD);
        message_count as usize >= threshold
    }

    fn compact(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
        let model =
            std::env::var("ASTERBOT_MODEL").unwrap_or_default();
        if model.is_empty() {
            return messages;
        }

        let keep_turns =
            std::env::var("ASTERBOT_KEEP_RECENT_TURNS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(DEFAULT_KEEP_RECENT_TURNS);

        let cut = find_cut_point(&messages, keep_turns);
        if cut == 0 {
            return messages;
        }

        let mut state = read_state();

        // Split into old (to summarise) and recent (to keep).
        let mut owned = messages;
        let recent = owned.split_off(cut);
        let old = owned;

        let formatted = format_messages_for_summary(&old);
        let prompt = build_compaction_prompt(
            &formatted,
            &state.conversation_summary,
            &state.user_summary,
            &state.bond_summary,
        );

        let tools = vec![build_compaction_tool()];
        let response = chat(&prompt, &tools, &model);

        // Parse the structured tool call response.
        if let Some(tc) = response.tool_calls.first() {
            match serde_json::from_str::<CompactionResult>(
                &tc.arguments_json,
            ) {
                Ok(result) => {
                    if !result.conversation_summary.is_empty() {
                        state.conversation_summary =
                            result.conversation_summary;
                    }
                    if !result.user_profile.is_empty() {
                        state.user_summary =
                            result.user_profile;
                    }
                    if !result.bond.is_empty() {
                        state.bond_summary = result.bond;
                    }
                }
                Err(e) => {
                    eprintln!(
                        "error: failed to parse compaction \
                         result: {e}"
                    );
                    let mut all = old;
                    all.extend(recent);
                    return all;
                }
            }
        } else {
            eprintln!(
                "error: compaction LLM did not return \
                 a tool call"
            );
            let mut all = old;
            all.extend(recent);
            return all;
        }

        // Advance the cursor.
        state.compacted_through += cut;
        write_state(&state);

        recent
    }
}

/// Find the cut point: keep the last `keep_turns` user
/// messages (and everything after them). Returns the
/// index of the oldest kept user message, or 0 if there
/// aren't enough turns to justify compaction.
fn find_cut_point(
    messages: &[ChatMessage],
    keep_turns: usize,
) -> usize {
    let user_indices: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| matches!(m.role, ChatRole::User))
        .map(|(i, _)| i)
        .collect();

    if user_indices.len() <= keep_turns {
        return 0;
    }

    // Cut at the oldest user message we want to keep.
    // Everything before it gets compacted.
    user_indices[user_indices.len() - keep_turns]
}

fn format_messages_for_summary(
    messages: &[ChatMessage],
) -> String {
    let mut out = String::new();
    for msg in messages {
        match msg.role {
            ChatRole::System => {}
            ChatRole::User => {
                out.push_str(&format!(
                    "[user]: {}\n\n",
                    msg.content,
                ));
            }
            ChatRole::Assistant => {
                if !msg.content.is_empty() {
                    out.push_str(&format!(
                        "[assistant]: {}\n\n",
                        msg.content,
                    ));
                }
                for tc in &msg.tool_calls {
                    let args = truncate_str(
                        &tc.arguments_json,
                        TOOL_RESULT_PREVIEW_CHARS,
                    );
                    out.push_str(&format!(
                        "[tool_call]: {}({})\n\n",
                        tc.name, args,
                    ));
                }
            }
            ChatRole::Tool => {
                let preview = truncate_str(
                    &msg.content,
                    TOOL_RESULT_PREVIEW_CHARS,
                );
                out.push_str(&format!(
                    "[tool_result]: {preview}\n\n",
                ));
            }
        }
    }
    out
}

fn build_compaction_tool() -> ToolDefinition {
    ToolDefinition {
        name: "update_context".to_string(),
        description: "Update the long-term conversation \
            context with summarised information."
            .to_string(),
        parameters_json_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "conversation_summary": {
                    "type": "string",
                    "description": "Concise narrative of the \
                        full conversation so far, incorporating \
                        the previous summary and new messages. \
                        Replace the previous summary entirely. \
                        Focus on: topics discussed, decisions \
                        made, tasks completed, and outstanding \
                        threads."
                },
                "user_profile": {
                    "type": "string",
                    "description": "Updated profile of the \
                        user. Merge new observations with \
                        existing. Include only clearly evidenced \
                        facts: name, role, background, \
                        preferences, technical level, \
                        communication style. Return existing \
                        unchanged if nothing new was learned."
                },
                "bond": {
                    "type": "string",
                    "description": "Updated notes on the \
                        user-assistant relationship. \
                        Communication patterns, shared \
                        references, humor, trust dynamics. \
                        Merge with existing. Return existing \
                        unchanged if nothing new."
                }
            },
            "required": [
                "conversation_summary",
                "user_profile",
                "bond"
            ]
        })
        .to_string(),
    }
}

fn build_compaction_prompt(
    formatted_messages: &str,
    existing_summary: &str,
    existing_user: &str,
    existing_bond: &str,
) -> Vec<ChatMessage> {
    let system = ChatMessage {
        role: ChatRole::System,
        content: "You are maintaining the long-term context \
            of an ongoing conversation between a user and an \
            AI assistant. Given the messages below and any \
            existing context, call the update_context tool \
            with three updated fields.\n\n\
            Be concise but thorough. Preserve important \
            details. If nothing new was learned for a field, \
            return the existing content unchanged."
            .to_string(),
        tool_calls: Vec::new(),
        tool_call_id: None,
    };

    fn or_none(s: &str) -> &str {
        if s.is_empty() { "None yet." } else { s }
    }

    let user_msg = ChatMessage {
        role: ChatRole::User,
        content: format!(
            "[Previous conversation summary:]\n{}\n\n\
             [Existing user profile:]\n{}\n\n\
             [Existing bond notes:]\n{}\n\n\
             [Messages to process:]\n{}",
            or_none(existing_summary),
            or_none(existing_user),
            or_none(existing_bond),
            formatted_messages,
        ),
        tool_calls: Vec::new(),
        tool_call_id: None,
    };

    vec![system, user_msg]
}

fn read_state() -> ConversationState {
    let bytes = match fs::read(HISTORY_FILE) {
        Ok(b) => b,
        Err(_) => return ConversationState::default(),
    };
    let contents = match String::from_utf8(bytes) {
        Ok(s) if !s.trim().is_empty() => s,
        _ => return ConversationState::default(),
    };
    // If the file is malformed or an old format, start fresh.
    serde_json::from_str(&contents).unwrap_or_else(|e| {
        eprintln!(
            "warning: resetting {HISTORY_FILE} \
             (parse error: {e})"
        );
        ConversationState::default()
    })
}

fn write_state(state: &ConversationState) {
    match serde_json::to_string_pretty(state) {
        Ok(json) => {
            if let Err(e) =
                fs::write(HISTORY_FILE, json.as_bytes())
            {
                eprintln!(
                    "error: failed to write \
                     {HISTORY_FILE}: {e}"
                );
            }
        }
        Err(e) => {
            eprintln!(
                "error: failed to serialise state: {e}"
            );
        }
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

bindings::export!(Component with_types_in bindings);
