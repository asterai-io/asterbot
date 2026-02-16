use crate::bindings::asterai::whatsapp::api;
use crate::bindings::asterai::whatsapp::types::Message;
use crate::bindings::asterbot::types::agent;
use crate::bindings::exports::asterai::whatsapp::incoming_handler::Guest;
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
    AllowList(Vec<String>),
    Public,
    Disabled,
}

static ACCESS_MODE: LazyLock<AccessMode> = LazyLock::new(|| init_access_mode());

struct Component;

impl Guest for Component {
    fn on_message(message: Message) {
        println!(
            "whatsapp-gateway: message from {} ({}): {}",
            message.sender.name, message.sender.phone, message.content
        );
        let self_user = api::get_self();
        if message.sender.phone == self_user.phone {
            return;
        }
        if !validate_access(&message) {
            return;
        }
        let response = agent::converse(&message.content);
        api::send_message(&response, &message.sender.phone);
    }
}

fn validate_access(message: &Message) -> bool {
    match &*ACCESS_MODE {
        AccessMode::AllowList(phones) => phones.contains(&message.sender.phone),
        AccessMode::Public => true,
        AccessMode::Disabled => {
            eprintln!(
                "whatsapp-gateway: disabled, set \
                 WHATSAPP_ALLOWED_PHONES or WHATSAPP_PUBLIC=true"
            );
            false
        }
    }
}

fn init_access_mode() -> AccessMode {
    let allowed_phones: Vec<String> = std::env::var("WHATSAPP_ALLOWED_PHONES")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().trim_start_matches('+').to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    let is_public = std::env::var("WHATSAPP_PUBLIC")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !allowed_phones.is_empty() && is_public {
        eprintln!(
            "WHATSAPP_ALLOWED_PHONES and WHATSAPP_PUBLIC=true are both set; \
             using WHATSAPP_ALLOWED_PHONES"
        );
        return AccessMode::AllowList(allowed_phones);
    }
    if !allowed_phones.is_empty() {
        return AccessMode::AllowList(allowed_phones);
    }
    if is_public {
        return AccessMode::Public;
    }
    AccessMode::Disabled
}

bindings::export!(Component with_types_in bindings);
