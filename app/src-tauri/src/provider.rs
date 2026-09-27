//! Codex profile groundwork. This module never reads or writes the user's
//! installed configuration on its own; Phase 2 supplies a chosen destination.

use crate::atomicfile::atomic_replace_validated;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;
use toml_edit::{value, DocumentMut};

// 这里原本有一套 `MaskedSecret` / `ProviderProfilePreview`（v0.0.160 建）。
// 已删除，理由：它没有消费方，而**这轮的保护方式更强**——
// 档位结构里**根本没有承载密钥的字段**（只存 `env_key` 这个变量名），
// 并且校验会在门口拒绝「一整串像密钥的东西被填进 env_key」。
// 留着一套没人调的掩码类型，会让人以为「密钥是被掩码保护的」，
// 而真实情况是「密钥压根进不来」。真需要显示掩码时，历史里有它。
fn invalid_toml() -> io::Error {
    // Parser diagnostics may quote the input line, which could contain a key.
    io::Error::new(io::ErrorKind::InvalidData, "invalid provider TOML")
}

fn replace_value_preserving_decor(doc: &mut DocumentMut, key: &str, new_value: &str) {
    let decor = doc
        .get(key)
        .and_then(|item| item.as_value())
        .map(|value| value.decor().clone());
    let mut item = value(new_value);
    if let Some(decor) = decor {
        *item
            .as_value_mut()
            .expect("toml_edit::value returns a value")
            .decor_mut() = decor;
    }
    doc[key] = item;
}

/// Update the native Codex profile layer while retaining unrelated TOML
/// comments, key order, plugin tables, and arrays. Provider definitions belong
/// in the base config; this layer only selects their name. A running Codex
/// process must be restarted separately before this change can take effect.
pub(crate) fn update_codex_profile(
    target: &Path,
    model: &str,
    model_provider: Option<&str>,
) -> io::Result<()> {
    if !target
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|name| name.ends_with(".config.toml") && name != ".config.toml")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a Codex profile path",
        ));
    }
    if model.trim().is_empty() || model_provider.is_some_and(|p| p.trim().is_empty()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty Codex profile field",
        ));
    }

    let original = match fs::read_to_string(target) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let mut doc: DocumentMut = original.parse().map_err(|_| invalid_toml())?;
    replace_value_preserving_decor(&mut doc, "model", model);
    if let Some(provider) = model_provider {
        replace_value_preserving_decor(&mut doc, "model_provider", provider);
    } else {
        // No override means inherit from the base config, not keep the previous one.
        doc.as_table_mut().remove("model_provider");
    }
    let updated = doc.to_string();
    atomic_replace_validated(target, updated.as_bytes(), |staged| {
        let text = fs::read_to_string(staged)?;
        text.parse::<DocumentMut>().map_err(|_| invalid_toml())?;
        Ok(())
    })
}

// MARK: - 档位（Phase 2）

/// 一个档位：记「用哪家 provider + 哪个模型」。
///
/// **不含任何凭据**：只记 `env_key` 这个**变量名**，密钥由用户自己放进环境变量。
/// 这是 ADR 0009 与 AGENTS.md 的红线（本项目不存储任何凭据），
/// 也是这套结构能成立的前提——所以 `env_key` 的校验只认「像变量名的东西」（见下）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodexProfile {
    /// 我们的标识。会写进 `[profiles.<id>]`，所以只允许保守字符集
    pub id: String,
    pub name: String,
    pub model: String,
    /// `[model_providers.<id>]` 的键
    pub provider_id: String,
    pub provider_name: String,
    pub base_url: String,
    /// **只存变量名**，不存值
    pub env_key: String,
    /// `chat` | `responses`
    pub wire_api: String,
}

