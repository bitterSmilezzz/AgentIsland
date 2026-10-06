//! Local reference compositions. Saving/opening never executes tools or changes configuration.
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io::Read, path::PathBuf};
const MAX_BYTES: u64 = 256 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub name: String,
    pub project_id: Option<String>,
    pub profile_id: Option<String>,
    pub layout_id: Option<String>,
    pub tools: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workspace {
    pub id: String,
    pub draft: Draft,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct List {
    pub schema_version: u32,
    pub revision: u64,
    pub items: Vec<Workspace>,
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
#[derive(Clone, Serialize)]
pub struct Choice {
    pub id: String,
    pub name: String,
    pub supported: bool,
}
#[derive(Serialize)]
pub struct Catalog {
    pub projects: Vec<Choice>,
    pub profiles: Vec<Choice>,
    pub layouts: Vec<Choice>,
    pub tools: Vec<Choice>,
    pub errors: Vec<String>,
}
#[derive(Serialize)]
pub struct Step {
    pub kind: String,
    pub target_id: String,
    pub name: String,
    pub available: bool,
    pub reason: Option<String>,
}
#[derive(Serialize)]
pub struct Preview {
    pub workspace: Workspace,
    pub revision: u64,
    pub steps: Vec<Step>,
}
pub struct Store {
    path: PathBuf,
}
impl Store {
    pub fn check_profile_context(
        &self,
        ctx: &crate::workspace_journal::Context,
    ) -> Result<(), String> {
        let list = self.list()?;
        if list.revision != ctx.expected_revision
            || !list
                .items
                .iter()
                .any(|w| w.id == ctx.id && w.draft.profile_id.as_deref() == Some(&ctx.profile_id))
        {
            return Err("工作空间或档位引用已变化，请重新核对".into());
        }
        Ok(())
    }
    pub fn check_layout_context(
        &self,
        ctx: &crate::window_layout_journal::WorkspaceContext,
    ) -> Result<(), String> {
        let list = self.list()?;
        if list.revision != ctx.expected_revision
            || !list
                .items
                .iter()
                .any(|w| w.id == ctx.id && w.draft.layout_id.as_deref() == Some(&ctx.layout_id))
        {
            return Err("工作空间或布局引用已变化，请重新核对".into());
        }
        Ok(())
    }
    pub fn journal(&self) -> crate::workspace_journal::Store {
        crate::workspace_journal::Store::new(
            self.path.with_file_name("workspace-operations.v1.json"),
        )
    }
    pub fn at_default() -> Self {
        Self {
            path: crate::settings::config_dir().join("workspaces.v1.json"),
        }
    }
    pub fn list(&self) -> Result<List, String> {
        let meta = match fs::symlink_metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(List::default()),
            Err(_) => return Err("工作空间无法读取，原文件已保留".into()),
        };
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_BYTES {
            return Err("工作空间文件类型或大小不受支持，原文件已保留".into());
        }
        let mut bytes = vec![];
        fs::File::open(&self.path)
            .map_err(|_| "工作空间无法打开")?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "工作空间无法读取")?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("工作空间过大，原文件已保留".into());
        }
        let list: List =
            serde_json::from_slice(&bytes).map_err(|_| "工作空间损坏或格式不支持，原文件已保留")?;
        validate(&list)?;
        Ok(list)
    }
    fn write(&self, list: &List) -> Result<(), String> {
        validate(list)?;
        let bytes = serde_json::to_vec_pretty(list).map_err(|_| "工作空间无法编码")?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("工作空间达到容量上限，未删除旧记录".into());
        }
        fs::create_dir_all(self.path.parent().ok_or("工作空间目录不可用")?)
            .map_err(|_| "工作空间目录无法创建")?;
        crate::atomicfile::atomic_replace_validated(&self.path, &bytes, |p| {
            let d: List = serde_json::from_slice(&fs::read(p)?).map_err(std::io::Error::other)?;
            validate(&d).map_err(std::io::Error::other)
        })
        .map_err(|_| "工作空间保存失败，原文件已保留".into())
    }
    pub fn save(
        &self,
        id: Option<String>,
        mut draft: Draft,
        revision: u64,
        catalog: &Catalog,
    ) -> Result<List, String> {
        let mut list = self.list()?;
        if list.revision != revision {
            return Err("工作空间已变化，请刷新后重试".into());
        }
        draft.name = draft.name.trim().into();
        validate_draft(&draft)?;
        if steps(&draft, catalog).iter().any(|s| !s.available) {
            return Err("所选引用已失效或不支持，请重新选择".into());
        }
        match id {
            Some(id) => {
                let item = list
                    .items
                    .iter_mut()
                    .find(|w| w.id == id)
                    .ok_or("工作空间已不存在")?;
                item.draft = draft;
            }
            None => list.items.push(Workspace {
                id: uuid::Uuid::new_v4().to_string(),
                draft,
            }),
        }
        list.revision = list
            .revision
            .checked_add(1)
            .filter(|r| *r <= 9_007_199_254_740_991)
            .ok_or("工作空间版本已耗尽")?;
        self.write(&list)?;
        Ok(list)
    }
    pub fn remove(&self, id: &str, revision: u64) -> Result<List, String> {
        let mut list = self.list()?;
        if list.revision != revision {
            return Err("工作空间已变化，请刷新后重试".into());
        }
        let i = list
            .items
            .iter()
            .position(|w| w.id == id)
            .ok_or("工作空间已不存在")?;
        list.items.remove(i);
        list.revision = list.revision.checked_add(1).ok_or("工作空间版本已耗尽")?;
        self.write(&list)?;
        Ok(list)
    }
    pub fn preview(&self, id: &str, revision: u64, catalog: &Catalog) -> Result<Preview, String> {
        let list = self.list()?;
        if list.revision != revision {
            return Err("工作空间已变化，请刷新后重试".into());
        }
        let workspace = list
            .items
            .into_iter()
            .find(|w| w.id == id)
            .ok_or("工作空间已不存在")?;
        let steps = steps(&workspace.draft, catalog);
        Ok(Preview {
            workspace,
            revision,
            steps,
        })
    }
}
fn validate_draft(d: &Draft) -> Result<(), String> {
    if d.name.is_empty()
        || d.name.trim() != d.name
        || d.name.chars().count() > 80
        || d.name.chars().any(char::is_control)
        || d.name.contains("-----BEGIN")
        || d.name
            .split_whitespace()
            .any(|s| s.starts_with("sk-") && s.len() > 20)
    {
        return Err("名称需为 1–80 个字符且不能包含凭据".into());
    }
    for id in [&d.project_id, &d.layout_id].into_iter().flatten() {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err("项目或布局引用无效".into());
        }
    }
    if d.profile_id.as_ref().is_some_and(|id| {
        id.is_empty()
            || id.len() > 80
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    }) {
        return Err("档位引用无效".into());
    }
    let builtins = crate::registry::builtin();
    let mut seen = HashSet::new();
    if d.tools.len() > 16
        || d.tools
            .iter()
            .any(|t| !seen.insert(t) || !builtins.iter().any(|p| p.id == *t))
    {
        return Err("工具引用无效或重复".into());
    }
    if d.project_id.is_none()
        && d.profile_id.is_none()
        && d.layout_id.is_none()
        && d.tools.is_empty()
    {
        return Err("至少选择一个项目、工具、档位或布局".into());
    }
    Ok(())
}
fn validate(list: &List) -> Result<(), String> {
    if list.schema_version != 1 || list.revision > 9_007_199_254_740_991 || list.items.len() > 50 {
        return Err("工作空间版本或容量不受支持，原文件已保留".into());
    }
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for w in &list.items {
        if uuid::Uuid::parse_str(&w.id).is_err()
            || !ids.insert(&w.id)
            || !names.insert(&w.draft.name)
        {
            return Err("工作空间身份或名称重复，原文件已保留".into());
        }
        validate_draft(&w.draft)?;
    }
    Ok(())
}
fn steps(d: &Draft, c: &Catalog) -> Vec<Step> {
    let mut steps = vec![];
    let refs = [
        ("project", d.project_id.as_ref(), &c.projects),
        ("profile", d.profile_id.as_ref(), &c.profiles),
        ("layout", d.layout_id.as_ref(), &c.layouts),
    ];
    for (kind, id, choices) in refs {
        if let Some(id) = id {
            steps.push(step(kind, id, choices));
        }
    }
    for tool in &d.tools {
        steps.push(step("tool", tool, &c.tools));
    }
    steps
}
fn step(kind: &str, id: &str, choices: &[Choice]) -> Step {
    let mut matches = choices.iter().filter(|c| c.id == id);
    let choice = matches.next();
    let ambiguous = matches.next().is_some();
    let available = !ambiguous && choice.is_some_and(|c| c.supported);
    Step {
        kind: kind.into(),
        target_id: id.into(),
        name: if ambiguous {
            "引用身份不明确".into()
        } else {
            choice
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "引用不可用".into())
        },
        available,
        reason: if available {
            None
        } else {
            Some(
                if ambiguous {
                    "来源含重复身份，请在原模块整理"
                } else if choice.is_some() {
                    "当前能力不支持，请在原模块核对"
                } else {
                    "引用已删除或来源无法读取，请重新选择"
                }
                .into(),
            )
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (crate::testutil::Sandbox, Store, Catalog, Draft) {
        let s = crate::testutil::Sandbox::new("workspace");
        let store = Store {
            path: s.path().join("workspaces.json"),
        };
        let p = uuid::Uuid::new_v4().to_string();
        let catalog = Catalog {
            projects: vec![Choice {
                id: p.clone(),
                name: "项目".into(),
                supported: true,
            }],
            profiles: vec![],
            layouts: vec![],
            tools: vec![],
            errors: vec![],
        };
        let d = Draft {
            name: "工作".into(),
            project_id: Some(p),
            profile_id: None,
            layout_id: None,
            tools: vec![],
        };
        (s, store, catalog, d)
    }
    #[test]
    fn configuration_scope_rechecks_workspace_revision_identity_and_profile_before_journaling() {
        let (_s, store, mut catalog, mut draft) = fixture();
        catalog.profiles.push(Choice {
            id: "fixture-profile".into(),
            name: "Fixture".into(),
            supported: true,
        });
        draft.profile_id = Some("fixture-profile".into());
        let list = store.save(None, draft, 0, &catalog).unwrap();
        let mut ctx = crate::workspace_journal::Context {
            id: list.items[0].id.clone(),
            expected_revision: list.revision,
            profile_id: "fixture-profile".into(),
            recovery_operation_id: None,
        };
        assert!(store.check_profile_context(&ctx).is_ok());
        ctx.profile_id = "wrong-profile".into();
        assert!(store.check_profile_context(&ctx).is_err());
        ctx.profile_id = "fixture-profile".into();
        ctx.expected_revision = 0;
        assert!(store.check_profile_context(&ctx).is_err());
        ctx.expected_revision = list.revision;
        let id = ctx.id.clone();
        ctx.id = uuid::Uuid::new_v4().to_string();
        assert!(store.check_profile_context(&ctx).is_err());
        ctx.id = id;
        store.remove(&ctx.id, list.revision).unwrap();
        assert!(store.check_profile_context(&ctx).is_err());
        assert!(store.journal().list().unwrap().is_empty());
    }
    #[test]
    fn roundtrip_edit_remove_and_stale_revision_leave_references_intact() {
        let (_s, store, c, mut d) = fixture();
        let l = store.save(None, d.clone(), 0, &c).unwrap();
        let bytes = fs::read(&store.path).unwrap();
        assert!(store.save(None, d.clone(), 0, &c).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        let preview = store.preview(&l.items[0].id, 1, &c).unwrap();
        assert!(preview.steps[0].available);
        assert_eq!(fs::read(&store.path).unwrap(), bytes);
        d.name = "另一空间".into();
        let l = store.save(Some(l.items[0].id.clone()), d, 1, &c).unwrap();
        assert_eq!(l.items[0].draft.name, "另一空间");
        assert_eq!(store.remove(&l.items[0].id, 2).unwrap().items.len(), 0);
    }
    #[test]
    fn deleted_unreadable_and_unsupported_references_never_substitute_latest() {
        let (_s, store, mut c, d) = fixture();
        let l = store.save(None, d.clone(), 0, &c).unwrap();
        c.projects[0].id = uuid::Uuid::new_v4().to_string();
        let p = store.preview(&l.items[0].id, 1, &c).unwrap();
        assert!(!p.steps[0].available);
        assert_eq!(p.steps[0].target_id, d.project_id.unwrap());
        assert!(store.save(None, l.items[0].draft.clone(), 1, &c).is_err());
        c.errors.push("fixture".into());
        assert!(store.save(None, l.items[0].draft.clone(), 1, &c).is_err());
    }
    #[test]
    fn invalid_unknown_corrupt_and_linked_files_are_preserved() {
        let (s, store, c, d) = fixture();
        for body in [
            "broken",
            r#"{"schema_version":2,"revision":0,"items":[]}"#,
            r#"{"schema_version":1,"revision":0,"items":[],"credential":"fixture"}"#,
        ] {
            fs::write(&store.path, body).unwrap();
            assert!(store.save(None, d.clone(), 0, &c).is_err());
            assert_eq!(fs::read_to_string(&store.path).unwrap(), body);
        }
        fs::remove_file(&store.path).unwrap();
        let mut bad = d.clone();
        bad.tools = vec!["codex".into(), "codex".into()];
        assert!(store.save(None, bad, 0, &c).is_err());
        assert!(!store.path.exists());
        #[cfg(unix)]
        {
            let other = s.path().join("other");
            fs::write(&other, "{}").unwrap();
            std::os::unix::fs::symlink(&other, &store.path).unwrap();
            assert!(store.save(None, d, 0, &c).is_err());
            assert_eq!(fs::read_to_string(other).unwrap(), "{}");
        }
    }
    #[test]
    fn unrelated_source_failure_is_visible_but_does_not_block_valid_selection() {
        let (_s, store, mut c, d) = fixture();
        c.errors.push("布局来源不可读".into());
        let l = store.save(None, d.clone(), 0, &c).unwrap();
        assert!(store.preview(&l.items[0].id, 1, &c).unwrap().steps[0].available);
        c.projects.push(c.projects[0].clone());
        let before = fs::read(&store.path).unwrap();
        assert!(!store.preview(&l.items[0].id, 1, &c).unwrap().steps[0].available);
        assert!(store.save(Some(l.items[0].id.clone()), d, 1, &c).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), before);
    }
    #[test]
    fn profile_catalog_is_bounded_metadata_and_legacy_protocol_is_not_available() {
        let s = crate::testutil::Sandbox::new("workspace-profiles");
        let p = s.path().join("profiles.json");
        let store = crate::provider::ProviderStore::new(s.path().into());
        assert!(store.workspace_choices().unwrap().is_empty());
        let profile = serde_json::json!({"id":"one","name":"日常","model":"fixture-model","provider_id":"fixture-provider","provider_name":"Fixture","base_url":"https://example.invalid/v1","env_key":"FIXTURE_AUTH","wire_api":"chat"});
        fs::write(&p, serde_json::to_vec(&vec![profile]).unwrap()).unwrap();
        let choices = store.workspace_choices().unwrap();
        assert!(!choices[0].supported);
        let serialized = serde_json::to_string(&choices).unwrap();
        assert!(
            !serialized.contains("base_url")
                && !serialized.contains("env_key")
                && !serialized.contains("example.invalid")
        );
        fs::write(&p, vec![b' '; 1024 * 1024 + 1]).unwrap();
        assert!(store.workspace_choices().is_err());
        fs::write(&p, "bad").unwrap();
        assert!(store.workspace_choices().is_err());
        assert_eq!(fs::read_to_string(&p).unwrap(), "bad");
        #[cfg(unix)]
        {
            fs::remove_file(&p).unwrap();
            let other = s.path().join("other");
            fs::write(&other, "[]").unwrap();
            std::os::unix::fs::symlink(other, &p).unwrap();
            assert!(store.workspace_choices().is_err());
        }
    }
    #[test]
    fn layout_context_requires_current_workspace_and_exact_layout_reference() {
        let sandbox = crate::testutil::Sandbox::new("workspace-layout-context");
        let store = Store {
            path: sandbox.path().join("workspaces.json"),
        };
        let layout = uuid::Uuid::new_v4().to_string();
        let id = uuid::Uuid::new_v4().to_string();
        let draft = Draft {
            name: "fixture".into(),
            project_id: None,
            profile_id: None,
            layout_id: Some(layout.clone()),
            tools: vec![],
        };
        store
            .write(&List {
                schema_version: 1,
                revision: 3,
                items: vec![Workspace {
                    id: id.clone(),
                    draft,
                }],
            })
            .unwrap();
        let mut ctx = crate::window_layout_journal::WorkspaceContext {
            id,
            expected_revision: 3,
            layout_id: layout,
            expected_rules_revision: 1,
        };
        assert!(store.check_layout_context(&ctx).is_ok());
        ctx.expected_revision = 2;
        assert!(store.check_layout_context(&ctx).is_err());
        ctx.expected_revision = 3;
        ctx.layout_id = uuid::Uuid::new_v4().to_string();
        assert!(store.check_layout_context(&ctx).is_err());
    }
}
