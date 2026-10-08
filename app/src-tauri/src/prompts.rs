//! Shared prompt library with isolated, explicitly previewed client guidance writes.
use crate::{
    atomicfile::{atomic_create_validated, atomic_replace_validated},
    private_text::known_private,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
const BODY_CAP: usize = 32 * 1024;
const FILE_CAP: usize = 64 * 1024;
const LIB_CAP: usize = 512 * 1024;
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub name: String,
    pub body: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prompt {
    pub id: String,
    pub draft: Draft,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct List {
    pub schema_version: u32,
    pub revision: u64,
    pub items: Vec<Prompt>,
}
impl Default for List {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            items: vec![],
        }
    }
}
#[derive(Serialize)]
pub struct Preview {
    #[serde(skip)]
    revision: String,
    pub plan_id: String,
    pub name: String,
    pub before: String,
    pub after: String,
    pub scope: &'static str,
}
#[derive(Serialize)]
pub struct Applied {
    pub backup_name: String,
    pub verified: bool,
    pub notice: &'static str,
}
#[derive(Serialize)]
pub struct Backup {
    pub name: String,
}
pub const SCOPE: &str =
    "Codex 用户级 AGENTS.md；项目规则可能覆盖，新会话加载后需核对。不会启动工具或修改当前会话。";
#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Client {
    #[default]
    Codex,
    Claude,
}

