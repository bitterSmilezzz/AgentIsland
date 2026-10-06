//! Native MCP configuration management; no server execution or authentication access.
use crate::{atomicfile::atomic_replace_validated, provider::config_revision};
use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::Path};
use toml_edit::{value, Array, DocumentMut, Item, Table};
const MAX_CONFIG: u64 = 2_000_000;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transport {
    Stdio {
        command: String,
        args: Vec<String>,
        env_vars: Vec<String>,
    },
    Http {
        url: String,
        bearer_token_env_var: Option<String>,
    },
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub name: String,
    pub enabled: bool,
    pub transport: Transport,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Save { config: Draft, replace: bool },
    SetEnabled { name: String, enabled: bool },
    Remove { name: String },
}
#[derive(Debug, Serialize)]
pub struct Entry {
    pub name: String,
    pub enabled: Option<bool>,
    pub draft: Option<Draft>,
    pub reason: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct Snapshot {
    pub revision: String,
    pub entries: Vec<Entry>,
    pub writable: bool,
    pub notice: String,
}
#[derive(Debug, Serialize)]
pub struct Preview {
    pub revision: String,
    pub plan_id: String,
    pub name: String,
    pub action: String,
    pub before: Option<Draft>,
    pub after: Option<Draft>,
    pub preserves_advanced: bool,
}
#[derive(Debug, Serialize)]
pub struct Applied {
    pub backup_name: String,
    pub revision: String,
    pub verified: bool,
    pub notice: String,
}

fn secret_text(text: &str) -> bool {
    ["sk-", "AIza", "-----BEGIN", "ghp_", "github_pat_"]
        .iter()
        .any(|prefix| text.contains(prefix))
}
fn safe_text(text: &str, max: usize) -> bool {
    !text.is_empty()
        && text.len() <= max
        && !text.chars().any(char::is_control)
        && !secret_text(text)
}
fn name_ok(name: &str) -> bool {
    safe_text(name, 80)
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_- .".contains(&c))
        && name.trim() == name
}
fn variable_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && name
            .bytes()
            .enumerate()
            .all(|(i, c)| c == b'_' || c.is_ascii_uppercase() || (i > 0 && c.is_ascii_digit()))
}
fn credential_arg(arg: &str) -> bool {
    let key = arg
        .split('=')
        .next()
        .unwrap_or("")
        .trim_start_matches('-')
        .to_ascii_lowercase();
    (arg.starts_with('-') || arg.contains('='))
        && [
            "token",
            "password",
            "secret",
            "api-key",
            "api_key",
            "apikey",
            "access-token",
            "bearer-token",
            "auth-token",
            "authorization",
        ]
        .contains(&key.as_str())
}
fn validate(draft: &Draft) -> Result<(), String> {
    if !name_ok(&draft.name) {
        return Err("MCP 名称不在支持范围".into());
    }
    match &draft.transport {
        Transport::Stdio {
            command,
            args,
            env_vars,
        } => {
            if !safe_text(command, 2048)
                || args.len() > 64
                || env_vars.len() > 64
                || args.iter().any(|arg| {
                    arg.len() > 2048
                        || arg.chars().any(char::is_control)
                        || secret_text(arg)
                        || credential_arg(arg)
                })
                || env_vars.iter().any(|name| !variable_ok(name))
            {
                return Err("命令或参数不受支持；凭据请使用环境变量引用".into());
            }
        }
        Transport::Http {
            url,
            bearer_token_env_var,
        } => {
            crate::connections::normalize_url(url)?;
            if bearer_token_env_var
                .as_ref()
                .is_some_and(|name| !variable_ok(name))
            {
                return Err("认证引用须为大写环境变量名".into());
            }
        }
    }
    Ok(())
}
fn parse(text: &str) -> Result<DocumentMut, String> {
    let doc = text
        .parse::<DocumentMut>()
        .map_err(|_| "Codex 配置格式不可读，原文件保留")?;
    if doc
        .get("mcp_servers")
        .is_some_and(|item| item.as_table_like().is_none())
    {
        return Err("MCP 配置结构不受支持，原文件保留".into());
    }
    if doc
        .get("mcp_servers")
        .and_then(Item::as_table_like)
        .is_some_and(|table| table.len() > 200)
    {
        return Err("MCP 配置超过 200 项，停止管理".into());
    }
    Ok(doc)
}
pub(crate) fn read(target: &Path) -> Result<String, String> {
    match fs::symlink_metadata(target) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(_) => return Err("Codex 配置不可读".into()),
        Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_CONFIG => {
            return Err("Codex 配置须为限额内的普通文件".into())
        }
        Ok(_) => {}
    }
    let mut data = Vec::new();
    fs::File::open(target)
        .map_err(|_| "Codex 配置不可读")?
        .take(MAX_CONFIG + 1)
        .read_to_end(&mut data)
        .map_err(|_| "Codex 配置不可读")?;
    if data.len() as u64 > MAX_CONFIG {
        return Err("Codex 配置超出读取限额".into());
    }
    String::from_utf8(data).map_err(|_| "Codex 配置编码不可读".into())
}
// Full-file rollback must never create another file containing literal credentials.
// Unknown environment/header maps stay in the original, but block managed backup/write.
fn private_key_name(key: &str) -> bool {
    [
        "env",
        "http_headers",
        "api_key",
        "apikey",
        "bearer_token",
        "experimental_bearer_token",
        "password",
        "client_secret",
        "secret",
        "token",
        "access_token",
        "refresh_token",
        "auth_token",
        "authorization",
    ]
    .contains(&key.to_ascii_lowercase().as_str())
}
fn private_entry(key: &str, value: &Item) -> bool {
    private_key_name(key)
        || (key == "args"
            && value.as_array().is_some_and(|args| {
                args.iter()
                    .any(|arg| arg.as_str().is_some_and(credential_arg))
            }))
        || private_fields(value)
}
fn private_value(value: &toml_edit::Value) -> bool {
    if let Some(table) = value.as_inline_table() {
        table.iter().any(|(key, value)| {
            private_key_name(key)
                || (key == "args"
                    && value.as_array().is_some_and(|args| {
                        args.iter()
                            .any(|arg| arg.as_str().is_some_and(credential_arg))
                    }))
                || private_value(value)
        })
    } else if let Some(array) = value.as_array() {
        array.iter().any(private_value)
    } else {
        secret_text(&value.to_string())
    }
}
fn private_fields(item: &Item) -> bool {
    if let Some(table) = item.as_table_like() {
        table.iter().any(|(key, value)| private_entry(key, value))
    } else if let Some(tables) = item.as_array_of_tables() {
        tables
            .iter()
            .any(|table| table.iter().any(|(key, value)| private_entry(key, value)))
    } else if let Some(value) = item.as_value() {
        private_value(value)
    } else {
        false
    }
}

