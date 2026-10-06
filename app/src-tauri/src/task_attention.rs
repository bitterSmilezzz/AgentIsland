//! Compact attention projections. Only current, unarchived task runs participate.
use crate::{session_navigation::Target, tasks::*};
use serde::Serialize;
#[derive(Debug, Serialize)]
pub struct Item {
    pub task_id: String,
    pub run_id: String,
    pub attention_id: Option<String>,
    pub label: &'static str,
    pub title: String,
    pub detail: Option<String>,
    pub agent_id: Option<String>,
    pub target: Option<Target>,
    pub failure: bool,
    pub observed: bool,
}
#[derive(Debug, Serialize)]
pub struct Summary {
    pub revision: u64,
    pub total: usize,
    pub human_count: usize,
    pub failed_count: usize,
    pub first: Option<Item>,
}
fn kind_info(kind: AttentionKind) -> (u8, &'static str) {
    match kind {
        AttentionKind::Answer => (0, "待回答"),
        AttentionKind::PlanApproval => (1, "确认方案"),
        AttentionKind::ResultReview => (2, "验收结果"),
        AttentionKind::Confirmation => (3, "需要确认"),
    }
}
pub fn summarize(data: &Data) -> Summary {
    let mut items = Vec::new();
    for task in data.tasks.iter().filter(|t| t.archived_ms.is_none()) {
        let Some(run) = data
            .runs
            .iter()
            .find(|r| Some(&r.id) == task.current_run_id.as_ref())
        else {
            continue;
        };
        let gate = data
            .attentions
            .iter()
            .filter(|a| a.run_id == run.id && a.state == AttentionState::Open)
            .min_by_key(|a| (kind_info(a.kind).0, &a.id));
        let (priority, label) = if let Some(gate) = gate {
            kind_info(gate.kind)
        } else if run.status == RunStatus::Waiting {
            (4, "等待处理")
        } else if run.status == RunStatus::Failed {
            (5, "执行失败")
        } else {
            continue;
        };
        let detail = gate
            .and_then(|g| g.artifact_id.as_ref())
            .and_then(|id| data.artifacts.iter().find(|a| &a.id == id))
            .map(|a| a.title.chars().take(140).collect());
        let source = run.source.as_ref().or(task.source.as_ref());
        let item = Item {
            task_id: task.id.clone(),
            run_id: run.id.clone(),
            attention_id: gate.map(|g| g.id.clone()),
            label,
            title: task.title.chars().take(140).collect(),
            detail,
            agent_id: source.map(|s| s.agent_id.clone()),
            target: source.and_then(|s| crate::task_sources::stored_target(s).ok()),
            observed: gate.is_some_and(|g| g.observed),
            failure: run.status == RunStatus::Failed && gate.is_none(),
        };
        items.push((
            priority,
            std::cmp::Reverse(task.updated_ms),
            task.id.clone(),
            item,
        ));
    }
    items.sort_by(|a, b| (&a.0, &a.1, &a.2).cmp(&(&b.0, &b.1, &b.2)));
    let total = items.len();
    let failed_count = items.iter().filter(|i| i.3.failure).count();
    Summary {
        revision: data.revision,
        total,
        human_count: total - failed_count,
        failed_count,
        first: items.into_iter().next().map(|i| i.3),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn push(d: &mut Data, name: &str, status: RunStatus, kind: Option<AttentionKind>, time: i64) {
        d.tasks.push(Task {
            id: name.into(),
            title: name.into(),
            project_id: None,
            source: None,
            current_run_id: Some(format!("r-{name}")),
            created_ms: time,
            updated_ms: time,
            archived_ms: None,
            revision: 1,
        });
        d.runs.push(Run {
            id: format!("r-{name}"),
            task_id: name.into(),
            source: None,
            status,
            started_ms: time,
            finished_ms: None,
            revision: 1,
        });
        if let Some(kind) = kind {
            d.attentions.push(Attention {
                observed: false,
                id: format!("a-{name}"),
                run_id: format!("r-{name}"),
                kind,
                state: AttentionState::Open,
                artifact_id: None,
                manual_resolution: false,
            });
        }
    }
    #[test]
    fn priorities_and_counts_do_not_confuse_failure_with_approval() {
        let mut d = Data::default();
        push(&mut d, "failed", RunStatus::Failed, None, 50);
        push(
            &mut d,
            "review",
            RunStatus::Ready,
            Some(AttentionKind::ResultReview),
            40,
        );
        push(
            &mut d,
            "answer-old",
            RunStatus::Waiting,
            Some(AttentionKind::Answer),
            10,
        );
        push(
            &mut d,
            "answer-new",
            RunStatus::Waiting,
            Some(AttentionKind::Answer),
            20,
        );
        let s = summarize(&d);
        assert_eq!(s.total, 4);
        assert_eq!(s.human_count, 3);
        assert_eq!(s.failed_count, 1);
        assert_eq!(s.first.unwrap().task_id, "answer-new");
    }
    #[test]
    fn archived_resolved_expired_and_historical_gates_do_not_count() {
        let mut d = Data::default();
        push(
            &mut d,
            "archived",
            RunStatus::Waiting,
            Some(AttentionKind::Answer),
            1,
        );
        d.tasks[0].archived_ms = Some(2);
        push(
            &mut d,
            "ready",
            RunStatus::Ready,
            Some(AttentionKind::ResultReview),
            1,
        );
        d.attentions[1].state = AttentionState::Resolved;
        push(&mut d, "running", RunStatus::Running, None, 1);
        d.attentions.push(Attention {
            observed: false,
            id: "past".into(),
            run_id: "old-run".into(),
            kind: AttentionKind::Answer,
            state: AttentionState::Open,
            artifact_id: None,
            manual_resolution: false,
        });
        assert_eq!(summarize(&d).total, 0);
        d.attentions[1].state = AttentionState::Expired;
        assert!(summarize(&d).first.is_none());
    }
    #[test]
    fn multiple_gates_on_one_task_are_one_actionable_task() {
        let mut d = Data::default();
        push(
            &mut d,
            "one",
            RunStatus::Ready,
            Some(AttentionKind::ResultReview),
            1,
        );
        d.attentions.push(Attention {
            observed: false,
            id: "question".into(),
            run_id: "r-one".into(),
            kind: AttentionKind::Answer,
            state: AttentionState::Open,
            artifact_id: None,
            manual_resolution: false,
        });
        let s = summarize(&d);
        assert_eq!(s.total, 1);
        assert_eq!(s.first.unwrap().label, "待回答");
    }
}
