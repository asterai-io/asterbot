use crate::bindings::asterai::fs::fs;
use crate::bindings::exports::asterbot::tool_gate::api::Guest as GateApiGuest;
use crate::bindings::exports::asterbot::types::tool_hook::{
    Guest as ToolHookGuest, HookResponse, HookResult,
};
use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce as AesNonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const GATE_DIR: &str = "tool-gate";
const PERMISSIONS_FILE: &str = "permissions.json";
const PENDING_DIR: &str = "pending";
const DEFAULT_TIMEOUT_SECS: u64 = 300;
const POLL_INTERVAL_MS: u64 = 500;

#[allow(warnings)]
mod bindings {
    wit_bindgen::generate!({
        path: "wit/package.wasm",
        world: "component",
        generate_all,
    });
}

struct Component;

static INITIALIZED: OnceLock<()> = OnceLock::new();

#[derive(Serialize, Deserialize)]
struct PermissionsData {
    info: String,
    permissions: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Clone)]
struct PendingRequest {
    id: String,
    nonce: String,
    component: String,
    function: String,
    args: String,
    status: String,
    created_at: u64,
}

fn ensure_initialized() {
    INITIALIZED.get_or_init(|| {
        clear_pending();
    });
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
    Err("error: no host directory available".to_string())
}

fn gate_dir() -> String {
    match resolve_host_dir() {
        Ok(dir) => format!("{dir}/{GATE_DIR}"),
        Err(_) => GATE_DIR.to_string(),
    }
}

fn permissions_path() -> String {
    format!("{}/{PERMISSIONS_FILE}", gate_dir())
}

fn pending_path() -> String {
    format!("{}/{PENDING_DIR}", gate_dir())
}

fn get_secret() -> Option<String> {
    std::env::var("ASTERBOT_TOOL_GATE_SECRET")
        .ok()
        .filter(|s| !s.is_empty())
}

fn derive_key(secret: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.finalize().into()
}

fn encrypt(data: &[u8], secret: &str) -> Option<Vec<u8>> {
    let key = derive_key(secret);
    let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
    let aes_nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher.encrypt(&aes_nonce, data).ok()?;
    let mut out = aes_nonce.to_vec();
    out.extend(ciphertext);
    Some(out)
}

fn decrypt(data: &[u8], secret: &str) -> Option<Vec<u8>> {
    if data.len() < 12 {
        return None;
    }
    let key = derive_key(secret);
    let cipher = Aes256Gcm::new_from_slice(&key).ok()?;
    let (nonce_bytes, ciphertext) = data.split_at(12);
    let aes_nonce = AesNonce::from_slice(nonce_bytes);
    cipher.decrypt(aes_nonce, ciphertext).ok()
}

fn ensure_dir(path: &str) {
    let _ = fs::mkdir(path);
}

fn encrypted_read<T: for<'de> Deserialize<'de>>(path: &str) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    let secret = get_secret()?;
    let plaintext = decrypt(&bytes, &secret)?;
    let json = String::from_utf8(plaintext).ok()?;
    serde_json::from_str(&json).ok()
}

fn encrypted_write<T: Serialize>(path: &str, data: &T) -> bool {
    let Some(secret) = get_secret() else {
        return false;
    };
    let Ok(json) = serde_json::to_string_pretty(data) else {
        return false;
    };
    let Some(encrypted) = encrypt(json.as_bytes(), &secret) else {
        return false;
    };
    ensure_dir(&gate_dir());
    fs::write(path, &encrypted).is_ok()
}

fn default_permissions() -> PermissionsData {
    PermissionsData {
        info: "Asterbot tool-gate permissions. \
               Managed by the tool-gate component — \
               do not modify directly."
            .to_string(),
        permissions: serde_json::Map::new(),
    }
}

fn read_permissions() -> PermissionsData {
    match encrypted_read::<PermissionsData>(&permissions_path()) {
        Some(data) => data,
        None => {
            let fresh = default_permissions();
            write_permissions(&fresh);
            fresh
        }
    }
}

fn write_permissions(data: &PermissionsData) {
    if !encrypted_write(&permissions_path(), data) {
        eprintln!("tool-gate: failed to write permissions");
    }
}

