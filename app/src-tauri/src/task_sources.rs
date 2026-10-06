//! Server-owned semantic source identities. Neither CPU nor UI-supplied URIs create bindings.
use crate::{models::AgentProfile, session::Signal, session_navigation, tasks};
use serde::Serialize;
use sha2::{Digest, Sha256};
#[derive(Clone, Serialize)]
pub struct Choice {
    pub source: tasks::Source,
    pub name: String,
    pub target: Option<session_navigation::Target>,
    pub observation: Observation,
}
#[derive(Clone, Serialize)]
pub struct Observation {
    pub source: tasks::Source,
    pub fingerprint: String,
    pub status: tasks::RunStatus,
    pub kind: Option<tasks::AttentionKind>,
    pub artifact: Option<crate::task_artifacts::Evidence>,
}
/// Navigation must refer to the observed source, even when two sessions open the same GUI.
pub fn selected_target(
    choice: Option<&Choice>,
    expected: &tasks::Source,
) -> Result<session_navigation::Target, String> {
    let choice = choice
        .filter(|choice| choice.source == *expected)
        .ok_or("会话来源已变化，请刷新后重试")?;
    choice
        .target
        .clone()
        .ok_or_else(|| "来源工具没有桌面跳转入口".into())
}
fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub fn active_is_eligible(
    signal: Option<&Signal>,
    running: bool,
    age: Option<f64>,
    window: f64,
) -> bool {
    !matches!(signal, Some(Signal::Active(..)))
        || (running && age.is_some_and(|age| age >= 0.0 && age <= window))
}
/// Database activity is dated by its selected event; a WAL write can leave DB mtime old.
pub fn source_age(context: &crate::models::SessionActiveContext, now_ms: i64) -> Option<f64> {
    if let Some(source) = &context.source_keys {
        return now_ms
            .checked_sub(source.activity_ms?)
            .filter(|age| *age >= 0)
            .map(|age| age as f64 / 1000.0);
    }
    context
        .source_path
        .as_deref()
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.elapsed().ok())
        .map(|age| age.as_secs_f64())
}
pub fn from_signal(
    profile: &AgentProfile,
    path: Option<&str>,
    signal: Option<&Signal>,
) -> Option<Choice> {
    let path = path.filter(|s| !s.is_empty())?;
    if profile
        .session_database
        .as_ref()
        .is_some_and(|db| db.path == path)
    {
        return None;
    }
    let signal = signal?;
    let target = session_navigation::resolve(profile, Some(path));
    let thread_id = target
        .as_ref()
        .and_then(|t| t.url.as_deref())
        .and_then(|u| u.strip_prefix("codex://threads/"))
        .map(str::to_owned);
    let source = tasks::Source {
        agent_id: profile.id.clone(),
        session_id: hash(path),
        thread_id,
    };
    Some(choice(profile, source, target, signal))
}

/// A database identity must come from the same semantic row, never the database filename.
pub fn from_context(
    profile: &AgentProfile,
    context: &crate::models::SessionActiveContext,
    signal: Option<&Signal>,
) -> Option<Choice> {
    if profile.id == "claude" && context.plan_identity_unavailable {
        return None;
    }
    let Some(keys) = context.source_keys.as_ref() else {
        let mut result = from_signal(profile, context.source_path.as_deref(), signal)?;
        if result.observation.status == tasks::RunStatus::Waiting {
            if let Some(kind) = context.attention_kind {
                result.observation.kind = Some(kind);
                result.observation.fingerprint = hash(&format!(
                    "{}:{}",
                    result.observation.fingerprint,
                    serde_json::to_string(&kind).ok()?
                ));
            }
        }
        let plan_version = context.plan_event_version.as_deref().filter(|v| {
            profile.id == "claude"
                && context.attention_kind == Some(tasks::AttentionKind::PlanApproval)
                && v.len() == 64
                && v.bytes().all(|b| b.is_ascii_hexdigit())
        });
        if let Some(version) = plan_version {
            result.observation.fingerprint =
                hash(&format!("{}:{version}", result.observation.fingerprint));
        }
        if let Some(evidence) = context.artifact.as_ref().filter(|e| e.valid()) {
            if plan_version.is_none() {
                result.observation.fingerprint = hash(&format!(
                    "{}:{}",
                    result.observation.fingerprint, evidence.event_ref
                ));
            }
            result.observation.artifact = Some(evidence.clone());
        }
        return Some(result);
    };
    let path = context.source_path.as_deref()?;
    if !profile
        .session_database
        .as_ref()
        .is_some_and(|db| db.path == path)
    {
        return None;
    }
    crate::session::database_source_keys(keys.keys.clone())?;
    let identity = serde_json::to_string(&("database", path, &keys.keys)).ok()?;
    let source = tasks::Source {
        agent_id: profile.id.clone(),
        session_id: hash(&identity),
        thread_id: None,
    };
    let mut result = choice(
        profile,
        source,
        session_navigation::resolve(profile, None),
        signal?,
    );
    if let Some(revision) = &keys.revision {
        result.observation.fingerprint =
            hash(&format!("{}:{revision}", result.observation.fingerprint));
    }
    Some(result)
}

