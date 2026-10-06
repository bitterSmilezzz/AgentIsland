//! Persistent layout preferences. No window handles, titles, rectangles or screen fingerprints.
use crate::window_layout::{Template, WindowCandidate};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io::Read, path::PathBuf};
const MAX_BYTES: u64 = 256 * 1024;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenPreference {
    Primary,
    Choose,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub tools: Vec<String>,
    pub template: Template,
    pub gap: f64,
    pub screen_preference: ScreenPreference,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleList {
    pub schema_version: u32,
    pub revision: u64,
    pub items: Vec<Rule>,
}
impl Default for RuleList {
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
            path: crate::settings::config_dir().join("window-layouts.v1.json"),
        }
    }
    fn load(&self) -> Result<RuleList, String> {
        let meta = match fs::symlink_metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(RuleList::default()),
            Err(_) => return Err("布局规则无法读取".into()),
        };
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_BYTES {
            return Err("布局规则文件类型或大小无效，原文件已保留".into());
        }
        let mut bytes = Vec::new();
        fs::File::open(&self.path)
            .map_err(|_| "布局规则无法打开")?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "布局规则无法读取")?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("布局规则文件过大，原文件已保留".into());
        }
        let data: RuleList =
            serde_json::from_slice(&bytes).map_err(|_| "布局规则损坏或格式不支持，原文件已保留")?;
        validate(&data)?;
        Ok(data)
    }
    pub fn list(&self) -> Result<RuleList, String> {
        self.load()
    }
    fn write(&self, data: &RuleList) -> Result<(), String> {
        validate(data)?;
        let bytes = serde_json::to_vec_pretty(data).map_err(|_| "布局规则无法编码")?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("布局规则过大，未保存".into());
        }
        fs::create_dir_all(self.path.parent().ok_or("布局规则目录无效")?)
            .map_err(|_| "布局规则目录无法创建")?;
        crate::atomicfile::atomic_replace_validated(&self.path, &bytes, |staged| {
            let bytes = fs::read(staged)?;
            let data: RuleList =
                serde_json::from_slice(&bytes).map_err(|_| std::io::Error::other("layout JSON"))?;
            validate(&data).map_err(|_| std::io::Error::other("invalid layout rules"))
        })
        .map_err(|_| "布局规则保存失败，原文件已保留".to_owned())
    }
    pub fn save(
        &self,
        name: String,
        tools: Vec<String>,
        template: Template,
        gap: f64,
        screen_preference: ScreenPreference,
        expected_revision: u64,
    ) -> Result<RuleList, String> {
        let mut data = self.load()?;
        if data.revision != expected_revision {
            return Err("布局规则已变化，请重新读取后保存".into());
        }
        let name = name.trim().to_owned();
        if data.items.iter().any(|r| r.name == name) {
            return Err("已有同名规则，请换一个名称".into());
        }
        data.items.push(Rule {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            tools,
            template,
            gap,
            screen_preference,
        });
        data.revision = data.revision.checked_add(1).ok_or("布局规则版本已耗尽")?;
        self.write(&data)?;
        Ok(data)
    }
    pub fn remove(&self, id: &str, expected_revision: u64) -> Result<RuleList, String> {
        let mut data = self.load()?;
        if data.revision != expected_revision {
            return Err("布局规则已变化，请重新读取".into());
        }
        let i = data
            .items
            .iter()
            .position(|r| r.id == id)
            .ok_or("布局规则已不存在")?;
        data.items.remove(i);
        data.revision = data.revision.checked_add(1).ok_or("布局规则版本已耗尽")?;
        self.write(&data)?;
        Ok(data)
    }
    pub fn get(&self, id: &str, expected_revision: u64) -> Result<Rule, String> {
        let data = self.load()?;
        if data.revision != expected_revision {
            return Err("布局规则已变化，请重新读取".into());
        }
        data.items
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| "布局规则已不存在".into())
    }
}
fn validate(data: &RuleList) -> Result<(), String> {
    if data.schema_version != 1 {
        return Err("布局规则版本不受支持，原文件已保留".into());
    }
    if data.items.len() > 50 {
        return Err("最多保存 50 个布局规则".into());
    }
    let profiles = crate::registry::builtin();
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for r in &data.items {
        if uuid::Uuid::parse_str(&r.id).is_err() || !ids.insert(&r.id) {
            return Err("布局规则身份无效，原文件已保留".into());
        }
        if r.name.trim() != r.name
            || r.name.is_empty()
            || r.name.chars().count() > 80
            || r.name.chars().any(char::is_control)
            || !names.insert(&r.name)
        {
            return Err("规则名称应为 1–80 个字符且不能重复".into());
        }
        if r.tools.is_empty()
            || r.tools.len() > 16
            || r.tools.iter().any(|id| {
                !profiles
                    .iter()
                    .any(|p| &p.id == id && !p.bundle_ids.is_empty())
            })
        {
            return Err("规则工具不受支持，请重新选择窗口".into());
        }
        if !r.gap.is_finite() || !(0.0..=64.0).contains(&r.gap) {
            return Err("窗口间距应在 0–64 点之间".into());
        }
        match r.template {
            Template::SideBySide if r.tools.len() != 2 => {
                return Err("左右并排需要 2 个窗口".into())
            }
            Template::MainAndTwo if r.tools.len() != 3 => {
                return Err("主辅排列需要 3 个窗口".into())
            }
            _ => {}
        }
    }
    Ok(())
}
#[derive(Serialize)]
pub struct Resolved {
    pub rule: Rule,
    pub selection: Vec<String>,
    pub needs_selection: bool,
    pub reason: Option<String>,
}
pub fn resolve(rule: Rule, windows: &[WindowCandidate]) -> Resolved {
    let mut selection = Vec::new();
    let mut seen = HashSet::new();
    let mut reason = None;
    let profiles = crate::registry::builtin();
    for tool in &rule.tools {
        let candidates: Vec<_> = windows.iter().filter(|w| &w.agent_id == tool).collect();
        let name = profiles
            .iter()
            .find(|p| &p.id == tool)
            .map(|p| p.name.as_str())
            .unwrap_or("工具");
        if !seen.insert(tool) || candidates.len() != 1 {
            reason = Some(format!("{name} 窗口缺失或有多个候选，请重新选择"));
            break;
        }
        if candidates[0].restriction.is_some() {
            reason = Some(format!("{name} 窗口暂不可调整，请检查限制"));
            break;
        }
        selection.push(candidates[0].window_id.clone());
    }
    let needs_selection = reason.is_some();
    if needs_selection {
        selection.clear();
    }
    Resolved {
        rule,
        selection,
        needs_selection,
        reason,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> Store {
        Store {
            path: std::env::temp_dir().join(format!("layout-rules-{}.json", uuid::Uuid::new_v4())),
        }
    }
    fn save(s: &Store, rev: u64) -> Result<RuleList, String> {
        s.save(
            "工作布局".into(),
            vec!["codex".into()],
            Template::Grid,
            12.0,
            ScreenPreference::Choose,
            rev,
        )
    }
    #[test]
    fn roundtrip_has_only_preferences_and_revision_conflicts_do_not_write() {
        let s = store();
        let list = save(&s, 0).unwrap();
        let before = fs::read(&s.path).unwrap();
        assert!(save(&s, 0).is_err());
        assert_eq!(fs::read(&s.path).unwrap(), before);
        let text = String::from_utf8(before).unwrap();
        for field in ["window_id", "title", "rect", "screen_id", "pid"] {
            assert!(!text.contains(field));
        }
        let r = s.get(&list.items[0].id, 1).unwrap();
        assert!(matches!(r.screen_preference, ScreenPreference::Choose));
        assert_eq!(s.remove(&r.id, 1).unwrap().revision, 2);
        fs::remove_file(s.path).unwrap();
    }
    #[test]
    fn corrupt_future_unknown_and_invalid_rules_are_preserved() {
        let s = store();
        for bytes in [
            "broken",
            r#"{"schema_version":2,"revision":0,"items":[]}"#,
            r#"{"schema_version":1,"revision":0,"items":[],"unknown":"fixture"}"#,
        ] {
            fs::write(&s.path, bytes).unwrap();
            assert!(save(&s, 0).is_err());
            assert_eq!(fs::read_to_string(&s.path).unwrap(), bytes);
        }
        fs::remove_file(&s.path).unwrap();
        assert!(s
            .save(
                "".into(),
                vec!["codex".into()],
                Template::Grid,
                12.0,
                ScreenPreference::Primary,
                0
            )
            .is_err());
        assert!(!s.path.exists());
    }
    #[test]
    fn oversized_and_symlink_files_are_not_replaced() {
        let s = store();
        let bytes = vec![b'x'; MAX_BYTES as usize + 1];
        fs::write(&s.path, &bytes).unwrap();
        assert!(save(&s, 0).is_err());
        assert_eq!(fs::metadata(&s.path).unwrap().len(), MAX_BYTES + 1);
        fs::remove_file(&s.path).unwrap();
        #[cfg(unix)]
        {
            let target = s.path.with_extension("target");
            fs::write(&target, "original").unwrap();
            std::os::unix::fs::symlink(&target, &s.path).unwrap();
            assert!(save(&s, 0).is_err());
            assert!(fs::symlink_metadata(&s.path)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(fs::read_to_string(&target).unwrap(), "original");
            fs::remove_file(&s.path).unwrap();
            fs::remove_file(target).unwrap();
        }
    }
    #[test]
    fn resolution_never_substitutes_an_ambiguous_or_newest_window() {
        let s = store();
        let mut r = save(&s, 0).unwrap().items.remove(0);
        let w = WindowCandidate {
            window_id: "fresh-id".into(),
            agent_id: "codex".into(),
            application: "Tool".into(),
            title: "private title".into(),
            screen_id: None,
            rect: crate::window_layout::Rect {
                x: 0.0,
                y: 0.0,
                width: 500.0,
                height: 700.0,
            },
            movable: true,
            resizable: true,
            restriction: None,
            minimum_size: None,
        };
        assert_eq!(resolve(r.clone(), &[w.clone()]).selection, vec!["fresh-id"]);
        assert!(resolve(r.clone(), &[w.clone(), w.clone()]).needs_selection);
        r.tools.push("codex".into());
        assert!(resolve(r, &[w.clone()]).needs_selection);
        fs::remove_file(s.path).unwrap();
    }
}