fn get_permission(key: &str) -> Option<String> {
    let data = read_permissions();
    data.permissions
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn generate_nonce() -> String {
    Uuid::now_v7().to_string()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn write_pending(req: &PendingRequest) {
    let dir = pending_path();
    ensure_dir(&dir);
    let path = format!("{dir}/{}.bin", req.id);
    encrypted_write(&path, req);
}

fn read_pending(id: &str) -> Option<PendingRequest> {
    let path = format!("{}/{id}.bin", pending_path());
    encrypted_read::<PendingRequest>(&path)
}

fn remove_pending(id: &str) {
    let path = format!("{}/{id}.bin", pending_path());
    let _ = fs::rm(&path, false);
}

fn clear_pending() {
    let _ = fs::rm(&pending_path(), true);
}

fn list_pending_all() -> Vec<PendingRequest> {
    let dir = pending_path();
    let entries = match fs::ls(&dir, false) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut requests = Vec::new();
    for entry in &entries {
        if !entry.name.ends_with(".bin") {
            continue;
        }
        let path = format!("{dir}/{}", entry.name);
        if let Some(req) = encrypted_read::<PendingRequest>(&path) {
            if req.status == "pending" {
                requests.push(req);
            }
        }
    }
    requests
}

fn get_timeout() -> Duration {
    let secs = std::env::var("ASTERBOT_TOOL_GATE_TIMEOUT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS);
    Duration::from_secs(secs)
}

impl ToolHookGuest for Component {
    fn before_call(component: String, function_name: String, args: String) -> HookResponse {
        ensure_initialized();
        let key = format!("{component}/{function_name}");
        match get_permission(&key).as_deref() {
            Some("allow") => {
                return HookResponse {
                    result: HookResult::Allow,
                    message: None,
                };
            }
            Some("deny") => {
                return HookResponse {
                    result: HookResult::Deny,
                    message: Some(format!("'{key}' is permanently denied")),
                };
            }
            _ => {}
        }
        let nonce = generate_nonce();
        let id = Uuid::now_v7().to_string();
        let pending = PendingRequest {
            id: id.clone(),
            nonce: nonce.clone(),
            component: component.clone(),
            function: function_name.clone(),
            args: args.clone(),
            status: "pending".to_string(),
            created_at: now_secs(),
        };
        write_pending(&pending);
        let timeout = get_timeout();
        let start = Instant::now();
        loop {
            if let Some(req) = read_pending(&id) {
                if req.status != "pending" && req.nonce == nonce {
                    remove_pending(&id);
                    if req.status == "approve_always" {
                        let mut perms = read_permissions();
                        perms
                            .permissions
                            .insert(key.clone(), serde_json::Value::String("allow".to_string()));
                        write_permissions(&perms);
                    }
                    return match req.status.as_str() {
                        "approve_once" | "approve_always" => HookResponse {
                            result: HookResult::Allow,
                            message: None,
                        },
                        _ => HookResponse {
                            result: HookResult::Deny,
                            message: Some(format!("'{key}' denied by user")),
                        },
                    };
                }
            } else {
                // File was deleted (corrupt or wiped) — deny.
                return HookResponse {
                    result: HookResult::Deny,
                    message: Some(format!("'{key}' pending request lost")),
                };
            }
            if start.elapsed() > timeout {
                remove_pending(&id);
                return HookResponse {
                    result: HookResult::Deny,
                    message: Some(format!("'{key}' timed out waiting for approval")),
                };
            }
            std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
        }
    }

    fn after_call(_component: String, _function_name: String, _args: String, _result: String) {}
}

impl GateApiGuest for Component {
    fn list_pending() -> String {
        ensure_initialized();
        let requests: Vec<serde_json::Value> = list_pending_all()
            .into_iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "component": r.component,
                    "function": r.function,
                    "args": r.args,
                    "created_at": r.created_at,
                })
            })
            .collect();
        serde_json::to_string(&requests).unwrap_or_else(|_| "[]".to_string())
    }

    fn confirm(id: String, action: String) -> String {
        ensure_initialized();
        let valid_actions = ["approve_once", "approve_always", "deny"];
        if !valid_actions.contains(&action.as_str()) {
            return serde_json::json!({
                "error": format!(
                    "invalid action '{}', expected one of: {}",
                    action,
                    valid_actions.join(", ")
                )
            })
            .to_string();
        }
        let Some(mut req) = read_pending(&id) else {
            return serde_json::json!({ "error": "pending request not found" }).to_string();
        };
        req.status = action;
        write_pending(&req);
        serde_json::json!({ "ok": true }).to_string()
    }

    fn get_permissions() -> String {
        ensure_initialized();
        let data = read_permissions();
        serde_json::to_string(&data.permissions).unwrap_or_else(|_| "{}".to_string())
    }

    fn update_permission(key: String, value: String) -> String {
        ensure_initialized();
        let valid_values = ["allow", "deny"];
        if !valid_values.contains(&value.as_str()) {
            return serde_json::json!({
                "error": format!(
                    "invalid value '{}', expected one of: {}",
                    value,
                    valid_values.join(", ")
                )
            })
            .to_string();
        }
        let mut data = read_permissions();
        data.permissions
            .insert(key.clone(), serde_json::Value::String(value.clone()));
        write_permissions(&data);
        serde_json::json!({ "ok": true }).to_string()
    }

    fn remove_permission(key: String) -> String {
        ensure_initialized();
        let mut data = read_permissions();
        data.permissions.remove(&key);
        write_permissions(&data);
        serde_json::json!({ "ok": true }).to_string()
    }
}

bindings::export!(Component with_types_in bindings);
