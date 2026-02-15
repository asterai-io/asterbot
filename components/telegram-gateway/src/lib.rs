use crate::bindings::asterai::telegram::api;
use crate::bindings::asterai::telegram::types::Message;
use crate::bindings::asterbot::types::agent;
use crate::bindings::exports::asterai::telegram::incoming_handler::Guest;
use std::sync::LazyLock;

#[allow(warnings)]
mod bindings {
    wit_bindgen::generate!({
        path: "wit/package.wasm",
        world: "component",
        generate_all,
    });
}

enum AccessMode {
    AllowList(Vec<i64>),
    Public,
    Disabled,
}

static ACCESS_MODE: LazyLock<AccessMode> = LazyLock::new(|| {
    let allowed_ids: Vec<i64> = std::env::var("TELEGRAM_ALLOWED_USER_IDS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let is_public = std::env::var("TELEGRAM_PUBLIC")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !allowed_ids.is_empty() && is_public {
        eprintln!(
            "TELEGRAM_ALLOWED_USER_IDS and TELEGRAM_PUBLIC=true are both set; \
             using TELEGRAM_ALLOWED_USER_IDS"
        );
        return AccessMode::AllowList(allowed_ids);
    }
    if !allowed_ids.is_empty() {
        return AccessMode::AllowList(allowed_ids);
    }
    if is_public {
        return AccessMode::Public;
    }
    AccessMode::Disabled
});

struct Component;

impl Guest for Component {
    fn on_message(message: Message) {
        let self_user = api::get_self();
        if message.sender.id == self_user.id {
            return;
        }
        match &*ACCESS_MODE {
            AccessMode::AllowList(ids) => {
                if !ids.contains(&message.sender.id) {
                    return;
                }
            }
            AccessMode::Public => {}
            AccessMode::Disabled => {
                eprintln!(
                    "message ignored: set TELEGRAM_ALLOWED_USER_IDS \
                     or TELEGRAM_PUBLIC=true to enable"
                );
                return;
            }
        }
        println!("processing message {message:#?}");
        let response = agent::converse(&message.content);
        api::send_message(&response, message.chat_id);
    }
}

bindings::export!(Component with_types_in bindings);