impl CodexProfile {
    /// 归一化并校验。**失败即拒绝保存**，不「尽力而为」：
    /// 一个写坏的档位会在切换时把用户的 `config.toml` 改成 Codex 读不懂的东西。
    pub fn normalized(mut self) -> Result<Self, String> {
        for (label, value) in [
            ("id", &self.id),
            ("provider_id", &self.provider_id),
            ("model", &self.model),
            ("base_url", &self.base_url),
        ] {
            if value.trim().is_empty() {
                return Err(format!("{label} 不能为空"));
            }
        }
        // 标识与变量名只允许保守字符集：它们会进 TOML 的键与值
        for (label, value) in [("id", &self.id), ("provider_id", &self.provider_id)] {
            if !value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
            {
                return Err(format!("{label} 只允许字母数字与 - _ ."));
            }
        }
        if !self
            .env_key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        {
            return Err("env_key 只允许大写字母、数字与下划线（它是环境变量名）".into());
        }
        if !self.base_url.starts_with("https://") && !self.base_url.starts_with("http://") {
            return Err("base_url 必须是 http(s) 地址".into());
        }
        if self.wire_api != "chat" && self.wire_api != "responses" {
            return Err("wire_api 只能是 chat 或 responses".into());
        }
        // 归一化留白，避免带空格的键写进 TOML 之后对不上
        self.id = self.id.trim().to_string();
        self.name = self.name.trim().to_string();
        self.model = self.model.trim().to_string();
        self.provider_id = self.provider_id.trim().to_string();
        self.provider_name = self.provider_name.trim().to_string();
        self.base_url = self.base_url.trim().to_string();
        self.env_key = self.env_key.trim().to_string();
        if self.name.is_empty() {
            self.name = self.id.clone();
        }
        if self.provider_name.is_empty() {
            self.provider_name = self.provider_id.clone();
        }
        Ok(self)
    }
}

/// 纯函数：给定 `config.toml` 原文与档位，产出新文本。
///
/// **只动三处**：顶层 `model` / `model_provider`，以及 `[model_providers.<id>]` 表。
/// 其余内容（注释、键顺序、缩进、插件表、数组）逐字保留——用户手写的格式被冲掉
/// 是最容易被立刻察觉的破坏（Phase 2 第 7 条）。
pub fn plan_codex_apply(original: &str, profile: &CodexProfile) -> io::Result<String> {
    let mut doc: DocumentMut = original.parse().map_err(|_| invalid_toml())?;
    replace_value_preserving_decor(&mut doc, "model", &profile.model);
    replace_value_preserving_decor(&mut doc, "model_provider", &profile.provider_id);

    // provider 表：已存在就只更新我们认识的四个键（用户额外写的 retry 等**不删**）
    let providers = doc["model_providers"]
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let table = providers
        .as_table_mut()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "model_providers 不是表"))?;
    let entry = table
        .entry(&profile.provider_id)
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let entry = entry
        .as_table_mut()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "provider 项不是表"))?;
    for (key, val) in [
        ("name", profile.provider_name.as_str()),
        ("base_url", profile.base_url.as_str()),
        ("env_key", profile.env_key.as_str()),
        ("wire_api", profile.wire_api.as_str()),
    ] {
        let decor = entry
            .get(key)
            .and_then(|item| item.as_value())
            .map(|value| value.decor().clone());
        let mut item = value(val);
        if let Some(decor) = decor {
            *item
                .as_value_mut()
                .expect("toml_edit::value returns a value")
                .decor_mut() = decor;
        }
        entry[key] = item;
    }
    Ok(doc.to_string())
}

/// 这台机器上的 Codex 配置文件：`$CODEX_HOME/config.toml`，缺省 `~/.codex/config.toml`。
pub fn codex_config_path() -> Option<std::path::PathBuf> {
    if let Ok(home) = std::env::var("CODEX_HOME") {
        if !home.trim().is_empty() {
            return Some(std::path::Path::new(home.trim()).join("config.toml"));
        }
    }
    dirs::home_dir().map(|home| home.join(".codex").join("config.toml"))
}

/// 目标必须是 `<某个目录>/.codex/config.toml` 或 `$CODEX_HOME/config.toml`。
///
/// 这一层校验是**唯一**允许碰真实配置的入口的护栏：文件名必须正好是 `config.toml`，
/// 且父目录名必须是 `.codex`（或由 `CODEX_HOME` 指定）。这样任何拼错的路径
/// ——包括差点被误写成目标的 `auth.json`——都进不来。
pub fn is_codex_config_target(target: &Path) -> bool {
    let name_ok = target
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|name| name == "config.toml");
    if !name_ok {
        return false;
    }
    if let Ok(home) = std::env::var("CODEX_HOME") {
        if !home.trim().is_empty() && target == std::path::Path::new(home.trim()).join("config.toml")
        {
            return true;
        }
    }
    target
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .is_some_and(|dir| dir == ".codex")
}

/// 备份文件名：`config-<时间戳>.toml`。时间戳由调用方给（可离线断言，不必等真实时钟）。
pub fn backup_file_name(now_ms: i64) -> String {
    format!("config-{now_ms}.toml")
}

