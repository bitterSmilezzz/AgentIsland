//! Connection preferences only: no credentials, responses, traffic or tool configuration.
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io::Read, path::PathBuf};

const MAX_BYTES: u64 = 256 * 1024;
const MAX_ITEMS: usize = 40;
const MAX_JS_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    NewApi,
    Magpie,
    CcSwitch,
    /// 自定义端点/本地模型网关：地址与认证都由用户声明，不套用任何内置服务语义。
    CustomEndpoint,
}

/// 一次读取要带的认证方案。**显式声明**，不从地址、响应或环境推断——
/// 推断意味着「换个地址就悄悄换了认证语义」，而那正是凭据流向失控的入口。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthScheme {
    /// 不发送任何认证头（公开状态接口）。
    #[default]
    None,
    /// `Authorization: Bearer <凭据>`。
    Bearer,
    /// 自定义请求头，如 `x-api-key`；头名必须是合法 HTTP token。
    Header { name: String },
}
impl AuthScheme {
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
    /// 自定义头名的合法性：非空、有界、全为 RFC 7230 token 字符。
    /// 允许 CR/LF 或空格等于允许请求头注入。
    fn valid_header_name(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 64
            && name.bytes().all(|b| {
                b.is_ascii_alphanumeric()
                    || matches!(
                        b,
                        b'!' | b'#'
                            | b'$'
                            | b'%'
                            | b'&'
                            | b'\''
                            | b'*'
                            | b'+'
                            | b'-'
                            | b'.'
                            | b'^'
                            | b'_'
                            | b'`'
                            | b'|'
                            | b'~'
                    )
            })
    }
}

/// 401 之后的刷新声明（RFC 6749 §6 的 refresh_token 授权）。
///
/// 只声明**端点与凭据来源名**：令牌本身始终从环境变量按需读取，不落盘、不进草稿。
/// 端点与业务地址受同一条传输策略约束——令牌端点用明文 HTTP 等于把刷新令牌交出去。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Refresh {
    pub token_url: String,
    pub client_id: String,
    pub refresh_token_env: String,
}

/// 订阅与余额查询协议配置（GW-01）。
///
/// 允许用户为服务连接显式声明查询端点与 JSON 取值路径。
/// 不做协议硬编码（不同服务返回格式各异：有的叫 remain_quota、有的叫 total_available），
/// 猜协议等于在网关升级时悄悄读错；未声明时诚实保持 `usage_verified = false`。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuotaConfig {
    /// 相对路径（如 "api/user/self"）或绝对地址（如 "https://api.example.com/quota"）
    pub endpoint: String,
    /// 字段提取路径（点分隔，例如 "data.quota"、"total_available"）
    pub balance_path: String,
    /// 货币或单位标识（如 "USD"、"CNY"、"Tokens"、"点"），缺省为空
    #[serde(default)]
    pub unit: Option<String>,
    /// 订阅套餐（plan/tier）字段的提取路径（如 "data.plan"、"membership.tier"）。
    ///
    /// 提取到的**原样显示**：套餐名是网关自己的说法，我们不翻译、不归一、
    /// 也不推断它对应哪一档能力。没声明或没取到就当没有——**不显示占位**。
    #[serde(default)]
    pub plan_path: Option<String>,
}


#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CredentialRef {
    Environment { name: String },
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub id: String,
    pub kind: Kind,
    pub name: String,
    pub base_url: String,
    pub credential_ref: Option<CredentialRef>,
    /// 旧配置文件没有这个字段；缺省为「不发送认证头」，保持既有行为不变。
    #[serde(default)]
    pub auth: AuthScheme,
    /// 旧配置没有这个字段：缺省即「401 不刷新」，与既有行为一致。
    #[serde(default)]
    pub refresh: Option<Refresh>,
    /// 旧配置没有这个字段：缺省即「不查询余额」，usage_verified 保持 false。
    #[serde(default)]
    pub quota: Option<QuotaConfig>,
    pub enabled: bool,
    pub created_ms: u64,
    pub updated_ms: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub id: Option<String>,
    pub kind: Kind,
    pub name: String,
    pub base_url: String,
    pub credential_ref: Option<CredentialRef>,
    #[serde(default)]
    pub auth: AuthScheme,
    #[serde(default)]
    pub refresh: Option<Refresh>,
    #[serde(default)]
    pub quota: Option<QuotaConfig>,
    pub enabled: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema_version: u32,
    pub revision: u64,
    pub items: Vec<Connection>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            items: Vec::new(),
        }
    }
}

