use crate::bindings::asterai::host::api;
use crate::bindings::asterbot::toolkit::types::ToolParam;
use crate::bindings::exports::asterbot::toolkit::toolkit::{Guest, ToolInfo};

#[allow(warnings)]
mod bindings;

struct Component;

const SKIP_INTERFACES: &[&str] = &["agent", "core", "toolkit", "types", "api"];

impl Guest for Component {
    fn list_tools() -> Vec<ToolInfo> {
        let mut tools = Vec::new();
        for name in tool_component_names() {
            let Some(info) = api::get_component(&name) else {
                continue;
            };
            for f in &info.functions {
                let iface = f.interface_name.as_deref().unwrap_or("");
                if SKIP_INTERFACES.contains(&iface) {
                    continue;
                }
                let function_name = match &f.interface_name {
                    Some(iface) => format!("{iface}/{}", f.name),
                    None => f.name.clone(),
                };
                let params = f
                    .inputs
                    .iter()
                    .map(|p| ToolParam {
                        name: p.name.clone(),
                        type_name: p.type_name.clone(),
                    })
                    .collect();
                let return_type = f
                    .output
                    .as_ref()
                    .map(|o| o.type_name.clone())
                    .unwrap_or_default();
                tools.push(ToolInfo {
                    component_name: name.clone(),
                    function_name,
                    description: f.description.clone().unwrap_or_default(),
                    params,
                    return_type,
                });
            }
        }
        tools
    }

    fn call_tool(component_name: String, function_name: String, args_json: String) -> String {
        let allowed = tool_component_names();
        if !allowed.iter().any(|n| n == &component_name) {
            return format!(
                "error: component '{}' is not in ASTERBOT_TOOLS",
                component_name,
            );
        }
        match api::call_component_function(&component_name, &function_name, &args_json) {
            Ok(result) => result,
            Err(e) => format!(
                "error: {}/{} failed ({:?}): {}",
                component_name, function_name, e.kind, e.message,
            ),
        }
    }

    fn format_tools_for_prompt() -> String {
        let tools = Self::list_tools();
        if tools.is_empty() {
            return "No tools available.".to_string();
        }
        let mut out = String::from("Available tools:\n");
        for t in &tools {
            out.push_str(&format!(
                "\n## {} / {}\n",
                t.component_name, t.function_name,
            ));
            out.push_str(&format!("Description: {}\n", t.description));
            out.push_str("Parameters:\n");
            if t.params.is_empty() {
                out.push_str("  (none)\n");
            } else {
                for p in &t.params {
                    out.push_str(&format!("  - {}: {}\n", p.name, p.type_name));
                }
            }
            out.push_str(&format!("Returns: {}\n", t.return_type));
        }
        out
    }
}

fn tool_component_names() -> Vec<String> {
    std::env::var("ASTERBOT_TOOLS")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

bindings::export!(Component with_types_in bindings);
