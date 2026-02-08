use crate::bindings::exports::asterbot::types::soul::Guest;

#[allow(warnings)]
mod bindings;

struct Component;

impl Guest for Component {
    fn get() -> String {
        let host_dir = match resolve_host_dir() {
            Ok(d) => d,
            Err(_) => return String::new(),
        };
        let path = format!("{host_dir}/SOUL.md");
        std::fs::read_to_string(&path).unwrap_or_default()
    }

    fn set(content: String) {
        let host_dir = match resolve_host_dir() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{e}");
                return;
            }
        };
        let path = format!("{host_dir}/SOUL.md");
        if let Err(e) = std::fs::write(&path, content) {
            eprintln!("error: failed to write SOUL.md: {e}");
        }
    }
}

fn resolve_host_dir() -> Result<String, String> {
    if let Ok(v) = std::env::var("ASTERBOT_HOST_DIR") {
        if !v.is_empty() {
            return Ok(v);
        }
    }
    if let Ok(dirs) = std::env::var("ASTERAI_ALLOWED_DIRS") {
        if let Some(first) = dirs.split(':').next() {
            if !first.is_empty() {
                return Ok(first.to_string());
            }
        }
    }
    Err("error: no host directory available — pass --allow-dir".to_string())
}

bindings::export!(Component with_types_in bindings);