pub(crate) fn can_backup(doc: &DocumentMut, text: &str) -> bool {
    !doc.get("mcp_servers")
        .and_then(Item::as_table_like)
        .is_some_and(|table| {
            table
                .iter()
                .any(|(name, item)| draft_from(name, item).is_none())
        })
        && !secret_text(text)
        && !doc
            .iter()
            .any(|(key, item)| private_key_name(key) || private_fields(item))
}
fn strings(item: Option<&Item>) -> Option<Vec<String>> {
    match item {
        None => Some(vec![]),
        Some(item) => item
            .as_array()?
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect(),
    }
}
fn draft_from(name: &str, item: &Item) -> Option<Draft> {
    let table = item.as_table_like()?;
    let enabled = match table.get("enabled") {
        None => true,
        Some(item) => item.as_bool()?,
    };
    let transport = match (table.get("command"), table.get("url")) {
        (Some(command), None) => Transport::Stdio {
            command: command.as_str()?.into(),
            args: strings(table.get("args"))?,
            env_vars: strings(table.get("env_vars"))?,
        },
        (None, Some(url)) => Transport::Http {
            url: url.as_str()?.into(),
            bearer_token_env_var: match table.get("bearer_token_env_var") {
                None => None,
                Some(item) => Some(item.as_str()?.into()),
            },
        },
        _ => return None,
    };
    let draft = Draft {
        name: name.into(),
        enabled,
        transport,
    };
    validate(&draft).ok()?;
    // Remote placement and private fields are not editable as local transport.
    if table
        .get("experimental_environment")
        .is_some_and(|value| value.as_str() != Some("local"))
        || private_fields(item)
    {
        return None;
    }
    Some(draft)
}
pub fn inspect(target: &Path) -> Result<Snapshot, String> {
    let text = read(target)?;
    let doc = parse(&text)?;
    let writable = can_backup(&doc, &text);
    let mut entries = vec![];
    if let Some(servers) = doc.get("mcp_servers").and_then(Item::as_table_like) {
        if servers.len() > 200 {
            return Err("MCP 配置超过 200 项，停止管理".into());
        }
        for (name, item) in servers.iter() {
            let draft = draft_from(name, item);
            entries.push(Entry {
                name: if name_ok(name) {
                    name.into()
                } else {
                    "名称不在安全展示范围".into()
                },
                enabled: item.get("enabled").and_then(Item::as_bool).or_else(|| {
                    item.as_table_like()
                        .filter(|t| !t.contains_key("enabled"))
                        .map(|_| true)
                }),
                reason: if draft.is_none() {
                    Some("配置包含未支持字段或传输形式；未展示原值".into())
                } else {
                    None
                },
                draft,
            });
        }
    }
    Ok(Snapshot { revision:config_revision(&text), entries, writable, notice:if writable { "仅管理 Codex 用户级配置；未验证服务器连接。客户端重新读取后生效。" } else { "配置含静态环境值、私有字段或未支持的 MCP 形式；保留只读，请先在客户端核对并改用环境变量引用。" }.into() })
}
fn array(values: &[String]) -> Item {
    let mut result = Array::new();
    for item in values {
        result.push(item.as_str());
    }
    value(result)
}
fn patch(text: &str, operation: &Operation) -> Result<(String, Preview), String> {
    let mut doc = parse(text)?;
    if !can_backup(&doc, text) {
        return Err("配置含私有字段，拒绝复制凭据或写入".into());
    }
    let (name, action) = match operation {
        Operation::Save { config, replace } => {
            validate(config)?;
            (&config.name, if *replace { "编辑" } else { "新增" })
        }
        Operation::SetEnabled { name, enabled } => (name, if *enabled { "启用" } else { "停用" }),
        Operation::Remove { name } => (name, "删除"),
    };
    if !name_ok(name) {
        return Err("MCP 名称不在支持范围".into());
    }
    let existing = doc
        .get("mcp_servers")
        .and_then(Item::as_table_like)
        .and_then(|table| table.get(name));
    if existing.is_some_and(|item| item.as_table_like().is_none()) {
        return Err("条目结构不受支持，原配置保留".into());
    }
    let before = existing.and_then(|item| draft_from(name, item));
    let preserves_advanced = !matches!(operation, Operation::Remove { .. })
        && existing.is_some_and(|item| {
            item.as_table_like().is_some_and(|table| {
                table.iter().any(|(key, _)| {
                    ![
                        "command",
                        "args",
                        "env_vars",
                        "url",
                        "bearer_token_env_var",
                        "enabled",
                    ]
                    .contains(&key)
                })
            })
        });
    match operation {
        Operation::Save { replace, .. } if *replace != existing.is_some() => {
            return Err("MCP 条目已变化，请刷新后重新编辑".into())
        }
        Operation::Save { replace: true, .. } if before.is_none() => {
            return Err("此传输形式不支持编辑，原配置保留".into())
        }
        Operation::SetEnabled { .. } | Operation::Remove { .. } if existing.is_none() => {
            return Err("MCP 条目已不存在，请刷新".into())
        }
        _ => {}
    }
    if existing.is_none()
        && doc
            .get("mcp_servers")
            .and_then(Item::as_table_like)
            .is_some_and(|table| table.len() >= 200)
    {
        return Err("MCP 配置达到 200 项上限".into());
    }
    if doc.get("mcp_servers").is_none() {
        doc["mcp_servers"] = Item::Table(Table::new());
    }
    let servers = doc["mcp_servers"]
        .as_table_like_mut()
        .ok_or("MCP 结构不受支持")?;
    match operation {
        Operation::Remove { .. } => {
            servers.remove(name);
        }
        Operation::SetEnabled { enabled, .. } => {
            servers
                .get_mut(name)
                .unwrap()
                .as_table_like_mut()
                .unwrap()
                .insert("enabled", value(*enabled));
        }
        Operation::Save { config, .. } => {
            if !servers.contains_key(name) {
                servers.insert(name, Item::Table(Table::new()));
            }
            let entry = servers.get_mut(name).unwrap().as_table_like_mut().unwrap();
            for key in ["command", "args", "env_vars", "url", "bearer_token_env_var"] {
                entry.remove(key);
            }
            entry.insert("enabled", value(config.enabled));
            match &config.transport {
                Transport::Stdio {
                    command,
                    args,
                    env_vars,
                } => {
                    entry.insert("command", value(command));
                    entry.insert("args", array(args));
                    entry.insert("env_vars", array(env_vars));
                }
                Transport::Http {
                    url,
                    bearer_token_env_var,
                } => {
                    entry.insert("url", value(url));
                    if let Some(reference) = bearer_token_env_var {
                        entry.insert("bearer_token_env_var", value(reference));
                    }
                }
            }
        }
    }
    let after = servers.get(name).and_then(|item| draft_from(name, item));
    let updated = doc.to_string();
    if updated.len() as u64 > MAX_CONFIG {
        return Err("更新后配置超过限额".into());
    }
    let plan_id = config_revision(&format!(
        "{}\n{}",
        config_revision(text),
        serde_json::to_string(operation).map_err(|_| "MCP 操作不可序列化")?
    ));
    Ok((
        updated,
        Preview {
            revision: config_revision(text),
            plan_id,
            name: name.clone(),
            action: action.into(),
            before,
            after,
            preserves_advanced,
        },
    ))
}
pub fn preview(target: &Path, operation: &Operation, revision: &str) -> Result<Preview, String> {
    let text = read(target)?;
    if config_revision(&text) != revision {
        return Err("配置已变化，请刷新后重新预览".into());
    }
    patch(&text, operation).map(|(_, preview)| preview)
}
pub fn apply(
    target: &Path,
    backups: &Path,
    operation: &Operation,
    revision: &str,
    plan_id: &str,
    now: i64,
) -> Result<Applied, String> {
    if !crate::provider::is_codex_config_target(target) {
        return Err("拒绝写入非 Codex 配置路径".into());
    }
    let original = read(target)?;
    if config_revision(&original) != revision {
        return Err("配置已变化，请刷新后重新预览".into());
    }
    let (updated, plan) = patch(&original, operation)?;
    if plan.plan_id != plan_id {
        return Err("预览与当前操作不一致，请重新预览".into());
    }
    commit_text(
        target,
        backups,
        &original,
        &updated,
        revision,
        now,
        "MCP 配置已写入；客户端重新读取后生效，服务器连接未验证。",
    )
}

