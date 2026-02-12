use crate::bindings::asterai::discord::api;
use crate::bindings::asterai::discord::types::{Message, User};
use crate::bindings::asterbot::types::agent;
use crate::bindings::exports::asterai::discord::incoming_handler::Guest;

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

bindings::export!(Component with_types_in bindings);
