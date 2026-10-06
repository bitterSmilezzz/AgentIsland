//! Durable metadata for server-executed workspace configuration operations.
use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::PathBuf};
const CAP: u64 = 512 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub id: String,
    pub expected_revision: u64,
    pub profile_id: String,
    pub recovery_operation_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Pending,
    Applied,
    Restored,
    Failed,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub operation_id: String,
    pub workspace_id: String,
    pub workspace_revision: u64,
    pub profile_id: String,
    pub recovery_of: Option<String>,
    pub created_ms: i64,
    pub phase: Phase,
    pub backup_name: Option<String>,
    pub warning: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Data {
    schema_version: u32,
    items: Vec<Record>,
}
pub struct Store {
    path: PathBuf,
}
#[derive(Serialize)]
pub struct Applied {
    #[serde(flatten)]
    pub result: crate::provider::ProviderApplyResult,
    pub operation_id: Option<String>,
}

impl Store {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
    pub fn list(&self) -> Result<Vec<Record>, String> {
        let meta = match fs::symlink_metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(_) => return Err("操作记录不可读，原件保留".into()),
        };
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > CAP {
            return Err("操作记录文件类型或大小不受支持".into());
        }
        let mut bytes = vec![];
        fs::File::open(&self.path)
            .map_err(|_| "操作记录无法打开")?
            .take(CAP + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "操作记录无法读取")?;
        if bytes.len() as u64 > CAP {
            return Err("操作记录超过容量".into());
        }
        let data: Data =
            serde_json::from_slice(&bytes).map_err(|_| "操作记录损坏或格式不支持，原件保留")?;
        validate(&data)?;
        Ok(data.items)
    }
    fn write(&self, items: Vec<Record>) -> Result<(), String> {
        let data = Data {
            schema_version: 1,
            items,
        };
        validate(&data)?;
        let bytes = serde_json::to_vec(&data).map_err(|_| "操作记录无法编码")?;
        if bytes.len() as u64 > CAP {
            return Err("操作记录达到容量上限，历史保留".into());
        }
        fs::create_dir_all(self.path.parent().ok_or("操作记录目录不可用")?)
            .map_err(|_| "操作记录目录无法创建")?;
        crate::atomicfile::atomic_replace_validated(&self.path, &bytes, |p| {
            let d: Data = serde_json::from_slice(&fs::read(p)?).map_err(std::io::Error::other)?;
            validate(&d).map_err(std::io::Error::other)
        })
        .map_err(|_| "操作记录保存失败，原件保留".into())
    }
    /// Prewrite failure prevents the mutation; postwrite failure never disguises its result.
    pub fn execute(
        &self,
        ctx: &Context,
        recovery: Option<&str>,
        write: impl FnOnce() -> Result<crate::provider::ProviderApplyResult, String>,
    ) -> Result<Applied, String> {
        let operation_id = self.begin(ctx, recovery)?;
        match write() {
            Ok(mut result) => {
                let recorded = self
                    .finish(
                        &operation_id,
                        Ok((&result.backup_name, result.record_warning.is_some())),
                        recovery.is_some(),
                    )
                    .is_ok();
                if !recorded {
                    result.record_warning=Some("配置已写入，工作空间操作记录未完成；保留本次备份并在配置页核对，勿重复应用。".into());
                }
                Ok(Applied {
                    result,
                    operation_id: recorded.then_some(operation_id),
                })
            }
            Err(error) => {
                let _ = self.finish(&operation_id, Err(()), recovery.is_some());
                Err(error)
            }
        }
    }
    pub fn begin(&self, ctx: &Context, recovery: Option<&str>) -> Result<String, String> {
        let mut items = self.list()?;
        if recovery.is_some() != ctx.recovery_operation_id.is_some() {
            return Err("恢复操作身份不完整".into());
        }
        if let Some(name) = recovery {
            let _previous = items
                .iter()
                .find(|r| {
                    Some(&r.operation_id) == ctx.recovery_operation_id.as_ref()
                        && r.workspace_id == ctx.id
                        && r.profile_id == ctx.profile_id
                        && r.backup_name.as_deref() == Some(name)
                        && matches!(r.phase, Phase::Applied | Phase::Restored)
                })
                .ok_or("恢复记录或备份不属于此组合")?;
        }
        let id = uuid::Uuid::new_v4().to_string();
        items.push(Record {
            operation_id: id.clone(),
            workspace_id: ctx.id.clone(),
            workspace_revision: ctx.expected_revision,
            profile_id: ctx.profile_id.clone(),
            recovery_of: ctx.recovery_operation_id.clone(),
            created_ms: crate::tokens::now_ms(),
            phase: Phase::Pending,
            backup_name: None,
            warning: false,
        });
        self.write(items)?;
        Ok(id)
    }
    pub fn finish(
        &self,
        id: &str,
        result: Result<(&str, bool), ()>,
        restore: bool,
    ) -> Result<(), String> {
        let mut items = self.list()?;
        let record = items
            .iter_mut()
            .find(|r| r.operation_id == id && r.phase == Phase::Pending)
            .ok_or("操作记录已变化")?;
        match result {
            Ok((backup, warning)) => {
                record.backup_name = Some(backup.into());
                record.warning = warning;
                record.phase = if restore {
                    Phase::Restored
                } else {
                    Phase::Applied
                };
            }
            Err(()) => record.phase = Phase::Failed,
        };
        self.write(items)
    }
}
fn validate(data: &Data) -> Result<(), String> {
    if data.schema_version != 1 || data.items.len() > 200 {
        return Err("操作记录格式或容量不受支持，历史保留".into());
    }
    let mut seen = std::collections::HashSet::new();
    for r in &data.items {
        if uuid::Uuid::parse_str(&r.operation_id).is_err()
            || !seen.insert(&r.operation_id)
            || uuid::Uuid::parse_str(&r.workspace_id).is_err()
            || r.workspace_revision > 9_007_199_254_740_991
            || r.profile_id.is_empty()
            || r.profile_id.len() > 160
            || r.profile_id.chars().any(char::is_control)
            || crate::private_text::known_private(&r.profile_id)
            || r.created_ms < 0
            || r.recovery_of
                .as_ref()
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_err())
            || r.backup_name
                .as_ref()
                .is_some_and(|name| crate::provider::parse_backup_name(name).is_none())
            || matches!(r.phase, Phase::Applied | Phase::Restored) != r.backup_name.is_some()
        {
            return Err("操作记录字段不受支持，原件保留".into());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn ctx() -> Context {
        Context {
            id: uuid::Uuid::new_v4().to_string(),
            expected_revision: 1,
            profile_id: "fixture-profile".into(),
            recovery_operation_id: None,
        }
    }
    #[test]
    fn record_failures_never_execute_or_disguise_a_successful_configuration_write() {
        let s = crate::testutil::Sandbox::new("workspace-journal-write-boundary");
        let path = s.path().join("journal.json");
        let target = s.path().join("fixture-config");
        let store = Store::new(path.clone());
        fs::write(&path, "unsupported").unwrap();
        assert!(store
            .execute(&ctx(), None, || {
                fs::write(&target, "must not happen").unwrap();
                unreachable!()
            })
            .is_err());
        assert!(!target.exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), "unsupported");
        fs::remove_file(&path).unwrap();
        let result = store
            .execute(&ctx(), None, || {
                fs::write(&target, "published config").unwrap();
                // An external edit prevents the final journal update after the actual write.
                fs::write(&path, "changed by external writer").unwrap();
                Ok(crate::provider::ProviderApplyResult {
                    config_path: "fixture-config".into(),
                    backup_name: "config-100.toml".into(),
                    record_warning: None,
                    limitations: "fixture",
                })
            })
            .unwrap();
        assert_eq!(fs::read_to_string(target).unwrap(), "published config");
        assert_eq!(result.result.backup_name, "config-100.toml");
        assert!(result.result.record_warning.unwrap().contains("勿重复应用"));
        assert!(result.operation_id.is_none());
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "changed by external writer"
        );
    }
    #[test]
    fn failed_mutation_records_failure_and_returns_the_original_error() {
        let s = crate::testutil::Sandbox::new("workspace-journal-mutation-fail");
        let store = Store::new(s.path().join("journal.json"));
        assert!(
            matches!(store.execute(&ctx(),None,||Err("source changed".into())),Err(error) if error=="source changed")
        );
        let items = store.list().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].phase, Phase::Failed);
        assert!(items[0].backup_name.is_none());
    }
    #[test]
    fn durable_pending_success_and_exact_restore_identity() {
        let s = crate::testutil::Sandbox::new("workspace-journal");
        let path = s.path().join("journal.json");
        let store = Store::new(path.clone());
        let mut c = ctx();
        let op = store.begin(&c, None).unwrap();
        assert_eq!(
            Store::new(path.clone()).list().unwrap()[0].phase,
            Phase::Pending
        );
        store
            .finish(&op, Ok(("config-100.toml", false)), false)
            .unwrap();
        assert!(store.finish(&op, Err(()), false).is_err());
        c.recovery_operation_id = Some(op);
        assert!(store.begin(&c, Some("config-101.toml")).is_err());
        let restore = store.begin(&c, Some("config-100.toml")).unwrap();
        store
            .finish(&restore, Ok(("config-102.toml", true)), true)
            .unwrap();
        let loaded = Store::new(path).list().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[1].phase, Phase::Restored);
        assert!(loaded[1].warning);
        c.id = uuid::Uuid::new_v4().to_string();
        assert!(store.begin(&c, Some("config-100.toml")).is_err());
    }
    #[test]
    fn malformed_future_and_capacity_preserve_bytes() {
        let s = crate::testutil::Sandbox::new("workspace-journal-bad");
        let path = s.path().join("journal.json");
        let store = Store::new(path.clone());
        for bytes in [
            b"broken".as_slice(),
            b"{\"schema_version\":99,\"items\":[]}",
        ] {
            fs::write(&path, bytes).unwrap();
            assert!(store.begin(&ctx(), None).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        let c = ctx();
        let records = (0..200)
            .map(|_| Record {
                operation_id: uuid::Uuid::new_v4().to_string(),
                workspace_id: c.id.clone(),
                workspace_revision: 1,
                profile_id: c.profile_id.clone(),
                recovery_of: None,
                created_ms: 1,
                phase: Phase::Pending,
                backup_name: None,
                warning: false,
            })
            .collect();
        store.write(records).unwrap();
        let before = fs::read(&path).unwrap();
        assert!(store.begin(&c, None).is_err());
        assert_eq!(fs::read(path).unwrap(), before);
    }
    #[cfg(unix)]
    #[test]
    fn linked_journal_cannot_write_external_file() {
        let s = crate::testutil::Sandbox::new("workspace-journal-link");
        let target = s.path().join("original");
        fs::write(&target, b"original").unwrap();
        let link = s.path().join("journal.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(Store::new(link).begin(&ctx(), None).is_err());
        assert_eq!(fs::read(target).unwrap(), b"original");
    }
}
