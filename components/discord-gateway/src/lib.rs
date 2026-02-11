use std::env;
use crate::bindings::asterai::discord::discord;
use crate::bindings::asterbot::types::agent;
use crate::bindings::exports::asterai::discord_message_listener::incoming_handler::{Guest, Message};

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
        if !check_should_proceed(&message){
            return;
        }
        println!("got message {message:#?}");
        let response = agent::converse(&message.content);
        discord::send_message(&response, &message.channel_id);
    }
}

fn check_should_proceed(message: &Message) -> bool {
    let reply_if_message_contains_opt = env::var("DISCORD_GATEWAY_REPLY_IF_MESSAGE_CONTAINS").ok();
    match reply_if_message_contains_opt {
        None => true,
        Some(substr) => message.content.to_lowercase().contains(&substr.to_lowercase()),
    }
}

bindings::export!(Component with_types_in bindings);