/// 切换档位：**先备份、再原子写**。
///
/// 顺序不能反：先写后备份，写失败时连原样都拿不回来。备份落在我们自己的目录
/// （不塞进用户的 `~/.codex`，那里已经够乱了）。返回备份文件路径，供「还原」用。
pub fn apply_codex_profile(
    target: &Path,
    backup_dir: &Path,
    profile: &CodexProfile,
    now_ms: i64,
) -> io::Result<std::path::PathBuf> {
    if !is_codex_config_target(target) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "拒绝写入非 Codex 配置路径",
        ));
    }
    let original = match fs::read_to_string(target) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    fs::create_dir_all(backup_dir)?;
    let backup = backup_dir.join(backup_file_name(now_ms));
    // 备份用原子写：备份本身写一半就没有可还原的东西了
    atomic_replace_validated(&backup, original.as_bytes(), |staged| {
        fs::read_to_string(staged).map(|_| ())
    })?;

    let updated = plan_codex_apply(&original, profile)?;
    atomic_replace_validated(target, updated.as_bytes(), |staged| {
        let text = fs::read_to_string(staged)?;
        text.parse::<DocumentMut>().map_err(|_| invalid_toml())?;
        Ok(())
    })?;
    Ok(backup)
}

/// 从备份还原（同样原子写）。备份必须自己也是一份合法 TOML——
/// 否则还原会把用户的配置换成一份读不懂的东西。
pub fn restore_codex_backup(target: &Path, backup: &Path) -> io::Result<()> {
    if !is_codex_config_target(target) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "拒绝写入非 Codex 配置路径",
        ));
    }
    let text = fs::read_to_string(backup)?;
    text.parse::<DocumentMut>().map_err(|_| invalid_toml())?;
    atomic_replace_validated(target, text.as_bytes(), |staged| {
        let staged_text = fs::read_to_string(staged)?;
        staged_text.parse::<DocumentMut>().map_err(|_| invalid_toml())?;
        Ok(())
    })
}

/// 这台机器上装了哪些可切换的工具。第一阶段**只有 Codex**：
/// 别的工具要等各自核实过配置文件形状再立项（Phase 2 第 4 条）。
#[derive(Debug, Clone, Serialize)]
pub struct ToolScan {
    pub tool: String,
    pub label: String,
    pub installed: bool,
    pub config_path: Option<String>,
}

pub fn scan_tools() -> Vec<ToolScan> {
    let codex = codex_config_path();
    let (installed, path) = match &codex {
        Some(p) => (
            p.exists(),
            Some(p.to_string_lossy().to_string()),
        ),
        None => (false, None),
    };
    vec![ToolScan {
        tool: "codex".into(),
        label: "Codex".into(),
        installed,
        config_path: path,
    }]
}

/// 当前生效的档位名：读 `config.toml` 里的**码（`model_provider`）**，
/// 反查我们存的档位。读不到就 `None`——**不猜**。
pub fn active_provider_id(original: &str) -> Option<String> {
    let doc: DocumentMut = original.parse().ok()?;
    doc.get("model_provider")
        .and_then(|item| item.as_str())
        .map(|s| s.to_string())
}

// MARK: - 给界面的 DTO（搬到这里是为了让「字段哨兵」够得着）

/// 界面读的状态。`limitations` 与切换结果里的是**同一段话**（见 `PROVIDER_LIMITATIONS`）。
#[derive(Debug, Clone, Serialize)]
pub struct ProviderStatus {
    pub installed: bool,
    pub config_path: Option<String>,
    /// `config.toml` 里当前生效的 provider id（读不到就是 `None`——**不猜**）
    pub active_provider_id: Option<String>,
    /// 反查出来的档位 id（能对上才有）
    pub active_profile_id: Option<String>,
    pub profile_count: usize,
    pub limitations: &'static str,
}

/// 一次切换的结果。**必带 `limitations`**：界面上少显示一次就没有第二道防线。
#[derive(Debug, Clone, Serialize)]
pub struct ProviderApplyResult {
    pub config_path: String,
    pub backup_name: String,
    pub limitations: &'static str,
}

/// **能力边界**：只要涉及档位切换，这段就随结果一起返回。
///
/// 放在 Rust 侧（而不是让界面各写一遍）是因为它必须**逐字**出现：
/// 用户切了档以为能用别家的模型、结果不能用，是这次改造最容易招来的误解。
pub const PROVIDER_LIMITATIONS: &str = "只切换本机已有的 Codex 档位（同厂商多账号、模型与 provider 选择），\
**不含跨厂商模型能力**：切了档不等于那个模型就能用。API key 不由本应用保存——\
档位只记环境变量名，你需要自己把它设进环境变量。切换后需重启正在运行的 Codex 会话才生效。";

