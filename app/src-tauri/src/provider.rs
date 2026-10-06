//! Codex profile groundwork. This module never reads or writes the user's
//! installed configuration on its own; Phase 2 supplies a chosen destination.

use crate::atomicfile::{atomic_create_validated, atomic_replace_validated};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::Path;
use toml_edit::{value, DocumentMut, Item};

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

// 这里原本有一个 `update_codex_profile`（只写顶层 model/model_provider，并支持
// 「不带 provider ⇒ 清掉覆盖」，来自 v0.0.160 的地基）。已删除，理由两条：
// ① 它的写入职责被 `plan_codex_apply` 完整取代（后者还写 provider 表，且走同一条路径校验）；
// ② 它唯一独有的行为是「清掉 model_provider 覆盖」，也就是「回到 Codex 默认」——
//    而这件事**用现有功能就能做到**：档位页里每次切换前都会留备份，从备份还原即可。
//    与其为它多养一条没人走的写入路径，不如让用户用已有且被用例覆盖的那条。
// 「回到默认」在界面上没有单独按钮，这一点记在 CHANGELOG 与 review 里，不藏在注释里。

// MARK: - 档位（Phase 2）

/// 一个档位：记「用哪家 provider + 哪个模型」。
///
/// **不含任何凭据**：只记 `env_key` 这个**变量名**，密钥由用户自己放进环境变量。
/// 这是 ADR 0009 与 AGENTS.md 的红线（本项目不存储任何凭据），
/// 也是这套结构能成立的前提——所以 `env_key` 的校验只认「像变量名的东西」（见下）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// `responses`; legacy `chat` remains readable but cannot be written to Codex.
    pub wire_api: String,
}

impl CodexProfile {
    pub fn ensure_writable_protocol(&self) -> Result<(), String> {
        if self.wire_api != "responses" {
            return Err("当前 Codex 仅支持 Responses；旧 Chat 档位需核对接口并改为 Responses 后再保存和应用".into());
        }
        Ok(())
    }
    /// Structural validation also permits legacy Chat records for non-destructive reads.
    /// Saving and application additionally require ensure_writable_protocol.
    /// 一个写坏的档位会在切换时把用户的 `config.toml` 改成 Codex 读不懂的东西。
    pub fn normalized(mut self) -> Result<Self, String> {
        for (label, value) in [
            ("id", &self.id),
            ("provider_id", &self.provider_id),
            ("model", &self.model),
            ("base_url", &self.base_url),
            ("env_key", &self.env_key),
        ] {
            if value.trim().is_empty() {
                return Err(format!("{label} 不能为空"));
            }
        }
        if safe_label(Some(self.model.trim())).is_none() {
            return Err("模型标识只允许字母、数字与 - _ . / : [ ] @，且不能包含密钥".into());
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
        validate_base_url(self.base_url.trim())?;
        if self.wire_api != "chat" && self.wire_api != "responses" {
            return Err("wire_api 只能是 chat 或 responses".into());
        }
        for text in [
            &self.id,
            &self.name,
            &self.provider_id,
            &self.provider_name,
            &self.env_key,
        ] {
            if text.len() > 640
                || text.chars().any(char::is_control)
                || text.contains("sk-")
                || text.contains("AIza")
                || text.contains("-----BEGIN")
            {
                return Err("档位标识与名称不能包含凭据、控制字符或过长内容".into());
            }
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
    profile
        .ensure_writable_protocol()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut doc: DocumentMut = original.parse().map_err(|_| invalid_toml())?;
    // A selected native Codex profile overrides root values. Until its other
    // overrides can be represented, refuse rather than silently changing root.
    if doc.get("profile").is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Codex 正在使用原生 profile；请先在 Codex 中取消该选择，再切换本机档位",
        ));
    }
    if let Some(entry) = doc
        .get("model_providers")
        .and_then(|v| v.get(&profile.provider_id))
    {
        if [
            "http_headers",
            "env_http_headers",
            "experimental_bearer_token",
            "query_params",
        ]
        .iter()
        .any(|key| entry.get(key).is_some())
            || entry.get("requires_openai_auth").and_then(|v| v.as_bool()) == Some(true)
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput,
                "目标 Provider 含有特殊认证字段；本应用不能替换该登录方式，请使用独立 Provider 标识"));
        }
    }
    replace_value_preserving_decor(&mut doc, "model", &profile.model);
    replace_value_preserving_decor(&mut doc, "model_provider", &profile.provider_id);

    // provider 表：已存在就只更新我们认识的四个键（用户额外写的 retry 等**不删**）
    let providers =
        doc["model_providers"].or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
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
        if !home.trim().is_empty()
            && target == std::path::Path::new(home.trim()).join("config.toml")
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
    apply_codex_profile_checked(target, backup_dir, profile, now_ms, None)
}

