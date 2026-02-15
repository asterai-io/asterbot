use crate::bindings::asterai::telegram::api;
use crate::bindings::asterai::telegram::types::Message;
use crate::bindings::asterbot::types::agent;
use crate::bindings::exports::asterai::telegram::incoming_handler::Guest;

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
        if message.sender.id == self_user.id {
            return;
        }
        println!("processing message {message:#?}");
        let response = agent::converse(&message.content);
        api::send_message(&response, message.chat_id);
    }
}

bindings::export!(Component with_types_in bindings);
