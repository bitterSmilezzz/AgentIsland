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
    // 聚合的单位是 **run**，不是「任务行」或「关注点」：
    // 一个 run 可能同时挂着多个 open attention（不同 kind 的 scope），
    // 也可能（数据异常时）被多个 task 指为 current_run——两者都只能算一条。
    // 所以既要按 run 去重，也要校验 run 的归属，否则同一次执行会被数成两次。
    let mut counted_runs = std::collections::HashSet::new();
    for task in data.tasks.iter().filter(|t| t.archived_ms.is_none()) {
        let Some(run) = data
            .runs
            .iter()
            .find(|r| Some(&r.id) == task.current_run_id.as_ref())
        else {
            continue;
        };
        if run.task_id != task.id || !counted_runs.insert(run.id.clone()) {
            continue;
        }
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

    /// 同一个 run 挂着多个 open attention（多 scope）时，聚合只算一条。
    ///
    /// 诚实标注：这条**不会**被「去掉去重」的变异杀死——当前的循环形状是「按任务取
    /// 当前 run 再挑一条代表」，多 attention 天然只出一条。它的作用是**钉住这个契约**：
    /// 谁把聚合改成「遍历 attention」，这条立刻变红。真正能抓「同一 run 被算两次」的是
    /// 下面那条归属/去重用例（变异实测 left:2 right:1）。
    #[test]
    fn several_open_scopes_on_one_run_count_once() {
        let mut d = Data::default();
        push(&mut d, "multi", RunStatus::Ready, Some(AttentionKind::PlanApproval), 50);
        d.attentions.push(Attention {
            observed: false,
            id: "a-multi-second".into(),
            run_id: "r-multi".into(),
            kind: AttentionKind::Answer,
            state: AttentionState::Open,
            artifact_id: None,
            manual_resolution: false,
        });
        let summary = summarize(&d);
        assert_eq!(summary.total, 1, "同一 run 的两个 scope 被数成了两条");
        assert_eq!(summary.human_count, 1);
        assert_eq!(summary.failed_count, 0);
        // 同一 run 内仍选优先级更高的那条作为代表（Answer=0 高于 PlanApproval=1）
        let first = summary.first.unwrap();
        assert_eq!(first.attention_id.as_deref(), Some("a-multi-second"));
        assert_eq!(first.label, "待回答");
    }

    /// 数据异常时两个 task 可能指向同一个 run：只按**归属者**算一次。
    #[test]
    fn a_run_claimed_by_two_tasks_counts_once_and_only_for_its_owner() {
        let mut d = Data::default();
        push(&mut d, "owner", RunStatus::Ready, Some(AttentionKind::Answer), 50);
        // 另一个任务把别人家的 run 当成自己的 current_run
        d.tasks.push(Task {
            id: "intruder".into(),
            title: "越权指向".into(),
            project_id: None,
            source: None,
            current_run_id: Some("r-owner".into()),
            created_ms: 60,
            updated_ms: 60,
            archived_ms: None,
            revision: 1,
        });
        let summary = summarize(&d);
        assert_eq!(summary.total, 1, "被两个任务指向的同一个 run 被数了两次");
        let item = summary.first.expect("归属者的那条仍应出现");
        assert_eq!(item.task_id, "owner", "这条只能算在 run 的真正归属任务上");
        assert_eq!(item.run_id, "r-owner");
    }
}
