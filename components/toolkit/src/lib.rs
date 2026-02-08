use crate::bindings::exports::asterbot::toolkit::toolkit::{Guest, ToolInfo};

#[allow(warnings)]
mod bindings;

struct Component;

impl Guest for Component {
    fn list_tools() -> Vec<ToolInfo> {
        Vec::new()
    }

    fn call_tool(component_name: String, function_name: String, args_json: String) -> String {
        String::new()
    }

    fn format_tools_for_prompt() -> String {
        String::new()
    }
}

bindings::export!(Component with_types_in bindings);