// MARK: - 本地档位库（我们自己的文件，不含任何凭据）

/// 档位库目录：`<app data>/AgentIsland/providers/`。
/// 备份也放这儿（`backups/`），不塞进用户的 `~/.codex`。
pub fn store_dir() -> std::path::PathBuf {
    crate::settings::config_dir().join("providers")
}

/// 档位库：一个 JSON 文件。**读坏了就当空清单**（降级而不是崩）——
/// 一份档位列表解析失败不该让整个应用起不来。
pub struct ProviderStore {
    dir: std::path::PathBuf,
}

impl ProviderStore {
    pub fn new(dir: std::path::PathBuf) -> Self {
        ProviderStore { dir }
    }

    pub fn at_default() -> Self {
        ProviderStore::new(store_dir())
    }

    fn profiles_path(&self) -> std::path::PathBuf {
        self.dir.join("profiles.json")
    }

    /// 备份目录
    pub fn backups_dir(&self) -> std::path::PathBuf {
        self.dir.join("backups")
    }

    pub fn list(&self) -> Vec<CodexProfile> {
        let Ok(text) = fs::read_to_string(self.profiles_path()) else {
            return Vec::new();
        };
        serde_json::from_str::<Vec<CodexProfile>>(&text).unwrap_or_default()
    }

    /// 保存（同 id 覆盖）。校验在 `CodexProfile::normalized` 里，**这里拒绝写坏的档位**。
    pub fn save(&self, profile: CodexProfile) -> Result<CodexProfile, String> {
        let profile = profile.normalized()?;
        let mut list = self.list();
        match list.iter_mut().find(|p| p.id == profile.id) {
            Some(existing) => *existing = profile.clone(),
            None => list.push(profile.clone()),
        }
        let json = serde_json::to_string_pretty(&list).map_err(|e| e.to_string())?;
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        atomic_replace_validated(&self.profiles_path(), json.as_bytes(), |staged| {
            let text = fs::read_to_string(staged)?;
            serde_json::from_str::<Vec<CodexProfile>>(&text)
                .map(|_| ())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "profile JSON"))
        })
        .map_err(|e| e.to_string())?;
        Ok(profile)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let mut list = self.list();
        list.retain(|p| p.id != id);
        let json = serde_json::to_string_pretty(&list).map_err(|e| e.to_string())?;
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        atomic_replace_validated(&self.profiles_path(), json.as_bytes(), |staged| {
            let text = fs::read_to_string(staged)?;
            serde_json::from_str::<Vec<CodexProfile>>(&text)
                .map(|_| ())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "profile JSON"))
        })
        .map_err(|e| e.to_string())
    }
}

/// 一份备份的样子（给界面列出来选）
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BackupInfo {
    pub name: String,
    pub created_ms: i64,
    pub bytes: u64,
}

/// 列出备份，**新的在前**。名字不合格的文件不进列表（我们只认得自己写的那些）。
pub fn list_backups(dir: &Path) -> Vec<BackupInfo> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<BackupInfo> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            let created_ms = parse_backup_name(&name)?;
            let bytes = entry.metadata().ok()?.len();
            Some(BackupInfo {
                name,
                created_ms,
                bytes,
            })
        })
        .collect();
    out.sort_by(|a, b| b.created_ms.cmp(&a.created_ms).then(a.name.cmp(&b.name)));
    out
}

/// `config-<毫秒时间戳>.toml` → 时间戳。名字不合格返回 `None`（**不猜**）。
pub fn parse_backup_name(name: &str) -> Option<i64> {
    let rest = name.strip_prefix("config-")?.strip_suffix(".toml")?;
    if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    rest.parse::<i64>().ok()
}