pub fn apply_codex_profile_checked(
    target: &Path,
    backup_dir: &Path,
    profile: &CodexProfile,
    now_ms: i64,
    expected_revision: Option<&str>,
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
    if expected_revision.is_some_and(|expected| config_revision(&original) != expected) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "配置已被其他窗口或工具修改；请刷新后重新确认",
        ));
    }
    let profile = profile
        .clone()
        .normalized()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let original_doc = original
        .parse::<DocumentMut>()
        .map_err(|_| invalid_toml())?;
    if !crate::mcp_config::can_backup(&original_doc, &original) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "配置含私有字段或未支持形式，保持只读；未创建备份",
        ));
    }
    let updated = plan_codex_apply(&original, &profile)?;
    fs::create_dir_all(backup_dir)?;
    let backup = backup_dir.join(backup_file_name(now_ms));
    // Validate and publish without ever replacing an earlier backup.
    atomic_create_validated(&backup, original.as_bytes(), |staged| {
        fs::read_to_string(staged).map(|_| ())
    })?;

    if expected_revision.is_some() {
        verify_config_revision(target, &config_revision(&original))
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    }
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
    let doc = text.parse::<DocumentMut>().map_err(|_| invalid_toml())?;
    if !crate::mcp_config::can_backup(&doc, &text) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "备份含私有字段或未支持形式，未还原",
        ));
    }
    atomic_replace_validated(target, text.as_bytes(), |staged| {
        let staged_text = fs::read_to_string(staged)?;
        staged_text
            .parse::<DocumentMut>()
            .map_err(|_| invalid_toml())?;
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
        Some(p) => (p.exists(), Some(p.to_string_lossy().to_string())),
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
    pub configured_model: Option<String>,
    pub configured_provider: Option<String>,
    pub current_draft: Option<CodexProfile>,
    pub capture_notice: String,
    pub config_error: Option<String>,
    pub revision: Option<String>,
    pub last_applied: Option<CodexProfile>,
    pub drifted: bool,
    pub record_error: Option<String>,
    pub limitations: &'static str,
}

/// 一次切换的结果。**必带 `limitations`**：界面上少显示一次就没有第二道防线。
#[derive(Debug, Clone, Serialize)]
pub struct ProviderApplyResult {
    pub config_path: String,
    pub backup_name: String,
    pub record_warning: Option<String>,
    pub limitations: &'static str,
}

/// **能力边界**：只要涉及档位切换，这段就随结果一起返回。
///
/// 放在 Rust 侧（而不是让界面各写一遍）是因为它必须**逐字**出现：
/// 用户切了档以为能用别家的模型、结果不能用，是这次改造最容易招来的误解。
/// 这段文字是**逐字上屏**的，所以它必须是「能直接显示的纯文本」——
/// 里面不能有 `**` 这类标记（我第一版写了 `**不含跨厂商模型能力**`，
/// 界面上就把两个星号原样显示出来了，截图才看见）。强调靠排版，不靠标记。
pub const PROVIDER_LIMITATIONS: &str =
    "只修改本机 Codex 配置，不验证账号或模型可用性；目标接口需要兼容 Codex 的 API 协议。\
API key 不由本应用保存，档位只记环境变量名。配置目标不等于运行中的会话模型；\
切换后请开启新会话，必要时重启 Codex。原生 profile 的覆盖配置暂不支持切换。";

fn validate_base_url(text: &str) -> Result<(), String> {
    let url = url::Url::parse(text).map_err(|_| "API 地址必须是有效的 http(s) 地址")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("API 地址必须是有效的 http(s) 地址".into());
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || text.contains("sk-")
        || text.contains("AIza")
    {
        return Err("API 地址不能包含认证信息、查询参数或片段；密钥请通过环境变量提供".into());
    }
    Ok(())
}

pub fn config_revision(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// Export only fields this tool can safely reproduce. Never return the TOML,
/// headers, auth fields, or parser diagnostics to the WebView.
pub fn capture_codex_profile(text: &str) -> Result<CodexProfile, String> {
    let doc: DocumentMut = text.parse().map_err(|_| "Codex 配置不是有效的 TOML")?;
    if doc.get("profile").is_some() {
        return Err("原生 profile 含有覆盖配置，暂不能保存为本应用档位".into());
    }
    let model = doc
        .get("model")
        .and_then(|v| v.as_str())
        .ok_or("当前配置没有明确的模型，请使用自定义档位")?;
    let provider_id = doc
        .get("model_provider")
        .and_then(|v| v.as_str())
        .ok_or("当前使用 Codex 原生默认配置，暂不能保存登录方式为档位")?;
    let entry = doc
        .get("model_providers")
        .and_then(|v| v.get(provider_id))
        .ok_or("当前 Provider 没有可保存的接口配置")?;
    // Dropping request headers or authentication switches changes semantics.
    // Such providers can be displayed, but cannot be captured lossily.
    let allowed = [
        "name",
        "base_url",
        "env_key",
        "wire_api",
        "request_max_retries",
        "stream_max_retries",
        "stream_idle_timeout_ms",
        "env_key_instructions",
    ];
    if entry
        .as_table()
        .is_none_or(|t| t.iter().any(|(key, _)| !allowed.contains(&key)))
    {
        return Err("当前 Provider 含有档位未支持的认证或扩展字段，请保留原配置".into());
    }
    let read = |key: &str| entry.get(key).and_then(|v| v.as_str());
    let profile = CodexProfile {
        id: "current".into(),
        name: "当前配置".into(),
        model: model.into(),
        provider_id: provider_id.into(),
        provider_name: read("name").unwrap_or(provider_id).into(),
        base_url: read("base_url")
            .ok_or("当前 Provider 未设置 API 地址")?
            .into(),
        env_key: read("env_key")
            .filter(|v| !v.is_empty())
            .ok_or("当前 Provider 不使用密钥环境变量，暂不能保存登录方式为档位")?
            .into(),
        wire_api: read("wire_api").unwrap_or("responses").into(),
    };
    let profile = profile.normalized()?;
    profile.ensure_writable_protocol()?;
    Ok(profile)
}

pub fn matches_codex_profile(text: &str, profile: &CodexProfile) -> bool {
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return false;
    };
    if doc.get("profile").is_some() {
        return false;
    }
    if doc.get("model").and_then(|v| v.as_str()) != Some(profile.model.as_str())
        || doc.get("model_provider").and_then(|v| v.as_str()) != Some(profile.provider_id.as_str())
    {
        return false;
    }
    let Some(entry) = doc
        .get("model_providers")
        .and_then(|v| v.get(&profile.provider_id))
    else {
        return false;
    };
    if [
        "http_headers",
        "env_http_headers",
        "experimental_bearer_token",
        "query_params",
    ]
    .iter()
    .any(|key| entry.get(key).is_some())
        || entry.get("requires_openai_auth").and_then(|v| v.as_bool()) == Some(true)
    {
        return false;
    }
    entry.get("base_url").and_then(|v| v.as_str()) == Some(profile.base_url.as_str())
        && entry.get("env_key").and_then(|v| v.as_str()) == Some(profile.env_key.as_str())
        && entry
            .get("wire_api")
            .and_then(|v| v.as_str())
            .unwrap_or("responses")
            == profile.wire_api
}

fn safe_label(value: Option<&str>) -> Option<String> {
    value
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 160
                && s.chars()
                    .all(|c| c.is_alphanumeric() || "-_./:[]@".contains(c))
                && !s.contains("sk-")
                && !s.contains("AIza")
        })
        .map(str::to_owned)
}

pub fn inspect_codex_config(
    path: Option<&Path>,
    profiles: &[CodexProfile],
    store: &ProviderStore,
) -> ProviderStatus {
    let mut status = ProviderStatus {
        installed: path.is_some_and(Path::exists),
        config_path: path.map(|p| p.to_string_lossy().into_owned()),
        active_provider_id: None,
        active_profile_id: None,
        profile_count: profiles.len(),
        configured_model: None,
        configured_provider: None,
        current_draft: None,
        capture_notice: String::new(),
        config_error: None,
        revision: None,
        last_applied: None,
        drifted: false,
        record_error: None,
        limitations: PROVIDER_LIMITATIONS,
    };
    match store.last_applied() {
        Ok(record) => status.last_applied = record,
        Err(error) => status.record_error = Some(error),
    }
    let text = match path.map(fs::read_to_string) {
        Some(Ok(text)) => text,
        _ => {
            status.config_error = Some("读不到 Codex 配置；请检查文件是否存在及读取权限".into());
            return status;
        }
    };
    let doc = match text.parse::<DocumentMut>() {
        Ok(doc) => doc,
        Err(_) => {
            status.config_error = Some("Codex 配置不是有效的 TOML".into());
            return status;
        }
    };
    if crate::mcp_config::can_backup(&doc, &text) {
        status.revision = Some(config_revision(&text));
    } else {
        status.config_error =
            Some("配置含私有字段或未支持形式，保持只读；凭据请使用环境变量引用".into());
    }
    let selected = doc
        .get("profile")
        .and_then(|v| v.as_str())
        .and_then(|id| doc.get("profiles").and_then(|v| v.get(id)));
    let effective = |key: &str| {
        selected
            .and_then(|p| p.get(key))
            .or_else(|| doc.get(key))
            .and_then(|v| v.as_str())
    };
    status.configured_model = safe_label(effective("model"));
    status.configured_provider = safe_label(effective("model_provider"));
    status.active_provider_id = status.configured_provider.clone();
    match capture_codex_profile(&text) {
        Ok(draft) => status.current_draft = Some(draft),
        Err(notice) => status.capture_notice = notice,
    }
    let matching: Vec<_> = profiles
        .iter()
        .filter(|p| matches_codex_profile(&text, p))
        .collect();
    // Identical aliases are ambiguous; never pick whichever happens to come first.
    if matching.len() == 1 {
        status.active_profile_id = Some(matching[0].id.clone());
    }
    status.drifted = status
        .last_applied
        .as_ref()
        .is_some_and(|p| !matches_codex_profile(&text, p));
    status
}

