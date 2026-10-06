//! Immutable source-event references; transcript bodies are read on demand and never stored.
use crate::private_text::known_private;
use crate::{
    models::AgentProfile,
    session::Signal,
    tasks::{Artifact, AttentionKind, Data, Source},
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
const MAX_BODY: usize = crate::claude_plan_capture::MAX_BODY;
const MAX_ENTRIES: usize = 10_000;
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Evidence {
    pub event_ref: String,
    pub kind: AttentionKind,
}
impl Evidence {
    pub fn title(&self) -> &'static str {
        match self.kind {
            AttentionKind::Answer => "来源问题",
            AttentionKind::PlanApproval => "来源方案",
            _ => "本轮结果",
        }
    }
    pub fn valid(&self) -> bool {
        matches!(
            self.kind,
            AttentionKind::Answer | AttentionKind::PlanApproval | AttentionKind::ResultReview
        ) && valid_ref(&self.event_ref)
    }
}
fn valid_ref(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|c| c.is_ascii_hexdigit())
}
fn hash(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}
// Intentionally lacks Debug: source text must not enter diagnostics.
#[derive(Serialize)]
pub struct Content {
    pub available: bool,
    pub text: Option<String>,
    pub notice: String,
    pub read_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transient_valid_for_ms: Option<u64>,
}
fn unavailable(notice: &str) -> Content {
    Content {
        available: false,
        text: None,
        notice: notice.into(),
        read_at_ms: crate::tokens::now_ms(),
        transient_valid_for_ms: None,
    }
}
struct Record {
    evidence: Evidence,
    signal_key: String,
    content: Content,
}

