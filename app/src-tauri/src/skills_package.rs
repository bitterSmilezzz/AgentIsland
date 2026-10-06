//! Local skill packages: preview, atomic directory publication, persistent recovery facts.
//! Paths and contents are kept in memory; serialized records contain names and fingerprints only.
use crate::skill_files::{self as files, Tree};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
const MAX_RECORDS: usize = 100;
const PLAN_TTL: i64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Tool {
    #[default]
    Codex,
    Claude,
}
impl Tool {
    fn directory(self) -> &'static str {
        match self {
            Self::Codex => ".agents",
            Self::Claude => ".claude",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
        }
    }
    fn permits(self, name: &str) -> bool {
        files::safe_name(name)
            && !(self == Self::Claude && matches!(name, "synced" | "anthropic-skills"))
    }
}
#[derive(Debug, Serialize)]
pub(crate) struct SyncEntry {
    pub id: String,
    pub tool: Tool,
    pub name: String,
    pub available: bool,
    pub notice: String,
}
#[derive(Debug, Serialize)]
pub(crate) struct SyncInventory {
    pub generation: String,
    pub entries: Vec<SyncEntry>,
    pub notices: Vec<String>,
    pub unavailable: Vec<Tool>,
}
struct SyncSource {
    tool: Tool,
    name: String,
    directory: (u64, u64),
    skill: String,
}
#[derive(Debug, Serialize)]
pub(crate) struct Change {
    pub path: String,
    pub kind: String,
}
#[derive(Debug, Serialize)]
pub(crate) struct Preview {
    pub plan_id: String,
    pub name: String,
    pub action: String,
    pub files: usize,
    pub bytes: usize,
    pub changes: Vec<Change>,
    pub change_count: usize,
    pub expires_ms: i64,
    pub notice: String,
    pub target: Option<Tool>,
    pub source: Option<Tool>,
}
#[derive(Debug, Serialize)]
pub(crate) struct Applied {
    pub id: String,
    pub name: String,
    pub notice: String,
}
#[derive(Debug, Serialize)]
pub(crate) struct Recovery {
    pub id: String,
    pub name: String,
    pub created_ms: i64,
    pub state: String,
    pub revision: String,
    pub notice: String,
    pub target: Option<Tool>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: u8,
    id: String,
    name: String,
    created_ms: i64,
    before: Option<String>,
    after: String,
    #[serde(default)]
    target: Tool,
}
enum SourceLocation {
    Directory(PathBuf),
    Tool {
        tool: Tool,
        name: String,
        identity: (u64, u64),
    },
    Edited {
        tool: Tool,
        name: String,
        identity: (u64, u64),
        before: String,
        content: Vec<u8>,
    },
}
struct EditTicket {
    tool: Tool,
    name: String,
    identity: (u64, u64),
    revision: String,
    header: String,
    expires_ms: i64,
}
#[derive(Debug, Serialize)]
pub(crate) struct EditDocument {
    pub ticket: String,
    pub tool: Tool,
    pub name: String,
    pub body: String,
    pub expires_ms: i64,
}
#[derive(Debug, Serialize)]
pub(crate) struct EditPreview {
    pub preview: Preview,
    pub before: String,
    pub after: String,
}
struct Plan {
    target: Tool,
    source: SourceLocation,
    revision: String,
    before: Option<String>,
    name: String,
    expires_ms: i64,
}
struct TrashPlan {
    id: String,
    revision: String,
    identity: (u64, u64),
    name: String,
    expires_ms: i64,
}
pub(crate) struct Store {
    home: PathBuf,
    plans: HashMap<String, Plan>,
    trash_plans: HashMap<String, TrashPlan>,
    sources: HashMap<String, SyncSource>,
    generation: String,
    edits: HashMap<String, EditTicket>,
}
struct Roots {
    records: File,
}

