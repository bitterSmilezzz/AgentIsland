//! Local Codex skill activation preferences. No skill contents, scripts or installations are touched.
use crate::{mcp_config, provider::config_revision};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};
const LIMIT: usize = 200;
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub id: String,
    pub enabled: bool,
}
#[derive(Debug, Serialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub linked: bool,
    pub enabled: bool,
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
    pub before: bool,
    pub after: bool,
    pub linked: bool,
}
struct LocalSkill {
    id: String,
    name: String,
    logical: PathBuf,
    canonical: PathBuf,
    linked: bool,
    identity: String,
}
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 160
        && !name.chars().any(char::is_control)
        && !["sk-", "AIza", "-----BEGIN", "ghp_", "github_pat_"]
            .iter()
            .any(|s| name.contains(s))
}
fn discover(home: &Path) -> Result<Vec<LocalSkill>, String> {
    let root = home.join(".agents/skills");
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(_) => return Err("用户 Skills 目录不可读；不代表没有技能".into()),
    };
    let mut entries = entries
        .take(LIMIT + 1)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Skills 目录读取不完整")?;
    if entries.len() > LIMIT {
        return Err("Skills 目录超过 200 项，停止管理；未修改文件".into());
    }
    entries.sort_by_key(|e| e.file_name());
    let mut out = vec![];
    for entry in entries {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            return Err("Skills 目录名称不在支持范围".into());
        };
        if name.starts_with('.') {
            continue;
        }
        if !safe_name(&name) {
            return Err("Skills 目录含不安全名称，保留只读".into());
        }
        let logical = entry.path().join("SKILL.md");
        let meta = match fs::metadata(&logical) {
            Ok(m) if m.is_file() => m,
            Ok(_) => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err("Skill 元数据不可读；未读取技能内容".into()),
        };
        let canonical = fs::canonicalize(&logical).map_err(|_| "Skill 链接目标不可核实")?;
        let physical = canonical
            .to_str()
            .filter(|s| !s.chars().any(char::is_control))
            .ok_or("Skill 路径不在支持范围")?;
        let logical_text = logical.to_str().ok_or("Skill 路径不在支持范围")?;
        let modified = meta
            .modified()
            .map_err(|_| "Skill 版本不可核实")?
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Skill 时间不可核实")?
            .as_nanos();
        let identity = format!("{logical_text}\n{physical}\n{}\n{modified}", meta.len());
        let id = config_revision(logical_text);
        out.push(LocalSkill {
            id,
            name,
            linked: logical != canonical,
            logical,
            canonical,
            identity,
        });
    }
    Ok(out)
}
fn parse(text: &str) -> Result<DocumentMut, String> {
    let doc = text
        .parse::<DocumentMut>()
        .map_err(|_| "Codex 配置不可读，原文件保留")?;
    if doc
        .get("skills")
        .is_some_and(|s| s.as_table_like().is_none())
    {
        return Err("Skills 配置结构不受支持，原文件保留".into());
    }
    // Preserve advanced fields, but malformed or ambiguous paths never become guessed defaults.
    if let Some(config) = doc.get("skills").and_then(|s| s.get("config")) {
        let table = config
            .as_array_of_tables()
            .ok_or("Skills 配置须为 [[skills.config]]，原文件保留")?;
        if table.len() > LIMIT {
            return Err("Skills 配置超过 200 项，停止管理".into());
        }
        let mut paths = std::collections::HashSet::new();
        for item in table.iter() {
            let path = item
                .get("path")
                .and_then(Item::as_str)
                .ok_or("Skill 配置缺少路径")?;
            if !Path::new(path).is_absolute()
                || path.chars().any(char::is_control)
                || !paths.insert(path)
            {
                return Err("Skill 配置路径重复或未明确，原文件保留".into());
            }
            if item.get("enabled").and_then(Item::as_bool).is_none() {
                return Err("Skill 启停状态不可读，原文件保留".into());
            }
        }
    }
    Ok(doc)
}
fn configured<'a>(doc: &'a DocumentMut, skill: &LocalSkill) -> Result<Option<&'a Table>, String> {
    let mut entries = doc
        .get("skills")
        .and_then(|s| s.get("config"))
        .and_then(Item::as_array_of_tables)
        .into_iter()
        .flat_map(|a| a.iter())
        .filter(|entry| {
            entry.get("path").and_then(Item::as_str).is_some_and(|p| {
                Path::new(p) == skill.logical
                    || Path::new(p) == skill.canonical
                    || fs::canonicalize(p).ok().as_ref() == Some(&skill.canonical)
            })
        });
    let first = entries.next();
    if entries.next().is_some() {
        return Err("同一 Skill 的链接与目标有重复配置，请先在客户端核对".into());
    }
    Ok(first)
}
fn revision(text: &str, skills: &[LocalSkill]) -> String {
    config_revision(&format!(
        "{}\n{}",
        config_revision(text),
        skills
            .iter()
            .map(|s| s.identity.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    ))
}
pub fn inspect(target: &Path, home: &Path) -> Result<Snapshot, String> {
    let text = mcp_config::read(target)?;
    let doc = parse(&text)?;
    let skills = discover(home)?;
    let writable = mcp_config::inspect(target).is_ok_and(|state| state.writable);
    let entries = skills
        .iter()
        .map(|skill| {
            let enabled = configured(&doc, skill)?
                .and_then(|t| t.get("enabled"))
                .and_then(Item::as_bool)
                .unwrap_or(true);
            Ok(Entry {
                id: skill.id.clone(),
                name: skill.name.clone(),
                linked: skill.linked,
                enabled,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Snapshot {
        revision: revision(&text, &skills),
        entries,
        writable,
        notice: if writable {
            "Codex 用户 Skills 配置；写入后重启客户端核对加载，不执行技能。"
        } else {
            "配置含私有字段或未支持形式，Skills 保留只读。"
        }
        .into(),
    })
}
fn plan(
    target: &Path,
    home: &Path,
    operation: &Operation,
    expected: &str,
) -> Result<(String, String, Preview), String> {
    if operation.id.len() != 64 || !operation.id.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Skill 标识不在支持范围".into());
    }
    let original = mcp_config::read(target)?;
    let skills = discover(home)?;
    let revision = revision(&original, &skills);
    if revision != expected {
        return Err("配置或 Skill 已变化，请刷新后重新预览".into());
    }
    let mut doc = parse(&original)?;
    if !mcp_config::inspect(target).is_ok_and(|state| state.writable) {
        return Err("配置含私有字段或未支持形式，拒绝备份和写入".into());
    }
    let skill = skills
        .iter()
        .find(|s| s.id == operation.id)
        .ok_or("所选 Skill 已不存在，请刷新")?;
    let existing = configured(&doc, skill)?;
    let before = existing
        .and_then(|t| t.get("enabled"))
        .and_then(Item::as_bool)
        .unwrap_or(true);
    if before == operation.enabled {
        return Err("Skill 配置无变化，未写入".into());
    }
    let chosen_path = existing
        .and_then(|t| t.get("path"))
        .and_then(Item::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| skill.canonical.to_string_lossy().into_owned());
    if doc.get("skills").is_none() {
        doc["skills"] = Item::Table(Table::new());
    }
    let section = doc["skills"]
        .as_table_like_mut()
        .ok_or("Skills 结构不受支持")?;
    if !section.contains_key("config") {
        section.insert("config", Item::ArrayOfTables(ArrayOfTables::new()));
    }
    let entries = section
        .get_mut("config")
        .and_then(Item::as_array_of_tables_mut)
        .ok_or("Skills 结构不受支持")?;
    let selected = entries
        .iter()
        .position(|t| t.get("path").and_then(Item::as_str) == Some(&chosen_path));
    if let Some(index) = selected {
        let entry = entries.get_mut(index).ok_or("Skill 条目已变化")?;
        let decor = entry
            .get("enabled")
            .and_then(Item::as_value)
            .map(|v| v.decor().clone());
        entry["enabled"] = value(operation.enabled);
        if let Some(decor) = decor {
            *entry["enabled"].as_value_mut().unwrap().decor_mut() = decor;
        }
    } else {
        if entries.len() >= LIMIT {
            return Err("Skills 配置达到上限，未写入".into());
        }
        let mut entry = Table::new();
        entry["path"] = value(chosen_path);
        entry["enabled"] = value(operation.enabled);
        entries.push(entry);
    }
    let updated = doc.to_string();
    let plan_id = config_revision(&format!(
        "{revision}\n{}\n{}",
        operation.id, operation.enabled
    ));
    Ok((
        original,
        updated,
        Preview {
            revision,
            plan_id,
            name: skill.name.clone(),
            before,
            after: operation.enabled,
            linked: skill.linked,
        },
    ))
}
pub fn preview(
    target: &Path,
    home: &Path,
    operation: &Operation,
    revision: &str,
) -> Result<Preview, String> {
    plan(target, home, operation, revision).map(|(_, _, p)| p)
}
pub fn apply(
    target: &Path,
    home: &Path,
    backups: &Path,
    operation: &Operation,
    revision: &str,
    plan_id: &str,
    now: i64,
) -> Result<mcp_config::Applied, String> {
    let (original, updated, preview) = plan(target, home, operation, revision)?;
    if preview.plan_id != plan_id {
        return Err("Skill 操作与预览不一致，请重新预览".into());
    }
    // Re-resolve metadata after preview validation; no file content is copied.
    if self::revision(&mcp_config::read(target)?, &discover(home)?) != revision {
        return Err("配置或 Skill 已变化，未写入".into());
    }
    mcp_config::commit_text(
        target,
        backups,
        &original,
        &updated,
        &config_revision(&original),
        now,
        "Skills 配置已写入；请重启 Codex 核对加载，技能文件未改动。",
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (crate::testutil::Sandbox, PathBuf, PathBuf) {
        let s = crate::testutil::Sandbox::new("skill-config");
        let target = s.path().join(".codex/config.toml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::create_dir_all(s.path().join(".agents/skills/sample")).unwrap();
        fs::write(
            s.path().join(".agents/skills/sample/SKILL.md"),
            "PRIVATE_SKILL_CONTENT",
        )
        .unwrap();
        fs::write(&target, "model='existing'\n[features]\nfixture=true\n").unwrap();
        let backups = s.path().join("backups");
        (s, target, backups)
    }
    #[test]
    fn toggle_has_pure_preview_exact_backup_and_preserves_skill_files() {
        let (s, t, b) = fixture();
        let original = fs::read_to_string(&t).unwrap();
        let snap = inspect(&t, s.path()).unwrap();
        let op = Operation {
            id: snap.entries[0].id.clone(),
            enabled: false,
        };
        let p = preview(&t, s.path(), &op, &snap.revision).unwrap();
        assert!(!b.exists());
        assert_eq!(fs::read_to_string(&t).unwrap(), original);
        let applied = apply(&t, s.path(), &b, &op, &p.revision, &p.plan_id, 1).unwrap();
        assert!(applied.verified);
        assert_eq!(
            fs::read_to_string(b.join(applied.backup_name)).unwrap(),
            original
        );
        assert_eq!(
            fs::read_to_string(s.path().join(".agents/skills/sample/SKILL.md")).unwrap(),
            "PRIVATE_SKILL_CONTENT"
        );
        let snap = inspect(&t, s.path()).unwrap();
        assert!(!snap.entries[0].enabled);
        let op = Operation {
            enabled: true,
            ..op
        };
        let p = preview(&t, s.path(), &op, &snap.revision).unwrap();
        apply(&t, s.path(), &b, &op, &p.revision, &p.plan_id, 2).unwrap();
        let text = fs::read_to_string(&t).unwrap();
        assert!(text.contains("model='existing'"));
        assert!(text.contains("fixture=true"));
        assert!(!serde_json::to_string(&snap)
            .unwrap()
            .contains("PRIVATE_SKILL_CONTENT"));
        crate::provider::restore_backup_by_name(&t, &b, &crate::provider::backup_file_name(1))
            .unwrap();
        assert_eq!(fs::read_to_string(&t).unwrap(), original);
    }
    #[test]
    fn stale_config_changed_skill_operation_and_missing_identity_are_rejected() {
        let (s, t, b) = fixture();
        let snap = inspect(&t, s.path()).unwrap();
        let op = Operation {
            id: snap.entries[0].id.clone(),
            enabled: false,
        };
        let p = preview(&t, s.path(), &op, &snap.revision).unwrap();
        fs::write(
            s.path().join(".agents/skills/sample/SKILL.md"),
            "CHANGED_SKILL_CONTENT",
        )
        .unwrap();
        assert!(apply(&t, s.path(), &b, &op, &p.revision, &p.plan_id, 1).is_err());
        assert!(!b.exists());
        let snap = inspect(&t, s.path()).unwrap();
        let p = preview(&t, s.path(), &op, &snap.revision).unwrap();
        assert!(apply(&t, s.path(), &b, &op, &p.revision, "wrong-plan", 1).is_err());
        assert!(preview(
            &t,
            s.path(),
            &Operation {
                id: "unknown".into(),
                enabled: false
            },
            &snap.revision
        )
        .is_err());
        fs::write(&t, "model='external'\n").unwrap();
        assert!(apply(&t, s.path(), &b, &op, &p.revision, &p.plan_id, 1).is_err());
        assert!(!b.exists());
    }
    #[test]
    fn malformed_duplicate_and_private_configuration_never_writes() {
        let (s, t, b) = fixture();
        for text in ["skills='bad'", "[[skills.config]]\npath='/fixture/SKILL.md'", "[[skills.config]]\npath='/fixture/SKILL.md'\nenabled=false\n[[skills.config]]\npath='/fixture/SKILL.md'\nenabled=true"] {fs::write(&t,text).unwrap();assert!(inspect(&t,s.path()).is_err());}
        fs::write(
            &t,
            "[model_providers.fixture]\nexperimental_bearer_token='PRIVATE_FIXED_PLACEHOLDER'\n",
        )
        .unwrap();
        let snap = inspect(&t, s.path()).unwrap();
        assert!(!snap.writable);
        let op = Operation {
            id: snap.entries[0].id.clone(),
            enabled: false,
        };
        assert!(preview(&t, s.path(), &op, &snap.revision).is_err());
        assert!(!b.exists());
    }
    #[cfg(unix)]
    #[test]
    fn aliases_share_configuration_and_ambiguous_overrides_are_read_only() {
        let (s, t, b) = fixture();
        let root = s.path().join(".agents/skills");
        std::os::unix::fs::symlink(root.join("sample"), root.join("alias")).unwrap();
        let logical = root.join("alias/SKILL.md");
        let text = format!(
            "[[skills.config]]\npath='{}'\nenabled=false\n",
            logical.display()
        );
        fs::write(&t, &text).unwrap();
        let snap = inspect(&t, s.path()).unwrap();
        assert_eq!(snap.entries.len(), 2);
        assert!(snap.entries.iter().all(|e| !e.enabled));
        let op = Operation {
            id: snap
                .entries
                .iter()
                .find(|e| e.name == "sample")
                .unwrap()
                .id
                .clone(),
            enabled: true,
        };
        let p = preview(&t, s.path(), &op, &snap.revision).unwrap();
        apply(&t, s.path(), &b, &op, &p.revision, &p.plan_id, 1).unwrap();
        assert!(inspect(&t, s.path())
            .unwrap()
            .entries
            .iter()
            .all(|e| e.enabled));
        fs::write(
            &t,
            format!(
                "{text}[[skills.config]]\npath='{}'\nenabled=true\n",
                root.join("sample/SKILL.md").display()
            ),
        )
        .unwrap();
        assert!(inspect(&t, s.path()).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_uses_target_identity_and_retargeting_invalidates_preview() {
        let (s, t, b) = fixture();
        let root = s.path().join(".agents/skills");
        std::os::unix::fs::symlink(root.join("sample"), root.join("linked")).unwrap();
        let snap = inspect(&t, s.path()).unwrap();
        let entry = snap.entries.iter().find(|e| e.name == "linked").unwrap();
        assert!(entry.linked);
        let op = Operation {
            id: entry.id.clone(),
            enabled: false,
        };
        let p = preview(&t, s.path(), &op, &snap.revision).unwrap();
        fs::create_dir_all(s.path().join("other")).unwrap();
        fs::write(s.path().join("other/SKILL.md"), "OTHER").unwrap();
        fs::remove_file(root.join("linked")).unwrap();
        std::os::unix::fs::symlink(s.path().join("other"), root.join("linked")).unwrap();
        assert!(apply(&t, s.path(), &b, &op, &p.revision, &p.plan_id, 1).is_err());
        assert!(!b.exists());
    }
}
