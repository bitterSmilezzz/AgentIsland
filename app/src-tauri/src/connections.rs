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
            if name.is_empty()
                || name.len() > 80
                || !name.starts_with(|c: char| c.is_ascii_uppercase() || c == '_')
                || !name
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                || contains_credential(name)
            {
                return Err("凭据引用须为大写环境变量名，不能填写密钥值".into());
            }
        }
    }
    Ok(())
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
            enabled: true,
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
}
