//! Persisted declarations only. These profiles neither authorize nor start execution.
use crate::agent_resources::{ModelSpec, ResourceBinding, ResourceRegistry, RoleSpec, RuntimeSpec};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io::Read, path::PathBuf};

const MAX_BYTES: u64 = 256 * 1024;
const MAX_PROFILES: usize = 100;
const MAX_ITEMS: usize = 64;
const MAX_TEXT_BYTES: usize = 256;
const MAX_JS_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub role: RoleSpec,
    pub runtime: RuntimeSpec,
    pub model: ModelSpec,
    pub memory_scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema_version: u32,
    pub revision: u64,
    pub profiles: Vec<Profile>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            profiles: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Error {
    pub code: String,
    pub message: String,
}
fn error(code: &str, message: &str) -> Error {
    Error {
        code: code.into(),
        message: message.into(),
    }
}
fn valid_text(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_TEXT_BYTES
        && s.trim() == s
        && !s.chars().any(char::is_control)
        && !crate::private_text::known_private(s)
}
fn valid_list(items: &[String]) -> bool {
    items.len() <= MAX_ITEMS
        && items.iter().all(|s| valid_text(s))
        && items.iter().collect::<HashSet<_>>().len() == items.len()
}
impl Profile {
    fn validate(&self) -> Result<(), Error> {
        if ![
            &self.name,
            &self.role.name,
            &self.runtime.name,
            &self.model.name,
            &self.memory_scope,
        ]
        .iter()
        .all(|s| valid_text(s))
            || !valid_list(&self.role.required_capabilities)
            || !valid_list(&self.role.granted_scopes)
            || !valid_list(&self.runtime.capabilities)
            || !valid_list(&self.model.capabilities)
            || self.runtime.max_concurrency == 0
            || self.runtime.max_concurrency > 1024
        {
            return Err(error(
                "invalid_input",
                "请检查名称、声明、范围和并发上限；不能包含已知凭据格式",
            ));
        }
        let mut registry = ResourceRegistry::new();
        registry.add_role(self.role.clone());
        registry.add_runtime(self.runtime.clone());
        registry.add_model(self.model.clone());
        registry
            .check_binding(&ResourceBinding {
                role: Some(self.role.name.clone()),
                runtime: Some(self.runtime.name.clone()),
                model: Some(self.model.name.clone()),
                memory_scope: Some(self.memory_scope.clone()),
            })
            .map_err(|_| error("incompatible", "角色声明与运行时、模型或记忆范围不相容"))
    }
}
impl Snapshot {
    fn validate(&self) -> Result<(), Error> {
        let bad = || error("read_only", "资源配置损坏或版本不兼容，原文件已保留");
        if self.schema_version != 1
            || self.revision > MAX_JS_INTEGER
            || self.profiles.len() > MAX_PROFILES
        {
            return Err(bad());
        }
        let mut ids = HashSet::new();
        for profile in &self.profiles {
            if uuid::Uuid::parse_str(&profile.id).is_err()
                || !ids.insert(&profile.id)
                || profile.validate().is_err()
            {
                return Err(bad());
            }
        }
        Ok(())
    }
}