pub fn verify_config_revision(target: &Path, expected: &str) -> Result<(), String> {
    let text = fs::read_to_string(target).map_err(|_| "读不到 Codex 配置，请刷新后重试")?;
    if config_revision(&text) != expected {
        return Err("配置已被其他窗口或工具修改；请刷新后重新确认".into());
    }
    Ok(())
}

// MARK: - 本地档位库（我们自己的文件，不含任何凭据）

/// 档位库目录：`<app data>/AgentIsland/providers/`。
/// 备份也放这儿（`backups/`），不塞进用户的 `~/.codex`。
pub fn store_dir() -> std::path::PathBuf {
    crate::settings::config_dir().join("providers")
}

/// 档位库：损坏与空清单不同。界面和写入都消费 checked 入口；
/// 解析失败保留原文件，不让一次保存覆盖用户现有档位。
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
        self.list_checked().unwrap_or_default()
    }

    pub fn list_checked(&self) -> Result<Vec<CodexProfile>, String> {
        let text = match fs::read_to_string(self.profiles_path()) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err("读不到档位清单，请检查读取权限".into()),
        };
        let list: Vec<CodexProfile> =
            serde_json::from_str(&text).map_err(|_| "档位清单损坏；请保留文件并恢复备份")?;
        list.into_iter().map(CodexProfile::normalized).collect()
    }

    /// Bounded metadata catalog for local workspace references, without changing profile storage.
    pub fn workspace_choices(&self) -> Result<Vec<crate::workspaces::Choice>, String> {
        use std::io::Read;
        let path = self.profiles_path();
        let meta = match fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
            Err(_) => return Err("档位来源无法读取".into()),
        };
        const CAP: u64 = 1024 * 1024;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > CAP {
            return Err("档位来源类型或容量不受支持".into());
        }
        let mut bytes = vec![];
        fs::File::open(path)
            .map_err(|_| "档位来源无法打开")?
            .take(CAP + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "档位来源无法读取")?;
        if bytes.len() as u64 > CAP {
            return Err("档位来源容量不受支持".into());
        }
        let profiles: Vec<CodexProfile> =
            serde_json::from_slice(&bytes).map_err(|_| "档位来源格式不受支持")?;
        if profiles.len() > 200 {
            return Err("工作空间最多读取 200 个档位，请在模型页整理".into());
        }
        profiles
            .into_iter()
            .map(|p| {
                let p = p.normalized().map_err(|_| "档位来源字段不受支持")?;
                Ok(crate::workspaces::Choice {
                    id: p.id,
                    name: p.name,
                    supported: p.wire_api == "responses",
                })
            })
            .collect()
    }

    pub fn profile_for_apply(
        &self,
        id: &str,
        expected: &CodexProfile,
    ) -> Result<CodexProfile, String> {
        let profile = self
            .list_checked()?
            .into_iter()
            .find(|p| p.id == id)
            .ok_or("没有这个档位，请刷新后重试")?;
        if &profile != expected {
            return Err("档位已被其他窗口修改，请刷新后重新预览".into());
        }
        Ok(profile)
    }

    pub fn last_applied(&self) -> Result<Option<CodexProfile>, String> {
        match fs::read_to_string(self.dir.join("applied.json")) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err("读不到上次应用记录，无法判断配置漂移".into()),
            Ok(text) => serde_json::from_str::<CodexProfile>(&text)
                .map_err(|_| "上次应用记录损坏，无法判断配置漂移".into())
                .and_then(CodexProfile::normalized)
                .map(Some),
        }
    }

    /// Configuration publication succeeds independently of the bookkeeping receipt.
    pub fn apply_with_receipt(
        &self,
        target: &Path,
        profile: &CodexProfile,
        revision: &str,
        now_ms: i64,
    ) -> Result<ProviderApplyResult, String> {
        let backup = apply_codex_profile_checked(
            target,
            &self.backups_dir(),
            profile,
            now_ms,
            Some(revision),
        )
        .map_err(|_| "配置切换未完成，请刷新核对配置与备份".to_string())?;
        Ok(ProviderApplyResult {
            config_path: target.to_string_lossy().into_owned(),
            backup_name: backup
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            record_warning: self
                .remember_applied(profile)
                .err()
                .map(|_| "配置已写入，应用记录未保存；请刷新核对，勿重复应用".into()),
            limitations: PROVIDER_LIMITATIONS,
        })
    }

    pub fn restore_with_receipt(
        &self,
        target: &Path,
        name: &str,
        revision: &str,
        backup_revision: &str,
    ) -> Result<ProviderApplyResult, String> {
        let backup =
            restore_previewed_backup(target, &self.backups_dir(), name, revision, backup_revision)?;
        Ok(ProviderApplyResult {
            config_path: target.to_string_lossy().into_owned(),
            backup_name: backup
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            record_warning: self
                .keep_current()
                .err()
                .map(|_| "配置已还原，应用记录未清除；请刷新核对，勿重复还原".into()),
            limitations: PROVIDER_LIMITATIONS,
        })
    }

    pub fn remember_applied(&self, profile: &CodexProfile) -> Result<(), String> {
        let profile = profile.clone().normalized()?;
        fs::create_dir_all(&self.dir).map_err(|_| "无法创建档位记录目录")?;
        let json = serde_json::to_vec(&profile).map_err(|_| "无法生成档位记录")?;
        atomic_replace_validated(&self.dir.join("applied.json"), &json, |staged| {
            let bytes = fs::read(staged)?;
            serde_json::from_slice::<CodexProfile>(&bytes).map_err(|_| invalid_toml())?;
            Ok(())
        })
        .map_err(|_| "配置已写入，但无法保存应用记录；请刷新核对".into())
    }

    pub fn keep_current(&self) -> Result<(), String> {
        match fs::remove_file(self.dir.join("applied.json")) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("无法更新应用记录".into()),
        }
    }

    /// 保存（同 id 覆盖）。校验在 `CodexProfile::normalized` 里，**这里拒绝写坏的档位**。
    pub fn save(&self, profile: CodexProfile) -> Result<CodexProfile, String> {
        let profile = profile.normalized()?;
        profile.ensure_writable_protocol()?;
        let mut list = self.list_checked()?;
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
        let mut list = self.list_checked()?;
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
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "备份名不合格"));
    }
    let backup = dir.join(name);
    if !backup.is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "备份不存在"));
    }
    restore_codex_backup(target, &backup)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileBundle {
    schema_version: u32,
    profiles: Vec<CodexProfile>,
}

