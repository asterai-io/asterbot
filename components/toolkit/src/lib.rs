use crate::bindings::asterai::host::api;
use crate::bindings::asterbot::types::types::ToolParam;
use crate::bindings::exports::asterbot::types::toolkit::{Guest, ToolInfo};
use serde_json::Value;

const HOOK_INTERFACE: &str = "asterbot:types/tool-hook";

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
    fn list_tools() -> Vec<ToolInfo> {
        let mut tools = Vec::new();
        for name in tool_component_names() {
            let Some(info) = api::get_component(&name) else {
                continue;
            };
            for f in &info.functions {
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
        let hooks = discover_hook_components();
        if let Some(deny_msg) =
            run_before_hooks(&hooks, &component_name, &function_name, &args_json)
        {
            return deny_msg;
        }
        let args = convert_args_to_array(&component_name, &function_name, &args_json);
        let result = match api::call_component_function(&component_name, &function_name, &args) {
            Ok(result) => result,
            Err(e) => format!(
                "error: {}/{} failed ({:?}): {}",
                component_name, function_name, e.kind, e.message,
            ),
        };
        run_after_hooks(&hooks, &component_name, &function_name, &args_json, &result);
        result
    }

    fn format_tools_for_prompt() -> String {
        let tools = Self::list_tools();
        if tools.is_empty() {
            return "No tools available.".to_string();
        }
        let mut out = String::from("Available tools:\n");
        for t in &tools {
            out.push_str(&format!(
                "\n## Tool\nComponent: {}\nFunction: {}\n",
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

fn convert_args_to_array(component: &str, function: &str, args_json: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(args_json) else {
        return args_json.to_string();
    };
    if value.is_array() {
        return args_json.to_string();
    }
    let Some(obj) = value.as_object() else {
        return format!("[{args_json}]");
    };
    let Some(info) = api::get_component(component) else {
        return args_json.to_string();
    };
    let (iface_name, func_name) = match function.split_once('/') {
        Some((i, f)) => (Some(i), f),
        None => (None, function),
    };
    let func = info
        .functions
        .iter()
        .find(|f| f.name == func_name && f.interface_name.as_deref() == iface_name);
    let Some(func) = func else {
        return args_json.to_string();
    };
    let arr: Vec<&Value> = func
        .inputs
        .iter()
        .map(|p| {
            obj.get(&p.name)
                .or_else(|| obj.get(&p.name.replace('-', "_")))
                .unwrap_or(&Value::Null)
        })
        .collect();
    serde_json::to_string(&arr).unwrap_or_else(|_| args_json.to_string())
}

fn tool_component_names() -> Vec<String> {
    std::env::var("ASTERBOT_TOOLS")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn discover_hook_components() -> Vec<String> {
    api::list_other_components()
        .into_iter()
        .filter(|c| {
            c.interfaces
                .iter()
                .any(|i| i.split('@').next().is_some_and(|name| name == HOOK_INTERFACE))
        })
        .map(|c| c.name)
        .collect()
}

/// Runs before-call hooks.
/// Returns Some(error message) if any hook denied, None if all allowed.
fn run_before_hooks(
    hooks: &[String],
    component: &str,
    function: &str,
    args: &str,
) -> Option<String> {
    for hook in hooks {
        let call_args = serde_json::json!([component, function, args]).to_string();
        match api::call_component_function(hook, "tool-hook/before-call", &call_args) {
            Ok(result) => {
                if let Ok(resp) = serde_json::from_str::<Value>(&result) {
                    let denied = resp
                        .get("result")
                        .and_then(|v| v.as_str())
                        .is_some_and(|r| r == "deny");
                    if denied {
                        let msg = resp
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("denied by hook");
                        return Some(format!("error: {msg}"));
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "warning: hook {hook} before-call failed: {:?}: {}",
                    e.kind, e.message,
                );
            }
        }
    }
    None
}

fn run_after_hooks(hooks: &[String], component: &str, function: &str, args: &str, result: &str) {
    for hook in hooks {
        let call_args = serde_json::json!([component, function, args, result]).to_string();
        if let Err(e) = api::call_component_function(hook, "tool-hook/after-call", &call_args) {
            eprintln!(
                "warning: hook {hook} after-call failed: {:?}: {}",
                e.kind, e.message,
            );
        }
    }
}

bindings::export!(Component with_types_in bindings);