/// 按**名字**还原（而不是按路径）：界面只能选我们列出来的那些备份，
/// 传别的名字一律拒绝——否则「还原」就成了「用任意文件覆盖用户配置」的入口。
pub fn restore_backup_by_name(target: &Path, dir: &Path, name: &str) -> io::Result<()> {
    if parse_backup_name(name).is_none() || name.contains('/') || name.contains('\\') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "备份名不合格",
        ));
    }
    let backup = dir.join(name);
    if !backup.is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "备份不存在"));
    }
    restore_codex_backup(target, &backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 只服务测试沙箱的取名（生产侧那份在 `atomicfile.rs` 里）
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new() -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let serial = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "agentisland-provider-test-{}-{stamp}-{serial}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn only_target_remains(root: &Path, target_name: &str) {
        let entries: Vec<_> = fs::read_dir(root)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            entries,
            vec![target_name],
            "staging file leaked: {entries:?}"
        );
    }

    #[test]
    fn codex_profile_update_preserves_unrelated_formatting_and_comments() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("fixture.config.toml");
        let before = "# Keep this comment\nmodel = 'old-model' # chosen by hand\nmodel_provider = 'old-provider' # account selection\n\n[plugins.\"fixture@marketplace\"]\nenabled = true\npaths = [\n  'alpha',\n  'beta',\n]\n";
        fs::write(&target, before).unwrap();
        update_codex_profile(&target, "new-model", Some("official-provider")).unwrap();
        let after = fs::read_to_string(&target).unwrap();
        assert!(after.contains("# Keep this comment"));
        assert!(
            after.contains("# chosen by hand"),
            "inline model comment was lost: {after}"
        );
        assert!(
            after.contains("# account selection"),
            "inline provider comment was lost: {after}"
        );
        assert!(after.contains("[plugins.\"fixture@marketplace\"]\nenabled = true\npaths = [\n  'alpha',\n  'beta',\n]"));
        let doc: DocumentMut = after.parse().unwrap();
        assert_eq!(doc["model"].as_str(), Some("new-model"));
        assert_eq!(doc["model_provider"].as_str(), Some("official-provider"));
        only_target_remains(&sandbox.0, "fixture.config.toml");
    }

    #[test]
    fn codex_profile_update_creates_a_missing_native_profile() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("fixture.config.toml");
        update_codex_profile(&target, "new-model", Some("official-provider")).unwrap();
        let text = fs::read_to_string(&target).unwrap();
        let doc: DocumentMut = text.parse().unwrap();
        assert_eq!(doc["model"].as_str(), Some("new-model"));
        assert_eq!(doc["model_provider"].as_str(), Some("official-provider"));
        only_target_remains(&sandbox.0, "fixture.config.toml");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&target).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn invalid_existing_profile_and_auth_path_cannot_be_overwritten() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("fixture.config.toml");
        fs::write(&target, "invalid = [").unwrap();
        assert_eq!(
            update_codex_profile(&target, "new-model", None)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "invalid = [");
        let auth = sandbox.0.join("auth.json");
        fs::write(&auth, "leave this file alone").unwrap();
        assert_eq!(
            update_codex_profile(&auth, "new-model", None)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(fs::read_to_string(auth).unwrap(), "leave this file alone");
    }

    #[test]
    fn clearing_provider_override_keeps_the_rest_of_the_profile() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join("fixture.config.toml");
        fs::write(
            &target,
            "# keep\nmodel = 'old'\nmodel_provider = 'old-provider'\n",
        )
        .unwrap();
        update_codex_profile(&target, "new", None).unwrap();
        let text = fs::read_to_string(&target).unwrap();
        assert!(text.starts_with("# keep\n"));
        let doc: DocumentMut = text.parse().unwrap();
        assert_eq!(doc["model"].as_str(), Some("new"));
        assert!(doc.get("model_provider").is_none());
    }
}

#[cfg(test)]
mod phase2_tests {
    use super::*;
    use std::path::PathBuf;

