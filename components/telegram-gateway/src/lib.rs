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

static ACCESS_MODE: LazyLock<AccessMode> = LazyLock::new(|| init_access_mode());

struct Component;

impl Guest for Component {
    fn on_message(message: Message) {
        println!(
            "telegram-gateway: message from {} (id={}): {}",
            message.sender.username, message.sender.id, message.content
        );
        let self_user = api::get_self();
        if message.sender.id == self_user.id {
            return;
        }
        if !validate_access(&message) {
            return;
        }
        let response = agent::converse(&message.content);
        api::send_message(&response, message.chat_id);
    }
}

fn validate_access(message: &Message) -> bool {
    match &*ACCESS_MODE {
        AccessMode::AllowList(ids) => ids.contains(&message.sender.id),
        AccessMode::Public => true,
        AccessMode::Disabled => {
            eprintln!(
                "telegram-gateway: disabled, set \
                     TELEGRAM_ALLOWED_USER_IDS or TELEGRAM_PUBLIC=true"
            );
            false
        }
    }
}

fn init_access_mode() -> AccessMode {
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
}

bindings::export!(Component with_types_in bindings);