#[derive(Serialize)]
pub struct ImportPreview {
    pub added: Vec<CodexProfile>,
    pub skipped: Vec<String>,
    pub revision: String,
}

impl ProviderStore {
    fn list_revision(list: &[CodexProfile]) -> Result<String, String> {
        let bytes = serde_json::to_vec(list).map_err(|_| "档位序列化失败")?;
        Ok(config_revision(
            &String::from_utf8(bytes).map_err(|_| "档位编码失败")?,
        ))
    }

    pub fn export_bundle(&self) -> Result<String, String> {
        serde_json::to_string_pretty(&ProfileBundle {
            schema_version: 1,
            profiles: self.list_checked()?,
        })
        .map_err(|_| "档位导出失败".into())
    }

    pub fn export_to_directory(&self, directory: &Path) -> Result<std::path::PathBuf, String> {
        let text = self.export_bundle()?;
        let path = directory.join(format!(
            "agentisland-profiles-{}.json",
            crate::tokens::now_ms()
        ));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| "下载目录不可写，或同名文件已存在；请稍后重试")?;
        use std::io::Write;
        if file
            .write_all(text.as_bytes())
            .and_then(|_| file.sync_all())
            .is_err()
        {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err("导出写入失败".into());
        }
        Ok(path)
    }

    pub fn preview_import(&self, text: &str) -> Result<ImportPreview, String> {
        if text.len() > 65536 {
            return Err("导入文件不能超过 64 KB".into());
        }
        let bundle: ProfileBundle = serde_json::from_str(text)
            .map_err(|_| "不是受支持的档位文件，或包含白名单以外的字段")?;
        if bundle.schema_version != 1 || bundle.profiles.len() > 100 {
            return Err("档位版本不支持，或数量超过 100".into());
        }
        let existing = self.list_checked()?;
        let mut seen = std::collections::HashSet::new();
        let mut added = Vec::new();
        let mut skipped = Vec::new();
        for profile in bundle.profiles {
            let profile = profile.normalized()?;
            if !seen.insert(profile.id.clone()) {
                return Err("导入文件有重复 ID，请先整理后重试".into());
            }
            if existing.iter().any(|p| p.id == profile.id) {
                skipped.push(profile.id);
            } else {
                added.push(profile);
            }
        }
        Ok(ImportPreview {
            added,
            skipped,
            revision: Self::list_revision(&existing)?,
        })
    }

    pub fn import_bundle(&self, text: &str, revision: &str) -> Result<usize, String> {
        let preview = self.preview_import(text)?;
        if preview.revision != revision {
            return Err("档位清单已变化，请重新预览导入".into());
        }
        if preview.added.is_empty() {
            return Ok(0);
        }
        let count = preview.added.len();
        let mut list = self.list_checked()?;
        if Self::list_revision(&list)? != revision {
            return Err("档位清单已变化，请重新预览导入".into());
        }
        list.extend(preview.added);
        let json = serde_json::to_vec_pretty(&list).map_err(|_| "导入编码失败")?;
        fs::create_dir_all(&self.dir).map_err(|_| "档位目录不可写")?;
        atomic_replace_validated(&self.profiles_path(), &json, |staged| {
            serde_json::from_slice::<Vec<CodexProfile>>(&fs::read(staged)?)
                .map(|_| ())
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "profile JSON"))
        })
        .map_err(|_| "档位导入失败，原清单保留")?;
        Ok(count)
    }
}

#[derive(Serialize)]
pub struct BackupPreview {
    pub writable: bool,
    pub mcp_changes: Vec<String>,
    pub skills_changes: Vec<String>,
    pub current: Vec<String>,
    pub backup: Vec<String>,
    pub revision: String,
    pub backup_revision: String,
    pub identical: bool,
}

fn config_summary(text: &str) -> Result<Vec<String>, String> {
    let doc: DocumentMut = text.parse().map_err(|_| "配置格式不可读，无法预览")?;
    let clean = |value: Option<&str>| {
        safe_label(value).unwrap_or_else(|| "未明确指定 / 不在安全预览范围".into())
    };
    let model = clean(doc.get("model").and_then(|v| v.as_str()));
    let provider = doc.get("model_provider").and_then(|v| v.as_str());
    let entry = provider.and_then(|id| doc.get("model_providers").and_then(|v| v.get(id)));
    let read = |key: &str| entry.and_then(|e| e.get(key)).and_then(|v| v.as_str());
    let base = read("base_url")
        .filter(|v| validate_base_url(v).is_ok())
        .unwrap_or("未明确指定 / 不在安全预览范围");
    let env = read("env_key")
        .filter(|v| {
            !v.is_empty()
                && v.len() <= 160
                && v.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        })
        .unwrap_or("未明确指定");
    let wire = read("wire_api")
        .filter(|v| *v == "chat" || *v == "responses")
        .unwrap_or("未明确指定");
    Ok(vec![
        model,
        clean(provider),
        base.into(),
        env.into(),
        wire.into(),
    ])
}

fn skills_backup_changes(current: &str, backup: &str) -> Result<Vec<String>, String> {
    let read = |text: &str| -> Result<Option<std::collections::BTreeMap<String, bool>>, String> {
        let doc = text.parse::<DocumentMut>().map_err(|_| "配置格式不可读")?;
        let Some(item) = doc.get("skills").and_then(|s| s.get("config")) else {
            return Ok(Some(Default::default()));
        };
        let Some(table) = item.as_array_of_tables() else {
            return Ok(None);
        };
        if table.len() > 200 {
            return Ok(None);
        }
        let mut out = std::collections::BTreeMap::new();
        for entry in table.iter() {
            let (Some(path), Some(enabled)) = (
                entry.get("path").and_then(Item::as_str),
                entry.get("enabled").and_then(Item::as_bool),
            ) else {
                return Ok(None);
            };
            if out.insert(path.into(), enabled).is_some() {
                return Ok(None);
            }
        }
        Ok(Some(out))
    };
    let (Some(current), Some(backup)) = (read(current)?, read(backup)?) else {
        return Ok(vec![
            "Skills 结构不在摘要范围，完整还原仍会覆盖该部分。".into()
        ]);
    };
    let names = current
        .keys()
        .chain(backup.keys())
        .collect::<std::collections::BTreeSet<_>>();
    let mut changes = vec![];
    for path in names {
        let name = Path::new(path)
            .parent()
            .and_then(Path::file_name)
            .and_then(|p| p.to_str());
        let name = safe_label(name).unwrap_or_else(|| "Skill 名称不在预览范围".into());
        let change = match (current.get(path), backup.get(path)) {
            (Some(_), None) => "移除启停覆盖，恢复默认配置",
            (None, Some(true)) => "恢复配置启用",
            (None, Some(false)) => "恢复配置停用",
            (Some(before), Some(after)) if before != after => {
                if *after {
                    "还原为配置启用"
                } else {
                    "还原为配置停用"
                }
            }
            _ => continue,
        };
        changes.push(format!("{name} · {change}"));
    }
    Ok(changes)
}

