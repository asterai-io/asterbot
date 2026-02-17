use crate::bindings::asterai::twilio::api;
use crate::bindings::asterai::twilio::types::Message;
use crate::bindings::asterbot::types::agent;
use crate::bindings::exports::asterai::twilio::incoming_handler::Guest;
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
            "twilio-gateway: message from {}: {}",
            message.sender.phone, message.content
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
                "twilio-gateway: disabled, set \
                 TWILIO_ALLOWED_PHONES or TWILIO_PUBLIC=true"
            );
            false
        }
    }
}

fn init_access_mode() -> AccessMode {
    let allowed_phones: Vec<String> = std::env::var("TWILIO_ALLOWED_PHONES")
        .unwrap_or_default()
        .split(',')
        .map(|s| {
            let s = s.trim();
            match s.starts_with('+') {
                true => s.to_owned(),
                false => format!("+{s}"),
            }
        })
        .filter(|s| s.len() > 1)
        .collect();
    let is_public = std::env::var("TWILIO_PUBLIC")
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !allowed_phones.is_empty() && is_public {
        eprintln!(
            "TWILIO_ALLOWED_PHONES and TWILIO_PUBLIC=true are both set; \
             using TWILIO_ALLOWED_PHONES"
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