pub struct Store {
    pub path: PathBuf,
    isolated: bool,
}
impl Store {
    pub fn at_default(isolated: bool) -> Self {
        Self {
            path: crate::settings::config_dir().join("chief-resources.v1.json"),
            isolated,
        }
    }
    pub fn load(&self) -> Result<Snapshot, Error> {
        if self.isolated {
            return Ok(Snapshot::default());
        }
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Snapshot::default()),
            Err(_) => return Err(error("io_error", "资源配置不可读，原文件已保留")),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES {
            return Err(error(
                "read_only",
                "资源配置类型或容量不受支持，原文件已保留",
            ));
        }
        let file = fs::File::open(&self.path)
            .map_err(|_| error("io_error", "资源配置不可读，原文件已保留"))?;
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| error("io_error", "资源配置不可读，原文件已保留"))?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(error("read_only", "资源配置超出容量，原文件已保留"));
        }
        let data: Snapshot = serde_json::from_slice(&bytes)
            .map_err(|_| error("read_only", "资源配置损坏或版本不兼容，原文件已保留"))?;
        data.validate()?;
        Ok(data)
    }
    fn write(&self, data: &Snapshot) -> Result<(), Error> {
        data.validate()?;
        let bytes =
            serde_json::to_vec_pretty(data).map_err(|_| error("io_error", "资源配置编码失败"))?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(error("capacity", "资源配置达到容量上限"));
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| error("io_error", "资源配置目录不可用"))?;
        fs::create_dir_all(parent).map_err(|_| error("io_error", "资源配置目录不可写"))?;
        crate::atomicfile::atomic_replace_validated(&self.path, &bytes, |path| {
            let check: Snapshot =
                serde_json::from_slice(&fs::read(path)?).map_err(std::io::Error::other)?;
            check
                .validate()
                .map_err(|_| std::io::Error::other("invalid resource data"))
        })
        .map_err(|_| error("io_error", "资源配置保存失败，原文件已保留"))
    }
    fn edit(
        &self,
        revision: u64,
        change: impl FnOnce(&mut Snapshot) -> Result<(), Error>,
    ) -> Result<Snapshot, Error> {
        let mut data = self.load()?;
        if data.revision != revision {
            return Err(error("stale_revision", "资源配置已更新，请刷新后重试"));
        }
        change(&mut data)?;
        data.revision = data
            .revision
            .checked_add(1)
            .filter(|n| *n <= MAX_JS_INTEGER)
            .ok_or_else(|| error("capacity", "资源配置版本已达上限"))?;
        self.write(&data)?;
        Ok(data)
    }
    pub fn save(&self, mut profile: Profile, revision: u64) -> Result<Snapshot, Error> {
        if self.isolated {
            return Err(error("unavailable", "测试实例不能修改资源配置"));
        }
        profile.validate()?;
        self.edit(revision, |data| {
            if profile.id.is_empty() {
                if data.profiles.len() >= MAX_PROFILES {
                    return Err(error("capacity", "资源配置数量已达上限"));
                }
                profile.id = uuid::Uuid::new_v4().to_string();
                data.profiles.push(profile);
            } else {
                let existing = data
                    .profiles
                    .iter_mut()
                    .find(|p| p.id == profile.id)
                    .ok_or_else(|| error("not_found", "资源配置不存在，请刷新后重试"))?;
                *existing = profile;
            }
            Ok(())
        })
    }
    pub fn delete(&self, id: &str, revision: u64) -> Result<Snapshot, Error> {
        if self.isolated {
            return Err(error("unavailable", "测试实例不能修改资源配置"));
        }
        self.edit(revision, |data| {
            let index = data
                .profiles
                .iter()
                .position(|p| p.id == id)
                .ok_or_else(|| error("not_found", "资源配置不存在，请刷新后重试"))?;
            data.profiles.remove(index);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let dir =
                std::env::temp_dir().join(format!("chief-resources-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn store(&self) -> Store {
            Store {
                path: self.0.join("resources.json"),
                isolated: false,
            }
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn profile() -> Profile {
        Profile {
            id: String::new(),
            name: "开发团队".into(),
            role: RoleSpec {
                name: "worker".into(),
                required_capabilities: vec!["code".into()],
                granted_scopes: vec!["topic-a".into()],
            },
            runtime: RuntimeSpec {
                name: "local".into(),
                capabilities: vec!["code".into()],
                max_concurrency: 2,
            },
            model: ModelSpec {
                name: "model".into(),
                capabilities: vec!["code".into()],
            },
            memory_scope: "topic-a".into(),
        }
    }
    #[test]
    fn create_reload_update_delete_and_reject_stale_revision() {
        let temp = Temp::new();
        let store = temp.store();
        let created = store.save(profile(), 0).unwrap();
        assert_eq!(created.revision, 1);
        assert!(uuid::Uuid::parse_str(&created.profiles[0].id).is_ok());
        assert_eq!(temp.store().load().unwrap(), created);
        let before = fs::read(&store.path).unwrap();
        assert_eq!(store.save(profile(), 0).unwrap_err().code, "stale_revision");
        assert_eq!(fs::read(&store.path).unwrap(), before);
        let mut edited = created.profiles[0].clone();
        edited.name = "审查团队".into();
        let updated = store.save(edited.clone(), 1).unwrap();
        assert_eq!(updated.profiles, vec![edited.clone()]);
        assert_eq!(store.delete(&edited.id, 2).unwrap().profiles.len(), 0);
        assert_eq!(store.delete(&edited.id, 3).unwrap_err().code, "not_found");
    }
    #[test]
    fn validation_rejects_incompatible_scope_capabilities_and_known_secrets_without_writing() {
        let temp = Temp::new();
        let store = temp.store();
        store.save(profile(), 0).unwrap();
        let original = fs::read(&store.path).unwrap();
        let mut bad = profile();
        bad.memory_scope = "other".into();
        assert_eq!(store.save(bad, 1).unwrap_err().code, "incompatible");
        let mut bad = profile();
        bad.model.capabilities.clear();
        assert_eq!(store.save(bad, 1).unwrap_err().code, "incompatible");
        let mut bad = profile();
        bad.role.name = format!("{}{}", "sk-", "a".repeat(24));
        assert_eq!(store.save(bad, 1).unwrap_err().code, "invalid_input");
        let mut bad = profile();
        bad.runtime.max_concurrency = 0;
        assert_eq!(store.save(bad, 1).unwrap_err().code, "invalid_input");
        let mut bad = profile();
        bad.role.required_capabilities = vec!["x".into(); MAX_ITEMS + 1];
        assert_eq!(store.save(bad, 1).unwrap_err().code, "invalid_input");
        assert_eq!(fs::read(&store.path).unwrap(), original);
    }
    #[test]
    fn damaged_future_unknown_and_oversized_files_are_preserved() {
        let temp = Temp::new();
        let store = temp.store();
        for bytes in [
            b"{broken".to_vec(),
            br#"{"schema_version":2,"revision":0,"profiles":[]}"#.to_vec(),
            br#"{"schema_version":1,"revision":0,"profiles":[],"unknown":true}"#.to_vec(),
            vec![b'x'; MAX_BYTES as usize + 1],
        ] {
            fs::write(&store.path, &bytes).unwrap();
            assert_eq!(store.load().unwrap_err().code, "read_only");
            assert!(store.save(profile(), 0).is_err());
            assert_eq!(fs::read(&store.path).unwrap(), bytes);
        }
    }
    #[cfg(unix)]
    #[test]
    fn symlink_is_not_read_or_replaced() {
        let temp = Temp::new();
        let store = temp.store();
        let target = temp.0.join("target");
        fs::write(&target, b"original").unwrap();
        std::os::unix::fs::symlink(&target, &store.path).unwrap();
        assert_eq!(store.save(profile(), 0).unwrap_err().code, "read_only");
        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert!(fs::symlink_metadata(&store.path)
            .unwrap()
            .file_type()
            .is_symlink());
    }
    #[test]
    fn same_revision_concurrent_writers_have_one_winner() {
        let temp = Temp::new();
        let store = std::sync::Arc::new(std::sync::Mutex::new(temp.store()));
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let store = store.clone();
                std::thread::spawn(move || store.lock().unwrap().save(profile(), 0))
            })
            .collect();
        let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| r.as_ref().is_err_and(|e| e.code == "stale_revision"))
                .count(),
            1
        );
        assert_eq!(store.lock().unwrap().load().unwrap().profiles.len(), 1);
    }

    #[test]
    fn smoke_and_background_instances_do_not_read_or_write_the_user_file() {
        let temp = Temp::new();
        let user_store = temp.store();
        user_store.save(profile(), 0).unwrap();
        let original = fs::read(&user_store.path).unwrap();
        for (flag, motion) in [
            ("--ui-smoke", false),
            ("--background-test", false),
            ("--memory-smoke", false),
            ("--dock-smoke", false),
            ("--motion-smoke", true),
        ] {
            let isolated =
                crate::isolated_instance_for_args([flag.to_string()].into_iter(), motion);
            assert!(isolated);
            let test_store = Store {
                path: user_store.path.clone(),
                isolated,
            };
            assert!(test_store.load().unwrap().profiles.is_empty());
            assert_eq!(
                test_store.save(profile(), 0).unwrap_err().code,
                "unavailable"
            );
            assert_eq!(
                test_store.delete("anything", 0).unwrap_err().code,
                "unavailable"
            );
            assert_eq!(fs::read(&user_store.path).unwrap(), original);
        }
        assert!(!crate::isolated_instance_for_args(
            ["agentisland".into()].into_iter(),
            false
        ));
        assert_eq!(user_store.load().unwrap().profiles.len(), 1);
    }
}