fn mcp_backup_changes(current: &str, backup: &str) -> Result<Vec<String>, String> {
    let current = current
        .parse::<DocumentMut>()
        .map_err(|_| "当前配置格式不可读")?;
    let backup = backup
        .parse::<DocumentMut>()
        .map_err(|_| "备份配置格式不可读")?;
    let table = |doc: &DocumentMut| -> std::collections::BTreeMap<String, String> {
        doc.get("mcp_servers")
            .and_then(|item| item.as_table_like())
            .map(|table| {
                table
                    .iter()
                    .map(|(name, item)| (name.to_owned(), config_revision(&item.to_string())))
                    .collect()
            })
            .unwrap_or_default()
    };
    if [&current, &backup].iter().any(|doc| {
        doc.get("mcp_servers")
            .is_some_and(|item| item.as_table_like().is_none())
    }) {
        return Ok(vec!["MCP 结构包含未支持形式，完整还原会覆盖此部分。".into()]);
    }
    let current = table(&current);
    let backup = table(&backup);
    let names: std::collections::BTreeSet<_> = current.keys().chain(backup.keys()).collect();
    let mut changes = Vec::new();
    for name in names.iter().take(200) {
        let action = match (current.get(*name), backup.get(*name)) {
            (None, Some(_)) => "恢复条目",
            (Some(_), None) => "移除条目",
            (Some(before), Some(after)) if before != after => "还原条目内容",
            _ => continue,
        };
        let label = safe_label(Some(name)).unwrap_or_else(|| "名称不在安全预览范围".into());
        changes.push(format!("{label} · {action}"));
    }
    if names.len() > 200 {
        changes.push("仅列出前 200 个名称；还原仍覆盖完整 MCP 配置。".into());
    }
    Ok(changes)
}

fn read_backup(dir: &Path, name: &str) -> Result<String, String> {
    if parse_backup_name(name).is_none() {
        return Err("备份名不合格".into());
    }
    let path = dir.join(name);
    if fs::symlink_metadata(&path)
        .map_err(|_| "备份不存在")?
        .file_type()
        .is_symlink()
    {
        return Err("不读取备份软链".into());
    }
    if fs::metadata(&path).map_err(|_| "备份不可读")?.len() > 2_000_000 {
        return Err("备份超过预览大小限制".into());
    }
    fs::read_to_string(path).map_err(|_| "备份不可读".into())
}

pub fn preview_backup(target: &Path, dir: &Path, name: &str) -> Result<BackupPreview, String> {
    let current = fs::read_to_string(target).map_err(|_| "当前配置不可读")?;
    let backup = read_backup(dir, name)?;
    Ok(BackupPreview {
        writable: [&current, &backup].iter().all(|text| {
            text.parse::<DocumentMut>()
                .is_ok_and(|doc| crate::mcp_config::can_backup(&doc, text))
        }),
        mcp_changes: mcp_backup_changes(&current, &backup)?,
        skills_changes: skills_backup_changes(&current, &backup)?,
        current: config_summary(&current)?,
        backup: config_summary(&backup)?,
        revision: config_revision(&current),
        backup_revision: config_revision(&backup),
        identical: current == backup,
    })
}

