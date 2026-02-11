use crate::bindings::asterai::host::api;
use crate::bindings::exports::asterbot::types::agent::Guest;

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
    fn converse(input: String) -> String {
        let core = std::env::var("ASTERBOT_CORE_COMPONENT").unwrap_or_default();
        let core = match core.is_empty() {
            true => "asterbot:core",
            false => &core,
        };
        let input_json = serde_json::to_string(&input).unwrap_or_default();
        let args = format!("[{input_json}]");
        match api::call_component_function(core, "core/converse", &args) {
            Ok(output) => serde_json::from_str::<String>(&output).unwrap_or_else(|_| output),
            Err(e) => format!("error: core component '{}' failed: {}", core, e.message,),
        }
    }
}

bindings::export!(Component with_types_in bindings);
