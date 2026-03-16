#[cfg(not(test))]
use crate::bindings::asterai::fs::fs;
#[cfg(not(test))]
use crate::bindings::asterai::llm::llm::{chat, ChatMessage, ChatRole, ToolCall, ToolDefinition};
#[cfg(not(test))]
use crate::bindings::exports::asterbot::types::history::Guest;
use serde::{Deserialize, Serialize};

const HISTORY_FILENAME: &str = "conversation.json";
const DEFAULT_COMPACTION_THRESHOLD: usize = 50;
const DEFAULT_KEEP_RECENT_TURNS: usize = 10;
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
        if model.is_empty() {
            return messages;
        }

        let keep_turns = std::env::var("ASTERBOT_KEEP_RECENT_TURNS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_KEEP_RECENT_TURNS);

        let roles: Vec<&str> = messages
            .iter()
            .map(|m| match m.role {
                ChatRole::User => "user",
                ChatRole::Assistant => "assistant",
                ChatRole::System => "system",
                ChatRole::Tool => "tool",
            })
            .collect();
        let cut = find_cut_point(&roles, keep_turns);
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

        // Always advance the cursor.
        state.compacted_through += cut;
        write_state(&state);

        recent
    }
}

/// Find the cut point: keep the last `keep_turns` user
/// messages (and everything after them). Returns the
/// index of the oldest kept user message, or 0 if there
/// aren't enough turns to justify compaction.
fn find_cut_point(roles: &[&str], keep_turns: usize) -> usize {
    let user_indices: Vec<usize> = roles
        .iter()
        .enumerate()
        .filter(|(_, r)| **r == "user")
        .map(|(i, _)| i)
        .collect();
    // Need at least 2 user turns to compact anything.
    if user_indices.len() <= 1 {
        return 0;
    }
    // Keep at most keep_turns, but always compact at least 1.
    let keep = keep_turns.min(user_indices.len() - 1);
    // Cut at the oldest user message we want to keep.
    // Everything before it gets compacted.
    user_indices[user_indices.len() - keep]
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

    fn assistant_tool_call(call_id: &str, name: &str) -> PersistedMessage {
        PersistedMessage {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: vec![PersistedToolCall {
                id: call_id.to_string(),
                name: name.to_string(),
                arguments_json: "{}".to_string(),
            }],
            tool_call_id: None,
        }
    }

    fn tool_result(content: &str, call_id: &str) -> PersistedMessage {
        PersistedMessage {
            role: "tool".to_string(),
            content: content.to_string(),
            tool_calls: vec![],
            tool_call_id: Some(call_id.to_string()),
        }
    }

    fn roles(msgs: &[PersistedMessage]) -> Vec<&str> {
        msgs.iter().map(|m| m.role.as_str()).collect()
    }

    /// Simulate what core does: load → compact → save.
    /// Returns (state_after, working_set_returned_to_core).
    fn simulate_compact(state: &mut ConversationState, keep_turns: usize) -> Vec<PersistedMessage> {
        // load: return history[compactedThrough..]
        let start = state.compacted_through.min(state.history.len());
        let working_set: Vec<PersistedMessage> = state.history[start..].to_vec();

        // compact
        let r = roles(&working_set);
        let cut = find_cut_point(&r, keep_turns);
        if cut == 0 {
            return working_set;
        }

        state.compacted_through += cut;
        state.conversation_summary = format!("Compacted through turn {}", state.compacted_through);

        // Return the trimmed working set
        working_set[cut..].to_vec()
    }

    /// Simulate what core does after compact: add new
    /// turn and save.
    fn simulate_save(state: &mut ConversationState, working_set: &[PersistedMessage]) {
        let start = state.compacted_through.min(state.history.len());
        state.history.truncate(start);
        state.history.extend_from_slice(working_set);
    }

    #[test]
    fn cut_point_basic() {
        // 5 user turns, keep 3
        let msgs = vec![
            user("1"),
            assistant("r1"),
            user("2"),
            assistant("r2"),
            user("3"),
            assistant("r3"),
            user("4"),
            assistant("r4"),
            user("5"),
            assistant("r5"),
        ];
        let r = roles(&msgs);
        // Keep last 3 users (3,4,5) at indices 4,6,8. Cut at 4.
        assert_eq!(find_cut_point(&r, 3), 4);
    }

    #[test]
    fn cut_point_not_enough_turns() {
        // 3 user turns, keep 3 → still compacts 1 turn
        // (keep is capped to user_turns - 1)
        let msgs = vec![
            user("1"),
            assistant("r1"),
            user("2"),
            assistant("r2"),
            user("3"),
            assistant("r3"),
        ];
        let r = roles(&msgs);
        // keep = min(3, 2) = 2, cut at user_indices[1] = 2
        assert_eq!(find_cut_point(&r, 3), 2);
    }

    #[test]
    fn cut_point_single_user_turn() {
        // 1 user turn → nothing to compact
        let msgs = vec![user("1"), assistant("r1")];
        let r = roles(&msgs);
        assert_eq!(find_cut_point(&r, 3), 0);
    }

    #[test]
    fn cut_point_exactly_one_more() {
        // 4 user turns, keep 3 → compact 1 turn
        let msgs = vec![
            user("1"),
            assistant("r1"),
            user("2"),
            assistant("r2"),
            user("3"),
            assistant("r3"),
            user("4"),
            assistant("r4"),
        ];
        let r = roles(&msgs);
        // Keep last 3 (2,3,4) at indices 2,4,6. Cut at 2.
        assert_eq!(find_cut_point(&r, 3), 2);
    }

    #[test]
    fn cut_point_with_tool_calls() {
        let msgs = vec![
            user("1"),                           // 0
            assistant_tool_call("c1", "search"), // 1
            tool_result("found it", "c1"),       // 2
            assistant("Here's what I found"),    // 3
            user("2"),                           // 4
            assistant("r2"),                     // 5
            user("3"),                           // 6
            assistant("r3"),                     // 7
            user("4"),                           // 8
            assistant("r4"),                     // 9
        ];
        let r = roles(&msgs);
        // 4 users at 0, 4, 6, 8. Keep 3 → cut at 4.
        assert_eq!(find_cut_point(&r, 3), 4);
    }

    #[test]
    fn cut_point_keep_1() {
        let msgs = vec![
            user("1"),
            assistant("r1"),
            user("2"),
            assistant("r2"),
            user("3"),
            assistant("r3"),
        ];
        let r = roles(&msgs);
        // Keep 1 → cut at index 4 (user "3")
        assert_eq!(find_cut_point(&r, 1), 4);
    }

    #[test]
    fn single_compaction_preserves_all_history() {
        let mut state = ConversationState {
            history: vec![
                user("1"),
                assistant("r1"),
                user("2"),
                assistant("r2"),
                user("3"),
                assistant("r3"),
                user("4"),
                assistant("r4"),
                user("5"),
                assistant("r5"),
            ],
            ..Default::default()
        };

        let trimmed = simulate_compact(&mut state, 3);

        // Cut at 4: compact first 2 user turns
        assert_eq!(state.compacted_through, 4);
        assert_eq!(trimmed.len(), 6); // 3 user turns + responses
        assert_eq!(trimmed[0].content, "3");

        // Core adds a new turn
        let mut working = trimmed;
        working.push(user("6"));
        working.push(assistant("r6"));
        simulate_save(&mut state, &working);

        // Full history preserved
        assert_eq!(state.history.len(), 12);
        assert_eq!(state.history[0].content, "1"); // archived
        assert_eq!(state.history[11].content, "r6"); // newest
    }

    #[test]
    fn multiple_compaction_cycles() {
        let mut state = ConversationState::default();

        // Build up 5 user turns (10 messages)
        for i in 1..=5 {
            state.history.push(user(&format!("u{i}")));
            state.history.push(assistant(&format!("a{i}")));
        }

        // Cycle 1: keep 3 → compact 2
        let trimmed = simulate_compact(&mut state, 3);
        assert_eq!(state.compacted_through, 4);
        assert_eq!(trimmed.len(), 6);

        // Core adds 3 more turns
        let mut working = trimmed;
        for i in 6..=8 {
            working.push(user(&format!("u{i}")));
            working.push(assistant(&format!("a{i}")));
        }
        simulate_save(&mut state, &working);
        assert_eq!(state.history.len(), 16);

        // Cycle 2: working set is 12 msgs, 6 user turns
        let trimmed = simulate_compact(&mut state, 3);
        // 6 users in working set, keep 3 → compact 3
        // User indices in working set: 0,2,4,6,8,10
        // Cut at index user_indices[6-3] = [3] = 6
        assert_eq!(state.compacted_through, 4 + 6);
        assert_eq!(trimmed.len(), 6); // last 3 turns

        // Core adds 1 more turn
        let mut working = trimmed;
        working.push(user("u9"));
        working.push(assistant("a9"));
        simulate_save(&mut state, &working);

        // ALL messages still in history
        assert_eq!(state.history.len(), 18);
        assert_eq!(state.history[0].content, "u1");
        assert_eq!(state.history[17].content, "a9");
    }

    #[test]
    fn compaction_with_threshold_10_keep_3_realistic() {
        // Reproduce the user's scenario: threshold=10, keep=3
        let mut state = ConversationState::default();
        let threshold = 10;
        let keep_turns = 3;

        // Send messages one turn at a time, compacting
        // when the working set hits threshold.
        for turn in 1..=10 {
            state.history.push(user(&format!("u{turn}")));
            state.history.push(assistant(&format!("a{turn}")));

            let working_len = state.history.len() - state.compacted_through;

            if working_len >= threshold {
                let trimmed = simulate_compact(&mut state, keep_turns);
                let mut working = trimmed;
                // (no new message this cycle, compact happened
                // before LLM call in the real flow)
                simulate_save(&mut state, &working);
            }
        }

        // After 10 turns (20 messages), compaction should
        // have fired at least twice. The cursor should have
        // advanced well past 2.
        assert!(
            state.compacted_through > 2,
            "compacted_through should be > 2, got {}",
            state.compacted_through,
        );
        // All 20 messages preserved
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
                let trimmed = simulate_compact(&mut state, 3);
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

    /// Simulate compact where the LLM call fails.
    /// Fallback: raw text summary, advance cursor,
    /// return trimmed working set.
    fn simulate_compact_llm_fails(
        state: &mut ConversationState,
        keep_turns: usize,
    ) -> Vec<PersistedMessage> {
        let start = state.compacted_through.min(state.history.len());
        let working_set: Vec<PersistedMessage> = state.history[start..].to_vec();

        let r = roles(&working_set);
        let cut = find_cut_point(&r, keep_turns);
        if cut == 0 {
            return working_set;
        }

        // LLM fails → fallback raw summary, still advance.
        let old = &working_set[..cut];
        let fallback: String = old
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

        state.compacted_through += cut;
        working_set[cut..].to_vec()
    }

    #[test]
    fn llm_failure_should_still_advance_cursor() {
        // When LLM fails, the cursor should still advance
        // (with a fallback summary) so the working set
        // stays bounded.
        let mut state = ConversationState::default();
        let threshold = 10;
        let keep_turns = 3;

        for turn in 1..=10 {
            state.history.push(user(&format!("u{turn}")));
            state.history.push(assistant(&format!("a{turn}")));

            let working_len = state.history.len() - state.compacted_through;

            if working_len >= threshold {
                let trimmed = simulate_compact_llm_fails(&mut state, keep_turns);
                simulate_save(&mut state, &trimmed);
            }
        }

        // Cursor should have advanced past 0
        assert!(
            state.compacted_through > 0,
            "cursor should advance even when LLM fails, \
             got compacted_through={}",
            state.compacted_through,
        );
        // Working set should be bounded
        let working_len = state.history.len() - state.compacted_through;
        assert!(
            working_len < state.history.len(),
            "working set ({working_len}) should be smaller \
             than total history ({})",
            state.history.len(),
        );
    }

    #[test]
    fn working_set_stays_bounded_despite_llm_failures() {
        // Even with repeated LLM failures, the working set
        // should never grow much past the threshold.
        let mut state = ConversationState::default();
        let threshold = 10;
        let keep_turns = 3;

        for turn in 1..=15 {
            state.history.push(user(&format!("u{turn}")));
            state.history.push(assistant(&format!("a{turn}")));

            let working_len = state.history.len() - state.compacted_through;

            if working_len >= threshold {
                let trimmed = simulate_compact_llm_fails(&mut state, keep_turns);
                simulate_save(&mut state, &trimmed);
            }
        }

        let working_len = state.history.len() - state.compacted_through;
        // Working set should stay roughly bounded: at most
        // threshold + one turn of headroom (2 messages).
        assert!(
            working_len <= threshold + 2,
            "working set ({working_len}) should stay \
             bounded near threshold ({threshold}), not \
             grow to {}",
            state.history.len(),
        );
    }

    #[test]
    fn no_wasted_llm_calls_when_stuck() {
        // Compaction should not fire every single turn
        // when the LLM keeps failing. Either the fallback
        // advances the cursor (so threshold isn't hit), or
        // there's a cooldown/backoff.
        let mut state = ConversationState::default();
        let threshold = 10;
        let keep_turns = 3;
        let mut compact_attempts = 0;

        for turn in 1..=15 {
            state.history.push(user(&format!("u{turn}")));
            state.history.push(assistant(&format!("a{turn}")));

            let working_len = state.history.len() - state.compacted_through;

            if working_len >= threshold {
                compact_attempts += 1;
                let trimmed = simulate_compact_llm_fails(&mut state, keep_turns);
                simulate_save(&mut state, &trimmed);
            }
        }

        // With these params, compaction legitimately fires
        // every ~2 turns (gap = (threshold - 2*keep) / 2).
        // Over 15 turns that's ~6. Without the fallback fix,
        // the cursor stays stuck and compaction fires on
        // EVERY turn after threshold (~11 times).
        assert!(
            compact_attempts <= 15 / 2,
            "compaction fired {compact_attempts} times for \
             15 turns — without fallback it would fire \
             every turn (~11 times)",
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
            user_summary: "Lorenzo".into(),
            conversation_summary: "Talked about Rust".into(),
            bond_summary: "Technical rapport".into(),
            ..Default::default()
        };
        let ctx = format_context(&state);
        assert!(ctx.contains("## User\nLorenzo"));
        assert!(ctx.contains("## Conversation so far\nTalked about Rust"));
        assert!(ctx.contains("## Bond\nTechnical rapport"));
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
