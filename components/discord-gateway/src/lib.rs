use crate::bindings::asterai::discord::api;
use crate::bindings::asterai::discord::types::{Message, User};
use crate::bindings::asterbot::types::agent;
use crate::bindings::exports::asterai::discord::incoming_handler::Guest;

const DISCORD_MAX_CHARS: usize = 2000;

#[allow(warnings)]
mod bindings {
    wit_bindgen::generate!({
        path: "wit/package.wasm",
        world: "component",
        generate_all,
    });
}

struct Component;

impl Guest for Component {
    fn on_message(message: Message) {
        let self_user = api::get_self();
        if !check_should_proceed(&message, &self_user) {
            return;
        }
        println!("processing message {message:#?}");
        let response = agent::converse(&message.content);
        let response = truncate_to_discord_limit(&response);
        api::send_message(&response, &message.channel_id);
    }
}

fn check_should_proceed(message: &Message, self_user: &User) -> bool {
    if message.author.id == self_user.id {
        // Do not proceed if message is from self.
        return false;
    }
    let mention = format!("<@{}>", self_user.id);
    let has_mention = message.content.contains(&mention);
    has_mention
}

fn truncate_to_discord_limit(text: &str) -> String {
    if text.len() <= DISCORD_MAX_CHARS {
        return text.to_string();
    }
    let truncated = &text[..DISCORD_MAX_CHARS - 3];
    // Avoid splitting a multi-byte char.
    let end = truncated
        .char_indices()
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    format!("{}...", &text[..end])
}

bindings::export!(Component with_types_in bindings);
