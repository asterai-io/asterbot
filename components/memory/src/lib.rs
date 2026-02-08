use crate::bindings::exports::asterbot::types::memory::Guest;

#[allow(warnings)]
mod bindings;

struct Component;

impl Guest for Component {
    fn list_all() -> Vec<String> {
        let host_dir = match resolve_host_dir() {
            Ok(d) => d,
            Err(_) => return Vec::new(),
        };
        let dir = format!("{host_dir}/memory");
        list_md_files(&dir)
    }

    fn get(name: String) -> String {
        let host_dir = match resolve_host_dir() {
            Ok(d) => d,
            Err(_) => return String::new(),
        };
        let path = format!("{host_dir}/memory/{name}.md");
        std::fs::read_to_string(&path).unwrap_or_default()
    }

    fn set(name: String, content: String) {
        let host_dir = match resolve_host_dir() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{e}");
                return;
            }
        };
        let dir = format!("{host_dir}/memory");
        let _ = std::fs::create_dir_all(&dir);
        let path = format!("{dir}/{name}.md");
        if let Err(e) = std::fs::write(&path, content) {
            eprintln!("error: failed to write memory/{name}.md: {e}");
        }
    }

    fn remove(name: String) {
        let host_dir = match resolve_host_dir() {
            Ok(d) => d,
            Err(_) => return,
        };
        let path = format!("{host_dir}/memory/{name}.md");
        let _ = std::fs::remove_file(&path);
    }
}

fn list_md_files(dir: &str) -> Vec<String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut names = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                names.push(stem.to_string());
            }
        }
    }
    names.sort();
    names
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