// Parse metadata only: preserve the complete original SKILL.md and never render parser snippets.
const MAX_METADATA: usize = 64 * 1024;
fn package_name(tree: &Tree) -> Result<String, String> {
    let bytes = tree
        .nodes
        .get("SKILL.md")
        .and_then(|node| node.bytes.as_ref())
        .ok_or("技能目录根部需要 SKILL.md")?;
    let text = std::str::from_utf8(bytes).map_err(|_| "SKILL.md 不是 UTF-8")?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    let first = lines.next().ok_or("SKILL.md 缺少 frontmatter")?;
    if first.trim_end() != "---" {
        return Err("SKILL.md 缺少 frontmatter".into());
    }
    let start = first.len();
    let mut end = start;
    let mut closed = false;
    for line in lines {
        if matches!(line.trim_end(), "---" | "...") {
            closed = true;
            break;
        }
        end += line.len();
        if end - start > MAX_METADATA {
            return Err("技能元数据超过 64 KiB，请精简说明；正文不受此限制".into());
        }
    }
    if !closed {
        return Err("SKILL.md 元数据缺少结束标记".into());
    }
    let options = serde_saphyr::options! {
        budget: serde_saphyr::budget! {
            max_documents: 1, max_depth: 16, flow_nesting_limit: 16,
            max_events: 4096, max_nodes: 2048, max_anchors: 32, max_aliases: 64,
            max_merge_keys: 64, max_recorded_anchor_events: 4096,
            max_recorded_anchor_bytes: 256 * 1024, max_total_scalar_bytes: 256 * 1024,
            max_total_comment_bytes: MAX_METADATA, max_inclusion_depth: 0,
        },
        duplicate_keys: serde_saphyr::DuplicateKeyPolicy::Error,
        reject_unsupported_tags: true, strict_booleans: true, with_snippet: false,
    };
    let metadata: serde_json::Value =
        serde_saphyr::from_str_with_options(&text[start..end], options)
            .map_err(|_| "技能元数据 YAML 不合法、字段重复或超出解析限额，请检查 SKILL.md")?;
    let fields = metadata.as_object().ok_or("技能元数据需要键值映射")?;
    let name = fields
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("技能名称需要字符串")?;
    let description = fields
        .get("description")
        .and_then(|v| v.as_str())
        .ok_or("技能说明需要字符串")?;
    if description.trim().is_empty() {
        return Err("技能说明不能为空".into());
    }
    if !files::safe_name(name) {
        return Err("技能名称需为 1–64 位小写字母、数字及单连字符".into());
    }
    Ok(name.to_owned())
}
fn split_body(content: &str) -> Result<(&str, &str), String> {
    let offset = if content.starts_with('\u{feff}') {
        3
    } else {
        0
    };
    let mut end = offset;
    let mut lines = content[offset..].split_inclusive('\n');
    let first = lines.next().ok_or("技能元数据缺失")?;
    if first.trim_end() != "---" {
        return Err("技能元数据缺失".into());
    }
    end += first.len();
    for line in lines {
        end += line.len();
        if matches!(line.trim_end(), "---" | "...") {
            return Ok(content.split_at(end));
        }
    }
    Err("技能元数据缺少结束标记".into())
}
fn differences(before: Option<&Tree>, after: &Tree) -> Vec<Change> {
    let empty = std::collections::BTreeMap::new();
    let old = before.map(|tree| &tree.nodes).unwrap_or(&empty);
    let mut keys = old.keys().chain(after.nodes.keys()).collect::<Vec<_>>();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .filter_map(|path| {
            let kind = match (old.get(path), after.nodes.get(path)) {
                (None, Some(_)) => "added",
                (Some(_), None) => "removed",
                (Some(a), Some(b)) if a.bytes != b.bytes || a.executable != b.executable => {
                    "changed"
                }
                _ => return None,
            };
            Some(Change {
                path: path.clone(),
                kind: kind.into(),
            })
        })
        .collect()
}
fn fingerprint(root: &File, name: &str) -> Result<Option<String>, String> {
    if !files::exists(root, name)? {
        return Ok(None);
    }
    Ok(Some(
        files::scan(&files::child(root, name, true)?)?.revision,
    ))
}
fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id)
}
fn digest_valid(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}