    /// 沙箱取名要带**序号**：`as_nanos()` 在 macOS 上分辨率很粗，
    /// 并行的两个用例完全可能取到同一个值 —— 那样两个用例共用一个目录，
    /// 先结束的那个 `Drop` 会把另一个的文件删掉。症状是「某次跑挂了三条、再跑全绿」。
    /// （同一类问题在 selftest 里已经踩过一次。）
    static NEXT_SANDBOX: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let serial = NEXT_SANDBOX.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "agentisland-provider2-{}-{stamp}-{serial}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        /// 造一个 `<sandbox>/.codex/config.toml`（生产侧的路径校验只认这个形状）
        fn codex_config(&self) -> PathBuf {
            let dir = self.0.join(".codex");
            fs::create_dir_all(&dir).unwrap();
            dir.join("config.toml")
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn profile() -> CodexProfile {
        CodexProfile {
            id: "work".into(),
            name: "工作账号".into(),
            model: "gpt-5".into(),
            provider_id: "acme".into(),
            provider_name: "Acme".into(),
            base_url: "https://api.example.invalid/v1".into(),
            env_key: "ACME_API_KEY".into(),
            wire_api: "responses".into(),
        }
    }

    #[test]
    fn applying_a_profile_backs_up_and_restores_byte_for_byte() {
        let sandbox = Sandbox::new();
        let target = sandbox.codex_config();
        let before = "# 手写的注释\nmodel = 'old-model' # 保留我\n\n[plugins.x]\nenabled = true\n";
        fs::write(&target, before).unwrap();
        let backups = sandbox.0.join("providers/backups");

        let backup = apply_codex_profile(&target, &backups, &profile(), 1_700_000_000_000).unwrap();
        assert_eq!(
            backup.file_name().unwrap().to_string_lossy(),
            "config-1700000000000.toml"
        );
        // 备份内容 = 改动前的原文，逐字节
        assert_eq!(fs::read_to_string(&backup).unwrap(), before);
        let after = fs::read_to_string(&target).unwrap();
        assert!(after.contains("# 手写的注释"));
        assert!(after.contains("# 保留我"), "行内注释被冲掉了：{after}");
        assert!(after.contains("[plugins.x]"));
        let doc: DocumentMut = after.parse().unwrap();
        assert_eq!(doc["model"].as_str(), Some("gpt-5"));
        assert_eq!(doc["model_provider"].as_str(), Some("acme"));
        assert_eq!(
            doc["model_providers"]["acme"]["wire_api"].as_str(),
            Some("responses")
        );
        assert_eq!(
            doc["model_providers"]["acme"]["env_key"].as_str(),
            Some("ACME_API_KEY")
        );

        // 一路改坏，再从备份还原：必须回到原样
        fs::write(&target, "model = 'broken'\n").unwrap();
        restore_codex_backup(&target, &backup).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), before);
    }

    #[test]
    fn a_broken_existing_config_is_never_half_written() {
        let sandbox = Sandbox::new();
        let target = sandbox.codex_config();
        fs::write(&target, "model = [ 越写越坏").unwrap();
        let backups = sandbox.0.join("providers/backups");
        assert!(apply_codex_profile(&target, &backups, &profile(), 1).is_err());
        // 原文件逐字节不动，暂存文件也没留下
        assert_eq!(fs::read_to_string(&target).unwrap(), "model = [ 越写越坏");
        let leftovers: Vec<_> = fs::read_dir(target.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(leftovers, vec!["config.toml"], "暂存文件泄漏：{leftovers:?}");
    }

    /// **分辨「先备份后写」与「先写后备份」**的用例。
    ///
    /// 让备份那一步注定失败（备份路径已是一个符号链接，`atomicfile` 拒写符号链接）：
    /// 正确顺序下配置**一个字节都不动**；先写后备份则会先把配置改掉、再报错，
    /// 用户既没拿到备份、配置又被改了。
    #[cfg(unix)]
    #[test]
    fn a_failed_backup_leaves_the_config_untouched() {
        let sandbox = Sandbox::new();
        let target = sandbox.codex_config();
        let before = "# keep\nmodel = 'old'\n";
        fs::write(&target, before).unwrap();
        let backups = sandbox.0.join("providers/backups");
        fs::create_dir_all(&backups).unwrap();
        // 备份目标位置先占一个符号链接
        let elsewhere = sandbox.0.join("elsewhere.toml");
        fs::write(&elsewhere, "not a backup").unwrap();
        std::os::unix::fs::symlink(&elsewhere, backups.join(backup_file_name(42))).unwrap();

        assert!(
            apply_codex_profile(&target, &backups, &profile(), 42).is_err(),
            "备份写不进去时必须报错"
        );
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            before,
            "备份失败却把配置改掉了：用户既没备份、配置又被动了"
        );
        assert_eq!(fs::read_to_string(&elsewhere).unwrap(), "not a backup");
    }

    #[test]
    fn only_a_real_codex_config_path_may_be_written() {
        let sandbox = Sandbox::new();
        let _ = sandbox.codex_config(); // 先建出 .codex/，否则下面的写入会因目录不存在而失败
        let auth = sandbox.0.join(".codex/auth.json");
        fs::write(&auth, "leave me alone").unwrap();
        assert!(!is_codex_config_target(&auth), "auth.json 绝不能是目标");
        assert!(is_codex_config_target(&sandbox.codex_config()));
        // 别的目录里同名文件也不行（只认 .codex 或 $CODEX_HOME）
        let other = sandbox.0.join("elsewhere/config.toml");
        fs::create_dir_all(other.parent().unwrap()).unwrap();
        assert!(!is_codex_config_target(&other));
        let err = apply_codex_profile(&auth, &sandbox.0.join("b"), &profile(), 1).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(fs::read_to_string(&auth).unwrap(), "leave me alone");
    }

    #[test]
    fn a_bad_profile_is_rejected_before_it_can_touch_the_config() {
        let cases: Vec<(&str, CodexProfile)> = vec![
            ("空 model", CodexProfile { model: "  ".into(), ..profile() }),
            (
                "id 带空格",
                CodexProfile { id: "my profile".into(), ..profile() },
            ),
            (
                "base_url 不是 http",
                CodexProfile { base_url: "ftp://x".into(), ..profile() },
            ),
            (
                "wire_api 不认识",
                CodexProfile { wire_api: "grpc".into(), ..profile() },
            ),
            (
                "env_key 里有小写",
                CodexProfile { env_key: "acme_key".into(), ..profile() },
            ),
            (
                "env_key 像一整串密钥",
                CodexProfile {
                    env_key: "sk-abc123def456".into(),
                    ..profile()
                },
            ),
        ];
        for (label, bad) in cases {
            assert!(bad.normalized().is_err(), "该被拒绝：{label}");
        }
        // 正常档位归一化后原样通过，且空展示名回落到 id
        let ok = CodexProfile {
            name: "  ".into(),
            provider_name: String::new(),
            ..profile()
        }
        .normalized()
        .unwrap();
        assert_eq!(ok.name, "work");
        assert_eq!(ok.provider_name, "acme");
    }

    #[test]
    fn applying_twice_keeps_the_users_extra_provider_keys() {
        let sandbox = Sandbox::new();
        let target = sandbox.codex_config();
        // 用户在同一个 provider 表里自己加了重试参数：**不能被我们删掉**
        fs::write(
            &target,
            "model = 'x'\n\n[model_providers.acme]\nname = 'Acme old'\nrequest_max_retries = 7\n",
        )
        .unwrap();
        let backups = sandbox.0.join("providers/backups");
        apply_codex_profile(&target, &backups, &profile(), 1).unwrap();
        let doc: DocumentMut = fs::read_to_string(&target).unwrap().parse().unwrap();
        assert_eq!(
            doc["model_providers"]["acme"]["request_max_retries"].as_integer(),
            Some(7),
            "用户自己写的键被删了"
        );
        assert_eq!(
            doc["model_providers"]["acme"]["name"].as_str(),
            Some("Acme")
        );
    }

    #[test]
    fn the_profile_store_round_trips_and_survives_a_corrupted_file() {
        let sandbox = Sandbox::new();
        let store = ProviderStore::new(sandbox.0.join("providers"));
        assert!(store.list().is_empty());
        store.save(profile()).unwrap();
        store
            .save(CodexProfile {
                name: "第二个".into(),
                id: "personal".into(),
                provider_id: "acme2".into(),
                ..profile()
            })
            .unwrap();
        assert_eq!(store.list().len(), 2);
        // 同 id 覆盖而不是追加
        store.save(CodexProfile { model: "gpt-5-mini".into(), ..profile() }).unwrap();
        let list = store.list();
        assert_eq!(list.len(), 2);
        assert_eq!(
            list.iter().find(|p| p.id == "work").unwrap().model,
            "gpt-5-mini"
        );
        // 写坏的 JSON：降级为空清单，**不 panic**
        fs::write(sandbox.0.join("providers/profiles.json"), "{ not json").unwrap();
        assert!(store.list().is_empty());
        // 删除
        let store = ProviderStore::new(sandbox.0.join("providers"));
        store.save(profile()).unwrap();
        store.delete("work").unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn the_outbound_profile_shape_carries_no_credential_shaped_string() {
        // DTO 里不该有任何「看起来像密钥」的串。**逐个看字符串「值」**，
        // 不要拿分隔符去切整段 JSON——那样连键名都会被当成可疑串（我第一版就是这么错的）。
        let json = serde_json::to_string(&profile()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mut strings = Vec::new();
        collect_strings(&value, &mut strings);
        for token in &strings {
            let entropy_ish = token.len() >= 20
                && token
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            let prefixed = ["sk-", "sk_", "ghp_", "github_pat_", "AKIA", "xoxb-"]
                .iter()
                .any(|p| token.starts_with(p));
            assert!(
                !entropy_ish && !prefixed,
                "档位 JSON 的值里出现了像密钥的串：{token:?}（完整 JSON：{json}）"
            );
        }
        // 我们**故意**存的只有环境变量名，而不是它的值
        assert!(strings.contains(&"ACME_API_KEY".to_string()));
        assert!(!json.contains("api_key\":\""), "不该有承载密钥值的字段");
    }

    fn collect_strings(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::String(s) => out.push(s.clone()),
            serde_json::Value::Array(items) => {
                for item in items {
                    collect_strings(item, out);
                }
            }
            serde_json::Value::Object(map) => {
                for item in map.values() {
                    collect_strings(item, out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn backups_are_listed_newest_first_and_odd_names_are_ignored() {
        let sandbox = Sandbox::new();
        let backups = sandbox.0.join("providers/backups");
        fs::create_dir_all(&backups).unwrap();
        for stamp in [1_000i64, 3_000, 2_000] {
            fs::write(backups.join(backup_file_name(stamp)), "model = 'x'\n").unwrap();
        }
        // 不属于我们的文件：不进列表，也不能被「按名还原」
        fs::write(backups.join("notes.txt"), "hand notes").unwrap();
        fs::write(backups.join("config-abc.toml"), "model = 'x'\n").unwrap();
        let list = list_backups(&backups);
        assert_eq!(
            list.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(),
            vec![
                "config-3000.toml",
                "config-2000.toml",
                "config-1000.toml"
            ]
        );
        assert_eq!(parse_backup_name("config-2000.toml"), Some(2_000));
        assert_eq!(parse_backup_name("config-abc.toml"), None);
        assert_eq!(parse_backup_name("config-.toml"), None);

        // 路径穿越与不存在的备份都被拒绝
        let target = sandbox.codex_config();
        fs::write(&target, "model = 'x'\n").unwrap();
        assert!(restore_backup_by_name(&target, &backups, "../../etc/passwd").is_err());
        assert!(restore_backup_by_name(&target, &backups, "config-9999.toml").is_err());
        assert!(restore_backup_by_name(&target, &backups, "notes.txt").is_err());
        // 合法的能还原
        restore_backup_by_name(&target, &backups, "config-2000.toml").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "model = 'x'\n");
    }

    #[test]
    fn the_active_provider_is_read_not_guessed() {
        assert_eq!(active_provider_id("model = 'x'\n"), None);
        assert_eq!(active_provider_id("not toml = ["), None);
        assert_eq!(
            active_provider_id("model = 'x'\nmodel_provider = 'acme'\n"),
            Some("acme".to_string())
        );
    }

    #[test]
    fn the_tool_scan_only_promises_codex() {
        // 第一阶段只做 Codex（Phase 2 第 4 条）：别的工具要单独核实再立项
        let tools = scan_tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool, "codex");
    }
}

#[cfg(test)]
mod ordering_tests {
    use toml_edit::{value, DocumentMut};

    /// 我们依赖的一条**库行为**：在已有表的文件里新增根键，`toml_edit` 会把它插到
    /// 第一张表**之前**。
    ///
    /// 为什么值得为库行为写用例：TOML 的语义是「表一旦开始，后面的键就属于那张表」，
    /// 所以根键若被排到表后面，Codex 读到的就不是我们写的那个键——
    /// 而这种错**不报错**，只是配置静默地不生效。换版本或换库时这条会先红。
    #[test]
    fn a_new_root_key_lands_before_the_first_table() {
        let original = "model = 'old'\n\n[plugins.x]\nenabled = true\n";
        let mut doc: DocumentMut = original.parse().unwrap();
        doc["model_provider"] = value("our-provider");
        let out = doc.to_string();
        let provider_at = out.find("model_provider").expect("新键应在");
        let table_at = out.find("[plugins.x]").expect("原表应在");
        assert!(
            provider_at < table_at,
            "根键被排到了表后面 ⇒ 它会挂到 [plugins.x] 里：\n{out}"
        );
        // 再解析一遍确认语义：新键确实是根键
        let reparsed: DocumentMut = out.parse().unwrap();
        assert_eq!(reparsed["model_provider"].as_str(), Some("our-provider"));
        assert!(reparsed["plugins"]["x"]["enabled"].as_bool() == Some(true));
    }

    /// 反过来：给**已存在**的表加键，不该动别的表，也不该新建根键
    #[test]
    fn updating_an_existing_table_key_stays_inside_that_table() {
        let original = "model = 'old'\n\n[model_providers.mine]\nname = 'Mine'\n";
        let mut doc: DocumentMut = original.parse().unwrap();
        doc["model_providers"]["mine"]["base_url"] = value("https://example.invalid/v1");
        let out = doc.to_string();
        let reparsed: DocumentMut = out.parse().unwrap();
        assert_eq!(
            reparsed["model_providers"]["mine"]["base_url"].as_str(),
            Some("https://example.invalid/v1")
        );
        assert_eq!(reparsed["model"].as_str(), Some("old"));
    }
}