fn choice(
    profile: &AgentProfile,
    source: tasks::Source,
    target: Option<session_navigation::Target>,
    signal: &Signal,
) -> Choice {
    let (fp, status, kind, namespace) = match signal {
        Signal::Active(fp, _) => (fp, tasks::RunStatus::Running, None, "active"),
        Signal::Attention(fp, _) => (
            fp,
            tasks::RunStatus::Waiting,
            Some(if profile.id == "codex" {
                tasks::AttentionKind::Answer
            } else {
                tasks::AttentionKind::Confirmation
            }),
            "attention",
        ),
        Signal::Completed(fp) => (fp, tasks::RunStatus::Ready, None, "completed"),
    };
    Choice {
        name: profile.name.clone(),
        target,
        observation: Observation {
            source: source.clone(),
            fingerprint: hash(&format!("{namespace}:{fp}")),
            status,
            kind,
            artifact: None,
        },
        source,
    }
}
pub fn stored_target(source: &tasks::Source) -> Result<session_navigation::Target, String> {
    let profile = crate::registry::builtin()
        .into_iter()
        .find(|p| p.id == source.agent_id)
        .ok_or("来源工具不受支持")?;
    if profile.id == "codex" {
        if let Some(thread) = &source.thread_id {
            let parsed = uuid::Uuid::parse_str(thread).map_err(|_| "来源会话标识损坏")?;
            // Reuse the same verified router, with a canonical UUID, never a persisted URL.
            return session_navigation::resolve(&profile, Some(&format!("rollout-{parsed}.jsonl")))
                .ok_or_else(|| "来源工具不支持跳转".into());
        }
    }
    session_navigation::resolve(&profile, None).ok_or_else(|| "来源工具没有桌面跳转入口".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn profile(id: &str) -> AgentProfile {
        crate::registry::builtin()
            .into_iter()
            .find(|p| p.id == id)
            .unwrap()
    }
    #[test]
    fn navigation_checks_source_identity_even_for_tool_only_targets() {
        let p = profile("claude");
        let original = from_signal(
            &p,
            Some("/fixture/session-one.jsonl"),
            Some(&Signal::Completed("one".into())),
        )
        .unwrap();
        let other = from_signal(
            &p,
            Some("/fixture/session-two.jsonl"),
            Some(&Signal::Completed("two".into())),
        )
        .unwrap();
        assert!(selected_target(Some(&other), &original.source).is_err());
        assert!(selected_target(None, &original.source).is_err());
        let mut missing = original.clone();
        missing.target = None;
        assert!(selected_target(Some(&missing), &original.source).is_err());
        let codex = from_signal(
            &profile("codex"),
            Some("/fixture/rollout-019c6e27-e55b-73d1-87d8-4e01f1f75043.jsonl"),
            Some(&Signal::Completed("one".into())),
        )
        .unwrap();
        assert!(
            selected_target(Some(&codex), &codex.source)
                .unwrap()
                .exact_session
        );
        let mut different_thread = codex.source.clone();
        different_thread.thread_id = None;
        assert!(selected_target(Some(&codex), &different_thread).is_err());
    }
    #[test]
    fn stale_or_exited_execution_is_not_observed_as_running() {
        let signal = Signal::Active("old".into(), None);
        assert!(!active_is_eligible(Some(&signal), true, Some(600.0), 60.0));
        assert!(!active_is_eligible(Some(&signal), false, Some(1.0), 60.0));
        assert!(!active_is_eligible(Some(&signal), true, None, 60.0));
        assert!(active_is_eligible(Some(&signal), true, Some(1.0), 60.0));
    }
    #[test]
    fn missing_semantic_evidence_or_source_never_creates_a_binding() {
        assert!(from_signal(&profile("codex"), Some("/fixture/session"), None).is_none());
        assert!(from_signal(
            &profile("codex"),
            None,
            Some(&Signal::Completed("fp".into()))
        )
        .is_none());
    }
    #[test]
    fn identity_is_stable_private_and_event_kinds_do_not_collide() {
        let p = profile("codex");
        let path = "/fixture/rollout-019c6e27-e55b-73d1-87d8-4e01f1f75043.jsonl";
        let a = from_signal(
            &p,
            Some(path),
            Some(&Signal::Active(
                "same".into(),
                Some("private action".into()),
            )),
        )
        .unwrap();
        let b = from_signal(&p, Some(path), Some(&Signal::Completed("same".into()))).unwrap();
        assert_eq!(a.source, b.source);
        assert_ne!(a.observation.fingerprint, b.observation.fingerprint);
        let encoded = serde_json::to_string(&a).unwrap();
        assert!(!encoded.contains(path));
        assert!(!encoded.contains("private action"));
        assert!(stored_target(&a.source).unwrap().exact_session);
    }
    #[test]
    fn database_activity_uses_the_row_clock_and_refuses_future_or_missing_times() {
        let mut context = crate::models::SessionActiveContext {
            source_path: Some("/fixture/nonexistent.sqlite".into()),
            source_keys: crate::session::database_source_keys(vec!["session".into()]),
            ..Default::default()
        };
        assert_eq!(source_age(&context, 10_000), None);
        context.source_keys.as_mut().unwrap().activity_ms = Some(9_000);
        assert_eq!(source_age(&context, 10_000), Some(1.0));
        let active = Signal::Active("event".into(), None);
        assert!(active_is_eligible(
            Some(&active),
            true,
            source_age(&context, 10_000),
            60.0
        ));
        assert!(!active_is_eligible(
            Some(&active),
            false,
            source_age(&context, 10_000),
            60.0
        ));
        assert!(!active_is_eligible(
            Some(&active),
            true,
            source_age(&context, 100_000),
            60.0
        ));
        context.source_keys.as_mut().unwrap().activity_ms = Some(10_001);
        assert_eq!(source_age(&context, 10_000), None);
    }
    #[test]
    fn database_identity_requires_parser_keys_and_its_declared_path() {
        let mut p = profile("dim");
        p.session_database.as_mut().unwrap().path = "/fixture/library.sqlite".into();
        let signal = Signal::Completed("event".into());
        assert!(from_signal(&p, Some("/fixture/library.sqlite"), Some(&signal)).is_none());
        let mut context = crate::models::SessionActiveContext {
            source_path: Some("/fixture/library.sqlite".into()),
            ..Default::default()
        };
        assert!(from_context(&p, &context, Some(&signal)).is_none());
        for keys in [
            vec![],
            vec!["".into()],
            vec!["line\nbreak".into()],
            vec!["x".repeat(513)],
            vec!["a".into(), "b".into(), "c".into()],
        ] {
            context.source_keys = Some(crate::models::DatabaseSource {
                keys,
                revision: None,
                activity_ms: None,
            });
            assert!(from_context(&p, &context, Some(&signal)).is_none());
        }
        context.source_keys = crate::session::database_source_keys(vec!["session".into()]);
        assert!(from_context(&p, &context, None).is_none());
        let a = from_context(&p, &context, Some(&signal)).unwrap();
        context.source_keys.as_mut().unwrap().revision = Some("next".into());
        let next = from_context(&p, &context, Some(&signal)).unwrap();
        assert_eq!(a.source, next.source);
        assert_ne!(a.observation.fingerprint, next.observation.fingerprint);
        context.source_path = Some("/fixture/other.sqlite".into());
        assert!(from_context(&p, &context, Some(&signal)).is_none());
    }
    #[test]
    fn historical_source_never_substitutes_the_latest_conversation() {
        let source = tasks::Source {
            agent_id: "codex".into(),
            session_id: "a".repeat(64),
            thread_id: Some("019c6e27-e55b-73d1-87d8-4e01f1f75043".into()),
        };
        assert_eq!(
            stored_target(&source).unwrap().url.as_deref(),
            Some("codex://threads/019c6e27-e55b-73d1-87d8-4e01f1f75043")
        );
        let generic = tasks::Source {
            agent_id: "traework".into(),
            thread_id: None,
            ..source
        };
        assert_eq!(stored_target(&generic).unwrap().label, "打开工具");
    }
}
