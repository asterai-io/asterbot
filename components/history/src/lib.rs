#[cfg(not(test))]
use crate::bindings::asterai::fs::fs;
#[cfg(not(test))]
use crate::bindings::asterai::llm::llm::{chat, ChatMessage, ChatRole, ToolCall, ToolDefinition};
#[cfg(not(test))]
use crate::bindings::exports::asterbot::types::history::Guest;
use serde::{Deserialize, Serialize};

const HISTORY_FILENAME: &str = "conversation.json";
const DEFAULT_COMPACTION_THRESHOLD: usize = 50;
const TOOL_RESULT_PREVIEW_CHARS: usize = 200;

#[cfg(not(test))]
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
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct PersistedMessage {
    role: String,
    content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<PersistedToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct PersistedToolCall {
    id: String,
    name: String,
    arguments_json: String,
}

#[cfg(not(test))]
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

#[cfg(not(test))]
impl Guest for Component {
    fn load() -> Vec<ChatMessage> {
        let state = read_state();
        let start = state.compacted_through.min(state.history.len());
        state.history[start..]
            .iter()
            .map(|m| m.to_chat_message())
            .collect()
    }

    fn save(messages: Vec<ChatMessage>) {
        let mut state = read_state();
        let start = state.compacted_through.min(state.history.len());
        // Keep the archived portion, replace the working set.
        state.history.truncate(start);
        state
            .history
            .extend(messages.iter().map(PersistedMessage::from_chat_message));
        write_state(&state);
    }

    fn clear() {
        let _ = fs::rm(&history_path(), false);
    }

    fn get_context() -> String {
        let state = read_state();
        format_context(&state)
    }

    fn should_compact(message_count: u32) -> bool {
        let threshold = std::env::var("ASTERBOT_COMPACTION_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_COMPACTION_THRESHOLD);
        message_count as usize >= threshold
    }

    fn compact(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
        let model = std::env::var("ASTERBOT_MODEL").unwrap_or_default();
        if model.is_empty() || messages.is_empty() {
            return messages;
        }
        let mut state = read_state();
        // Compact the entire working set into summaries.
        let old = messages;
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
        let llm_ok = if let Some(tc) = response.tool_calls.first() {
            match serde_json::from_str::<CompactionResult>(&tc.arguments_json) {
                Ok(result) => {
                    if !result.conversation_summary.is_empty() {
                        state.conversation_summary = result.conversation_summary;
                    }
                    if !result.user_profile.is_empty() {
                        state.user_summary = result.user_profile;
                    }
                    if !result.bond.is_empty() {
                        state.bond_summary = result.bond;
                    }
                    true
                }
                Err(e) => {
                    eprintln!(
                        "error: failed to parse compaction \
                         result: {e}"
                    );
                    false
                }
            }
        } else {
            eprintln!(
                "error: compaction LLM did not return \
                 a tool call"
            );
            false
        };
        // Fallback: raw text summary when LLM fails.
        if !llm_ok {
            let fallback = truncate_str(&formatted, 500);
            if state.conversation_summary.is_empty() {
                state.conversation_summary = fallback;
            } else {
                state.conversation_summary = format!(
                    "{}\n\n[auto-compacted]\n{}",
                    state.conversation_summary, fallback,
                );
            }
        }
        // Advance cursor past all compacted messages.
        state.compacted_through += old.len();
        write_state(&state);
        vec![]
    }
}

fn format_context(state: &ConversationState) -> String {
    let mut parts = Vec::new();

    if !state.user_summary.is_empty() {
        parts.push(format!("## User\n{}", state.user_summary,));
    }
    if !state.conversation_summary.is_empty() {
        parts.push(format!(
            "## Conversation so far\n{}",
            state.conversation_summary,
        ));
    }
    if !state.bond_summary.is_empty() {
        parts.push(format!("## Bond\n{}", state.bond_summary,));
    }

    parts.join("\n\n")
}

#[cfg(not(test))]
fn format_messages_for_summary(messages: &[ChatMessage]) -> String {
    let mut out = String::new();
    for msg in messages {
        match msg.role {
            ChatRole::System => {}
            ChatRole::User => {
                out.push_str(&format!("[user]: {}\n\n", msg.content,));
            }
            ChatRole::Assistant => {
                if !msg.content.is_empty() {
                    out.push_str(&format!("[assistant]: {}\n\n", msg.content,));
                }
                for tc in &msg.tool_calls {
                    let args = truncate_str(&tc.arguments_json, TOOL_RESULT_PREVIEW_CHARS);
                    out.push_str(&format!("[tool_call]: {}({})\n\n", tc.name, args,));
                }
            }
            ChatRole::Tool => {
                let preview = truncate_str(&msg.content, TOOL_RESULT_PREVIEW_CHARS);
                out.push_str(&format!("[tool_result]: {preview}\n\n",));
            }
        }
    }
    out
}

#[cfg(not(test))]
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

#[cfg(not(test))]
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
        if s.is_empty() {
            "None yet."
        } else {
            s
        }
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

#[cfg(not(test))]
fn history_path() -> String {
    let dir = std::env::var("ASTERBOT_HOST_DIR")
        .or_else(|_| {
            std::env::var("ASTERAI_ALLOWED_DIRS")
                .map(|dirs| dirs.split(':').next().unwrap_or_default().to_string())
        })
        .unwrap_or_default();
    if dir.is_empty() {
        HISTORY_FILENAME.to_string()
    } else {
        format!("{dir}/{HISTORY_FILENAME}")
    }
}

#[cfg(not(test))]
fn read_state() -> ConversationState {
    let path = history_path();
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(_) => return ConversationState::default(),
    };
    let contents = match String::from_utf8(bytes) {
        Ok(s) if !s.trim().is_empty() => s,
        _ => return ConversationState::default(),
    };
    // If the file is malformed or an old format, start fresh.
    serde_json::from_str(&contents).unwrap_or_else(|e| {
        eprintln!("warning: resetting {path} (parse error: {e})");
        ConversationState::default()
    })
}

#[cfg(not(test))]
fn write_state(state: &ConversationState) {
    let path = history_path();
    match serde_json::to_string_pretty(state) {
        Ok(json) => {
            if let Err(e) = fs::write(&path, json.as_bytes()) {
                eprintln!("error: failed to write {path}: {e}");
            }
        }
        Err(e) => {
            eprintln!("error: failed to serialise state: {e}");
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

#[cfg(not(test))]
bindings::export!(Component with_types_in bindings);

#[cfg(test)]
mod tests {
    use super::*;

    fn user(content: &str) -> PersistedMessage {
        PersistedMessage {
            role: "user".to_string(),
            content: content.to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }
    }

    fn assistant(content: &str) -> PersistedMessage {
        PersistedMessage {
            role: "assistant".to_string(),
            content: content.to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }
    }

    /// Simulate what core does: load → compact → save.
    /// Returns the working set returned to core (empty after compaction).
    fn simulate_compact(state: &mut ConversationState) -> Vec<PersistedMessage> {
        let start = state.compacted_through.min(state.history.len());
        let working_set: Vec<PersistedMessage> = state.history[start..].to_vec();

        if working_set.is_empty() {
            return working_set;
        }

        state.compacted_through += working_set.len();
        state.conversation_summary = format!("Compacted through index {}", state.compacted_through);

        vec![]
    }

    /// Simulate what core does after compact: add new
    /// turn and save.
    fn simulate_save(state: &mut ConversationState, working_set: &[PersistedMessage]) {
        let start = state.compacted_through.min(state.history.len());
        state.history.truncate(start);
        state.history.extend_from_slice(working_set);
    }

    /// Simulate compact where the LLM call fails.
    /// Fallback: raw text summary, advance cursor.
    fn simulate_compact_llm_fails(state: &mut ConversationState) -> Vec<PersistedMessage> {
        let start = state.compacted_through.min(state.history.len());
        let working_set: Vec<PersistedMessage> = state.history[start..].to_vec();

        if working_set.is_empty() {
            return working_set;
        }

        // LLM fails → fallback raw summary, still advance.
        let fallback: String = working_set
            .iter()
            .map(|m| format!("[{}]: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");
        if state.conversation_summary.is_empty() {
            state.conversation_summary = fallback;
        } else {
            state.conversation_summary = format!(
                "{}\n\n[auto-compacted]\n{}",
                state.conversation_summary, fallback,
            );
        }

        state.compacted_through += working_set.len();
        vec![]
    }

    #[test]
    fn single_compaction_compacts_everything() {
        let mut state = ConversationState {
            history: vec![
                user("1"),
                assistant("r1"),
                user("2"),
                assistant("r2"),
                user("3"),
                assistant("r3"),
            ],
            ..Default::default()
        };

        let trimmed = simulate_compact(&mut state);

        // All 6 messages compacted
        assert_eq!(state.compacted_through, 6);
        assert!(trimmed.is_empty());

        // Core adds a new turn
        let mut working = trimmed;
        working.push(user("4"));
        working.push(assistant("r4"));
        simulate_save(&mut state, &working);

        // Full history preserved
        assert_eq!(state.history.len(), 8);
        assert_eq!(state.history[0].content, "1"); // archived
        assert_eq!(state.history[7].content, "r4"); // newest
    }

    #[test]
    fn multiple_compaction_cycles() {
        let mut state = ConversationState::default();
        let threshold = 10;

        // Build up 5 turns (10 messages)
        for i in 1..=5 {
            state.history.push(user(&format!("u{i}")));
            state.history.push(assistant(&format!("a{i}")));
        }

        // Cycle 1: compact all 10
        let trimmed = simulate_compact(&mut state);
        assert_eq!(state.compacted_through, 10);
        assert!(trimmed.is_empty());

        // Core adds 6 more turns
        let mut working = trimmed;
        for i in 6..=11 {
            working.push(user(&format!("u{i}")));
            working.push(assistant(&format!("a{i}")));
        }
        simulate_save(&mut state, &working);
        assert_eq!(state.history.len(), 22);

        // Cycle 2: working set is 12 msgs, compact all
        let working_len = state.history.len() - state.compacted_through;
        assert_eq!(working_len, 12);
        let trimmed = simulate_compact(&mut state);
        assert_eq!(state.compacted_through, 22);
        assert!(trimmed.is_empty());

        // Core adds 1 more turn
        let mut working = trimmed;
        working.push(user("u12"));
        working.push(assistant("a12"));
        simulate_save(&mut state, &working);

        // ALL messages still in history
        assert_eq!(state.history.len(), 24);
        assert_eq!(state.history[0].content, "u1");
        assert_eq!(state.history[23].content, "a12");
    }

    #[test]
    fn compaction_with_threshold_10_realistic() {
        let mut state = ConversationState::default();
        let threshold = 10;

        for turn in 1..=10 {
            state.history.push(user(&format!("u{turn}")));
            state.history.push(assistant(&format!("a{turn}")));

            let working_len = state.history.len() - state.compacted_through;

            if working_len >= threshold {
                let trimmed = simulate_compact(&mut state);
                simulate_save(&mut state, &trimmed);
            }
        }

        // Compaction fires at 10 msgs and compacts everything.
        // Then new turns accumulate until threshold again.
        assert!(
            state.compacted_through >= 10,
            "compacted_through should be >= 10, got {}",
            state.compacted_through,
        );
        assert_eq!(state.history.len(), 20);
        assert_eq!(state.history[0].content, "u1");
        assert_eq!(state.history[19].content, "a10");
    }

    #[test]
    fn compaction_never_loses_messages() {
        let mut state = ConversationState::default();

        for turn in 1..=20 {
            state.history.push(user(&format!("u{turn}")));
            state.history.push(assistant(&format!("a{turn}")));

            let working_len = state.history.len() - state.compacted_through;
            if working_len >= 10 {
                let trimmed = simulate_compact(&mut state);
                simulate_save(&mut state, &trimmed);
            }
        }

        // 40 messages, all preserved
        assert_eq!(state.history.len(), 40);
        for i in 1..=20 {
            let user_idx = (i - 1) * 2;
            assert_eq!(state.history[user_idx].content, format!("u{i}"),);
        }
    }

    #[test]
    fn llm_failure_still_advances_cursor() {
        let mut state = ConversationState::default();
        let threshold = 10;

        for turn in 1..=10 {
            state.history.push(user(&format!("u{turn}")));
            state.history.push(assistant(&format!("a{turn}")));

            let working_len = state.history.len() - state.compacted_through;

            if working_len >= threshold {
                let trimmed = simulate_compact_llm_fails(&mut state);
                simulate_save(&mut state, &trimmed);
            }
        }

        assert!(
            state.compacted_through > 0,
            "cursor should advance even when LLM fails, \
             got compacted_through={}",
            state.compacted_through,
        );
        // Fallback summary should exist
        assert!(!state.conversation_summary.is_empty());
    }

    #[test]
    fn working_set_stays_bounded_despite_llm_failures() {
        let mut state = ConversationState::default();
        let threshold = 10;

        for turn in 1..=15 {
            state.history.push(user(&format!("u{turn}")));
            state.history.push(assistant(&format!("a{turn}")));

            let working_len = state.history.len() - state.compacted_through;

            if working_len >= threshold {
                let trimmed = simulate_compact_llm_fails(&mut state);
                simulate_save(&mut state, &trimmed);
            }
        }

        let working_len = state.history.len() - state.compacted_through;
        assert!(
            working_len < threshold,
            "working set ({working_len}) should be < threshold ({threshold})",
        );
    }

    #[test]
    fn no_wasted_llm_calls() {
        // Since we compact everything, each compaction fully
        // resets the working set. Compaction should only fire
        // when threshold is hit again from scratch.
        let mut state = ConversationState::default();
        let threshold = 10;
        let mut compact_attempts = 0;

        for turn in 1..=15 {
            state.history.push(user(&format!("u{turn}")));
            state.history.push(assistant(&format!("a{turn}")));

            let working_len = state.history.len() - state.compacted_through;

            if working_len >= threshold {
                compact_attempts += 1;
                let trimmed = simulate_compact_llm_fails(&mut state);
                simulate_save(&mut state, &trimmed);
            }
        }

        // With full compaction, should only fire twice:
        // once at 10 msgs, once at 20 msgs (but we only have 15 turns = 30 msgs,
        // second fire at turn 10 after first compaction).
        assert!(
            compact_attempts <= 3,
            "compaction fired {compact_attempts} times — \
             expected at most 3 for 15 turns with threshold 10",
        );
    }

    #[test]
    fn state_serializes_camel_case() {
        let state = ConversationState {
            history: vec![user("hello"), assistant("hi")],
            compacted_through: 0,
            conversation_summary: "A greeting".into(),
            user_summary: "Friendly".into(),
            bond_summary: "New".into(),
        };
        let json = serde_json::to_string(&state).unwrap();

        assert!(json.contains("\"compactedThrough\""));
        assert!(json.contains("\"conversationSummary\""));
        assert!(json.contains("\"userSummary\""));
        assert!(json.contains("\"bondSummary\""));
    }

    #[test]
    fn state_round_trips() {
        let state = ConversationState {
            history: vec![user("hello"), assistant("hi")],
            compacted_through: 5,
            conversation_summary: "summary".into(),
            user_summary: "user".into(),
            bond_summary: "bond".into(),
        };
        let json = serde_json::to_string_pretty(&state).unwrap();
        let parsed: ConversationState = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.compacted_through, 5);
        assert_eq!(parsed.conversation_summary, "summary");
        assert_eq!(parsed.history.len(), 2);
    }

    #[test]
    fn old_array_format_fails_parse() {
        let old = r#"[{"role":"user","content":"hi"}]"#;
        let result: Result<ConversationState, _> = serde_json::from_str(old);
        assert!(result.is_err());
    }

    #[test]
    fn context_empty_when_no_summaries() {
        let state = ConversationState::default();
        assert_eq!(format_context(&state), "");
    }

    #[test]
    fn context_includes_all_sections() {
        let state = ConversationState {
            user_summary: "Likes Rust and WASM".into(),
            conversation_summary: "Discussed foo and bar".into(),
            bond_summary: "Casual and technical".into(),
            ..Default::default()
        };
        let ctx = format_context(&state);
        assert!(ctx.contains("## User\nLikes Rust and WASM"));
        assert!(ctx.contains("## Conversation so far\nDiscussed foo and bar"));
        assert!(ctx.contains("## Bond\nCasual and technical"));
    }

    #[test]
    fn truncate_short_string() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_exact_boundary() {
        assert_eq!(truncate_str("ab", 2), "ab");
    }

    #[test]
    fn truncate_over_limit() {
        assert_eq!(truncate_str("abc", 2), "ab...");
    }

    #[test]
    fn truncate_empty() {
        assert_eq!(truncate_str("", 5), "");
    }

    #[test]
    fn truncate_unicode_boundary() {
        // "café" — é is 2 bytes in UTF-8
        assert_eq!(truncate_str("café", 4), "caf...");
        assert_eq!(truncate_str("café", 5), "café");
    }
}