fn readable(text: String, secret: bool) -> Content {
    if secret || known_private(&text) {
        return unavailable("内容涉及私密输入或凭据，请在来源工具中查看。");
    }
    if text.len() > MAX_BODY {
        return unavailable("内容超过显示范围，请在来源工具中查看。");
    }
    if text.trim().is_empty() {
        return unavailable("此事件没有可展示的内容，请打开来源。");
    }
    Content {
        available: true,
        text: Some(text),
        notice: "来源记录 · 只读，打开不会回答或批准操作。".into(),
        read_at_ms: crate::tokens::now_ms(),
        transient_valid_for_ms: None,
    }
}
fn record(line: &str, path: &str, include_body: bool) -> Option<Record> {
    let doc: Value = serde_json::from_str(line).ok()?;
    record_doc(&doc, path, include_body)
}
fn record_doc(doc: &Value, path: &str, include_body: bool) -> Option<Record> {
    let record_type = doc.get("type")?.as_str()?;
    if record_type == "assistant" {
        return claude_tools(doc).find_map(|tool| claude_record(tool, path, include_body));
    }
    let payload = doc.get("payload")?;
    let kind = payload.get("type")?.as_str()?;
    let (key, artifact_kind, content) = match (record_type, kind) {
        ("response_item", "function_call")
            if payload.get("name")?.as_str()? == "request_user_input" =>
        {
            let id = payload.get("call_id")?.as_str()?;
            if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
                return None;
            }
            let args: Value = serde_json::from_str(payload.get("arguments")?.as_str()?).ok()?;
            let questions = args.get("questions")?.as_array()?;
            if questions.is_empty() || questions.len() > 10 {
                return None;
            }
            let secret = questions.iter().any(|q| {
                q.get("isSecret")
                    .or_else(|| q.get("is_secret"))
                    .and_then(Value::as_bool)
                    == Some(true)
            });
            let mut body = String::new();
            for question in questions {
                let question_text = question.get("question")?.as_str()?;
                if include_body && !secret {
                    body.push_str(question_text);
                    body.push('\n');
                }
                if let Some(options) = question.get("options").and_then(Value::as_array) {
                    if options.len() > 20 {
                        return None;
                    }
                    for option in options {
                        let label = option.get("label")?.as_str()?;
                        if include_body && !secret {
                            body.push_str("• ");
                            body.push_str(label);
                        }
                        if let Some(description) = option.get("description").and_then(Value::as_str)
                        {
                            if include_body && !secret {
                                body.push_str(" — ");
                                body.push_str(description);
                            }
                        }
                        if include_body && !secret {
                            body.push('\n');
                        }
                    }
                }
                if include_body && !secret {
                    body.push('\n');
                }
            }
            (
                id,
                AttentionKind::Answer,
                if include_body {
                    readable(body.trim_end().to_string(), secret)
                } else {
                    unavailable("按需读取来源内容。")
                },
            )
        }
        ("event_msg", "task_complete") => {
            if payload.get("error").is_some_and(|error| !error.is_null()) {
                return None;
            }
            let turn = payload.get("turn_id")?.as_str()?;
            if turn.is_empty() || turn.len() > 256 || turn.chars().any(char::is_control) {
                return None;
            }
            let text = payload.get("last_agent_message")?.as_str()?;
            if text.trim().is_empty() {
                return None;
            }
            (
                turn,
                AttentionKind::ResultReview,
                if !include_body {
                    unavailable("按需读取来源内容。")
                } else if text.len() > MAX_BODY {
                    unavailable("内容超过显示范围，请在来源工具中查看。")
                } else {
                    readable(text.to_string(), false)
                },
            )
        }
        _ => return None,
    };
    let event_ref = hash(&serde_json::to_vec(payload).ok()?);
    Some(Record {
        evidence: Evidence {
            event_ref,
            kind: artifact_kind,
        },
        signal_key: crate::session::fingerprint(path, key),
        content,
    })
}
/// Only the record that actually produced the selected semantic signal can create a reference.
pub fn selected(lines: &[String], path: &str, signal: Option<&Signal>) -> Option<Evidence> {
    selected_with_capture(lines, path, signal, None)
}
/// The receiver lifecycle supplies an enabled cache; ordinary probes never enable collection.
fn selected_with_capture(
    lines: &[String],
    path: &str,
    signal: Option<&Signal>,
    mut capture: Option<&mut crate::claude_plan_capture::Store>,
) -> Option<Evidence> {
    let (key, attention) = match signal? {
        Signal::Attention(key, _) => (key, true),
        Signal::Completed(key) => (key, false),
        _ => return None,
    };
    for line in lines.iter().rev() {
        if let Some(record) = records(line, path, false).into_iter().find(|record| {
            &record.signal_key == key
                && if attention {
                    matches!(
                        record.evidence.kind,
                        AttentionKind::Answer | AttentionKind::PlanApproval
                    )
                } else {
                    record.evidence.kind == AttentionKind::ResultReview
                }
        }) {
            return Some(record.evidence);
        }
        if !attention {
            continue;
        }
        let Some(cache) = capture.as_deref_mut() else {
            continue;
        };
        let Ok(doc) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        for tool in claude_tools(&doc) {
            let Some(id) = tool.get("id").and_then(Value::as_str) else {
                continue;
            };
            if crate::session::fingerprint(path, id) != *key
                || claude_kind(tool) != Some(AttentionKind::PlanApproval)
            {
                continue;
            }
            if let Some(event_ref) = cache.bind_selected(
                std::path::Path::new(path),
                &doc,
                tool,
                std::time::Instant::now(),
            ) {
                return Some(Evidence {
                    event_ref,
                    kind: AttentionKind::PlanApproval,
                });
            }
        }
    }
    None
}
pub(crate) fn selected_plan_version(
    lines: &[String],
    path: &str,
    signal: Option<&Signal>,
    roots: &[std::path::PathBuf],
) -> Result<Option<String>, ()> {
    let Some(Signal::Attention(key, _)) = signal else {
        return Ok(None);
    };
    for line in lines.iter().rev() {
        let Ok(doc) = crate::claude_plan_capture::strict_json(line.as_bytes()) else {
            continue;
        };
        for tool in claude_tools(&doc) {
            let Some(id) = tool.get("id").and_then(Value::as_str) else {
                continue;
            };
            if crate::session::fingerprint(path, id) != *key
                || claude_kind(tool) != Some(AttentionKind::PlanApproval)
            {
                continue;
            }
            if tool
                .get("input")
                .and_then(|v| v.get("plan"))
                .and_then(Value::as_str)
                .is_some_and(|s| !s.trim().is_empty())
            {
                return Ok(None);
            }
            // Legacy records without the hook's required main-message identities stay on their old path.
            if !["uuid", "sessionId"].iter().all(|key| {
                doc.get(key).and_then(Value::as_str).is_some_and(|s| {
                    uuid::Uuid::parse_str(s)
                        .is_ok_and(|u| !u.is_nil() && u.hyphenated().to_string() == s)
                })
            }) {
                return Ok(None);
            }
            return crate::claude_plan_capture::event_version(
                std::path::Path::new(path),
                &doc,
                tool,
                roots,
            )
            .map(|v| Some(v))
            .ok_or(());
        }
    }
    Ok(None)
}
/// Overlay only a freshly reverified version of the already selected semantic event.
pub(crate) fn attach_capture(
    context: &mut crate::models::SessionActiveContext,
    signal: Option<&Signal>,
    roots: &[std::path::PathBuf],
    capture: Option<&mut crate::claude_plan_capture::Store>,
) {
    if context.artifact.is_some()
        || context.plan_identity_unavailable
        || context.plan_event_version.is_none()
        || capture.is_none()
    {
        return;
    }
    let Some(path) = context.source_path.as_deref() else {
        return;
    };
    let Ok(lines) = crate::session::read_tail_lines_transient(path) else {
        return;
    };
    if selected_plan_version(&lines, path, signal, roots)
        .ok()
        .flatten()
        != context.plan_event_version
    {
        return;
    }
    context.artifact = selected_with_capture(&lines, path, signal, capture);
}
fn claude_tools(doc: &Value) -> impl Iterator<Item = &Value> {
    let main = doc.get("type").and_then(Value::as_str) == Some("assistant")
        && doc.get("isSidechain").and_then(Value::as_bool) != Some(true);
    doc.get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
        .filter(move |item| main && item.get("type").and_then(Value::as_str) == Some("tool_use"))
}
fn records(line: &str, path: &str, include_body: bool) -> Vec<Record> {
    let Ok(doc) = serde_json::from_str::<Value>(line) else {
        return vec![];
    };
    if doc.get("type").and_then(Value::as_str) == Some("assistant") {
        claude_tools(&doc)
            .filter_map(|tool| claude_record(tool, path, include_body))
            .collect()
    } else {
        record_doc(&doc, path, include_body).into_iter().collect()
    }
}
fn claude_kind(tool: &Value) -> Option<AttentionKind> {
    match tool.get("name")?.as_str()? {
        "AskUserQuestion" => Some(AttentionKind::Answer),
        "ExitPlanMode" => Some(AttentionKind::PlanApproval),
        _ => None,
    }
}
pub fn claude_attention_kind(
    lines: &[String],
    path: &str,
    signal: Option<&Signal>,
) -> Option<AttentionKind> {
    let Signal::Attention(key, _) = signal? else {
        return None;
    };
    lines.iter().rev().find_map(|line| {
        let doc: Value = serde_json::from_str(line).ok()?;
        let matched = claude_tools(&doc).find_map(|tool| {
            let id = tool.get("id")?.as_str()?;
            if id.is_empty()
                || id.len() > 256
                || id.chars().any(char::is_control)
                || crate::session::fingerprint(path, id) != *key
            {
                return None;
            }
            claude_kind(tool)
        });
        matched
    })
}
fn claude_record(tool: &Value, path: &str, include_body: bool) -> Option<Record> {
    let kind = claude_kind(tool)?;
    let id = tool.get("id")?.as_str()?;
    if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
        return None;
    }
    let input = tool.get("input")?;
    let content = if kind == AttentionKind::PlanApproval {
        // Hook-injected plan data is not necessarily in the transcript. Never read a guessed plan file.
        let plan = input.get("plan")?.as_str()?;
        if plan.trim().is_empty() {
            return None;
        }
        if !include_body {
            unavailable("按需读取来源内容。")
        } else if plan.len() > MAX_BODY {
            unavailable("内容超过显示范围，请在来源工具中查看。")
        } else {
            readable(
                plan.to_string(),
                input
                    .get("isSecret")
                    .or_else(|| input.get("is_secret"))
                    .and_then(Value::as_bool)
                    == Some(true),
            )
        }
    } else {
        let qs = input.get("questions")?.as_array()?;
        if qs.is_empty() || qs.len() > 4 {
            return None;
        }
        let secret = input
            .get("isSecret")
            .or_else(|| input.get("is_secret"))
            .and_then(Value::as_bool)
            == Some(true)
            || qs.iter().any(|q| {
                q.get("isSecret")
                    .or_else(|| q.get("is_secret"))
                    .and_then(Value::as_bool)
                    == Some(true)
            });
        let mut body = String::new();
        for q in qs {
            let question = q.get("question")?.as_str()?;
            let options = q.get("options")?.as_array()?;
            if options.len() > 20 {
                return None;
            }
            if include_body && !secret {
                body.push_str(question);
                body.push('\n');
                if q.get("multiSelect").and_then(Value::as_bool) == Some(true) {
                    body.push_str("可多选\n");
                }
            }
            for option in options {
                let label = option.get("label")?.as_str()?;
                if include_body && !secret {
                    body.push_str("• ");
                    body.push_str(label);
                    if let Some(d) = option.get("description").and_then(Value::as_str) {
                        body.push_str(" — ");
                        body.push_str(d);
                    }
                    body.push('\n');
                }
            }
            if body.len() > MAX_BODY {
                return Some(Record {
                    evidence: Evidence {
                        event_ref: hash(&serde_json::to_vec(tool).ok()?),
                        kind,
                    },
                    signal_key: crate::session::fingerprint(path, id),
                    content: unavailable("内容超过显示范围，请在来源工具中查看。"),
                });
            }
        }
        if include_body {
            readable(body.trim_end().to_string(), secret)
        } else {
            unavailable("按需读取来源内容。")
        }
    };
    Some(Record {
        evidence: Evidence {
            event_ref: hash(&serde_json::to_vec(tool).ok()?),
            kind,
        },
        signal_key: crate::session::fingerprint(path, id),
        content,
    })
}
fn read_path(path: &std::path::Path, event_ref: &str, kind: AttentionKind) -> Content {
    read_path_with_capture(path, event_ref, kind, None)
}
fn read_path_with_capture(
    path: &std::path::Path,
    event_ref: &str,
    kind: AttentionKind,
    mut capture: Option<&mut crate::claude_plan_capture::Store>,
) -> Content {
    if !std::fs::symlink_metadata(path)
        .is_ok_and(|meta| meta.is_file() && !meta.file_type().is_symlink())
    {
        return unavailable("来源文件身份已变化，请打开来源。");
    }
    let Some(path) = path.to_str() else {
        return unavailable("来源路径无法读取，请打开来源。");
    };
    let Ok(lines) = crate::session::read_tail_lines_transient(path) else {
        return unavailable("来源记录无法读取，请打开来源。");
    };
    for line in lines.iter().rev() {
        let Ok(doc) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if doc.get("type").and_then(Value::as_str) == Some("assistant") {
            for tool in claude_tools(&doc) {
                if claude_record(tool, path, false).is_some_and(|record| {
                    record.evidence.event_ref == event_ref && record.evidence.kind == kind
                }) {
                    if let Some(record) = claude_record(tool, path, true) {
                        return record.content;
                    }
                }
                if kind == AttentionKind::PlanApproval {
                    if let Some((body, remaining)) = capture.as_deref_mut().and_then(|cache| {
                        cache.read_bound_with_ttl(
                            std::path::Path::new(path),
                            &doc,
                            tool,
                            event_ref,
                            std::time::Instant::now(),
                        )
                    }) {
                        let mut content = readable(body, false);
                        if content.available {
                            content.transient_valid_for_ms = Some(remaining);
                            content.notice = "来源方案 · 只读，正文临时保留，打开不会批准。".into();
                        }
                        return content;
                    }
                }
            }
        } else if record_doc(&doc, path, false).is_some_and(|record| {
            record.evidence.event_ref == event_ref && record.evidence.kind == kind
        }) {
            if let Some(record) = record_doc(&doc, path, true) {
                return record.content;
            }
        }
    }
    if kind == AttentionKind::PlanApproval {
        return unavailable("对应方案正文不可用，可能已过期、停止采集或来源变化；请打开来源。");
    }
    unavailable("对应版本未在可读范围内，可能已变更或移出尾窗；请打开来源。")
}
/// Bound directory traversal; no caller-provided path or newest-session substitution.
fn locate(profile: &AgentProfile, source: &Source) -> Option<std::path::PathBuf> {
    if !matches!(profile.id.as_str(), "codex" | "claude") || source.agent_id != profile.id {
        return None;
    }
    let mut count = 0;
    for root in &profile.session_dirs {
        let root = std::path::Path::new(root);
        let Ok(meta) = std::fs::symlink_metadata(root) else {
            continue;
        };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            continue;
        }
        for entry in walkdir::WalkDir::new(root)
            .max_depth(5)
            .follow_links(false)
            .into_iter()
        {
            count += 1;
            if count > MAX_ENTRIES {
                return None;
            }
            let Ok(entry) = entry else {
                continue;
            };
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|s| s.to_str()) != Some("jsonl")
            {
                continue;
            }
            let Some(path) = entry.path().to_str() else {
                continue;
            };
            if hash(path.as_bytes()) == source.session_id {
                return Some(entry.path().to_path_buf());
            }
        }
    }
    None
}
pub fn read(
    data: &Data,
    task: &str,
    run: &str,
    artifact: &str,
    revision: u64,
    profile: &AgentProfile,
) -> Result<Content, String> {
    read_with_capture(data, task, run, artifact, revision, profile, None)
}
pub(crate) fn read_with_capture(
    data: &Data,
    task: &str,
    run: &str,
    artifact: &str,
    revision: u64,
    profile: &AgentProfile,
    capture: Option<&mut crate::claude_plan_capture::Store>,
) -> Result<Content, String> {
    let source = data
        .navigation_source(task, Some(run), Some(artifact), revision)
        .map_err(|error| error.message)?;
    let artifact: &Artifact = data
        .artifacts
        .iter()
        .find(|a| a.id == artifact)
        .ok_or("产出引用不存在")?;
    let Some(event_ref) = artifact
        .event_ref
        .as_deref()
        .filter(|value| valid_ref(value))
    else {
        return Ok(unavailable("这条本地记录没有可读取的来源内容。"));
    };
    let Some(path) = locate(profile, source) else {
        return Ok(unavailable("来源文件未找到或查找范围已用尽，请打开来源。"));
    };
    Ok(read_path_with_capture(
        &path,
        event_ref,
        artifact.kind,
        capture,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SessionDialect;
    fn line(payload: Value, type_: &str) -> String {
        serde_json::json!({"type":type_,"payload":payload}).to_string()
    }
    fn question(id: &str, text: &str, secret: bool) -> String {
        line(
            serde_json::json!({"type":"function_call","name":"request_user_input","call_id":id,"arguments":serde_json::json!({"questions":[{"id":"q","question":text,"isSecret":secret,"options":[{"label":"继续","description":"保留当前方向"}]}]}).to_string()}),
            "response_item",
        )
    }
    fn result(turn: &str, text: &str) -> String {
        line(
            serde_json::json!({"type":"task_complete","turn_id":turn,"last_agent_message":text}),
            "event_msg",
        )
    }
    struct Fixture {
        sandbox: crate::testutil::Sandbox,
        path: std::path::PathBuf,
        profile: AgentProfile,
        store: crate::tasks::Store,
    }
    impl Fixture {
        fn new() -> Self {
            let sandbox = crate::testutil::Sandbox::new("task-artifact-source");
            let path = sandbox
                .path()
                .join("rollout-019c6e27-e55b-73d1-87d8-4e01f1f75043.jsonl");
            let mut profile = crate::registry::builtin()
                .into_iter()
                .find(|p| p.id == "codex")
                .unwrap();
            profile.session_dirs = vec![sandbox.path().to_string_lossy().into()];
            let store = crate::tasks::Store {
                path: sandbox.path().join("tasks.json"),
            };
            Self {
                sandbox,
                path,
                profile,
                store,
            }
        }
        fn observe(&self, lines: &[String]) -> crate::task_sources::Observation {
            std::fs::write(&self.path, lines.join("\n") + "\n").unwrap();
            let (probe, mut context) = crate::session::probe_dialect(
                &self.profile.id,
                SessionDialect::GenericTail,
                self.path.to_str().unwrap(),
            );
            context.source_path = Some(self.path.to_string_lossy().into());
            let roots = self
                .profile
                .session_dirs
                .iter()
                .map(std::path::PathBuf::from)
                .collect::<Vec<_>>();
            let version = selected_plan_version(
                lines,
                self.path.to_str().unwrap(),
                probe.signal.as_ref(),
                &roots,
            );
            context.plan_identity_unavailable = version.is_err();
            context.plan_event_version = version.ok().flatten();
            crate::task_sources::from_context(&self.profile, &context, probe.signal.as_ref())
                .unwrap()
                .observation
        }
        fn bind(&self, source: Source) -> Data {
            let d = self.store.create("任务", None, 0, 1).unwrap();
            self.store
                .link(&d.tasks[0].id, source, d.revision, 2)
                .unwrap()
        }
        fn read(&self, d: &Data, index: usize) -> Content {
            let a = &d.artifacts[index];
            read(
                d,
                &d.tasks[0].id,
                &a.run_id,
                &a.id,
                d.revision,
                &self.profile,
            )
            .unwrap()
        }
    }
    #[test]
    fn actual_probe_sync_reference_and_on_demand_read_preserve_question_and_result_history() {
        let f = Fixture::new();
        let q = question("call-a", "选择下一步怎么做？", false);
        let done = result("turn-a", "实现已完成\n请检查运行结果。");
        let observation = f.observe(&[q.clone()]);
        f.bind(observation.source.clone());
        let d = f.store.sync(&[observation.clone()], 3).unwrap().unwrap();
        assert_eq!(d.artifacts.len(), 1);
        assert_eq!(
            d.attentions[0].artifact_id.as_deref(),
            Some(d.artifacts[0].id.as_str())
        );
        assert!(f.store.sync(&[observation], 4).unwrap().is_none());
        assert!(f.read(&d, 0).text.unwrap().contains("选择下一步"));
        let serialized = std::fs::read_to_string(&f.store.path).unwrap();
        assert!(!serialized.contains("选择下一步"));
        assert!(!serialized.contains("call-a"));
        assert!(!serialized.contains(f.path.to_str().unwrap()));
        let closed = line(
            serde_json::json!({"type":"function_call_output","call_id":"call-a","output":"已回答"}),
            "response_item",
        );
        let o = f.observe(&[q.clone(), closed.clone(), done.clone()]);
        let d = f.store.sync(&[o], 5).unwrap().unwrap();
        assert_eq!(d.artifacts.len(), 2);
        assert!(d
            .attentions
            .iter()
            .all(|a| a.state != crate::tasks::AttentionState::Open));
        assert!(d
            .runs
            .iter()
            .all(|r| r.status != crate::tasks::RunStatus::Accepted));
        assert!(f.read(&d, 1).text.unwrap().contains("实现已完成"));
        let next = question("call-b", "第二轮待回答", false);
        let o = f.observe(&[q, closed, done, next]);
        let d = f.store.sync(&[o], 6).unwrap().unwrap();
        assert_eq!(d.runs.len(), 2);
        assert_eq!(d.artifacts[0].run_id, d.artifacts[1].run_id);
        assert_ne!(d.artifacts[1].run_id, d.artifacts[2].run_id);
        assert!(f.read(&d, 0).text.unwrap().contains("选择下一步"));
        d.validate().unwrap();
    }
    #[test]
    fn reference_version_ownership_and_source_loss_never_substitute_latest_content() {
        let f = Fixture::new();
        let o = f.observe(&[question("call", "原版本", false)]);
        f.bind(o.source.clone());
        let d = f.store.sync(&[o], 3).unwrap().unwrap();
        let changed = f.observe(&[question("call", "修改后的问题", false)]);
        let next = f.store.sync(&[changed], 4).unwrap().unwrap();
        assert_eq!(next.artifacts.len(), 2);
        assert!(!f.read(&next, 0).available);
        assert!(f.read(&next, 1).text.unwrap().contains("修改后的问题"));
        let a = &next.artifacts[1];
        assert!(read(
            &next,
            &next.tasks[0].id,
            &a.run_id,
            &a.id,
            d.revision,
            &f.profile
        )
        .is_err());
        assert!(read(
            &next,
            &next.tasks[0].id,
            "foreign-run",
            &a.id,
            next.revision,
            &f.profile
        )
        .is_err());
        let d = f.store.create("其他任务", None, next.revision, 5).unwrap();
        assert!(read(&d, &d.tasks[1].id, &a.run_id, &a.id, d.revision, &f.profile).is_err());
        std::fs::remove_file(&f.path).unwrap();
        assert!(!f.read(&d, 1).available);
        std::fs::write(
            f.sandbox.path().join("different.jsonl"),
            result("other", "不相关结果"),
        )
        .unwrap();
        assert!(!f.read(&d, 1).available);
    }
    #[test]
    fn secret_oversized_and_malformed_records_are_never_exposed_or_guessed() {
        let private = question("call", "私密问题", true);
        let r = record(&private, "fixture", true).unwrap();
        assert!(!r.content.available);
        assert!(r.content.text.is_none());
        let normal = record(
            &result("turn", "task-references 使用 api_key 环境变量名称"),
            "fixture",
            true,
        )
        .unwrap();
        assert!(normal.content.available, "identifiers are not credentials");
        let fake = ["sk", "-", "not-a-real-key-with-long-value"].concat();
        assert!(
            !record(&result("turn", &fake), "fixture", true)
                .unwrap()
                .content
                .available
        );
        assert!(
            !record(&result("turn", "password=fixture-value"), "fixture", true)
                .unwrap()
                .content
                .available
        );
        assert!(
            !record(&result("turn", &"字".repeat(MAX_BODY)), "fixture", true)
                .unwrap()
                .content
                .available
        );
        for payload in [
            serde_json::json!({"type":"task_complete","turn_id":"t"}),
            serde_json::json!({"type":"task_complete","turn_id":"t","last_agent_message":"failed","error":{}}),
            serde_json::json!({"type":"task_complete","last_agent_message":"unbound"}),
        ] {
            assert!(record(&line(payload, "event_msg"), "fixture", true).is_none());
        }
        assert!(selected(
            &[private],
            "fixture",
            Some(&Signal::Active("call".into(), None))
        )
        .is_none());
    }
    #[test]
    fn invalid_reference_and_symlink_source_cannot_read_or_mutate_records() {
        let f = Fixture::new();
        let o = f.observe(&[result("turn", "安全结果")]);
        f.bind(o.source.clone());
        let mut d = f.store.sync(&[o], 3).unwrap().unwrap();
        let before = std::fs::read(&f.store.path).unwrap();
        let content = f.read(&d, 0);
        assert!(content.available);
        assert_eq!(before, std::fs::read(&f.store.path).unwrap());
        d.artifacts[0].event_ref = Some("invalid".into());
        assert!(d.validate().is_err());
        #[cfg(unix)]
        {
            std::fs::remove_file(&f.path).unwrap();
            let external = f.sandbox.path().join("other.jsonl");
            std::fs::write(&external, result("turn", "安全结果")).unwrap();
            std::os::unix::fs::symlink(external, &f.path).unwrap();
            assert!(!f.read(&f.store.load().unwrap(), 0).available);
        }
    }
    #[cfg(unix)]
    #[test]
    fn captured_plan_overlay_follows_actual_probe_and_persists_only_its_version_reference() {
        let mut f = claude_fixture();
        let root = f.sandbox.path().canonicalize().unwrap();
        let session = "019c6e27-e55b-73d1-87d8-4e01f1f75043";
        let message = "019c7714-3b77-74d1-9866-e1f484aae2ab";
        f.path = root.join(format!("{session}.jsonl"));
        f.profile.session_dirs = vec![root.to_string_lossy().into()];
        let plan=serde_json::json!({"type":"assistant","uuid":message,"sessionId":session,"isSidechain":false,"message":{"content":[{"type":"tool_use","id":"p","name":"ExitPlanMode","input":{}}]}}).to_string();
        let mut o = f.observe(&[plan.clone()]);
        assert!(o.artifact.is_none());
        f.bind(o.source.clone());
        let mut cache = crate::claude_plan_capture::Store::default();
        cache.enable(vec![root]).unwrap();
        let payload = serde_json::json!({"hook_event_name":"PreToolUse","tool_name":"ExitPlanMode","session_id":session,"transcript_path":f.path,"tool_use_id":"p","tool_input":{"plan":"# 只读采集方案\n复核后再实施。","planFilePath":"/never/read/plan.md"}});
        cache
            .ingest(
                &serde_json::to_vec(&payload).unwrap(),
                std::time::Instant::now(),
            )
            .unwrap();
        assert!(selected_with_capture(
            &[plan.clone()],
            f.path.to_str().unwrap(),
            Some(&Signal::Active("p".into(), None)),
            Some(&mut cache)
        )
        .is_none());
        let (probe, mut context) = crate::session::probe_dialect(
            "claude",
            SessionDialect::GenericTail,
            f.path.to_str().unwrap(),
        );
        context.artifact = selected_with_capture(
            &[plan.clone()],
            f.path.to_str().unwrap(),
            probe.signal.as_ref(),
            Some(&mut cache),
        );
        assert!(context.artifact.is_some());
        context.source_path = Some(f.path.to_string_lossy().into());
        context.attention_kind = Some(AttentionKind::PlanApproval);
        context.plan_identity_unavailable = false;
        context.plan_event_version = selected_plan_version(
            &[plan.clone()],
            f.path.to_str().unwrap(),
            probe.signal.as_ref(),
            &[f.sandbox.path().canonicalize().unwrap()],
        )
        .unwrap();
        o = crate::task_sources::from_context(&f.profile, &context, probe.signal.as_ref())
            .unwrap()
            .observation;
        let d = f.store.sync(&[o.clone()], 3).unwrap().unwrap();
        assert!(f.store.sync(&[o], 4).unwrap().is_none());
        let a = &d.artifacts[0];
        assert_eq!(a.kind, AttentionKind::PlanApproval);
        let r = a.event_ref.as_ref().unwrap();
        let content = read_path_with_capture(&f.path, r, a.kind, Some(&mut cache));
        assert!(content.text.unwrap().contains("只读采集方案"));
        assert!(d
            .runs
            .iter()
            .all(|r| r.status != crate::tasks::RunStatus::Accepted));
        let saved = std::fs::read_to_string(&f.store.path).unwrap();
        assert!(
            !saved.contains("只读采集方案")
                && !saved.contains("never/read")
                && !saved.contains(f.path.to_str().unwrap())
        );
        assert!(
            !read_path(&f.path, r, a.kind).available,
            "disabled receiver cannot pretend the persisted ref contains a body"
        );
        cache.disable();
        assert!(!read_path_with_capture(&f.path, r, a.kind, Some(&mut cache)).available);
    }
    #[cfg(unix)]
    fn captured_fixture() -> (Fixture, String, Value, crate::claude_plan_capture::Store) {
        let mut f = claude_fixture();
        let root = f.sandbox.path().canonicalize().unwrap();
        let session = "019c6e27-e55b-73d1-87d8-4e01f1f75043";
        f.path = root.join(format!("{session}.jsonl"));
        f.profile.session_dirs = vec![root.to_string_lossy().into()];
        let plan=serde_json::json!({"type":"assistant","uuid":"019c7714-3b77-74d1-9866-e1f484aae2ab","sessionId":session,"isSidechain":false,"message":{"content":[{"type":"tool_use","id":"p","name":"ExitPlanMode","input":{}}]}}).to_string();
        let payload = serde_json::json!({"hook_event_name":"PreToolUse","tool_name":"ExitPlanMode","session_id":session,"transcript_path":f.path,"tool_use_id":"p","tool_input":{"plan":"# 临时方案正文"}});
        let mut cache = crate::claude_plan_capture::Store::default();
        cache.enable(vec![root]).unwrap();
        (f, plan, payload, cache)
    }
    #[cfg(unix)]
    fn captured_observation(
        f: &Fixture,
        plan: &str,
        cache: &mut crate::claude_plan_capture::Store,
    ) -> crate::task_sources::Observation {
        let mut o = f.observe(&[plan.into()]);
        let (probe, _) = crate::session::probe_dialect(
            "claude",
            SessionDialect::GenericTail,
            f.path.to_str().unwrap(),
        );
        o.artifact = selected_with_capture(
            &[plan.into()],
            f.path.to_str().unwrap(),
            probe.signal.as_ref(),
            Some(cache),
        );
        o
    }
    #[cfg(unix)]
    #[test]
    fn late_capture_expiry_stop_eviction_and_restart_preserve_same_plan_gate() {
        let (f, plan, payload, mut cache) = captured_fixture();
        let initial = f.observe(&[plan.clone()]);
        f.bind(initial.source.clone());
        let initial_data = f.store.sync(&[initial.clone()], 3).unwrap().unwrap();
        let gate = initial_data.attentions[0].id.clone();
        let run = initial_data.runs[0].id.clone();
        cache
            .ingest(
                &serde_json::to_vec(&payload).unwrap(),
                std::time::Instant::now(),
            )
            .unwrap();
        let captured = captured_observation(&f, &plan, &mut cache);
        assert_eq!(initial.fingerprint, captured.fingerprint);
        let data = f.store.sync(&[captured.clone()], 4).unwrap().unwrap();
        assert_eq!(data.attentions.len(), 1);
        assert_eq!(data.runs.len(), 1);
        assert_eq!(data.attentions[0].id, gate);
        assert_eq!(data.runs[0].id, run);
        assert_eq!(data.event_receipts.len(), 1);
        let reference = data.artifacts[0].event_ref.as_deref().unwrap();
        assert!(
            read_path_with_capture(
                &f.path,
                reference,
                AttentionKind::PlanApproval,
                Some(&mut cache)
            )
            .available
        );
        assert!(f.store.sync(&[captured.clone()], 5).unwrap().is_none());
        cache.purge_expired(std::time::Instant::now() + std::time::Duration::from_secs(16 * 60));
        let unavailable = captured_observation(&f, &plan, &mut cache);
        assert!(unavailable.artifact.is_none());
        assert_eq!(unavailable.fingerprint, initial.fingerprint);
        assert!(f.store.sync(&[unavailable], 6).unwrap().is_none());
        assert!(
            !read_path_with_capture(
                &f.path,
                reference,
                AttentionKind::PlanApproval,
                Some(&mut cache)
            )
            .available
        );
        cache.disable();
        let stopped = captured_observation(&f, &plan, &mut cache);
        assert!(f.store.sync(&[stopped], 7).unwrap().is_none());
        let restarted = crate::tasks::Store {
            path: f.store.path.clone(),
        };
        assert!(restarted
            .sync(&[f.observe(&[plan.clone()])], 8)
            .unwrap()
            .is_none());
        cache
            .enable(vec![f.sandbox.path().canonicalize().unwrap()])
            .unwrap();
        cache
            .ingest(
                &serde_json::to_vec(&payload).unwrap(),
                std::time::Instant::now(),
            )
            .unwrap();
        for i in 0..33 {
            let mut p = payload.clone();
            p["tool_use_id"] = format!("other-{i}").into();
            cache
                .ingest(&serde_json::to_vec(&p).unwrap(), std::time::Instant::now())
                .unwrap();
        }
        let evicted = captured_observation(&f, &plan, &mut cache);
        assert!(evicted.artifact.is_none());
        assert!(restarted.sync(&[evicted], 9).unwrap().is_none());
        let saved = std::fs::read_to_string(&f.store.path).unwrap();
        assert!(!saved.contains("临时方案正文") && !saved.contains(f.path.to_str().unwrap()));
        assert_eq!(restarted.load().unwrap().attentions[0].id, gate);
    }
    #[cfg(unix)]
    #[test]
    fn handled_or_superseded_gate_never_adopts_late_body() {
        for superseded in [false, true] {
            let (f, plan, payload, mut cache) = captured_fixture();
            let initial = f.observe(&[plan.clone()]);
            f.bind(initial.source.clone());
            let data = f.store.sync(&[initial], 3).unwrap().unwrap();
            if superseded {
                f.store
                    .record(
                        &data.tasks[0].id,
                        crate::tasks::RunStatus::Ready,
                        None,
                        None,
                        data.revision,
                        4,
                    )
                    .unwrap();
            } else {
                f.store
                    .handled(
                        &data.tasks[0].id,
                        &data.runs[0].id,
                        &data.attentions[0].id,
                        data.revision,
                        4,
                    )
                    .unwrap();
            }
            cache
                .ingest(
                    &serde_json::to_vec(&payload).unwrap(),
                    std::time::Instant::now(),
                )
                .unwrap();
            let captured = captured_observation(&f, &plan, &mut cache);
            assert!(captured.artifact.is_some());
            assert!(f.store.sync(&[captured], 5).unwrap().is_none());
            let current = f.store.load().unwrap();
            assert!(current.artifacts.is_empty());
            assert!(current
                .attentions
                .iter()
                .all(|a| a.state != crate::tasks::AttentionState::Open));
        }
    }
    #[cfg(unix)]
    #[test]
    fn changed_capture_body_never_replaces_stored_exact_reference() {
        let (f, plan, payload, mut cache) = captured_fixture();
        let initial = f.observe(&[plan.clone()]);
        f.bind(initial.source.clone());
        f.store.sync(&[initial], 3).unwrap();
        cache
            .ingest(
                &serde_json::to_vec(&payload).unwrap(),
                std::time::Instant::now(),
            )
            .unwrap();
        let captured = captured_observation(&f, &plan, &mut cache);
        let data = f.store.sync(&[captured], 4).unwrap().unwrap();
        let original = data.artifacts[0].event_ref.clone().unwrap();
        cache.disable();
        cache
            .enable(vec![f.sandbox.path().canonicalize().unwrap()])
            .unwrap();
        let mut changed = payload;
        changed["tool_input"]["plan"] = "# 另一版本方案".into();
        cache
            .ingest(
                &serde_json::to_vec(&changed).unwrap(),
                std::time::Instant::now(),
            )
            .unwrap();
        let next = captured_observation(&f, &plan, &mut cache);
        assert_ne!(next.artifact.as_ref().unwrap().event_ref, original);
        assert!(f.store.sync(&[next], 5).unwrap().is_none());
        assert!(
            !read_path_with_capture(
                &f.path,
                &original,
                AttentionKind::PlanApproval,
                Some(&mut cache)
            )
            .available
        );
        assert_eq!(
            f.store.load().unwrap().artifacts[0].event_ref.as_deref(),
            Some(original.as_str())
        );
    }
    #[cfg(unix)]
    #[test]
    fn main_message_and_source_replacement_change_version_but_unverified_source_cannot_observe() {
        let (f, plan, _, _) = captured_fixture();
        let initial = f.observe(&[plan.clone()]);
        let mut changed: Value = serde_json::from_str(&plan).unwrap();
        changed["uuid"] = "019c6e27-e55b-73d1-87d8-4e01f1f75043".into();
        let next = f.observe(&[changed.to_string()]);
        assert_ne!(initial.fingerprint, next.fingerprint);
        let old = f.path.with_extension("moved");
        std::fs::rename(&f.path, &old).unwrap();
        let replaced = f.observe(&[plan.clone()]);
        assert_ne!(initial.fingerprint, replaced.fingerprint);
        std::fs::remove_file(&f.path).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&old, &f.path).unwrap();
        let signal = Some(Signal::Attention(
            crate::session::fingerprint(f.path.to_str().unwrap(), "p"),
            "确认方案".into(),
        ));
        let roots = vec![f.sandbox.path().canonicalize().unwrap()];
        assert!(
            selected_plan_version(&[plan], f.path.to_str().unwrap(), signal.as_ref(), &roots)
                .is_err()
        );
        let context = crate::models::SessionActiveContext {
            source_path: Some(f.path.to_string_lossy().into()),
            attention_kind: Some(AttentionKind::PlanApproval),
            plan_identity_unavailable: true,
            ..Default::default()
        };
        assert!(crate::task_sources::from_context(&f.profile, &context, signal.as_ref()).is_none());
    }
    #[cfg(unix)]
    #[test]
    fn older_plan_receipt_cannot_attach_body_to_newer_gate_in_same_run() {
        let (f, plan, payload, mut cache) = captured_fixture();
        let initial = f.observe(&[plan.clone()]);
        f.bind(initial.source.clone());
        let first = f.store.sync(&[initial], 3).unwrap().unwrap();
        let mut next: Value = serde_json::from_str(&plan).unwrap();
        next["uuid"] = "019c6e27-e55b-73d1-87d8-4e01f1f75043".into();
        next["message"]["content"][0]["id"] = "next".into();
        let newer = f.observe(&[next.to_string()]);
        let current = f.store.sync(&[newer], 4).unwrap().unwrap();
        assert_eq!(first.runs[0].id, current.runs[0].id);
        assert_eq!(current.attentions.len(), 2);
        cache
            .ingest(
                &serde_json::to_vec(&payload).unwrap(),
                std::time::Instant::now(),
            )
            .unwrap();
        let old = captured_observation(&f, &plan, &mut cache);
        assert!(old.artifact.is_some());
        assert!(f.store.sync(&[old], 5).unwrap().is_none());
        assert!(f.store.load().unwrap().artifacts.is_empty());
    }
    fn claude_line(id: &str, name: &str, input: Value) -> String {
        serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":id,"name":name,"input":input}]}}).to_string()
    }
    fn claude_fixture() -> Fixture {
        let mut f = Fixture::new();
        let mut p = crate::registry::builtin()
            .into_iter()
            .find(|p| p.id == "claude")
            .unwrap();
        p.session_dirs = f.profile.session_dirs.clone();
        f.profile = p;
        f
    }
    #[test]
    fn claude_questions_and_inline_plan_bind_specific_versions_without_approving() {
        let f = claude_fixture();
        let q = claude_line(
            "q",
            "AskUserQuestion",
            serde_json::json!({"questions":[{"question":"选哪种布局？","multiSelect":true,"options":[{"label":"并排","description":"两个窗口"}]}]}),
        );
        let o = f.observe(&[q.clone()]);
        assert_eq!(o.kind, Some(AttentionKind::Answer));
        f.bind(o.source.clone());
        let d = f.store.sync(&[o.clone()], 3).unwrap().unwrap();
        assert!(f.store.sync(&[o], 4).unwrap().is_none());
        let body = f.read(&d, 0).text.unwrap();
        assert!(body.contains("可多选"));
        assert!(body.contains("并排 — 两个窗口"));
        let plan = claude_line(
            "p",
            "ExitPlanMode",
            serde_json::json!({"plan":"# 实现方案\n先复核窗口身份，再预览布局。"}),
        );
        let o = f.observe(&[q.clone(), plan.clone()]);
        assert_eq!(o.kind, Some(AttentionKind::PlanApproval));
        let d = f.store.sync(&[o], 5).unwrap().unwrap();
        assert_eq!(d.artifacts.len(), 2);
        assert!(f.read(&d, 1).text.unwrap().contains("实现方案"));
        assert!(f.read(&d, 0).text.unwrap().contains("选哪种"));
        assert_eq!(
            d.attentions
                .iter()
                .filter(|a| a.state == crate::tasks::AttentionState::Open)
                .count(),
            1
        );
        assert!(d
            .runs
            .iter()
            .all(|r| r.status != crate::tasks::RunStatus::Accepted));
        let saved = std::fs::read_to_string(&f.store.path).unwrap();
        assert!(!saved.contains("实现方案") && !saved.contains("选哪种"));
        let changed = claude_line(
            "p",
            "ExitPlanMode",
            serde_json::json!({"plan":"# 修订方案\n先检查权限。"}),
        );
        let o = f.observe(&[q, changed]);
        let d = f.store.sync(&[o], 6).unwrap().unwrap();
        assert!(!f.read(&d, 1).available);
        assert!(f.read(&d, 2).text.unwrap().contains("修订方案"));
    }
    #[test]
    fn claude_plan_without_transcript_body_is_classified_but_never_reads_a_guessed_file() {
        let f = claude_fixture();
        let plan = claude_line(
            "p",
            "ExitPlanMode",
            serde_json::json!({"planFilePath":"/outside/plan.md","allowedPrompts":[]}),
        );
        let o = f.observe(&[plan]);
        assert_eq!(o.kind, Some(AttentionKind::PlanApproval));
        assert!(o.artifact.is_none());
        f.bind(o.source.clone());
        let (old_probe, _) = crate::session::probe_dialect(
            "claude",
            SessionDialect::GenericTail,
            f.path.to_str().unwrap(),
        );
        let old = crate::task_sources::from_signal(
            &f.profile,
            f.path.to_str(),
            old_probe.signal.as_ref(),
        )
        .unwrap()
        .observation;
        assert_ne!(old.fingerprint, o.fingerprint);
        f.store.sync(&[old], 3).unwrap().unwrap();
        let d = f.store.sync(&[o], 4).unwrap().unwrap();
        assert!(d.artifacts.is_empty());
        let open: Vec<_> = d
            .attentions
            .iter()
            .filter(|a| a.state == crate::tasks::AttentionState::Open)
            .collect();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].kind, AttentionKind::PlanApproval);
        let closed=serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"p","content":"not an approval signal"}]}}).to_string();
        let (probe, context) = crate::session::probe_dialect(
            "claude",
            SessionDialect::GenericTail,
            f.path.to_str().unwrap(),
        );
        assert!(context.artifact.is_none());
        let mut lines = vec![
            claude_line("p", "ExitPlanMode", serde_json::json!({})),
            closed,
        ];
        let (probe2, _) = {
            std::fs::write(&f.path, lines.join("\n")).unwrap();
            crate::session::probe_dialect(
                "claude",
                SessionDialect::GenericTail,
                f.path.to_str().unwrap(),
            )
        };
        assert!(!matches!(probe2.signal, Some(Signal::Attention(..))));
        assert!(matches!(probe.signal, Some(Signal::Attention(..))));
        lines.push(claude_line(
            "q",
            "AskUserQuestion",
            serde_json::json!({"questions":[]}),
        ));
        let o = f.observe(&lines);
        assert_eq!(o.kind, Some(AttentionKind::Answer));
        assert!(o.artifact.is_none());
    }
    #[test]
    fn claude_content_respects_private_inputs_and_selected_tool_identity() {
        let q = claude_line(
            "q",
            "AskUserQuestion",
            serde_json::json!({"questions":[{"question":"私密问题","isSecret":true,"options":[{"label":"继续"}]}]}),
        );
        assert!(!record(&q, "fixture", true).unwrap().content.available);
        let plan = claude_line(
            "p",
            "ExitPlanMode",
            serde_json::json!({"plan":"password=fixture-value"}),
        );
        assert!(!record(&plan, "fixture", true).unwrap().content.available);
        assert!(selected(
            &[plan.clone()],
            "fixture",
            Some(&Signal::Attention(
                crate::session::fingerprint("fixture", "other"),
                "".into()
            ))
        )
        .is_none());
        assert!(
            claude_attention_kind(&[plan], "fixture", Some(&Signal::Active("p".into(), None)))
                .is_none()
        );
    }
    fn batch(tools: Value, sidechain: bool) -> String {
        serde_json::json!({"type":"assistant","isSidechain":sidechain,"message":{"content":tools}})
            .to_string()
    }
    fn tool(id: &str, name: &str, input: Value) -> Value {
        serde_json::json!({"type":"tool_use","id":id,"name":name,"input":input})
    }
    fn reply(id: &str, sidechain: bool) -> String {
        serde_json::json!({"type":"user","isSidechain":sidechain,"message":{"content":[{"type":"tool_result","tool_use_id":id,"content":"done"}]}}).to_string()
    }
    #[test]
    fn claude_parallel_batch_selects_pending_human_gate_and_reads_exact_body() {
        let f = claude_fixture();
        let calls = batch(
            serde_json::json!([
                tool(
                    "question",
                    "AskUserQuestion",
                    serde_json::json!({"questions":[{"question":"需要哪种布局？","options":[{"label":"并排"}]}]})
                ),
                tool(
                    "plan",
                    "ExitPlanMode",
                    serde_json::json!({"plan":"# 对应方案"})
                ),
                tool(
                    "read",
                    "Read",
                    serde_json::json!({"file_path":"fixture.rs"})
                )
            ]),
            false,
        );
        let o = f.observe(&[calls.clone()]);
        assert_eq!(o.kind, Some(AttentionKind::PlanApproval));
        f.bind(o.source.clone());
        let d = f.store.sync(&[o], 3).unwrap().unwrap();
        assert_eq!(f.read(&d, 0).text.as_deref(), Some("# 对应方案"));
        let o = f.observe(&[calls.clone(), reply("plan", false), reply("read", false)]);
        assert_eq!(o.kind, Some(AttentionKind::Answer));
        let d = f.store.sync(&[o], 4).unwrap().unwrap();
        assert!(f.read(&d, 1).text.unwrap().contains("需要哪种布局"));
        assert_eq!(f.read(&d, 0).text.as_deref(), Some("# 对应方案"));
        let o = f.observe(&[calls, reply("plan", false), reply("question", false)]);
        assert!(o.kind.is_none());
        let saved = std::fs::read_to_string(&f.store.path).unwrap();
        assert!(!saved.contains("需要哪种布局") && !saved.contains("对应方案"));
    }
    #[test]
    fn claude_sidechains_and_older_results_cannot_replace_or_close_main_gate() {
        let f = claude_fixture();
        let main = claude_line(
            "same",
            "ExitPlanMode",
            serde_json::json!({"plan":"主会话方案"}),
        );
        let child = batch(
            serde_json::json!([tool(
                "same",
                "AskUserQuestion",
                serde_json::json!({"questions":[{"question":"子会话问题","options":[]}]})
            )]),
            true,
        );
        let o = f.observe(&[
            reply("same", false),
            main.clone(),
            child.clone(),
            reply("same", true),
        ]);
        assert_eq!(o.kind, Some(AttentionKind::PlanApproval));
        f.bind(o.source.clone());
        let d = f.store.sync(&[o], 3).unwrap().unwrap();
        assert_eq!(f.read(&d, 0).text.as_deref(), Some("主会话方案"));
        std::fs::write(&f.path, [main, child, reply("same", false)].join("\n")).unwrap();
        let (probe, _) = crate::session::probe_dialect(
            "claude",
            SessionDialect::GenericTail,
            f.path.to_str().unwrap(),
        );
        assert!(probe.signal.is_none());
    }
    #[test]
    fn claude_private_question_hint_and_body_remain_generic() {
        let f = claude_fixture();
        for input in [
            serde_json::json!({"is_secret":true,"questions":[{"question":"私密选择","options":[]}]}),
            serde_json::json!({"questions":[{"question":"password=fixture-value","options":[]}]}),
        ] {
            let o = f.observe(&[claude_line("private", "AskUserQuestion", input)]);
            assert_eq!(o.kind, Some(AttentionKind::Answer));
            let (probe, _) = crate::session::probe_dialect(
                "claude",
                SessionDialect::GenericTail,
                f.path.to_str().unwrap(),
            );
            assert!(
                matches!(probe.signal,Some(Signal::Attention(_,ref hint)) if hint=="请回答问题")
            );
            let evidence = o.artifact.unwrap();
            assert!(!read_path(&f.path, &evidence.event_ref, evidence.kind).available);
        }
        let o = f.observe(&[claude_line(
            "private",
            "ExitPlanMode",
            serde_json::json!({"isSecret":true,"plan":"私密方案"}),
        )]);
        let evidence = o.artifact.unwrap();
        assert!(!read_path(&f.path, &evidence.event_ref, evidence.kind).available);
    }

    #[test]
    fn claude_main_interrupt_cancels_batch_but_child_interrupt_does_not() {
        let f = claude_fixture();
        let main = claude_line(
            "q",
            "AskUserQuestion",
            serde_json::json!({"questions":[{"question":"是否继续？","options":[]}]}),
        );
        let interrupt = |sidechain| {
            serde_json::json!({"type":"user","isSidechain":sidechain,"message":{"content":"interrupted by user"}}).to_string()
        };
        let o = f.observe(&[main.clone(), interrupt(true)]);
        assert_eq!(o.kind, Some(AttentionKind::Answer));
        std::fs::write(&f.path, [main, interrupt(false)].join("\n")).unwrap();
        let (probe, _) = crate::session::probe_dialect(
            "claude",
            SessionDialect::GenericTail,
            f.path.to_str().unwrap(),
        );
        assert!(probe.signal.is_none());
        for id in ["", "bad\nidentifier"] {
            std::fs::write(
                &f.path,
                claude_line(id, "ExitPlanMode", serde_json::json!({"plan":"不可引用"})),
            )
            .unwrap();
            let (probe, context) = crate::session::probe_dialect(
                "claude",
                SessionDialect::GenericTail,
                f.path.to_str().unwrap(),
            );
            assert!(probe.signal.is_none() && context.artifact.is_none());
        }
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn managed_receiver_to_production_overlay_and_reader_preserves_plan_gate() {
        use std::{
            io::{Read, Write},
            os::unix::{fs::PermissionsExt, net::UnixStream},
        };
        let (f, plan, payload, _) = captured_fixture();
        let root = f.sandbox.path().canonicalize().unwrap();
        let config = root.join("claude-config");
        let parent = root.join("runtime-data");
        std::fs::create_dir(&config).unwrap();
        std::fs::create_dir(&parent).unwrap();
        let binary = root.join("fixture-binary");
        std::fs::write(&binary, b"fixture").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        let endpoint = std::path::PathBuf::from(format!(
            "/private/tmp/ai-chain-{}-{}",
            std::process::id(),
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
        struct EndpointCleanup(std::path::PathBuf);
        impl Drop for EndpointCleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir(&self.0);
            }
        }
        let _cleanup = EndpointCleanup(endpoint.clone());
        let mut config = crate::claude_hook_config::native::Store::new(
            config,
            parent,
            binary,
            root.join(".Trash"),
        );
        let revision = config.observe().unwrap().revision;
        let preview = config
            .preview(crate::claude_hook_config::Action::Enable, &revision, None)
            .unwrap();
        config.apply(&preview.plan_id).unwrap();
        let revision = config.observe().unwrap().revision;
        let mut runtime =
            crate::claude_plan_runtime::Runtime::new(config, endpoint.clone(), vec![root.clone()]);
        let initial = f.observe(&[plan.clone()]);
        f.bind(initial.source.clone());
        let original = f.store.sync(&[initial.clone()], 3).unwrap().unwrap();
        runtime.resume(&revision).unwrap();
        let bytes = serde_json::to_vec(&payload).unwrap();
        let mut stream = UnixStream::connect(endpoint.join("receiver.sock")).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        stream
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .unwrap();
        stream.write_all(&bytes).unwrap();
        stream.shutdown(std::net::Shutdown::Write).unwrap();
        let mut ack = [255];
        stream.read_exact(&mut ack).unwrap();
        assert_eq!(ack, [0]);
        let (probe, mut context) = crate::session::probe_dialect(
            "claude",
            SessionDialect::GenericTail,
            f.path.to_str().unwrap(),
        );
        context.source_path = Some(f.path.to_string_lossy().into());
        context.plan_identity_unavailable = false;
        context.plan_event_version = selected_plan_version(
            &[plan],
            f.path.to_str().unwrap(),
            probe.signal.as_ref(),
            &[root.clone()],
        )
        .unwrap();
        runtime.with_cache(|cache| {
            attach_capture(&mut context, probe.signal.as_ref(), &[root], cache)
        });
        let observed =
            crate::task_sources::from_context(&f.profile, &context, probe.signal.as_ref())
                .unwrap()
                .observation;
        assert_eq!(observed.fingerprint, initial.fingerprint);
        let data = f.store.sync(&[observed], 4).unwrap().unwrap();
        assert_eq!(data.attentions[0].id, original.attentions[0].id);
        assert_eq!(data.runs[0].id, original.runs[0].id);
        assert_eq!(data.event_receipts.len(), 1);
        let artifact = &data.artifacts[0];
        let read = |cache: Option<&mut crate::claude_plan_capture::Store>| {
            read_with_capture(
                &data,
                &data.tasks[0].id,
                &artifact.run_id,
                &artifact.id,
                data.revision,
                &f.profile,
                cache,
            )
            .unwrap()
        };
        assert!(runtime
            .with_cache(read)
            .text
            .unwrap()
            .contains("临时方案正文"));
        assert!(!std::fs::read_to_string(&f.store.path)
            .unwrap()
            .contains("临时方案正文"));
        runtime.pause().unwrap();
        assert!(!runtime.with_cache(read).available);
        assert_eq!(
            f.store.load().unwrap().attentions[0].id,
            original.attentions[0].id
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires controlled real-client fixture under .scratch/claude-real-client"]
    fn real_client_persisted_plan_uses_exact_transcript_reader() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(".scratch/claude-real-client")
            .canonicalize()
            .unwrap();
        let hook: Value =
            serde_json::from_slice(&std::fs::read(root.join("hook-input.json")).unwrap()).unwrap();
        let path = std::path::PathBuf::from(hook["transcript_path"].as_str().unwrap())
            .canonicalize()
            .unwrap();
        assert!(path.starts_with(root.join("session-config/projects").canonicalize().unwrap()));
        let lines: Vec<String> = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect();
        let record = lines
            .iter()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find_map(|doc| {
                claude_tools(&doc)
                    .find(|tool| tool["id"] == hook["tool_use_id"])
                    .and_then(|tool| claude_record(tool, path.to_str().unwrap(), false))
            })
            .unwrap();
        let content = read_path(
            &path,
            &record.evidence.event_ref,
            AttentionKind::PlanApproval,
        );
        assert!(content.available);
        assert_eq!(content.text.as_deref(), hook["tool_input"]["plan"].as_str());
        assert!(content.transient_valid_for_ms.is_none());
        let (probe, _) = crate::session::probe_dialect(
            "claude",
            SessionDialect::GenericTail,
            path.to_str().unwrap(),
        );
        assert_ne!(
            claude_attention_kind(&lines, path.to_str().unwrap(), probe.signal.as_ref()),
            Some(AttentionKind::PlanApproval)
        );
    }
}