pub struct Store {
    path: PathBuf,
}
impl Store {
    pub fn at_default() -> Self {
        Self {
            path: crate::settings::config_dir().join("connections.v1.json"),
        }
    }
    fn load(&self) -> Result<Snapshot, String> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Snapshot::default())
            }
            Err(_) => return Err("连接配置无法读取".into()),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES {
            return Err("连接配置文件类型或大小无效，原文件已保留".into());
        }
        let mut bytes = Vec::new();
        fs::File::open(&self.path)
            .map_err(|_| "连接配置无法打开")?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "连接配置无法读取")?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("连接配置过大，原文件已保留".into());
        }
        let data: Snapshot =
            serde_json::from_slice(&bytes).map_err(|_| "连接配置损坏或格式不支持，原文件已保留")?;
        validate(&data)?;
        Ok(data)
    }
    pub fn list(&self) -> Result<Snapshot, String> {
        self.load()
    }
    pub fn get(&self, id: &str, expected_revision: u64) -> Result<Connection, String> {
        let data = self.checked(expected_revision)?;
        data.items
            .into_iter()
            .find(|item| item.id == id)
            .ok_or_else(|| "连接已不存在".into())
    }
    fn checked(&self, revision: u64) -> Result<Snapshot, String> {
        let data = self.load()?;
        if data.revision != revision {
            return Err("连接配置已变化，请刷新后重试".into());
        }
        Ok(data)
    }
    fn write(&self, data: &Snapshot) -> Result<(), String> {
        validate(data)?;
        let bytes = serde_json::to_vec_pretty(data).map_err(|_| "连接配置无法编码")?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("连接配置过大，未保存".into());
        }
        fs::create_dir_all(self.path.parent().ok_or("连接配置目录无效")?)
            .map_err(|_| "连接配置目录无法创建")?;
        crate::atomicfile::atomic_replace_validated(&self.path, &bytes, |staged| {
            let data: Snapshot = serde_json::from_slice(&fs::read(staged)?)
                .map_err(|_| std::io::Error::other("invalid connection JSON"))?;
            validate(&data).map_err(|_| std::io::Error::other("invalid connection preferences"))
        })
        .map_err(|_| "连接配置保存失败，原文件已保留".to_owned())
    }
    pub fn save(&self, draft: Draft, revision: u64, now_ms: u64) -> Result<Snapshot, String> {
        let mut data = self.checked(revision)?;
        let previous = match draft.id.as_deref() {
            Some(id) => Some(
                data.items
                    .iter()
                    .position(|item| item.id == id)
                    .ok_or("连接已不存在，请刷新")?,
            ),
            None => None,
        };
        let item = Connection {
            id: previous
                .map(|i| data.items[i].id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            kind: draft.kind,
            name: draft.name.trim().to_owned(),
            base_url: normalize_url(&draft.base_url)?,
            credential_ref: draft.credential_ref,
            auth: draft.auth,
            refresh: draft.refresh,
            quota: draft.quota,
            enabled: draft.enabled,
            created_ms: previous.map(|i| data.items[i].created_ms).unwrap_or(now_ms),
            updated_ms: previous
                .map(|i| data.items[i].updated_ms.max(now_ms))
                .unwrap_or(now_ms),
        };
        if let Some(index) = previous {
            data.items[index] = item;
        } else {
            data.items.push(item);
        }
        for entry in &data.items {
            if !plaintext_credential_allowed(entry) {
                return Err(
                    "会发送凭据的地址必须使用 HTTPS（本机回环地址除外），未保存".into(),
                );
            }
        }
        data.revision = data.revision.checked_add(1).ok_or("连接配置版本已耗尽")?;
        self.write(&data)?;
        Ok(data)
    }
    pub fn remove(&self, id: &str, revision: u64) -> Result<Snapshot, String> {
        let mut data = self.checked(revision)?;
        let index = data
            .items
            .iter()
            .position(|item| item.id == id)
            .ok_or("连接已不存在，请刷新")?;
        data.items.remove(index);
        data.revision = data.revision.checked_add(1).ok_or("连接配置版本已耗尽")?;
        self.write(&data)?;
        Ok(data)
    }
}