pub fn restore_previewed_backup(
    target: &Path,
    dir: &Path,
    name: &str,
    revision: &str,
    backup_revision: &str,
) -> Result<std::path::PathBuf, String> {
    if !is_codex_config_target(target) {
        return Err("拒绝写入非 Codex 配置路径".into());
    }
    let current = fs::read_to_string(target).map_err(|_| "当前配置不可读")?;
    let backup = read_backup(dir, name)?;
    if config_revision(&current) != revision || config_revision(&backup) != backup_revision {
        return Err("配置或备份已变化，请重新预览".into());
    }
    config_summary(&backup)?;
    for text in [&current, &backup] {
        let doc = text.parse::<DocumentMut>().map_err(|_| "配置格式不可读")?;
        if !crate::mcp_config::can_backup(&doc, text) {
            return Err("当前配置或备份含私有字段或未支持形式，未还原，也未创建新备份".into());
        }
    }
    // The pre-restore state is recoverable too. Publish the exact bytes that were checked.
    fs::create_dir_all(dir).map_err(|_| "备份目录不可写")?;
    let name = format!("config-{}.toml", crate::tokens::now_ms());
    atomic_create_validated(&dir.join(&name), current.as_bytes(), |staged| {
        fs::read_to_string(staged).map(|_| ())
    })
    .map_err(|_| "还原前备份失败或名称已占用，配置未改动")?;
    if fs::read_to_string(target).map_err(|_| "当前配置不可读")? != current {
        return Err("当前配置已变化，请重新预览".into());
    }
    atomic_replace_validated(target, backup.as_bytes(), |staged| {
        fs::read_to_string(staged)?
            .parse::<DocumentMut>()
            .map(|_| ())
            .map_err(|_| invalid_toml())
    })
    .map_err(|_| "还原失败，原配置与备份已保留".to_string())?;
    Ok(dir.join(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn unrelated_private_fields_block_profile_backup_and_restore_without_copying_them() {
        let sandbox = Sandbox::new();
        let target = sandbox.codex_config();
        let store = ProviderStore::new(sandbox.0.join("providers"));
        let original="model='old'\n[model_providers.unrelated]\nhttp_headers={ Authorization='fixture-value' }\n";
        fs::write(&target, original).unwrap();
        assert!(apply_codex_profile(&target, &store.backups_dir(), &profile(), 1).is_err());
        assert!(!store.backups_dir().exists());
        let status = inspect_codex_config(Some(&target), &[profile()], &store);
        assert!(status.revision.is_none());
        assert!(status.config_error.is_some());
        fs::create_dir_all(store.backups_dir()).unwrap();
        fs::write(store.backups_dir().join("config-1.toml"), "model='safe'\n").unwrap();
        assert!(restore_previewed_backup(
            &target,
            &store.backups_dir(),
            "config-1.toml",
            &config_revision(original),
            &config_revision("model='safe'\n")
        )
        .is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), original);
        assert_eq!(fs::read_dir(store.backups_dir()).unwrap().count(), 1);
        fs::write(store.backups_dir().join("config-2.toml"), original).unwrap();
        assert!(restore_codex_backup(&target, &store.backups_dir().join("config-2.toml")).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), original);
    }

    #[test]
    fn duplicate_backup_timestamp_preserves_prior_backup_and_current_config() {
        let sandbox = Sandbox::new();
        let target = sandbox.codex_config();
        let original = "# original fixture\nmodel = 'old'\n";
        fs::write(&target, original).unwrap();
        let backups = sandbox.0.join("backups");
        let first = apply_codex_profile(&target, &backups, &profile(), 42).unwrap();
        let after_first = fs::read_to_string(&target).unwrap();
        let mut second = profile();
        second.model = "another-model".into();
        let attempted = apply_codex_profile_checked(
            &target,
            &backups,
            &second,
            42,
            Some(&config_revision(&after_first)),
        );
        assert!(
            attempted.is_err(),
            "a backup collision must stop before configuration publication"
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), after_first);
        assert_eq!(fs::read_to_string(first).unwrap(), original);
        assert_eq!(fs::read_dir(backups).unwrap().count(), 1);
    }

    #[test]
    fn apply_receipt_preserves_config_and_backup_when_record_cannot_be_saved() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join(".codex/config.toml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let original = "# receipt fixture\nmodel = \"old\"\n";
        fs::write(&target, original).unwrap();
        let store = ProviderStore::new(sandbox.0.join("providers"));
        fs::create_dir_all(store.dir.join("applied.json")).unwrap();
        let result = store
            .apply_with_receipt(&target, &profile(), &config_revision(original), 123)
            .unwrap();
        assert!(result.record_warning.is_some());
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            plan_codex_apply(original, &profile()).unwrap()
        );
        assert_eq!(
            fs::read_to_string(store.backups_dir().join(result.backup_name)).unwrap(),
            original
        );
    }

    #[test]
    fn restore_receipt_identifies_recovery_backup_despite_record_cleanup_failure() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join(".codex/config.toml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let original = "# restore fixture\nmodel = \"old\"\n";
        fs::write(&target, original).unwrap();
        let store = ProviderStore::new(sandbox.0.join("providers"));
        let applied = store
            .apply_with_receipt(&target, &profile(), &config_revision(original), 321)
            .unwrap();
        assert!(applied.record_warning.is_none());
        let before_restore = fs::read_to_string(&target).unwrap();
        fs::remove_file(store.dir.join("applied.json")).unwrap();
        fs::create_dir(store.dir.join("applied.json")).unwrap();
        let restored = store
            .restore_with_receipt(
                &target,
                &applied.backup_name,
                &config_revision(&before_restore),
                &config_revision(original),
            )
            .unwrap();
        assert!(restored.record_warning.is_some());
        assert_ne!(restored.backup_name, applied.backup_name);
        assert_eq!(fs::read_to_string(&target).unwrap(), original);
        assert_eq!(
            fs::read_to_string(store.backups_dir().join(restored.backup_name)).unwrap(),
            before_restore
        );
    }

    #[test]
    fn import_whitelist_conflicts_and_revision_keep_existing_profiles() {
        let sandbox = crate::testutil::Sandbox::new("profile-import");
        let store = ProviderStore::new(sandbox.path().join("providers"));
        store.save(profile()).unwrap();
        let mut other = profile();
        other.id = "other".into();
        other.name = "Other".into();
        let bundle = serde_json::to_string(&ProfileBundle {
            schema_version: 1,
            profiles: vec![profile(), other.clone()],
        })
        .unwrap();
        let preview = store.preview_import(&bundle).unwrap();
        assert_eq!(preview.skipped, vec!["work"]);
        assert_eq!(preview.added.len(), 1);
        assert_eq!(store.import_bundle(&bundle, &preview.revision).unwrap(), 1);
        assert!(store.import_bundle(&bundle, &preview.revision).is_err());
        assert_eq!(store.list_checked().unwrap(), vec![profile(), other]);
        let mut with_unknown: serde_json::Value = serde_json::from_str(&bundle).unwrap();
        with_unknown["profiles"][0]["api_key"] = "fixture-value".into();
        assert!(store.preview_import(&with_unknown.to_string()).is_err());
        let duplicated = serde_json::to_string(&ProfileBundle {
            schema_version: 1,
            profiles: vec![profile(), profile()],
        })
        .unwrap();
        assert!(store.preview_import(&duplicated).is_err());
        assert_eq!(store.list_checked().unwrap().len(), 2);
        let exported = store.export_bundle().unwrap();
        let output = store.export_to_directory(sandbox.path()).unwrap();
        assert_eq!(fs::read_to_string(output).unwrap(), exported);
        assert!(!exported.contains("api_key"));
        assert!(!exported.contains("fixture-value"));
    }

    #[test]
    fn restore_checks_both_versions_and_preserves_pre_restore_state() {
        let sandbox = crate::testutil::Sandbox::new("preview-restore");
        let target = sandbox.path().join(".codex/config.toml");
        let dir = sandbox.path().join("backups");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::create_dir_all(&dir).unwrap();
        let current = "model='current'\n[mcp_servers.private]\ncommand='private-command'\n";
        fs::write(&target, current).unwrap();
        let name = "config-42.toml";
        let saved =
            "model='saved'\n[model_providers.acme.http_headers]\nAuthorization='fixture-value'\n";
        fs::write(dir.join(name), saved).unwrap();
        let preview = preview_backup(&target, &dir, name).unwrap();
        assert!(!preview.writable);
        let json = serde_json::to_string(&preview).unwrap();
        assert!(!json.contains("private-command"));
        assert!(!json.contains("fixture-value"));
        fs::write(dir.join(name), "model='changed'").unwrap();
        assert!(restore_previewed_backup(
            &target,
            &dir,
            name,
            &preview.revision,
            &preview.backup_revision
        )
        .is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), current);
        fs::write(dir.join(name), saved).unwrap();
        fs::write(&target, "model='externally-changed'").unwrap();
        assert!(restore_previewed_backup(
            &target,
            &dir,
            name,
            &preview.revision,
            &preview.backup_revision
        )
        .is_err());
        fs::write(&target, current).unwrap();
        assert!(restore_previewed_backup(
            &target,
            &dir,
            name,
            &preview.revision,
            &preview.backup_revision
        )
        .is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), current);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        // Redacted preview remains available, but known literal-auth snapshots cannot be republished.
        let saved = "model='saved'\n[model_providers.acme]\nname='fixture'\n";
        fs::write(dir.join(name), saved).unwrap();
        let preview = preview_backup(&target, &dir, name).unwrap();
        assert!(preview.writable);
        restore_previewed_backup(
            &target,
            &dir,
            name,
            &preview.revision,
            &preview.backup_revision,
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), saved);
        assert!(fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .any(|e| fs::read_to_string(e.path()).unwrap() == current));
    }

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

        /// 造一个 `<sandbox>/.codex/config.toml`（生产侧的路径校验只认这个形状）
        fn codex_config(&self) -> PathBuf {
            let dir = self.0.join(".codex");
            fs::create_dir_all(&dir).unwrap();
            dir.join("config.toml")
        }
    }

    #[test]
    fn legacy_chat_profiles_remain_readable_but_save_and_apply_are_zero_write() {
        let s = Sandbox::new();
        let store = ProviderStore::new(s.0.join("providers"));
        fs::create_dir_all(&store.dir).unwrap();
        let legacy = CodexProfile {
            id: "legacy".into(),
            wire_api: "chat".into(),
            ..profile()
        };
        let existing = serde_json::to_vec(&vec![legacy.clone(), profile()]).unwrap();
        fs::write(store.profiles_path(), &existing).unwrap();
        assert_eq!(store.list_checked().unwrap().len(), 2);
        assert_eq!(store.list_checked().unwrap()[0], legacy);
        assert!(store
            .save(legacy.clone())
            .unwrap_err()
            .contains("Responses"));
        assert_eq!(fs::read(store.profiles_path()).unwrap(), existing);
        let target = s.codex_config();
        fs::write(&target, "model='existing'\n").unwrap();
        let backup = s.0.join("backups");
        assert!(apply_codex_profile_checked(&target, &backup, &legacy, 1, None).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "model='existing'\n");
        assert!(!backup.exists());
        assert!(plan_codex_apply("", &legacy).is_err());
        let migrated = CodexProfile {
            wire_api: "responses".into(),
            ..legacy.clone()
        };
        store.save(migrated.clone()).unwrap();
        assert_eq!(store.list_checked().unwrap()[0], migrated);
        assert_eq!(store.list_checked().unwrap()[1], profile());
        store.delete(&legacy.id).unwrap();
        assert_eq!(store.list_checked().unwrap(), vec![profile()]);
    }
    #[test]
    fn unsupported_capture_is_not_silently_converted() {
        let text = "model='model'\nmodel_provider='fixture'\n[model_providers.fixture]\nname='Fixture'\nbase_url='https://example.invalid/v1'\nenv_key='FIXTURE_KEY'\nwire_api='chat'\n";
        assert!(capture_codex_profile(text)
            .unwrap_err()
            .contains("Responses"));
        assert_eq!(
            capture_codex_profile(&text.replace("wire_api='chat'", "wire_api='responses'"))
                .unwrap()
                .wire_api,
            "responses"
        );
    }
    #[test]
    fn skill_backup_summary_shows_activation_without_private_paths() {
        let before = "[[skills.config]]\npath='/fixture-private/sample/SKILL.md'\nenabled=false\n";
        let after = "[[skills.config]]\npath='/fixture-private/sample/SKILL.md'\nenabled=true\n[[skills.config]]\npath='/fixture-private/new-skill/SKILL.md'\nenabled=false\n";
        let changes = skills_backup_changes(before, after).unwrap();
        assert!(changes
            .iter()
            .any(|c| c.contains("sample · 还原为配置启用")));
        assert!(changes
            .iter()
            .any(|c| c.contains("new-skill · 恢复配置停用")));
        assert!(!changes.join(" ").contains("fixture-private"));
        assert!(skills_backup_changes(after, before)
            .unwrap()
            .iter()
            .any(|c| c.contains("移除启停覆盖")));
    }
    /// 一份合用的档位 fixture
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

    /// 能力边界那段是**逐字上屏**的，所以它必须同时满足两件事：
    /// ① 内容是**能直接显示的纯文本**（不能有 `**` 这类标记——我第一版写了粗体标记，
    ///    界面上原样显示了两个星号，是截图才看见的）；
    /// ② 边界**确实写到了**（少写一句，界面就少一道防线）。
    #[test]
    fn the_capability_boundary_text_is_display_ready_and_says_the_boundary() {
        let text = PROVIDER_LIMITATIONS;
        for markup in ["**", "__", "<", ">", "`", "](", "!["] {
            assert!(
                !text.contains(markup),
                "能力边界是逐字显示的，不能带标记 {markup:?}：{text}"
            );
        }
        for must_say in [
            "Codex",    // 只做 Codex
            "兼容",     // 按协议能力判断，不能按厂商一刀切
            "环境变量", // key 不由我们保存
            "重启",     // 生效条件
        ] {
            assert!(
                text.contains(must_say),
                "能力边界少说了 {must_say:?}：{text}"
            );
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
        assert_eq!(
            leftovers,
            vec!["config.toml"],
            "暂存文件泄漏：{leftovers:?}"
        );
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
            (
                "空 model",
                CodexProfile {
                    model: "  ".into(),
                    ..profile()
                },
            ),
            (
                "id 带空格",
                CodexProfile {
                    id: "my profile".into(),
                    ..profile()
                },
            ),
            (
                "base_url 不是 http",
                CodexProfile {
                    base_url: "ftp://x".into(),
                    ..profile()
                },
            ),
            (
                "wire_api 不认识",
                CodexProfile {
                    wire_api: "grpc".into(),
                    ..profile()
                },
            ),
            (
                "env_key 里有小写",
                CodexProfile {
                    env_key: "acme_key".into(),
                    ..profile()
                },
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
    fn matching_ignores_unmanaged_keys_but_apply_refuses_special_auth() {
        let plain = plan_codex_apply("", &profile()).unwrap();
        let with_extra = format!("{plain}\nretry = 3\n");
        assert!(matches_codex_profile(&with_extra, &profile()));
        assert!(
            capture_codex_profile(&with_extra).is_err(),
            "cannot promise to snapshot extensions"
        );
        let with_auth = format!("{plain}\nrequires_openai_auth = true\n");
        assert!(plan_codex_apply(&with_auth, &profile()).is_err());
        assert!(!matches_codex_profile(&with_auth, &profile()));
        assert_eq!(
            plan_codex_apply(&with_extra, &profile()).unwrap(),
            with_extra
        );
    }

    #[test]
    fn editing_or_deleting_a_profile_invalidates_its_pending_preview() {
        let sandbox = Sandbox::new();
        let store = ProviderStore::new(sandbox.0.join("providers"));
        store.save(profile()).unwrap();
        assert_eq!(
            store.profile_for_apply("work", &profile()).unwrap(),
            profile()
        );
        store
            .save(CodexProfile {
                model: "changed-model".into(),
                ..profile()
            })
            .unwrap();
        assert!(store.profile_for_apply("work", &profile()).is_err());
        store.delete("work").unwrap();
        assert!(store.profile_for_apply("work", &profile()).is_err());
    }

    #[test]
    fn configuration_matching_uses_model_endpoint_auth_reference_and_protocol() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join(".codex/config.toml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let store = ProviderStore::new(sandbox.0.join("providers"));
        let cheap = CodexProfile {
            id: "cheap".into(),
            model: "gpt-5-mini".into(),
            ..profile()
        };
        let text = plan_codex_apply("# preserved\n", &cheap).unwrap();
        fs::write(&target, &text).unwrap();
        let status = inspect_codex_config(Some(&target), &[profile(), cheap.clone()], &store);
        assert_eq!(status.active_profile_id.as_deref(), Some("cheap"));
        assert_eq!(status.configured_model.as_deref(), Some("gpt-5-mini"));
        for changed in [
            CodexProfile {
                base_url: "https://other.example.invalid/v1".into(),
                ..cheap.clone()
            },
            CodexProfile {
                env_key: "OTHER_API_KEY".into(),
                ..cheap.clone()
            },
            CodexProfile {
                wire_api: "chat".into(),
                ..cheap.clone()
            },
        ] {
            assert!(!matches_codex_profile(&text, &changed));
        }
        let alias = CodexProfile {
            id: "alias".into(),
            ..cheap.clone()
        };
        assert!(inspect_codex_config(Some(&target), &[cheap, alias], &store)
            .active_profile_id
            .is_none());
    }

    #[test]
    fn external_changes_are_visible_and_keep_current_never_writes_codex() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join(".codex/config.toml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let store = ProviderStore::new(sandbox.0.join("providers"));
        let before = plan_codex_apply("# user formatting\n", &profile()).unwrap();
        fs::write(&target, &before).unwrap();
        store.remember_applied(&profile()).unwrap();
        assert!(!inspect_codex_config(Some(&target), &[profile()], &store).drifted);
        let comment_only = format!("# external comment\n{before}");
        fs::write(&target, &comment_only).unwrap();
        assert!(!inspect_codex_config(Some(&target), &[profile()], &store).drifted);
        let external = before.replace("gpt-5", "another-model");
        fs::write(&target, &external).unwrap();
        let status = inspect_codex_config(Some(&target), &[profile()], &store);
        assert!(status.drifted);
        assert!(status.active_profile_id.is_none());
        assert_eq!(status.last_applied.unwrap(), profile());
        assert!(verify_config_revision(&target, &config_revision(&before)).is_err());
        store.keep_current().unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), external);
        assert!(!inspect_codex_config(Some(&target), &[profile()], &store).drifted);
    }

    #[test]
    fn stale_preview_cannot_overwrite_external_configuration_or_create_a_backup() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join(".codex/config.toml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let before = plan_codex_apply("", &profile()).unwrap();
        let external = before.replace("gpt-5", "another-model");
        fs::write(&target, &external).unwrap();
        let backups = sandbox.0.join("backups");
        let result = apply_codex_profile_checked(
            &target,
            &backups,
            &profile(),
            42,
            Some(&config_revision(&before)),
        );
        assert!(result.is_err());
        assert!(!backups.exists());
        assert_eq!(fs::read_to_string(&target).unwrap(), external);
        apply_codex_profile_checked(
            &target,
            &backups,
            &profile(),
            42,
            Some(&config_revision(&external)),
        )
        .unwrap();
        assert!(matches_codex_profile(
            &fs::read_to_string(&target).unwrap(),
            &profile()
        ));
        assert_eq!(
            fs::read_to_string(backups.join(backup_file_name(42))).unwrap(),
            external
        );
    }

    #[test]
    fn capture_and_inspection_do_not_export_auth_fields_or_raw_parser_errors() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join(".codex/config.toml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let store = ProviderStore::new(sandbox.0.join("providers"));
        let before = plan_codex_apply("", &profile()).unwrap();
        let private = "private-fixture-value";
        let with_header = format!("{before}\nhttp_headers = {{ Authorization = '{private}' }}\n");
        assert!(capture_codex_profile(&with_header).is_err());
        fs::write(&target, &with_header).unwrap();
        let json =
            serde_json::to_string(&inspect_codex_config(Some(&target), &[], &store)).unwrap();
        assert!(!json.contains(private));
        assert!(!json.contains("Authorization"));
        fs::write(&target, format!("unclosed = '{private}")).unwrap();
        let json =
            serde_json::to_string(&inspect_codex_config(Some(&target), &[], &store)).unwrap();
        assert!(!json.contains(private));
        assert!(inspect_codex_config(Some(&target), &[], &store)
            .config_error
            .is_some());
        for address in [
            "https://user:pass@example.invalid/v1",
            "https://example.invalid/v1?token=fixture",
            "https://example.invalid/v1#fragment",
        ] {
            assert!(CodexProfile {
                base_url: address.into(),
                ..profile()
            }
            .normalized()
            .is_err());
        }
        assert_eq!(
            capture_codex_profile(&before).unwrap().model,
            profile().model
        );
    }

    #[test]
    fn native_profile_overrides_are_displayed_but_not_lossily_captured_or_overwritten() {
        let sandbox = Sandbox::new();
        let target = sandbox.0.join(".codex/config.toml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let text = "model = 'root-model'\nprofile = 'work'\n[profiles.work]\nmodel = 'selected-model'\nmodel_provider = 'selected-provider'\n";
        fs::write(&target, text).unwrap();
        let store = ProviderStore::new(sandbox.0.join("providers"));
        let status = inspect_codex_config(Some(&target), &[], &store);
        assert_eq!(status.configured_model.as_deref(), Some("selected-model"));
        assert_eq!(
            status.configured_provider.as_deref(),
            Some("selected-provider")
        );
        assert!(status.current_draft.is_none());
        assert!(plan_codex_apply(text, &profile()).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), text);
        assert!(capture_codex_profile("model = 'native-model'").is_err());
    }

    #[test]
    fn the_profile_store_round_trips_and_preserves_a_corrupted_file() {
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
        store
            .save(CodexProfile {
                model: "gpt-5-mini".into(),
                ..profile()
            })
            .unwrap();
        let list = store.list();
        assert_eq!(list.len(), 2);
        assert_eq!(
            list.iter().find(|p| p.id == "work").unwrap().model,
            "gpt-5-mini"
        );
        // 写坏的 JSON：降级为空清单，**不 panic**
        fs::write(sandbox.0.join("providers/profiles.json"), "{ not json").unwrap();
        assert!(store.list().is_empty());
        assert!(store.list_checked().is_err());
        assert!(store.save(profile()).is_err());
        assert!(store.delete("work").is_err());
        assert_eq!(
            fs::read_to_string(store.profiles_path()).unwrap(),
            "{ not json"
        );
        // 恢复清单后才允许再次写入，损坏的原文件不能被空清单覆盖。
        fs::remove_file(store.profiles_path()).unwrap();
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
            vec!["config-3000.toml", "config-2000.toml", "config-1000.toml"]
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

#[cfg(test)]
mod mcp_backup_preview_tests {
    use super::*;
    #[test]
    fn full_restore_lists_mcp_additions_removals_and_changes_without_values() {
        let current="[mcp_servers.old]\ncommand='fixture'\n[mcp_servers.same]\nurl='https://example.invalid/mcp'\nenabled=true\nhttp_headers={Authorization='PRIVATE_FIXTURE_CURRENT'}\n";
        let backup="[mcp_servers.new]\ncommand='fixture'\n[mcp_servers.same]\nurl='https://example.invalid/mcp'\nenabled=false\nhttp_headers={Authorization='PRIVATE_FIXTURE_BACKUP'}\n";
        let changes = mcp_backup_changes(current, backup).unwrap();
        assert!(changes.iter().any(|change| change == "new · 恢复条目"));
        assert!(changes.iter().any(|change| change == "old · 移除条目"));
        assert!(changes.iter().any(|change| change == "same · 还原条目内容"));
        assert!(!changes.join(" ").contains("PRIVATE_FIXTURE"));
        assert!(mcp_backup_changes(current, current).unwrap().is_empty());
    }
}