impl Store {
    pub(crate) fn new(home: PathBuf) -> Self {
        Self {
            home,
            plans: HashMap::new(),
            trash_plans: HashMap::new(),
            sources: HashMap::new(),
            generation: String::new(),
            edits: HashMap::new(),
        }
    }
    fn roots(&self, create: bool) -> Result<Roots, String> {
        let home = files::open_dir(&self.home)?;
        let agents = if create {
            files::ensure_dir(&home, ".agents")?
        } else {
            files::child(&home, ".agents", true)?
        };
        let records = if create {
            files::ensure_dir(&agents, ".agentisland-skill-installs")?
        } else {
            files::child(&agents, ".agentisland-skill-installs", true)?
        };
        Ok(Roots { records })
    }
    fn skills_root(&self, target: Tool, create: bool) -> Result<Option<File>, String> {
        let home = files::open_dir(&self.home)?;
        if !create && !files::exists(&home, target.directory())? {
            return Ok(None);
        }
        let base = if create {
            files::ensure_dir(&home, target.directory())?
        } else {
            files::child(&home, target.directory(), true)?
        };
        if !create && !files::exists(&base, "skills")? {
            return Ok(None);
        }
        Ok(Some(if create {
            files::ensure_dir(&base, "skills")?
        } else {
            files::child(&base, "skills", true)?
        }))
    }
    fn verify_skills_root(&self, target: Tool, opened: &File) -> Result<(), String> {
        let current = self
            .skills_root(target, false)?
            .ok_or("当前技能根目录已变化")?;
        if files::identity(&current)? != files::identity(opened)? {
            return Err("当前技能根目录身份已变化".into());
        }
        Ok(())
    }
    fn verify_records_root(&self, opened: &Roots) -> Result<(), String> {
        let current = self.roots(false)?;
        if files::identity(&current.records)? != files::identity(&opened.records)? {
            return Err("当前恢复记录根目录身份已变化".into());
        }
        Ok(())
    }
    fn existing(&self, target: Tool, name: &str) -> Result<Option<Tree>, String> {
        let Some(root) = self.skills_root(target, false)? else {
            return Ok(None);
        };
        if !files::exists(&root, name)? {
            return Ok(None);
        }
        let tree = files::scan(&files::child(&root, name, true)?)?;
        if package_name(&tree).as_deref() != Ok(name) {
            return Err("同名目录的技能身份无法核实，不覆盖；请检查其 SKILL.md".into());
        }
        Ok(Some(tree))
    }
    pub(crate) fn inventory(&mut self) -> Result<SyncInventory, String> {
        let mut sources = HashMap::new();
        let mut entries = vec![];
        let mut notices = vec![];
        let mut unavailable = vec![];
        for tool in [Tool::Codex, Tool::Claude] {
            let root = match self.skills_root(tool, false) {
                Ok(Some(root)) => root,
                Ok(None) => continue,
                Err(_) => {
                    notices.push(format!("{} 用户目录不可读；没有推断为空", tool.label()));
                    unavailable.push(tool);
                    continue;
                }
            };
            let names = match files::names(&root) {
                Ok(names) if names.len() <= 200 => names,
                _ => {
                    notices.push(format!("{} 用户目录不可完整读取或超过200项", tool.label()));
                    unavailable.push(tool);
                    continue;
                }
            };
            for name in names {
                if !tool.permits(&name) {
                    notices.push(format!("{} 有保留或不受支持目录，未纳入同步", tool.label()));
                    continue;
                }
                let candidate = (|| {
                    let directory = files::child(&root, &name, true)?;
                    let skill = files::child(&directory, "SKILL.md", false)?;
                    Ok::<_, String>((files::identity(&directory)?, files::file_stamp(&skill)?))
                })();
                let (id, available, notice) = match candidate {
                    Ok((directory, skill)) => {
                        let id = uuid::Uuid::new_v4().to_string();
                        sources.insert(
                            id.clone(),
                            SyncSource {
                                tool,
                                name: name.clone(),
                                directory,
                                skill,
                            },
                        );
                        (id, true, "目录候选，完整内容在预览时核对".to_string())
                    }
                    Err(_) => (
                        String::new(),
                        false,
                        "目录链接、缺少SKILL.md或元数据不可核实；未跟随链接".into(),
                    ),
                };
                entries.push(SyncEntry {
                    id,
                    tool,
                    name,
                    available,
                    notice,
                });
            }
        }
        notices.sort();
        notices.dedup();
        let generation = uuid::Uuid::new_v4().to_string();
        self.sources = sources;
        self.generation = generation.clone();
        Ok(SyncInventory {
            generation,
            entries,
            notices,
            unavailable,
        })
    }
    pub(crate) fn sync_preview(
        &mut self,
        id: &str,
        generation: &str,
        target: Tool,
        now: i64,
    ) -> Result<Preview, String> {
        if generation != self.generation {
            return Err("技能清单已更新，请刷新后重新选择来源".into());
        }
        let source = self.sources.get(id).ok_or("来源技能不可核实，请刷新清单")?;
        if source.tool == target {
            return Err("来源与目标工具相同，无需同步".into());
        }
        let root = self
            .skills_root(source.tool, false)?
            .ok_or("来源目录已变化，请刷新")?;
        let directory = files::child(&root, &source.name, true)?;
        let skill = files::child(&directory, "SKILL.md", false)?;
        if files::identity(&directory)? != source.directory
            || files::file_stamp(&skill)? != source.skill
        {
            return Err("来源技能元数据已变化，请刷新清单".into());
        }
        let source_tool = source.tool;
        let location = SourceLocation::Tool {
            tool: source_tool,
            name: source.name.clone(),
            identity: source.directory,
        };
        let mut preview = self.prepare(location, target, now)?;
        preview.source = Some(source_tool);
        preview.notice=format!("{} → {} 用户级。复制完整包，更新保留旧包；工具专属字段和正文语法不转换，需在目标工具核对加载。",source_tool.label(),target.label());
        Ok(preview)
    }
    pub(crate) fn edit_read(
        &mut self,
        id: &str,
        generation: &str,
        now: i64,
    ) -> Result<EditDocument, String> {
        if !cfg!(target_os = "macos") {
            return Err("本平台技能编辑尚未验证".into());
        }
        self.edits.retain(|_, ticket| ticket.expires_ms >= now);
        if self.edits.len() >= 16 {
            return Err("打开的技能正文过多，请关闭旧编辑后重试".into());
        }
        if generation != self.generation {
            return Err("技能清单已更新，请刷新后选择".into());
        }
        let source = self.sources.get(id).ok_or("来源技能不可核实，请刷新清单")?;
        let root = self
            .skills_root(source.tool, false)?
            .ok_or("来源目录已变化")?;
        let directory = files::child(&root, &source.name, true)?;
        if files::identity(&directory)? != source.directory
            || files::file_stamp(&files::child(&directory, "SKILL.md", false)?)? != source.skill
        {
            return Err("技能元数据已变化，请刷新清单".into());
        }
        let tree = files::scan(&directory)?;
        if package_name(&tree)? != source.name {
            return Err("技能名称与目录不一致，不编辑此目录".into());
        }
        let content = std::str::from_utf8(
            tree.nodes["SKILL.md"]
                .bytes
                .as_ref()
                .ok_or("正文类型不支持")?,
        )
        .map_err(|_| "正文不是UTF-8")?;
        let (header, body) = split_body(content)?;
        let ticket = uuid::Uuid::new_v4().to_string();
        let expires_ms = now.checked_add(30 * 60 * 1000).ok_or("时间不在支持范围")?;
        let result = EditDocument {
            ticket: ticket.clone(),
            tool: source.tool,
            name: source.name.clone(),
            body: body.into(),
            expires_ms,
        };
        self.edits.insert(
            ticket,
            EditTicket {
                tool: source.tool,
                name: source.name.clone(),
                identity: source.directory,
                revision: tree.revision,
                header: header.into(),
                expires_ms,
            },
        );
        Ok(result)
    }
    pub(crate) fn edit_preview(
        &mut self,
        ticket: &str,
        body: String,
        now: i64,
    ) -> Result<EditPreview, String> {
        let edit = self.edits.get(ticket).ok_or("编辑已关闭，请重新打开正文")?;
        if now > edit.expires_ms || now < edit.expires_ms - 30 * 60 * 1000 {
            return Err("编辑读取已到期或时钟变化；请保留草稿后重新读取".into());
        }
        if body.len() > 4 * 1024 * 1024 || crate::private_text::known_private(&body) {
            return Err("正文超过4 MiB或包含已识别凭据，未修改文件".into());
        }
        let root = self
            .skills_root(edit.tool, false)?
            .ok_or("目标技能目录已变化")?;
        let directory = files::child(&root, &edit.name, true)?;
        if files::identity(&directory)? != edit.identity {
            return Err("目标目录身份已变化，请保留草稿后重新读取".into());
        }
        let tree = files::scan(&directory)?;
        if tree.revision != edit.revision {
            return Err("技能包在编辑期间已变化，请保留草稿后重新读取".into());
        }
        let old = std::str::from_utf8(
            tree.nodes["SKILL.md"]
                .bytes
                .as_ref()
                .ok_or("正文类型不支持")?,
        )
        .map_err(|_| "正文不是UTF-8")?;
        let before = split_body(old)?.1.to_string();
        let source = SourceLocation::Edited {
            tool: edit.tool,
            name: edit.name.clone(),
            identity: edit.identity,
            before: edit.revision.clone(),
            content: format!(
                "{}{}{}",
                edit.header,
                if edit.header.ends_with('\n') || body.is_empty() {
                    ""
                } else {
                    "\n"
                },
                body
            )
            .into_bytes(),
        };
        let target = edit.tool;
        let mut preview = self.prepare(source, target, now)?;
        preview.action = "edit".into();
        preview.notice=format!("编辑 {} 用户级 SKILL.md 正文，元数据与资源保留；确认后保留旧包，可从记录恢复。客户端加载需核对。",target.label());
        Ok(EditPreview {
            preview,
            before,
            after: body,
        })
    }
    pub(crate) fn edit_close(&mut self, ticket: &str) {
        self.edits.remove(ticket);
    }
    pub(crate) fn preview(&mut self, source: &Path, now: i64) -> Result<Preview, String> {
        self.preview_for(source, Tool::Codex, now)
    }
    pub(crate) fn preview_for(
        &mut self,
        source: &Path,
        target: Tool,
        now: i64,
    ) -> Result<Preview, String> {
        self.prepare(SourceLocation::Directory(source.to_path_buf()), target, now)
    }
    fn source_tree(&self, source: &SourceLocation) -> Result<Tree, String> {
        let directory = match source {
            SourceLocation::Directory(path) => files::open_dir(path)?,
            SourceLocation::Tool {
                tool,
                name,
                identity,
            }
            | SourceLocation::Edited {
                tool,
                name,
                identity,
                ..
            } => {
                let root = self
                    .skills_root(*tool, false)?
                    .ok_or("来源工具目录已变化")?;
                let directory = files::child(&root, name, true)?;
                if files::identity(&directory)? != *identity {
                    return Err("来源目录身份已变化，请刷新清单".into());
                }
                directory
            }
        };
        let mut tree = files::scan(&directory)?;
        if let SourceLocation::Edited {
            before, content, ..
        } = source
        {
            if &tree.revision != before {
                return Err("技能包在编辑期间已变化，请保留草稿后重新读取".into());
            }
            files::replace_skill(&mut tree, content.clone())?;
        }
        Ok(tree)
    }
    fn prepare(
        &mut self,
        source: SourceLocation,
        target: Tool,
        now: i64,
    ) -> Result<Preview, String> {
        if !cfg!(target_os = "macos") {
            return Err("本平台技能包安装尚未验证".into());
        }
        self.plans.retain(|_, plan| plan.expires_ms >= now);
        self.trash_plans.retain(|_, plan| plan.expires_ms >= now);
        if self.plans.len() + self.trash_plans.len() >= 16 {
            return Err("待处理预览过多，请取消旧预览后再选目录".into());
        }
        let tree = self.source_tree(&source)?;
        let name = package_name(&tree)?;
        if !target.permits(&name) {
            return Err("目标工具保留此名称，未安装；请使用其他技能名称".into());
        }
        let before = self.existing(target, &name)?;
        if before
            .as_ref()
            .is_some_and(|old| old.revision == tree.revision)
        {
            return Err("这个技能包与已安装文件一致，无需更新".into());
        }
        let mut changes = differences(before.as_ref(), &tree);
        let count = changes.len();
        changes.truncate(100);
        let expires = now.checked_add(PLAN_TTL).ok_or("时间不在支持范围")?;
        let id = uuid::Uuid::new_v4().to_string();
        let preview = Preview {
            plan_id: id.clone(),
            name: name.clone(),
            action: if before.is_some() {
                "update"
            } else {
                "install"
            }
            .into(),
            files: tree.files,
            bytes: tree.bytes,
            changes,
            change_count: count,
            expires_ms: expires,
            notice: format!(
                "目标：{} 用户级 Skills。确认后发布完整目录，更新保留旧包；客户端加载需单独核对。",
                target.label()
            ),
            target: Some(target),
            source: None,
        };
        self.plans.insert(
            id,
            Plan {
                target,
                source,
                revision: tree.revision,
                before: before.map(|old| old.revision),
                name,
                expires_ms: expires,
            },
        );
        Ok(preview)
    }
    pub(crate) fn cancel(&mut self, id: &str) {
        self.plans.remove(id);
        self.trash_plans.remove(id);
    }
    pub(crate) fn apply(&mut self, id: &str, now: i64) -> Result<Applied, String> {
        let plan = self.plans.get(id).ok_or("预览已失效，请重新预览")?;
        if now > plan.expires_ms || now < plan.expires_ms - PLAN_TTL {
            return Err("预览已到期或时钟变化，请重新预览".into());
        }
        let tree = self.source_tree(&plan.source)?;
        if tree.revision != plan.revision || package_name(&tree)? != plan.name {
            return Err("来源技能已变化，请重新预览；未安装".into());
        }
        if self
            .existing(plan.target, &plan.name)?
            .map(|old| old.revision)
            != plan.before
        {
            return Err("目标目录已变化，请重新预览；未安装".into());
        }
        let roots = self.roots(true)?;
        let skills = self
            .skills_root(plan.target, true)?
            .ok_or("目标技能目录不可用")?;
        if files::names(&roots.records)?.len() >= MAX_RECORDS {
            return Err("恢复记录达到 100 项，安装暂停；请核对并将不再需要的记录移入废纸篓".into());
        }
        let transaction_id = uuid::Uuid::new_v4().to_string();
        let directory = files::create_dir(&roots.records, &transaction_id)?;
        let stage = files::create_dir(&directory, "package")?;
        files::stage(&stage, &tree)?;
        let receipt = Receipt {
            schema: 1,
            id: transaction_id.clone(),
            name: plan.name.clone(),
            created_ms: now,
            before: plan.before.clone(),
            after: tree.revision.clone(),
            target: plan.target,
        };
        let bytes = serde_json::to_vec(&receipt).map_err(|_| "安装记录无法序列化")?;
        files::write_new(&directory, "receipt.json", &bytes, false)?;
        if fingerprint(&skills, &plan.name)? != plan.before {
            return Err("目标在暂存期间变化，未发布；恢复记录保留".into());
        }
        self.verify_skills_root(plan.target, &skills)
            .and_then(|_| self.verify_records_root(&roots))
            .map_err(|_| "目标或记录目录链在暂存期间变化，未发布；暂存保留，请重新核对")?;
        files::publish(
            &directory,
            "package",
            &skills,
            &plan.name,
            plan.before.is_some(),
        )?;
        let name = plan.name.clone();
        let target = plan.target;
        let syncing = matches!(plan.source, SourceLocation::Tool { .. });
        let editing = matches!(plan.source, SourceLocation::Edited { .. });
        self.plans.remove(id);
        self.verify_skills_root(target, &skills)
            .and_then(|_| self.verify_records_root(&roots))
            .map_err(|_| "技能包已发布，但当前目标或记录目录链已变化；请核对原件，不自动回移")?;
        let observed = fingerprint(&skills, &name)
            .map_err(|_| "技能目录已发布，但目标读回失败；请核对安装记录，保留目录未删除")?;
        let old = fingerprint(&directory, "package")
            .map_err(|_| "技能目录已发布，但保留目录读回失败；请核对安装记录")?;
        if observed != Some(receipt.after) || old != receipt.before {
            return Err("目录已发布，但读回发现变化；旧包保留，请核对恢复记录".into());
        }
        Ok(Applied {
            id: transaction_id,
            name,
            notice: format!(
                "技能包已{}至 {} 用户级；可从记录恢复。请在目标客户端核对加载。",
                if editing {
                    "保存"
                } else if syncing {
                    "同步"
                } else {
                    "安装"
                },
                target.label()
            ),
        })
    }
    fn receipt(roots: &Roots, id: &str) -> Result<(File, Receipt), String> {
        if !valid_id(id) {
            return Err("安装记录标识不受支持".into());
        }
        let directory = files::child(&roots.records, id, true)?;
        let mut file = files::child(&directory, "receipt.json", false)?;
        if !file
            .metadata()
            .is_ok_and(|meta| meta.is_file() && meta.len() <= 4096)
        {
            return Err("安装记录类型或大小不受支持".into());
        }
        let mut text = String::new();
        file.by_ref()
            .take(4097)
            .read_to_string(&mut text)
            .map_err(|_| "安装记录不可读")?;
        if text.len() > 4096 {
            return Err("安装记录超过上限".into());
        }
        let receipt: Receipt = serde_json::from_str(&text).map_err(|_| "安装记录损坏，原件保留")?;
        if receipt.schema != 1
            || receipt.id != id
            || !receipt.target.permits(&receipt.name)
            || !digest_valid(&receipt.after)
            || receipt
                .before
                .as_ref()
                .is_some_and(|before| !digest_valid(before))
        {
            return Err("安装记录结构不受支持，原件保留".into());
        }
        Ok((directory, receipt))
    }
    fn recovery(&self, roots: &Roots, id: &str) -> Result<Recovery, String> {
        let (directory, receipt) = Self::receipt(roots, id)?;
        let live = match self.skills_root(receipt.target, false)? {
            Some(root) => fingerprint(&root, &receipt.name)?,
            None => None,
        };
        let kept = fingerprint(&directory, "package")?;
        let (state, notice) = if live == Some(receipt.after.clone()) && kept == receipt.before {
            (
                "applied",
                if receipt.before.is_some() {
                    "可恢复更新前的完整技能包"
                } else {
                    "可将本次安装移回保留目录"
                },
            )
        } else if live == receipt.before && kept == Some(receipt.after.clone()) {
            ("inactive", "目标未应用此包或已移回；保留目录不自动删除")
        } else {
            ("conflict", "目标或保留包已变化；不自动覆盖，请在本机核对")
        };
        let revision = crate::provider::config_revision(&format!(
            "{id}\n{:?}\n{:?}\n{:?}",
            receipt.target, live, kept
        ));
        Ok(Recovery {
            id: id.into(),
            name: receipt.name,
            created_ms: receipt.created_ms,
            state: state.into(),
            revision,
            notice: notice.into(),
            target: Some(receipt.target),
        })
    }
    pub(crate) fn recoveries(&self) -> Result<Vec<Recovery>, String> {
        let home = files::open_dir(&self.home)?;
        if !files::exists(&home, ".agents")? {
            return Ok(vec![]);
        }
        let agents = files::child(&home, ".agents", true)?;
        if !files::exists(&agents, ".agentisland-skill-installs")? {
            return Ok(vec![]);
        }
        let roots = self.roots(false)?;
        let names = files::names(&roots.records)?;
        // Installation capacity is 100; read all bounded entries so cleanup stays available above it.
        let mut records = names
            .iter()
            .map(|id| {
                self.recovery(&roots, id).unwrap_or_else(|_| Recovery {
                    id: if valid_id(id) {
                        id.clone()
                    } else {
                        String::new()
                    },
                    name: "未核实的安装记录".into(),
                    created_ms: 0,
                    state: "unreadable".into(),
                    revision: String::new(),
                    target: None,
                    notice: "记录或保留目录不可读；原件保留，恢复不可用。".into(),
                })
            })
            .collect::<Vec<_>>();
        records.sort_by(|a, b| b.created_ms.cmp(&a.created_ms).then(a.id.cmp(&b.id)));
        Ok(records)
    }
    pub(crate) fn trash_preview(&mut self, id: &str, now: i64) -> Result<Preview, String> {
        if !cfg!(target_os = "macos") || !valid_id(id) {
            return Err("本平台或记录标识不支持清理".into());
        }
        self.plans.retain(|_, plan| plan.expires_ms >= now);
        self.trash_plans.retain(|_, plan| plan.expires_ms >= now);
        if self.plans.len() + self.trash_plans.len() >= 16 {
            return Err("待处理预览过多，请先取消旧预览".into());
        }
        let roots = self.roots(false)?;
        let directory = files::child(&roots.records, id, true)?;
        let tree = files::scan_record(&directory)?;
        let identity = files::identity(&directory)?;
        let name = Self::receipt(&roots, id)
            .map(|(_, receipt)| receipt.name)
            .unwrap_or_else(|_| "未核实的安装记录".into());
        let plan_id = uuid::Uuid::new_v4().to_string();
        let expires_ms = now.checked_add(PLAN_TTL).ok_or("时间不在支持范围")?;
        let mut changes = tree
            .nodes
            .keys()
            .map(|path| Change {
                path: path.clone(),
                kind: "moved".into(),
            })
            .collect::<Vec<_>>();
        let change_count = changes.len();
        changes.truncate(100);
        let preview=Preview {plan_id:plan_id.clone(),name:name.clone(),action:"trash".into(),files:tree.files,bytes:tree.bytes,
            changes,change_count,expires_ms,target:None,source:None,notice:"记录与保留包将移入废纸篓，已安装技能及启停配置不变。此条恢复将移除，可在废纸篓手动取回；转移不释放磁盘空间。".into()};
        self.trash_plans.insert(
            plan_id,
            TrashPlan {
                id: id.into(),
                revision: tree.revision,
                identity,
                name,
                expires_ms,
            },
        );
        Ok(preview)
    }
    pub(crate) fn trash(&mut self, plan_id: &str, now: i64) -> Result<Applied, String> {
        let plan = self
            .trash_plans
            .get(plan_id)
            .ok_or("清理预览已失效，请重新核对记录")?;
        if now > plan.expires_ms || now < plan.expires_ms - PLAN_TTL {
            return Err("清理预览到期或时钟变化，请重新预览".into());
        }
        let roots = self.roots(false)?;
        let directory = files::child(&roots.records, &plan.id, true)?;
        if files::identity(&directory)? != plan.identity
            || files::scan_record(&directory)?.revision != plan.revision
        {
            return Err("安装记录或保留包已变化，请重新预览；未移入废纸篓".into());
        }
        let home = files::open_dir(&self.home)?;
        let trash = files::ensure_dir(&home, ".Trash")
            .map_err(|_| "废纸篓不可用或权限不足，记录仍保留；未改目标技能")?;
        let destination = format!("AgentIsland-skill-record-{}", uuid::Uuid::new_v4());
        self.verify_records_root(&roots)
            .map_err(|_| "恢复记录目录链已变化，未移入废纸篓")?;
        let current_trash = files::child(&home, ".Trash", true)?;
        if files::identity(&current_trash)? != files::identity(&trash)? {
            return Err("废纸篓目录身份已变化，未移入".into());
        }
        files::publish(&roots.records, &plan.id, &trash, &destination, false)
            .map_err(|_| "移入废纸篓失败；记录保留，未改目标技能")?;
        let (id, name, revision, identity) = (
            plan.id.clone(),
            plan.name.clone(),
            plan.revision.clone(),
            plan.identity,
        );
        self.trash_plans.remove(plan_id);
        self.verify_records_root(&roots)
            .map_err(|_| "记录已移出，但当前记录目录链已变化；请核对原件")?;
        let current_trash = files::child(&home, ".Trash", true)
            .map_err(|_| "记录已移出，但当前废纸篓目录链不可核实")?;
        if files::identity(&current_trash).map_err(|_| "记录已移出，废纸篓身份不可核实")?
            != files::identity(&trash).map_err(|_| "记录已移出，原废纸篓身份不可核实")?
        {
            return Err("记录已移出，但当前废纸篓目录身份已变化；请核对原件".into());
        }
        let moved = files::child(&trash, &destination, true)
            .map_err(|_| "记录已移入废纸篓，但读回不可用；请核对废纸篓与安装记录")?;
        if files::identity(&moved).map_err(|_| "记录已移入废纸篓，但身份读回不可用")? != identity
            || files::scan_record(&moved)
                .map_err(|_| "记录已移入废纸篓，但内容读回不可用")?
                .revision
                != revision
            || files::exists(&roots.records, &id)
                .map_err(|_| "记录已移入废纸篓，但来源读回不可用")?
        {
            return Err("记录已移入废纸篓，但读回发现变化；请核对原件，不自动回移".into());
        }
        Ok(Applied {id,name,notice:"记录与保留包已移入废纸篓，目标技能及启停配置不变。可手动取回目录；应用内该条恢复已移除。".into()})
    }
    pub(crate) fn restore(&self, id: &str, expected: &str) -> Result<Applied, String> {
        let roots = self.roots(false)?;
        let state = self.recovery(&roots, id)?;
        if state.state != "applied" || state.revision != expected {
            return Err("恢复对象已变化，请刷新；未覆盖文件".into());
        }
        let (directory, receipt) = Self::receipt(&roots, id)?;
        let skills = self
            .skills_root(receipt.target, false)?
            .ok_or("恢复目标目录已变化")?;
        self.verify_skills_root(receipt.target, &skills)
            .and_then(|_| self.verify_records_root(&roots))
            .map_err(|_| "恢复目标或记录目录链已变化，未恢复；请重新核对")?;
        if self.recovery(&roots, id)?.revision != expected {
            return Err("恢复对象在核对期间变化，未恢复".into());
        }
        if receipt.before.is_some() {
            files::publish(&directory, "package", &skills, &receipt.name, true)?;
        } else {
            files::publish(&skills, &receipt.name, &directory, "package", false)?;
        }
        self.verify_skills_root(receipt.target, &skills)
            .and_then(|_| self.verify_records_root(&roots))
            .map_err(|_| "已执行技能恢复，但当前目标或记录目录链已变化；请核对原件，不自动回移")?;
        let after = self
            .recovery(&roots, id)
            .map_err(|_| "已执行技能恢复，但读回不可用；请核对目录及安装记录")?;
        if after.state != "inactive" {
            return Err("已执行恢复，但读回发现变化，请核对保留目录".into());
        }
        Ok(Applied {
            id: id.into(),
            name: receipt.name,
            notice: format!(
                "{} 技能目录已恢复，本次包仍保留。启停配置未改动，请在客户端核对加载。",
                receipt.target.label()
            ),
        })
    }
    pub(crate) fn restore_preview(&self, id: &str, expected: &str) -> Result<Preview, String> {
        let roots = self.roots(false)?;
        let state = self.recovery(&roots, id)?;
        if state.state != "applied" || state.revision != expected {
            return Err("恢复对象已变化，请刷新记录".into());
        }
        let (directory, receipt) = Self::receipt(&roots, id)?;
        let skills = self
            .skills_root(receipt.target, false)?
            .ok_or("恢复目标目录已变化")?;
        let live = files::scan(&files::child(&skills, &receipt.name, true)?)?;
        let kept = if receipt.before.is_some() {
            files::scan(&files::child(&directory, "package", true)?)?
        } else {
            Tree {
                nodes: std::collections::BTreeMap::new(),
                files: 0,
                bytes: 0,
                revision: String::new(),
            }
        };
        let mut changes = differences(Some(&live), &kept);
        let change_count = changes.len();
        changes.truncate(100);
        Ok(Preview {plan_id:state.revision,name:receipt.name,action:if receipt.before.is_some(){"restore"}else{"remove"}.into(),
            files:kept.files,bytes:kept.bytes,changes,change_count,expires_ms:i64::MAX,target:Some(receipt.target),source:None,
            notice:"恢复前再次核实当前与保留目录；检测到后续修改就拒绝。本次包移回保留目录，启停配置不变。".into()})
    }
}

#[cfg(test)]
#[path = "skills_package_tests.rs"]
mod tests;