/// Shared guarded config transaction. Callers bind their own operation and source identity first.
pub(crate) fn commit_text(
    target: &Path,
    backups: &Path,
    original: &str,
    updated: &str,
    revision: &str,
    now: i64,
    success_notice: &str,
) -> Result<Applied, String> {
    if !crate::provider::is_codex_config_target(target) {
        return Err("拒绝写入非 Codex 配置路径".into());
    }
    if updated.len() > MAX_CONFIG as usize || !can_backup(&parse(original)?, original) {
        return Err("配置超出支持范围或含私有字段，未写入".into());
    }
    if !can_backup(&parse(updated)?, updated) {
        return Err("目标配置含未支持的私有字段，未写入".into());
    }
    if config_revision(original) != revision || config_revision(&read(target)?) != revision {
        return Err("配置已变化，请刷新后重新预览".into());
    }
    if original == updated {
        return Err("配置无变化，未写入".into());
    }
    fs::create_dir_all(backups).map_err(|_| "备份目录不可用，未写入")?;
    let backup_name = crate::provider::backup_file_name(now);
    let backup = backups.join(&backup_name);
    if fs::symlink_metadata(&backup).is_ok() {
        return Err("备份名称已占用，未写入，请重试".into());
    }
    crate::atomicfile::atomic_create_validated(&backup, original.as_bytes(), |path| {
        let text = fs::read_to_string(path)?;
        text.parse::<DocumentMut>().map_err(std::io::Error::other)?;
        Ok(())
    })
    .map_err(|_| "备份失败，未写入配置")?;
    if config_revision(&read(target)?) != revision {
        return Err("配置已变化，备份已保留，未写入目标".into());
    }
    fs::create_dir_all(target.parent().ok_or("配置目录不可用")?)
        .map_err(|_| "配置目录不可用，备份已保留")?;
    atomic_replace_validated(target, updated.as_bytes(), |path| {
        let text = fs::read_to_string(path)?;
        text.parse::<DocumentMut>().map_err(std::io::Error::other)?;
        Ok(())
    })
    .map_err(|_| "配置写入失败，备份已保留")?;
    let verified = read(target).is_ok_and(|text| text == updated);
    Ok(Applied {
        backup_name,
        revision: config_revision(&updated),
        verified,
        notice: if verified {
            success_notice
        } else {
            "配置已经写入，但读回未匹配；请刷新核对，可从备份还原。"
        }
        .into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn draft(name: &str) -> Draft {
        Draft {
            name: name.into(),
            enabled: true,
            transport: Transport::Http {
                url: "https://example.invalid/mcp".into(),
                bearer_token_env_var: Some("FIXTURE_AUTH".into()),
            },
        }
    }
    fn fixture() -> (
        crate::testutil::Sandbox,
        std::path::PathBuf,
        std::path::PathBuf,
    ) {
        let sandbox = crate::testutil::Sandbox::new("mcp-config");
        let target = sandbox.path().join(".codex/config.toml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let backups = sandbox.path().join("backups");
        (sandbox, target, backups)
    }
    #[test]
    fn backup_guard_covers_private_fields_in_nested_arrays_and_table_arrays() {
        for text in [
            "[[custom.entries]]\npassword='fixture-value'\n", // nosec: synthetic parser fixture, never a real credential
            "custom=[{ nested={ authorization='fixture-value' } }]\n",
            "custom=[{ args=['--password','fixture-value'] }]\n",
        ] {
            let doc = text.parse::<DocumentMut>().unwrap();
            assert!(!can_backup(&doc, text));
        }
        let safe = "[[skills.config]]\npath='/fixture/SKILL.md'\nenabled=false\n";
        assert!(can_backup(&safe.parse::<DocumentMut>().unwrap(), safe));
    }

    #[test]
    fn add_edit_switch_transport_preserves_unrelated_fields_and_advanced_policy() {
        let text = "# keep\nmodel='fixture-model'\n[mcp_servers.docs]\nurl='https://example.invalid/mcp'\nstartup_timeout_sec=20\ndisabled_tools=['write']\n[mcp_servers.other]\ncommand='fixture-server'\n";
        let config = Draft {
            name: "docs".into(),
            enabled: false,
            transport: Transport::Stdio {
                command: "fixture-cli".into(),
                args: vec!["--readonly".into()],
                env_vars: vec!["FIXTURE_AUTH".into()],
            },
        };
        let (updated, plan) = patch(
            text,
            &Operation::Save {
                config,
                replace: true,
            },
        )
        .unwrap();
        let doc = parse(&updated).unwrap();
        assert_eq!(doc["model"].as_str(), Some("fixture-model"));
        assert_eq!(
            doc["mcp_servers"]["other"]["command"].as_str(),
            Some("fixture-server")
        );
        assert_eq!(
            doc["mcp_servers"]["docs"]["startup_timeout_sec"].as_integer(),
            Some(20)
        );
        assert_eq!(
            doc["mcp_servers"]["docs"]["disabled_tools"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(doc["mcp_servers"]["docs"].get("url").is_none());
        assert!(plan.preserves_advanced);
        assert!(updated.starts_with("# keep"));
    }
    #[test]
    fn preview_is_read_only_apply_backs_up_and_existing_restore_remains_compatible() {
        let (_sandbox, target, backups) = fixture();
        fs::write(&target, "model='fixture'\n").unwrap();
        let original = fs::read_to_string(&target).unwrap();
        let operation = Operation::Save {
            config: draft("docs"),
            replace: false,
        };
        let revision = config_revision(&original);
        let plan = preview(&target, &operation, &revision).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), original);
        assert!(!backups.exists());
        let result = apply(&target, &backups, &operation, &revision, &plan.plan_id, 1).unwrap();
        assert!(result.verified);
        assert_eq!(
            fs::read_to_string(backups.join(&result.backup_name)).unwrap(),
            original
        );
        assert_eq!(inspect(&target).unwrap().entries.len(), 1);
        crate::provider::restore_backup_by_name(&target, &backups, &result.backup_name).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), original);
    }
    #[test]
    fn stale_revision_changed_operation_and_duplicate_name_do_not_write() {
        let (_sandbox, target, backups) = fixture();
        fs::write(&target, "model='fixture'\n").unwrap();
        let original = read(&target).unwrap();
        let op = Operation::Save {
            config: draft("docs"),
            replace: false,
        };
        let plan = preview(&target, &op, &config_revision(&original)).unwrap();
        let different = Operation::Save {
            config: draft("other"),
            replace: false,
        };
        assert!(apply(
            &target,
            &backups,
            &different,
            &plan.revision,
            &plan.plan_id,
            2
        )
        .is_err());
        fs::write(&target, "model='changed'\n").unwrap();
        assert!(apply(&target, &backups, &op, &plan.revision, &plan.plan_id, 2).is_err());
        assert_eq!(read(&target).unwrap(), "model='changed'\n");
        assert!(!backups.exists());
        let text = "[mcp_servers.docs]\nurl='https://example.invalid/mcp'\n";
        assert!(patch(text, &op).is_err());
        assert!(patch(
            "",
            &Operation::Save {
                config: draft("docs"),
                replace: true
            }
        )
        .is_err());
    }
    #[test]
    fn disable_and_remove_are_explicit_patches_without_touching_other_servers() {
        let text="[mcp_servers.docs]\nurl='https://example.invalid/mcp'\n[mcp_servers.other]\ncommand='fixture'\n";
        let (disabled, _) = patch(
            text,
            &Operation::SetEnabled {
                name: "docs".into(),
                enabled: false,
            },
        )
        .unwrap();
        assert_eq!(
            parse(&disabled).unwrap()["mcp_servers"]["docs"]["enabled"].as_bool(),
            Some(false)
        );
        let (removed, plan) = patch(
            &disabled,
            &Operation::Remove {
                name: "docs".into(),
            },
        )
        .unwrap();
        assert!(plan.after.is_none());
        assert!(parse(&removed).unwrap()["mcp_servers"]
            .get("docs")
            .is_none());
        assert_eq!(
            parse(&removed).unwrap()["mcp_servers"]["other"]["command"].as_str(),
            Some("fixture")
        );
    }
    #[test]
    fn corrupt_large_symlink_and_private_configs_are_preserved_without_credential_output() {
        let (_sandbox, target, backups) = fixture();
        fs::write(&target, "[invalid").unwrap();
        assert!(inspect(&target).is_err());
        fs::write(&target, "x".repeat(MAX_CONFIG as usize + 1)).unwrap();
        assert!(inspect(&target).is_err());
        let private="[mcp_servers.docs]\nurl='https://example.invalid/mcp'\nhttp_headers={Authorization='PRIVATE_FIXTURE_VALUE'}\n";
        fs::write(&target, private).unwrap();
        let snapshot = inspect(&target).unwrap();
        assert!(!snapshot.writable);
        assert!(snapshot.entries[0].draft.is_none());
        assert!(!serde_json::to_string(&snapshot)
            .unwrap()
            .contains("PRIVATE_FIXTURE_VALUE"));
        assert!(preview(
            &target,
            &Operation::Remove {
                name: "docs".into()
            },
            &snapshot.revision
        )
        .is_err());
        assert_eq!(read(&target).unwrap(), private);
        assert!(!backups.exists());
        assert!(!can_backup(
            &parse("api_key='PRIVATE_FIXTURE_VALUE'").unwrap(), // nosec: 本地私有字段测试的固定占位值，不是真实凭据
            "api_key='PRIVATE_FIXTURE_VALUE'" // nosec: 本地私有字段测试的固定占位值，不是真实凭据
        ));
        #[cfg(unix)]
        {
            fs::remove_file(&target).unwrap();
            std::os::unix::fs::symlink("missing.toml", &target).unwrap();
            assert!(inspect(&target).is_err());
        }
    }
    #[test]
    fn url_auth_raw_secret_extra_fields_and_ambiguous_transport_are_refused() {
        let mut config = draft("docs");
        config.transport = Transport::Http {
            url: "https://user:fixture@example.invalid/mcp".into(),
            bearer_token_env_var: None,
        };
        assert!(validate(&config).is_err());
        config.transport = Transport::Stdio {
            command: "fixture".into(),
            args: vec!["--api-key=PRIVATE_FIXTURE_VALUE".into()],
            env_vars: vec![],
        };
        assert!(validate(&config).is_err());
        let extra = serde_json::json!({"name":"docs","enabled":true,"transport":{"kind":"http","url":"https://example.invalid/mcp","bearer_token_env_var":null,"token":"PRIVATE_FIXTURE_VALUE"}});
        assert!(serde_json::from_value::<Draft>(extra).is_err());
        assert!(draft_from(
            "docs",
            &parse("[mcp_servers.docs]\nurl='https://example.invalid/mcp'\ncommand='fixture'")
                .unwrap()["mcp_servers"]["docs"]
        )
        .is_none());
    }
    #[test]
    fn missing_config_can_be_created_and_backup_collision_preserves_existing_history() {
        let (_sandbox, target, backups) = fixture();
        let operation = Operation::Save {
            config: draft("docs"),
            replace: false,
        };
        let plan = preview(&target, &operation, &config_revision("")).unwrap();
        fs::create_dir_all(&backups).unwrap();
        fs::write(backups.join("config-3.toml"), "model='keep'").unwrap();
        assert!(apply(
            &target,
            &backups,
            &operation,
            &plan.revision,
            &plan.plan_id,
            3
        )
        .is_err());
        assert!(!target.exists());
        assert_eq!(
            read(&backups.join("config-3.toml")).unwrap(),
            "model='keep'"
        );
        assert!(
            apply(
                &target,
                &backups,
                &operation,
                &plan.revision,
                &plan.plan_id,
                4
            )
            .unwrap()
            .verified
        );
    }
}

#[cfg(test)]
mod argument_boundary_tests {
    use super::*;
    #[test]
    fn ordinary_package_names_and_token_counts_are_not_credential_arguments() {
        let mut config = Draft {
            name: "fixture".into(),
            enabled: true,
            transport: Transport::Stdio {
                command: "fixture-program".into(),
                args: vec!["@example/token-mcp".into(), "--tokens-limit=1024".into()],
                env_vars: vec![],
            },
        };
        assert!(validate(&config).is_ok());
        config.transport = Transport::Stdio {
            command: "fixture-program".into(),
            args: vec!["--token".into(), "PRIVATE_FIXTURE_VALUE".into()],
            env_vars: vec![],
        };
        assert!(validate(&config).is_err());
        assert!(credential_arg("API_KEY=PRIVATE_FIXTURE_VALUE"));
    }
}

#[cfg(test)]
mod provider_auth_boundary_tests {
    use super::*;
    #[test]
    fn literal_provider_auth_and_auth_arguments_block_backup_but_environment_references_do_not() {
        let mut doc = DocumentMut::new();
        doc["model_providers"] = Item::Table(Table::new());
        doc["model_providers"]["fixture"] = Item::Table(Table::new());
        doc["model_providers"]["fixture"]["experimental_bearer_token"] =
            value("PRIVATE_FIXTURE_VALUE");
        let text = doc.to_string();
        assert!(!can_backup(&doc, &text));
        assert!(patch(
            &text,
            &Operation::Save {
                config: Draft {
                    name: "docs".into(),
                    enabled: true,
                    transport: Transport::Http {
                        url: "https://example.invalid/mcp".into(),
                        bearer_token_env_var: None
                    }
                },
                replace: false
            }
        )
        .is_err());
        doc["model_providers"]["fixture"]
            .as_table_mut()
            .unwrap()
            .remove("experimental_bearer_token");
        doc["model_providers"]["fixture"]["env_key"] = value("FIXTURE_AUTH");
        assert!(can_backup(&doc, &doc.to_string()));
        doc["model_providers"]["fixture"]["auth"] = Item::Table(Table::new());
        doc["model_providers"]["fixture"]["auth"]["args"] =
            array(&["--token".into(), "PRIVATE_FIXTURE_VALUE".into()]);
        assert!(!can_backup(&doc, &doc.to_string()));
    }
}