fn contains_credential(text: &str) -> bool {
    text.contains("sk-") || text.contains("AIza") || text.contains("-----BEGIN")
}
pub fn normalize_url(text: &str) -> Result<String, String> {
    if text.len() > 2048 || text.chars().any(char::is_control) || contains_credential(text) {
        return Err("服务地址过长或包含凭据信息".into());
    }
    let text = text.trim();
    let mut url = url::Url::parse(text).map_err(|_| "请输入有效的 HTTP 或 HTTPS 服务地址")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.port() == Some(0)
    {
        return Err("请输入有效的 HTTP 或 HTTPS 服务地址".into());
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("服务地址不能包含登录信息、查询参数或片段".into());
    }
    // Preserve reverse-proxy prefixes when joining service-specific relative routes.
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url.into())
}
/// 环境变量名判据：**只有一处**。凭据引用与刷新令牌引用必须同形，
/// 否则同一个概念在两个字段上会有两种合法性，界面提示也就无法一致。
fn valid_env_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 80
        && name.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn validate(data: &Snapshot) -> Result<(), String> {
    if data.schema_version != 1 {
        return Err("连接配置版本不受支持，原文件已保留".into());
    }
    if data.items.len() > MAX_ITEMS {
        return Err("最多保存 40 个服务连接".into());
    }
    if data.revision > MAX_JS_INTEGER {
        return Err("连接配置版本超出范围，原文件已保留".into());
    }
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for item in &data.items {
        if !uuid::Uuid::parse_str(&item.id).is_ok_and(|id| id.to_string() == item.id)
            || !ids.insert(&item.id)
        {
            return Err("连接身份无效，原文件已保留".into());
        }
        if item.name.is_empty()
            || item.name.trim() != item.name
            || item.name.chars().count() > 80
            || item.name.chars().any(char::is_control)
            || contains_credential(&item.name)
            || !names.insert(&item.name)
        {
            return Err("连接名称应为 1–80 个字符且不能重复或含凭据".into());
        }
        if normalize_url(&item.base_url)? != item.base_url {
            return Err("连接地址格式无效，原文件已保留".into());
        }
        if item.updated_ms < item.created_ms || item.updated_ms > MAX_JS_INTEGER {
            return Err("连接时间记录无效，原文件已保留".into());
        }
        if let Some(CredentialRef::Environment { name }) = &item.credential_ref {
            if !valid_env_name(name) || contains_credential(name) {
                return Err("凭据引用须为大写环境变量名，不能填写密钥值".into());
            }
        }
        if let Some(refresh) = &item.refresh {
            if item.auth.is_none() {
                return Err("声明了令牌刷新却没有认证方案".into());
            }
            let endpoint = normalize_url(&refresh.token_url)?;
            let parsed = url::Url::parse(&endpoint).map_err(|_| "令牌端点地址格式无效".to_string())?;
            // 与业务地址同一条策略：明文远端端点会在建连前被拒绝。
            crate::connection_http::transport_policy(&parsed)
                .map_err(|_| "令牌端点必须使用 HTTPS（本机回环地址除外）".to_string())?;
            if refresh.client_id.is_empty()
                || refresh.client_id.len() > 200
                || refresh.client_id.chars().any(char::is_control)
                || contains_credential(&refresh.client_id)
            {
                return Err("客户端标识无效，不能含控制字符或凭据值".into());
            }
            if !valid_env_name(&refresh.refresh_token_env)
                || contains_credential(&refresh.refresh_token_env)
            {
                return Err("刷新令牌引用须为大写环境变量名，不能填写令牌值".into());
            }
        }
        // 声明要发认证头却没有凭据来源，是一个必然读不通的配置：尽早说清楚，
        // 而不是等用户点了「读取状态」才收到一句泛泛的失败。
        if let Some(quota) = &item.quota {
            if quota.endpoint.is_empty()
                || quota.endpoint.len() > 2048
                || quota.endpoint.chars().any(char::is_control)
                || contains_credential(&quota.endpoint)
            {
                return Err("余额查询端点无效，不能含控制字符或凭据值".into());
            }
            // 相对路径禁止路径穿越或根目录跳转
            if !quota.endpoint.starts_with("http://") && !quota.endpoint.starts_with("https://") {
                if quota.endpoint.starts_with('/')
                    || quota.endpoint.starts_with('\\')
                    || quota.endpoint.split('/').any(|part| part == "..")
                    || !quota.endpoint.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '.' | '?' | '=' | '&'))
                {
                    return Err("余额相对路径格式无效，不得包含 .. 或非常规字符".into());
                }
            }
            if quota.balance_path.is_empty()
                || quota.balance_path.len() > 100
                || contains_credential(&quota.balance_path)
                || !quota.balance_path.split('.').all(|seg| !seg.is_empty() && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            {
                return Err("余额提取路径须为点分隔的属性名（如 data.quota），不能包含凭据值".into());
            }
            if let Some(unit) = &quota.unit {
                if unit.chars().count() > 16 || unit.chars().any(char::is_control) || contains_credential(unit) {
                    return Err("余额单位不能超过 16 个字符，不能含控制字符或凭据值".into());
                }
            }
            if let Some(plan) = &quota.plan_path {
                // 与余额路径同一条判据：点分隔的属性名，不许穿越、不许带凭据。
                if plan.is_empty()
                    || plan.len() > 100
                    || contains_credential(plan)
                    || !plan.split('.').all(|seg| !seg.is_empty() && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                {
                    return Err("套餐提取路径须为点分隔的属性名（如 data.plan），不能包含凭据值".into());
                }
            }
        }
                if !item.auth.is_none() && item.credential_ref.is_none() {
            return Err("该认证方案需要先指定凭据来源（环境变量名）".into());
        }
        if let AuthScheme::Header { name } = &item.auth {
            if !AuthScheme::valid_header_name(name) || contains_credential(name) {
                return Err("自定义认证头名必须是合法的 HTTP 头名，不能填写凭据值".into());
            }
        }
    }
    Ok(())
}