pub struct Store {
    library: PathBuf,
    target: PathBuf,
    backups: PathBuf,
}
fn read(path: &Path, cap: usize) -> Result<Option<String>, String> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("文件无法读取，原件已保留".into()),
    };
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > cap as u64 {
        return Err("文件类型或大小不受支持，原件已保留".into());
    }
    let mut bytes = vec![];
    fs::File::open(path)
        .map_err(|_| "文件无法打开")?
        .take((cap + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "文件无法读取")?;
    if bytes.len() > cap {
        return Err("文件超过读取上限".into());
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "文件不是 UTF-8，原件已保留".into())
}
fn safe(text: &str, cap: usize) -> Result<(), String> {
    if text.len() > cap
        || text
            .chars()
            .any(|c| c == '\0' || (c.is_control() && !matches!(c, '\n' | '\r' | '\t')))
        || known_private(text)
    {
        Err("内容超限或涉及已识别凭据，未保存或写入".into())
    } else {
        Ok(())
    }
}
fn validate(list: &List) -> Result<(), String> {
    if list.schema_version != 1 || list.revision > 9_007_199_254_740_991 || list.items.len() > 100 {
        return Err("提示词格式或容量不受支持，原件已保留".into());
    }
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for p in &list.items {
        if uuid::Uuid::parse_str(&p.id).is_err()
            || !ids.insert(&p.id)
            || !names.insert(&p.draft.name)
        {
            return Err("提示词身份或名称重复".into());
        }
        validate_draft(&p.draft)?;
    }
    Ok(())
}
fn validate_draft(d: &Draft) -> Result<(), String> {
    safe(&d.name, 160)?;
    safe(&d.body, BODY_CAP)?;
    if d.name.trim().is_empty()
        || d.name.trim() != d.name
        || d.name.chars().any(char::is_control)
        || d.body.trim().is_empty()
    {
        return Err("填写名称与提示词正文".into());
    }
    Ok(())
}
fn backup_name(name: &str) -> bool {
    name.strip_prefix("instructions-")
        .and_then(|s| s.strip_suffix(".md"))
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
}
impl Store {
    pub fn at_default() -> Self {
        Self {
            library: crate::settings::config_dir().join("prompts.v1.json"),
            target: crate::provider::codex_config_path()
                .unwrap_or_default()
                .with_file_name("AGENTS.md"),
            backups: crate::settings::config_dir().join("instruction-backups"),
        }
    }
    /// The UI selects a known client, never a filesystem path. Existing Codex backups stay put.
    pub fn for_client(&self, client: Client) -> Result<Self, String> {
        let (target, backups) = match client {
            Client::Codex => (self.target.clone(), self.backups.clone()),
            Client::Claude => {
                // Custom launch environments are not observable from this desktop process.
                if std::env::var_os("CLAUDE_CONFIG_DIR").is_some() {
                    return Err(
                        "Claude Code 使用自定义配置目录，当前仅支持默认用户目录；未改写指令".into(),
                    );
                }
                let home = dirs::home_dir().ok_or("用户目录不可用")?;
                (home.join(".claude/CLAUDE.md"), self.backups.join("claude"))
            }
        };
        Ok(Self {
            library: self.library.clone(),
            target,
            backups,
        })
    }
    fn scope(&self) -> &'static str {
        if self.target.file_name().is_some_and(|n| n == "CLAUDE.md") {
            "Claude Code 默认用户级 ~/.claude/CLAUDE.md；其他规则会共同加载，自定义配置目录不适用。请开启新会话并用 /context 核对。"
        } else {
            SCOPE
        }
    }
    pub fn list(&self) -> Result<List, String> {
        let Some(text) = read(&self.library, LIB_CAP)? else {
            return Ok(List::default());
        };
        let list: List =
            serde_json::from_str(&text).map_err(|_| "提示词库损坏或格式不支持，原件已保留")?;
        validate(&list)?;
        Ok(list)
    }
    fn write(&self, list: &List) -> Result<(), String> {
        validate(list)?;
        let bytes = serde_json::to_vec_pretty(list).map_err(|_| "提示词无法编码")?;
        if bytes.len() > LIB_CAP {
            return Err("提示词库达到容量上限，旧记录保留".into());
        }
        fs::create_dir_all(self.library.parent().ok_or("提示词目录不可用")?)
            .map_err(|_| "提示词目录无法创建")?;
        atomic_replace_validated(&self.library, &bytes, |p| {
            let v: List = serde_json::from_slice(&fs::read(p)?).map_err(std::io::Error::other)?;
            validate(&v).map_err(std::io::Error::other)
        })
        .map_err(|_| "提示词保存失败，原件已保留".into())
    }
    pub fn save(
        &self,
        id: Option<String>,
        mut draft: Draft,
        revision: u64,
    ) -> Result<List, String> {
        let mut list = self.list()?;
        if list.revision != revision {
            return Err("提示词库已变化，请刷新后重试".into());
        }
        draft.name = draft.name.trim().into();
        validate_draft(&draft)?;
        if let Some(id) = id {
            list.items
                .iter_mut()
                .find(|p| p.id == id)
                .ok_or("提示词已不存在")?
                .draft = draft;
        } else {
            list.items.push(Prompt {
                id: uuid::Uuid::new_v4().to_string(),
                draft,
            });
        }
        list.revision = list.revision.checked_add(1).ok_or("提示词版本已耗尽")?;
        self.write(&list)?;
        Ok(list)
    }
    pub fn remove(&self, id: &str, revision: u64) -> Result<List, String> {
        let mut list = self.list()?;
        if list.revision != revision {
            return Err("提示词库已变化，请刷新后重试".into());
        }
        let i = list
            .items
            .iter()
            .position(|p| p.id == id)
            .ok_or("提示词已不存在")?;
        list.items.remove(i);
        list.revision += 1;
        self.write(&list)?;
        Ok(list)
    }
    fn current(&self) -> Result<(String, String), String> {
        let claude = self.target.file_name().is_some_and(|n| n == "CLAUDE.md");
        if self
            .target
            .file_name()
            .is_none_or(|n| n != "AGENTS.md" && n != "CLAUDE.md")
            || self
                .target
                .parent()
                .is_none_or(|p| p.as_os_str().is_empty())
        {
            return Err("工具指令目录不可用".into());
        }
        let override_body = if claude {
            None
        } else {
            read(&self.target.with_file_name("AGENTS.override.md"), FILE_CAP)?
        };
        if override_body.as_ref().is_some_and(|s| !s.trim().is_empty()) {
            return Err("存在用户级 AGENTS.override.md，请先在原工具核对；未改写指令".into());
        }
        let body = read(&self.target, FILE_CAP)?;
        safe(body.as_deref().unwrap_or(""), FILE_CAP)?;
        let revision =
            hash(&serde_json::to_vec(&(&body, &override_body)).map_err(|_| "指令版本不可核实")?);
        Ok((body.unwrap_or_default(), revision))
    }
    fn plan(&self, name: String, after: String, identity: &str) -> Result<Preview, String> {
        safe(&after, FILE_CAP)?;
        let (before, revision) = self.current()?;
        if before == after {
            return Err("指令内容相同，无需写入".into());
        }
        let plan_id = hash(
            &serde_json::to_vec(&(&self.target, &revision, identity, &after))
                .map_err(|_| "预览无法生成")?,
        );
        Ok(Preview {
            revision,
            plan_id,
            name,
            before,
            after,
            scope: self.scope(),
        })
    }
    pub fn preview(&self, id: &str, revision: u64) -> Result<Preview, String> {
        let list = self.list()?;
        if list.revision != revision {
            return Err("提示词库已变化，请刷新后预览".into());
        }
        let prompt = list
            .items
            .into_iter()
            .find(|p| p.id == id)
            .ok_or("提示词已不存在")?;
        self.plan(
            prompt.draft.name,
            prompt.draft.body,
            &format!("prompt:{id}:{revision}"),
        )
    }
    fn publish(&self, plan: Preview) -> Result<Applied, String> {
        let (_, revision) = self.current()?;
        if revision != plan.revision {
            return Err("指令已变化，请重新预览".into());
        }
        fs::create_dir_all(&self.backups).map_err(|_| "备份目录无法创建，未写入指令")?;
        let backup_name = format!("instructions-{}.md", uuid::Uuid::new_v4());
        let backup = self.backups.join(&backup_name);
        atomic_create_validated(&backup, plan.before.as_bytes(), |p| {
            let body = fs::read_to_string(p)?;
            safe(&body, FILE_CAP).map_err(std::io::Error::other)
        })
        .map_err(|_| "备份失败，指令未改动")?;
        if self.current()?.1 != revision {
            return Err("指令或覆盖文件已变化，备份保留，请重新预览".into());
        }
        fs::create_dir_all(self.target.parent().ok_or("指令目录不可用")?)
            .map_err(|_| "指令目录无法创建，备份保留")?;
        atomic_replace_validated(&self.target, plan.after.as_bytes(), |p| {
            let body = fs::read_to_string(p)?;
            safe(&body, FILE_CAP).map_err(std::io::Error::other)
        })
        .map_err(|_| "指令写入失败，备份已保留")?;
        let verified = self.current().is_ok_and(|(body, _)| body == plan.after);
        Ok(Applied {
            backup_name,
            verified,
            notice: if verified {
                self.scope()
            } else {
                "指令已写入，读回或覆盖状态未确认；请在原工具核对，勿重复应用。"
            },
        })
    }
    pub fn apply(&self, id: &str, revision: u64, plan_id: &str) -> Result<Applied, String> {
        let plan = self.preview(id, revision)?;
        if plan.plan_id != plan_id {
            return Err("指令或提示词已变化，请重新预览".into());
        }
        self.publish(plan)
    }
    pub fn backups(&self) -> Result<Vec<Backup>, String> {
        let entries = match fs::read_dir(&self.backups) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(_) => return Err("指令备份目录不可读".into()),
        };
        let mut out = vec![];
        for (i, entry) in entries.take(201).enumerate() {
            if i == 200 {
                return Err("指令备份目录超过 200 项，未删除历史".into());
            }
            let entry = entry.map_err(|_| "备份读取不完整")?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if backup_name(&name) && entry.file_type().map_err(|_| "备份类型不可核实")?.is_file()
            {
                out.push(Backup { name });
            }
        }
        if out.len() > 200 {
            return Err("指令备份超过 200 项，未删除历史".into());
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
    pub fn preview_restore(&self, name: &str) -> Result<Preview, String> {
        if !backup_name(name) {
            return Err("备份名称不在支持范围".into());
        }
        let body = read(&self.backups.join(name), FILE_CAP)?.ok_or("备份已不存在")?;
        self.plan(name.into(), body, &format!("backup:{name}"))
    }
    pub fn restore(&self, name: &str, plan_id: &str) -> Result<Applied, String> {
        let plan = self.preview_restore(name)?;
        if plan.plan_id != plan_id {
            return Err("指令或备份已变化，请重新预览".into());
        }
        self.publish(plan)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (crate::testutil::Sandbox, Store) {
        let s = crate::testutil::Sandbox::new("prompts");
        let store = Store {
            library: s.path().join("prompts.json"),
            target: s.path().join(".codex/AGENTS.md"),
            backups: s.path().join("backups"),
        };
        (s, store)
    }
    fn draft() -> Draft {
        Draft {
            name: "Review".into(),
            body: "# Review\nExplain risks and test results.\n".into(),
        }
    }
    #[test]
    fn clients_share_library_but_never_plans_or_backups() {
        let (sandbox, codex) = fixture();
        let claude = Store {
            library: codex.library.clone(),
            target: sandbox.path().join(".claude/CLAUDE.md"),
            backups: codex.backups.join("claude"),
        };
        let list = codex.save(None, draft(), 0).unwrap();
        let id = &list.items[0].id;
        assert_eq!(claude.list().unwrap().revision, 1);
        let a = codex.preview(id, 1).unwrap();
        let b = claude.preview(id, 1).unwrap();
        assert_ne!(a.plan_id, b.plan_id);
        assert!(claude.apply(id, 1, &a.plan_id).is_err());
        assert!(!claude.target.exists());
        // A Codex override must have no effect on the Claude instruction protocol.
        fs::create_dir_all(claude.target.parent().unwrap()).unwrap();
        fs::write(
            claude.target.with_file_name("AGENTS.override.md"),
            "Other client",
        )
        .unwrap();
        let applied = claude.apply(id, 1, &b.plan_id).unwrap();
        assert!(applied.verified);
        assert!(!codex.target.exists());
        assert!(codex.backups().unwrap().is_empty());
        assert!(codex.preview_restore(&applied.backup_name).is_err());
        let restore = claude.preview_restore(&applied.backup_name).unwrap();
        fs::write(&claude.target, "External edit").unwrap();
        assert!(claude
            .restore(&applied.backup_name, &restore.plan_id)
            .is_err());
        assert_eq!(fs::read_to_string(&claude.target).unwrap(), "External edit");
        let restore = claude.preview_restore(&applied.backup_name).unwrap();
        assert!(
            claude
                .restore(&applied.backup_name, &restore.plan_id)
                .unwrap()
                .verified
        );
        assert_eq!(fs::read_to_string(&claude.target).unwrap(), "");
    }
    #[test]
    fn changed_library_or_backup_invalidates_the_exact_saved_preview() {
        let (_s, store) = fixture();
        let list = store.save(None, draft(), 0).unwrap();
        let id = &list.items[0].id;
        let plan = store.preview(id, 1).unwrap();
        let mut changed = draft();
        changed.body = "New saved guidance".into();
        store.save(Some(id.clone()), changed, 1).unwrap();
        assert!(store.apply(id, 1, &plan.plan_id).is_err());
        assert!(!store.target.exists());
        assert!(!store.backups.exists());
        let plan = store.preview(id, 2).unwrap();
        let result = store.apply(id, 2, &plan.plan_id).unwrap();
        let restore = store.preview_restore(&result.backup_name).unwrap();
        fs::write(store.backups.join(&result.backup_name), "Changed backup").unwrap();
        assert!(store
            .restore(&result.backup_name, &restore.plan_id)
            .is_err());
        assert_eq!(
            fs::read_to_string(&store.target).unwrap(),
            "New saved guidance"
        );
        assert_eq!(store.backups().unwrap().len(), 1);
    }

    #[test]
    fn library_crud_is_versioned_and_never_writes_client_guidance() {
        let (_s, store) = fixture();
        let list = store.save(None, draft(), 0).unwrap();
        assert!(!store.target.exists());
        let id = &list.items[0].id;
        assert!(store.save(Some(id.clone()), draft(), 0).is_err());
        let mut other = draft();
        other.name = "Second".into();
        let list = store.save(Some(id.clone()), other, 1).unwrap();
        assert_eq!(list.revision, 2);
        assert_eq!(store.remove(&list.items[0].id, 2).unwrap().items.len(), 0);
        assert!(!store.backups.exists());
        fs::write(
            &store.library,
            "{\"schema_version\":99,\"revision\":0,\"items\":[]}",
        )
        .unwrap();
        let before = fs::read(&store.library).unwrap();
        assert!(store.list().is_err());
        assert!(store.save(None, draft(), 0).is_err());
        assert_eq!(fs::read(store.library).unwrap(), before);
    }
    #[test]
    fn apply_restore_and_modified_sources_use_exact_preview_and_recoverable_bytes() {
        let (_s, store) = fixture();
        let list = store.save(None, draft(), 0).unwrap();
        let id = &list.items[0].id;
        let plan = store.preview(id, 1).unwrap();
        assert!(!store.target.exists());
        let applied = store.apply(id, 1, &plan.plan_id).unwrap();
        assert!(applied.verified);
        assert_eq!(fs::read_to_string(&store.target).unwrap(), draft().body);
        assert_eq!(
            fs::read_to_string(store.backups.join(&applied.backup_name)).unwrap(),
            ""
        );
        assert!(store.apply(id, 1, &plan.plan_id).is_err());
        let plan = store.preview_restore(&applied.backup_name).unwrap();
        let before = fs::read(&store.target).unwrap();
        let restored = store.restore(&applied.backup_name, &plan.plan_id).unwrap();
        assert_eq!(fs::read(&store.target).unwrap(), b"");
        assert_eq!(
            fs::read(store.backups.join(restored.backup_name)).unwrap(),
            before
        );
        assert_eq!(store.backups().unwrap().len(), 2);
        let stale = store.preview(id, 1).unwrap();
        fs::write(&store.target, "External guidance").unwrap();
        assert!(store.apply(id, 1, &stale.plan_id).is_err());
        assert_eq!(store.backups().unwrap().len(), 2);
        assert_eq!(
            fs::read_to_string(store.target).unwrap(),
            "External guidance"
        );
    }
    #[test]
    fn changes_between_plan_and_publication_and_global_override_block_all_writes() {
        let (_s, store) = fixture();
        let list = store.save(None, draft(), 0).unwrap();
        let id = &list.items[0].id;
        let plan = store.preview(id, 1).unwrap();
        fs::create_dir_all(store.target.parent().unwrap()).unwrap();
        fs::write(&store.target, "External").unwrap();
        assert!(store.publish(plan).is_err());
        assert!(!store.backups.exists());
        let plan = store.preview(id, 1).unwrap();
        fs::write(
            store.target.with_file_name("AGENTS.override.md"),
            "Override",
        )
        .unwrap();
        assert!(store.apply(id, 1, &plan.plan_id).is_err());
        assert!(!store.backups.exists());
        assert_eq!(fs::read_to_string(store.target).unwrap(), "External");
    }
    #[test]
    fn private_and_oversized_material_never_enters_library_or_backup() {
        let (_s, store) = fixture();
        let mut d = draft();
        d.body = format!("{}=fixture-value", "password");
        assert!(store.save(None, d, 0).is_err());
        assert!(!store.library.exists());
        let mut d = draft();
        d.body = "x".repeat(BODY_CAP + 1);
        assert!(store.save(None, d, 0).is_err());
        let list = store.save(None, draft(), 0).unwrap();
        fs::create_dir_all(store.target.parent().unwrap()).unwrap();
        let original = format!("{}=fixture-value", "password");
        fs::write(&store.target, &original).unwrap();
        assert!(store.preview(&list.items[0].id, 1).is_err());
        assert!(!store.backups.exists());
        assert_eq!(fs::read_to_string(store.target).unwrap(), original);
    }
    #[cfg(unix)]
    #[test]
    fn linked_guidance_and_backup_are_not_followed() {
        let (s, store) = fixture();
        let list = store.save(None, draft(), 0).unwrap();
        let other = s.path().join("external.md");
        fs::write(&other, "Keep").unwrap();
        fs::create_dir_all(store.target.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&other, &store.target).unwrap();
        assert!(store.preview(&list.items[0].id, 1).is_err());
        fs::create_dir_all(&store.backups).unwrap();
        let name = format!("instructions-{}.md", uuid::Uuid::new_v4());
        std::os::unix::fs::symlink(&other, store.backups.join(&name)).unwrap();
        assert!(store.preview_restore(&name).is_err());
        assert_eq!(fs::read_to_string(other).unwrap(), "Keep");
    }

    #[cfg(target_os = "macos")]
    mod real_clients {
        use super::*;
        use serde_json::json;
        use std::process::Command;

        const ORIGINAL: &str = "AGENTISLAND_FIXTURE_GUIDANCE_ORIGINAL";
        const FIRST: &str = "AGENTISLAND_FIXTURE_GUIDANCE_FIRST";
        const SECOND: &str = "AGENTISLAND_FIXTURE_GUIDANCE_SECOND";
        const OVERRIDE: &str = "AGENTISLAND_FIXTURE_GUIDANCE_OVERRIDE";

        fn exercise(kind: &str) {
            let prefix = format!("AGENTISLAND_{}_CLIENT", kind.to_uppercase());
            let client = std::env::var_os(&prefix).expect("set an explicit signed client binary");
            let version = std::env::var(format!("{prefix}_VERSION"))
                .expect("pin the actual client version for this acceptance");
            let sandbox = crate::testutil::Sandbox::new("instructions-client");
            let home = sandbox.path();
            fs::write(home.join(".agentisland-client-fixture"), "fixture-only\n").unwrap();
            let store = Store {
                library: home.join("prompts.json"),
                target: home.join(if kind == "codex" {
                    ".codex/AGENTS.md"
                } else {
                    ".claude/CLAUDE.md"
                }),
                backups: home.join("backups"),
            };
            fs::create_dir_all(store.target.parent().unwrap()).unwrap();
            fs::write(&store.target, ORIGINAL).unwrap();
            let verify = |stage: &str, marker: &str, reject: bool| {
                let expected = home.join("expected.json");
                let absent: Vec<_> = [ORIGINAL, FIRST, SECOND, OVERRIDE]
                    .into_iter()
                    .filter(|candidate| *candidate != marker)
                    .collect();
                fs::write(
                    &expected,
                    serde_json::to_vec(&json!({"marker":marker,"absent":absent})).unwrap(),
                )
                .unwrap();
                let output = Command::new("/usr/bin/python3")
                    .arg(
                        Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("../../scripts/check-client-instructions.py"),
                    )
                    .arg("--kind")
                    .arg(kind)
                    .arg("--client")
                    .arg(&client)
                    .arg("--version")
                    .arg(&version)
                    .arg("--fixture")
                    .arg(home)
                    .arg("--expected")
                    .arg(expected)
                    .output()
                    .unwrap();
                if reject {
                    assert!(
                        !output.status.success(),
                        "wrong guidance must fail acceptance"
                    );
                    assert_eq!(
                        String::from_utf8_lossy(&output.stderr).trim(),
                        "FAIL: client guidance differs from the applied version"
                    );
                    println!("{stage}: PASS expected guidance mismatch was detected");
                } else {
                    assert!(
                        output.status.success(),
                        "{stage}: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                    assert_eq!(result["passed"], true);
                    assert_eq!(result["kind"], kind);
                    assert_eq!(result["version"], version);
                    assert_eq!(result["isolation_verified"], true);
                    assert_eq!(result["raw_output_saved"], false);
                    assert_eq!(result["client_sha256"].as_str().unwrap().len(), 64);
                    println!(
                        "{stage}: {}",
                        String::from_utf8_lossy(&output.stdout).trim()
                    );
                }
            };
            let list = store
                .save(
                    None,
                    Draft {
                        name: "Fixture guidance".into(),
                        body: FIRST.into(),
                    },
                    0,
                )
                .unwrap();
            let id = list.items[0].id.clone();
            let plan = store.preview(&id, list.revision).unwrap();
            verify(
                "save and preview preserve original guidance",
                ORIGINAL,
                false,
            );
            verify("incorrect expected guidance is rejected", FIRST, true);
            let first = store.apply(&id, list.revision, &plan.plan_id).unwrap();
            assert!(first.verified);
            verify("first application reaches a new client", FIRST, false);
            let list = store
                .save(
                    Some(id.clone()),
                    Draft {
                        name: "Fixture guidance".into(),
                        body: SECOND.into(),
                    },
                    list.revision,
                )
                .unwrap();
            verify(
                "saved update alone does not change client guidance",
                FIRST,
                false,
            );
            let plan = store.preview(&id, list.revision).unwrap();
            let second = store.apply(&id, list.revision, &plan.plan_id).unwrap();
            assert!(second.verified);
            verify("second application excludes old guidance", SECOND, false);
            // Recreate the Store to exercise persisted library and backup state.
            let store = Store {
                library: store.library.clone(),
                target: store.target.clone(),
                backups: store.backups.clone(),
            };
            assert_eq!(store.list().unwrap().revision, list.revision);
            let plan = store.preview_restore(&second.backup_name).unwrap();
            assert!(
                store
                    .restore(&second.backup_name, &plan.plan_id)
                    .unwrap()
                    .verified
            );
            verify("persisted backup restores first guidance", FIRST, false);
            let plan = store.preview_restore(&first.backup_name).unwrap();
            assert!(
                store
                    .restore(&first.backup_name, &plan.plan_id)
                    .unwrap()
                    .verified
            );
            verify(
                "original guidance reaches the new client again",
                ORIGINAL,
                false,
            );
            if kind == "codex" {
                fs::write(store.target.with_file_name("AGENTS.override.md"), OVERRIDE).unwrap();
                assert!(store.preview(&id, list.revision).is_err());
                assert_eq!(fs::read_to_string(&store.target).unwrap(), ORIGINAL);
                verify(
                    "global override loads while application is blocked",
                    OVERRIDE,
                    false,
                );
            }
        }

        #[test]
        #[ignore = "explicit Codex binary/version and macOS isolated loopback acceptance"]
        fn real_codex_client_loads_applied_and_restored_guidance() {
            exercise("codex");
        }

        #[test]
        #[ignore = "explicit Claude binary/version and macOS isolated loopback acceptance"]
        fn real_claude_client_loads_applied_and_restored_guidance() {
            exercise("claude");
        }
    }
}
