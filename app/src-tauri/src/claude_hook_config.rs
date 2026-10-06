//! Previewed ownership-scoped hook configuration; no receiver activation or source decisions.
//! macOS implementation. All bodies stay internal; public DTOs contain only safe hook metadata.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const CAP: usize = 256 * 1024;
const LIMIT: usize = 20;
const TTL: Duration = Duration::from_secs(5 * 60);
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    Enable,
    Disable,
    Restore,
    TrashBackup,
}
#[derive(Debug, Serialize)]
pub(crate) struct Error {
    pub code: &'static str,
    pub notice: &'static str,
    pub applied: bool,
}
fn fail(code: &'static str) -> Error {
    Error {
        code,
        notice: match code {
            "private" => "配置含认证值或未核实的环境变量，不能备份或修改。",
            "changed" => "配置或目录已变化，请重新读取。",
            "ownership" => "只读钩子的归属无法核实，请检查 Claude 配置。",
            "expired" => "预览已失效，请重新核对。",
            "capacity" => "备份已达上限，原记录保留。",
            "missing" => "Claude 用户目录尚未就绪，请先打开工具。",
            _ => "配置无法核实，原文件保留。",
        },
        applied: false,
    }
}
fn uncertain() -> Error {
    Error {
        code: "uncertain",
        notice: "修改结果未完整核实，请重新读取；不要重复确认。",
        applied: true,
    }
}
#[derive(Serialize)]
pub(crate) struct Inspection {
    pub revision: String,
    pub installed: bool,
    pub writable: bool,
    pub notice: &'static str,
    pub backups: Vec<String>,
    pub backup_records: Vec<BackupRecord>,
}
#[derive(Serialize)]
pub(crate) struct BackupRecord {
    pub id: String,
    pub created_ms: i64,
    pub configured_before: bool,
    pub restore_available: bool,
}
#[derive(Serialize)]
pub(crate) struct Preview {
    pub plan_id: String,
    pub action: Action,
    pub installed_before: bool,
    pub installed_after: bool,
    pub scope: &'static str,
    pub notice: &'static str,
}
#[derive(Serialize)]
pub(crate) struct Applied {
    pub verified: bool,
    pub installed: bool,
    pub backup_id: String,
    pub notice: &'static str,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimePreference {
    pub schema_version: u32,
    pub enabled: bool,
    pub binding: String,
}
pub(crate) struct RuntimeProof {
    pub revision: String,
    pub eligible: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Owner {
    schema_version: u32,
    command: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Backup {
    schema_version: u32,
    before: Option<String>,
    owner_before: Option<Owner>,
    after_hash: String,
    owner_after_hash: String,
}
fn credential_key(key: &str) -> bool {
    let key = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    [
        "apikey",
        "accesstoken",
        "authtoken",
        "password",
        "secret",
        "credential",
        "authorization",
        "privatekey",
        "clienttoken",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}
fn empty(value: &Value) -> bool {
    value.is_null()
        || value.as_str() == Some("")
        || value.as_object().is_some_and(|v| v.is_empty())
        || value.as_array().is_some_and(|v| v.is_empty())
}
fn reference(value: &str) -> bool {
    let name = value
        .strip_prefix("env:")
        .or_else(|| value.strip_prefix("${").and_then(|v| v.strip_suffix('}')));
    name.is_some_and(|name| {
        !name.is_empty()
            && name.len() <= 80
            && name
                .bytes()
                .enumerate()
                .all(|(i, b)| b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit()))
    })
}
fn safe(value: &Value, in_env: bool) -> bool {
    if in_env && !value.is_object() {
        return false;
    }
    match value {
        Value::Object(object) => object.iter().all(|(key, value)| {
            if credential_key(key) && !empty(value) && !value.as_str().is_some_and(reference) {
                return false;
            }
            if in_env && !empty(value) && !value.as_str().is_some_and(reference) {
                return false;
            }
            safe(value, key == "env")
        }),
        Value::Array(rows) => rows.iter().all(|value| safe(value, in_env)),
        Value::String(text) => !crate::private_text::known_private(text),
        _ => true,
    }
}
fn decode(bytes: Option<&[u8]>) -> Result<Value, Error> {
    let value = match bytes {
        None => serde_json::json!({}),
        Some(bytes) => {
            crate::claude_plan_capture::strict_json(bytes).map_err(|_| fail("invalid"))?
        }
    };
    if !value.is_object() {
        return Err(fail("invalid"));
    }
    Ok(value)
}
fn encode(value: &Value) -> Result<Vec<u8>, Error> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|_| fail("invalid"))?;
    bytes.push(b'\n');
    if bytes.len() > CAP {
        return Err(fail("invalid"));
    }
    Ok(bytes)
}
fn hook(command: &str) -> Value {
    serde_json::json!({"type":"command","command":command,"timeout":5})
}
fn groups(value: &Value) -> Result<&[Value], Error> {
    match value.get("hooks") {
        None => Ok(&[]),
        Some(hooks) => {
            let hooks = hooks.as_object().ok_or_else(|| fail("invalid"))?;
            match hooks.get("PreToolUse") {
                None => Ok(&[]),
                Some(rows) => rows
                    .as_array()
                    .map(Vec::as_slice)
                    .ok_or_else(|| fail("invalid")),
            }
        }
    }
}
fn installed(value: &Value, owner: Option<&Owner>, _command: &str) -> Result<bool, Error> {
    let mut matches = 0;
    let mut reserved = 0;
    for group in groups(value)? {
        let object = group.as_object().ok_or_else(|| fail("invalid"))?;
        let rows = object
            .get("hooks")
            .and_then(Value::as_array)
            .ok_or_else(|| fail("invalid"))?;
        for item in rows {
            if item
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|s| s.contains(crate::claude_plan_receiver::FLAG))
            {
                reserved += 1;
            }
            if owner.is_some_and(|o| o.schema_version == 1)
                && object.get("matcher").and_then(Value::as_str) == Some("ExitPlanMode")
                && owner.is_some_and(|o| *item == hook(&o.command))
            {
                matches += 1;
            }
        }
    }
    if reserved == 0 {
        return Ok(false);
    }
    if reserved == 1 && matches == 1 {
        Ok(true)
    } else {
        Err(fail("ownership"))
    }
}
fn edit(mut value: Value, enable: bool, command: &str) -> Result<Value, Error> {
    if enable {
        let root = value.as_object_mut().ok_or_else(|| fail("invalid"))?;
        let hooks = root
            .entry("hooks")
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or_else(|| fail("invalid"))?;
        let rows = hooks
            .entry("PreToolUse")
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .ok_or_else(|| fail("invalid"))?;
        rows.push(serde_json::json!({"matcher":"ExitPlanMode","hooks":[hook(command)]}));
    } else {
        if let Some(rows) = value
            .get_mut("hooks")
            .and_then(|v| v.get_mut("PreToolUse"))
            .and_then(Value::as_array_mut)
        {
            for group in rows.iter_mut() {
                if group.get("matcher").and_then(Value::as_str) == Some("ExitPlanMode") {
                    if let Some(items) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                        items.retain(|v| *v != hook(command));
                    }
                }
            }
            // Remove only the empty exact group shape we introduced; unknown group metadata survives.
            rows.retain(|v| v != &serde_json::json!({"matcher":"ExitPlanMode","hooks":[]}));
        }
    }
    Ok(value)
}
#[cfg(target_os = "macos")]
#[path = "claude_hook_config_macos.rs"]
pub(crate) mod native;