/// 会发送凭据的地址必须是 HTTPS；本机回环地址除外（本地模型网关的常见形态）。
///
/// 只在**保存**时执行这条判据，`validate` 不检查它：已经存在于磁盘上的明文配置
/// 依然要能读出来并显示给用户，由读取路径在建立连接之前拒绝并给出明确原因——
/// 「保存新错误配置」要拦住，「让用户看不见自己的旧配置」则是另一种伤害。
pub(crate) fn plaintext_credential_allowed(item: &Connection) -> bool {
    if !item.auth.is_none() {
        match url::Url::parse(&item.base_url) {
            Ok(url) => {
                if crate::connection_http::transport_policy(&url).is_err() {
                    return false;
                }
            }
            Err(_) => return false,
        }
    }
    if let Some(refresh) = &item.refresh {
        if let Ok(url) = url::Url::parse(&refresh.token_url) {
            if crate::connection_http::transport_policy(&url).is_err() {
                return false;
            }
        }
    }
    if let Some(quota) = &item.quota {
        if quota.endpoint.starts_with("http://") || quota.endpoint.starts_with("https://") {
            if let Ok(url) = url::Url::parse(&quota.endpoint) {
                if crate::connection_http::transport_policy(&url).is_err() {
                    return false;
                }
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Sandbox;
    fn store(root: &Sandbox) -> Store {
        Store {
            path: root.path().join("connections.v1.json"),
        }
    }
    fn draft() -> Draft {
        Draft {
            id: None,
            kind: Kind::NewApi,
            name: "本地服务".into(),
            base_url: "http://localhost:8080/proxy".into(),
            credential_ref: Some(CredentialRef::Environment {
                name: "FIXTURE_AUTH".into(),
            }),
            auth: AuthScheme::None,
            refresh: None,
            quota: None,
            enabled: true,
        }
    }
    /// 套餐路径的校验：与余额路径同一条判据（点分隔属性名，不许穿越/带凭据）。
    #[test]
    fn the_plan_path_is_validated_like_the_balance_path() {
        let root = Sandbox::new("connections-plan-path");
        let s = store(&root);
        let mut good = draft();
        good.quota = Some(QuotaConfig {
            endpoint: "api/user/quota".into(),
            balance_path: "data.quota".into(),
            unit: None,
            plan_path: Some("data.plan".into()),
        });
        assert!(s.save(good, 0, 100).is_ok(), "合法的套餐路径应被接受");

        for bad in ["", "data..plan", "data.plan path", "..plan", "sk-ABCDEFGH12345"] {
            let mut d_bad = draft();
            d_bad.quota = Some(QuotaConfig {
                endpoint: "api/user/quota".into(),
                balance_path: "data.quota".into(),
                unit: None,
                plan_path: Some(bad.into()),
            });
            assert!(
                s.save(d_bad, 0, 100).is_err(),
                "非法套餐路径 {bad:?} 不该被接受"
            );
        }
    }

    #[test]
    fn roundtrip_edit_disable_remove_preserves_identity_and_creation() {
        let root = Sandbox::new("connections");
        let s = store(&root);
        let first = s.save(draft(), 0, 100).unwrap();
        let id = first.items[0].id.clone();
        assert_eq!(first.items[0].base_url, "http://localhost:8080/proxy/");
        let mut edit = draft();
        edit.id = Some(id.clone());
        edit.enabled = false;
        edit.name = "调整后".into();
        let second = s.save(edit, 1, 90).unwrap();
        assert_eq!(second.items.len(), 1);
        assert_eq!(second.items[0].created_ms, 100);
        assert_eq!(second.items[0].updated_ms, 100);
        assert!(!s.get(&id, 2).unwrap().enabled);
        assert!(s.get(&id, 1).is_err());
        assert_eq!(s.remove(&id, 2).unwrap().revision, 3);
        assert!(s.list().unwrap().items.is_empty());
        let text = fs::read_to_string(&s.path).unwrap();
        assert!(!text.contains("FIXTURE_AUTH"));
    }
    #[test]
    fn stale_missing_duplicate_and_invalid_updates_leave_original_unchanged() {
        let root = Sandbox::new("connections");
        let s = store(&root);
        s.save(draft(), 0, 100).unwrap();
        let before = fs::read(&s.path).unwrap();
        assert!(s.save(draft(), 0, 110).is_err());
        assert!(s.save(draft(), 1, 110).is_err());
        let mut missing = draft();
        missing.id = Some(uuid::Uuid::new_v4().to_string());
        assert!(s.save(missing, 1, 110).is_err());
        assert!(s.remove("missing", 1).is_err());
        let mut invalid = draft();
        invalid.name = String::new();
        assert!(s.save(invalid, 1, 110).is_err());
        assert_eq!(fs::read(&s.path).unwrap(), before);
    }
    #[test]
    fn credential_values_and_unknown_fields_cannot_enter_drafts() {
        let root = Sandbox::new("connections");
        let s = store(&root);
        for name in [
            "123_VALUE",
            "has space",
            "lowercase",
            "AUTH=value",
            "NAME\n",
        ] {
            let mut input = draft();
            input.credential_ref = Some(CredentialRef::Environment { name: name.into() });
            assert!(s.save(input, 0, 100).is_err());
            assert!(!s.path.exists());
        }
        let mut value = serde_json::json!({"id":null,"kind":"new_api","name":"Fixture","base_url":"https://example.invalid/","credential_ref":{"kind":"environment","name":"FIXTURE_AUTH","value":"fixture"},"enabled":true});
        assert!(serde_json::from_value::<Draft>(value.clone()).is_err());
        value["credential_ref"] = serde_json::json!({"kind":"keychain","name":"remote.smtpEmail"});
        assert!(
            serde_json::from_value::<Draft>(value).is_err(),
            "unimplemented credential sources must be refused"
        );
        let data = s.save(draft(), 0, 100).unwrap();
        let encoded = serde_json::to_value(data).unwrap();
        assert_eq!(
            encoded["items"][0]["credential_ref"],
            serde_json::json!({"kind":"environment","name":"FIXTURE_AUTH"})
        );
    }
    #[test]
    fn urls_preserve_proxy_prefixes_but_reject_authentication_and_non_http() {
        assert_eq!(
            normalize_url("https://EXAMPLE.invalid/proxy").unwrap(),
            "https://example.invalid/proxy/"
        );
        assert_eq!(
            normalize_url("http://[::1]:8080/").unwrap(),
            "http://[::1]:8080/"
        );
        for input in [
            "file:///tmp",
            "https://name@example.invalid/",
            "https://example.invalid/?auth=fixture",
            "https://example.invalid/#fragment",
            "http://localhost:0/",
            "https://example.invalid/\n",
            "bad address",
        ] {
            assert!(normalize_url(input).is_err());
        }
    }
    #[test]
    fn corrupt_future_unknown_and_oversized_files_are_preserved() {
        let root = Sandbox::new("connections");
        let s = store(&root);
        for text in [
            "broken",
            r#"{"schema_version":2,"revision":0,"items":[]}"#,
            r#"{"schema_version":1,"revision":0,"items":[],"unknown":"fixture"}"#,
        ] {
            fs::write(&s.path, text).unwrap();
            assert!(s.list().is_err());
            assert!(s.save(draft(), 0, 100).is_err());
            assert_eq!(fs::read_to_string(&s.path).unwrap(), text);
        }
        fs::write(&s.path, vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
        assert!(s.save(draft(), 0, 100).is_err());
        assert_eq!(fs::metadata(&s.path).unwrap().len(), MAX_BYTES + 1);
    }
    #[test]
    fn connection_count_and_javascript_revision_bound_are_enforced() {
        let root = Sandbox::new("connections");
        let s = store(&root);
        for i in 0..MAX_ITEMS {
            let mut input = draft();
            input.name = format!("Fixture {i}");
            s.save(input, i as u64, 100).unwrap();
        }
        let before = fs::read(&s.path).unwrap();
        assert!(s.save(draft(), MAX_ITEMS as u64, 100).is_err());
        assert_eq!(fs::read(&s.path).unwrap(), before);
        let mut full = s.list().unwrap();
        full.revision = MAX_JS_INTEGER;
        s.write(&full).unwrap();
        let before = fs::read(&s.path).unwrap();
        assert!(s.remove(&full.items[0].id, MAX_JS_INTEGER).is_err());
        assert_eq!(fs::read(&s.path).unwrap(), before);
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_and_failed_staging_do_not_destroy_the_existing_file() {
        use std::os::unix::fs::PermissionsExt;
        let root = Sandbox::new("connections");
        let s = store(&root);
        let target = root.path().join("original");
        fs::write(&target, "original").unwrap();
        std::os::unix::fs::symlink(&target, &s.path).unwrap();
        assert!(s.save(draft(), 0, 100).is_err());
        assert!(fs::symlink_metadata(&s.path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_to_string(target).unwrap(), "original");
        fs::remove_file(&s.path).unwrap();
        s.save(draft(), 0, 100).unwrap();
        let before = fs::read(&s.path).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            fs::set_permissions(root.path(), fs::Permissions::from_mode(0o500)).unwrap();
            let result = s.remove(&s.list().unwrap().items[0].id, 1);
            let after = fs::read(&s.path).unwrap();
            fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
            assert!(result.is_err());
            assert_eq!(after, before);
        }
    }

    #[test]
    fn invalid_auth_scheme_is_refused_without_changing_the_saved_configuration() {
        let root = Sandbox::new("connections-auth-scheme");
        let s = store(&root);
        let base = s.save(draft(), 0, 100).unwrap();
        let id = base.items[0].id.clone();

        // 自定义头名必须是合法 HTTP token：CR/LF 与空格等于请求头注入。
        for bad in ["", "bad name", "evil\r\nX-Injected: 1", &"a".repeat(65)] {
            let mut next = draft();
            next.id = Some(id.clone());
            next.kind = Kind::CustomEndpoint;
            next.auth = AuthScheme::Header { name: bad.to_string() };
            assert!(s.save(next, 1, 101).is_err(), "{bad:?}");
        }
        // 声明要发认证头却没有凭据来源：这是一个必然读不通的配置。
        let mut orphan = draft();
        orphan.id = Some(id.clone());
        orphan.kind = Kind::CustomEndpoint;
        orphan.auth = AuthScheme::Bearer;
        orphan.credential_ref = None;
        assert!(s.save(orphan, 1, 102).is_err());
        assert_eq!(s.list().unwrap().items[0].base_url, "http://localhost:8080/proxy/");
    }

    #[test]
    fn save_refuses_plaintext_credentials_to_non_loopback_hosts() {
        let root = Sandbox::new("connections-plaintext");
        let s = store(&root);
        let first = s.save(draft(), 0, 100).unwrap();
        let id = first.items[0].id.clone();
        let secret_ref = Some(CredentialRef::Environment {
            name: "FIXTURE_AUTH".into(),
        });

        // 本机回环地址 + 认证方案：允许（本地模型网关的常见形态）。
        let local = Draft {
            id: Some(id.clone()),
            kind: Kind::CustomEndpoint,
            name: "本机网关".into(),
            base_url: "http://127.0.0.1:11434".into(),
            credential_ref: secret_ref.clone(),
            auth: AuthScheme::Bearer,
            refresh: None,
            quota: None,
            enabled: true,
        };
        let second = s.save(local, 1, 101).unwrap();
        assert_eq!(second.items[0].base_url, "http://127.0.0.1:11434/");

        // 远端明文 + 认证方案：拒绝保存，磁盘上的配置保持上一次的可用状态。
        let remote = Draft {
            id: Some(id.clone()),
            kind: Kind::CustomEndpoint,
            name: "本机网关".into(),
            base_url: "http://gateway.example".into(),
            credential_ref: secret_ref,
            auth: AuthScheme::Bearer,
            refresh: None,
            quota: None,
            enabled: true,
        };
        assert!(s.save(remote, second.revision, 102).is_err());
        assert_eq!(
            s.list().unwrap().items[0].base_url,
            "http://127.0.0.1:11434/"
        );

        // 远端明文但不发送认证头（公开状态接口）：不受这条规则约束。
        let public = Draft {
            id: Some(id),
            kind: Kind::CustomEndpoint,
            name: "本机网关".into(),
            base_url: "http://gateway.example".into(),
            credential_ref: None,
            auth: AuthScheme::None,
            refresh: None,
            quota: None,
            enabled: true,
        };
        assert!(s.save(public, second.revision, 103).is_ok());
    }

    /// 令牌端点与业务地址受**同一条**传输策略约束：刷新令牌也是凭据，
    /// 明文远端端点会在建连前被拒。另含「声明刷新但没有认证方案」的配置。
    #[test]
    fn refresh_declaration_requires_secure_endpoint_and_an_auth_scheme() {
        let dir = crate::testutil::Sandbox::new("connections-refresh");
        let s = store(&dir);
        let base = Draft {
            id: None,
            kind: Kind::CustomEndpoint,
            name: "网关".into(),
            base_url: "https://gateway.example/".into(),
            credential_ref: Some(CredentialRef::Environment {
                name: "FIXTURE_AUTH".into(),
            }),
            auth: AuthScheme::Bearer,
            refresh: None,
            quota: None,
            enabled: true,
        };
        let created = s.save(base, 0, 1).unwrap();

        let with = |token_url: &str, env: &str| Draft {
            id: Some(created.items[0].id.clone()),
            kind: Kind::CustomEndpoint,
            name: "网关".into(),
            base_url: "https://gateway.example/".into(),
            credential_ref: Some(CredentialRef::Environment {
                name: "FIXTURE_AUTH".into(),
            }),
            auth: AuthScheme::Bearer,
            refresh: Some(Refresh {
                token_url: token_url.into(),
                client_id: "fixture-client".into(),
                refresh_token_env: env.into(),
            }),
            quota: None,
            enabled: true,
        };

        // 明文远端令牌端点：拒绝（保存时）。
        assert!(s
            .save(with("http://gateway.example/token", "FIXTURE_REFRESH"), created.revision, 2)
            .is_err());
        // 本机回环允许明文（本地模型网关的常见形态）。
        assert!(s
            .save(with("http://127.0.0.1:9999/token", "FIXTURE_REFRESH"), created.revision, 3)
            .is_ok());
        // 刷新令牌引用必须是大写环境变量名，不能直接填令牌值。
        let second = s.list().unwrap();
        assert!(s
            .save(with("https://gateway.example/token", "not-an-env"), second.revision, 4)
            .is_err());
        // 声明刷新却没有认证方案：必然读不通，尽早拒绝。
        let mut no_auth = with("https://gateway.example/token", "FIXTURE_REFRESH");
        no_auth.auth = AuthScheme::None;
        assert!(s.save(no_auth, second.revision, 5).is_err());
    }

    /// 余额声明同样受约束：相对路径不得穿越、提取路径必须是点分属性名、
    /// 绝对明文远端端点保存即拒绝（与业务地址同一条传输策略）。
    #[test]
    fn quota_declaration_rejects_traversal_bad_paths_and_plaintext_remote() {
        let dir = crate::testutil::Sandbox::new("connections-quota");
        let s = store(&dir);
        let base = Draft {
            id: None,
            kind: Kind::CustomEndpoint,
            name: "网关".into(),
            base_url: "https://gateway.example/".into(),
            credential_ref: Some(CredentialRef::Environment {
                name: "FIXTURE_AUTH".into(),
            }),
            auth: AuthScheme::Bearer,
            refresh: None,
            quota: None,
            enabled: true,
        };
        let created = s.save(base, 0, 1).unwrap();
        let with = |endpoint: &str, path: &str| Draft {
            id: Some(created.items[0].id.clone()),
            kind: Kind::CustomEndpoint,
            name: "网关".into(),
            base_url: "https://gateway.example/".into(),
            credential_ref: Some(CredentialRef::Environment {
                name: "FIXTURE_AUTH".into(),
            }),
            auth: AuthScheme::Bearer,
            refresh: None,
            quota: Some(QuotaConfig {
                plan_path: None,
                endpoint: endpoint.into(),
                balance_path: path.into(),
                unit: Some("Tokens".into()),
            }),
            enabled: true,
        };

        assert!(s.save(with("../../etc/passwd", "data.quota"), created.revision, 2).is_err(),
                "路径穿越必须拒绝");
        assert!(s.save(with("api/user/quota", "data..quota"), created.revision, 3).is_err(),
                "空段路径必须拒绝");
        assert!(s.save(with("http://gateway.example/quota", "data.quota"), created.revision, 4).is_err(),
                "明文远端余额端点必须拒绝");
        assert!(s.save(with("api/user/quota", "data.remain_quota"), created.revision, 5).is_ok(),
                "合法声明必须保存");
    }
}
