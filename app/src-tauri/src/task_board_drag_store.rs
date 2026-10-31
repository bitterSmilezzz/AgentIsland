#[cfg(test)]
mod tests {
    use crate::{
        task_board_drag::BoardColumn,
        tasks::{RunStatus, Store},
    };
    use std::fs;

    fn store_in(tag: &str) -> (crate::testutil::Sandbox, Store) {
        let sandbox = crate::testutil::Sandbox::new(tag);
        let store = Store {
            path: sandbox.path().join("tasks.v1.json"),
        };
        (sandbox, store)
    }

    #[test]
    fn task_board_drag_store_moves_project_without_touching_run_history() {
        let (_sandbox, store) = store_in("tasks-board-drag-store");
        let first_project = store.project_create("前端", 0).unwrap();
        let first_id = first_project.projects[0].id.clone();
        let second_project = store
            .project_create("接口", first_project.revision)
            .unwrap();
        let second_id = second_project.projects[1].id.clone();
        let created = store
            .create("检查项目拖放", Some(first_id), second_project.revision, 10)
            .unwrap();
        let task_id = created.tasks[0].id.clone();
        let running = store
            .record(
                &task_id,
                RunStatus::Running,
                None,
                None,
                created.revision,
                20,
            )
            .unwrap();
        let before = store.load().unwrap();
        let before_bytes = fs::read(&store.path).unwrap();

        let moved = store
            .board_move_project(
                &task_id,
                BoardColumn::Running,
                Some(second_id.clone()),
                before.revision,
                30,
            )
            .unwrap();
        assert_eq!(moved.revision, before.revision + 1);
        let task = moved.tasks.iter().find(|task| task.id == task_id).unwrap();
        assert_eq!(task.project_id.as_deref(), Some(second_id.as_str()));
        assert_eq!(task.revision, before.tasks[0].revision + 1);
        assert_eq!(task.updated_ms, 30);
        assert_eq!(moved.runs.len(), running.runs.len());
        assert_eq!(
            serde_json::to_value(&moved.runs).unwrap(),
            serde_json::to_value(&before.runs).unwrap()
        );
        for (after, prior) in [
            (
                serde_json::to_value(&moved.attentions).unwrap(),
                serde_json::to_value(&before.attentions).unwrap(),
            ),
            (
                serde_json::to_value(&moved.artifacts).unwrap(),
                serde_json::to_value(&before.artifacts).unwrap(),
            ),
            (
                serde_json::to_value(&moved.event_receipts).unwrap(),
                serde_json::to_value(&before.event_receipts).unwrap(),
            ),
        ] {
            assert_eq!(after, prior, "项目归属拖放不得更改任务历史或来源回执");
        }
        assert_ne!(fs::read(&store.path).unwrap(), before_bytes);
        let reloaded = Store {
            path: store.path.clone(),
        }
        .load()
        .unwrap();
        assert_eq!(
            reloaded.tasks[0].project_id.as_deref(),
            Some(second_id.as_str())
        );

        let unassigned = store
            .board_move_project(&task_id, BoardColumn::Running, None, reloaded.revision, 40)
            .unwrap();
        assert_eq!(unassigned.tasks[0].project_id, None);
        assert_eq!(unassigned.tasks[0].updated_ms, 40);
        assert_eq!(unassigned.runs.len(), 1);
    }

    #[test]
    fn task_board_drag_store_noop_and_refusals_preserve_file_bytes() {
        let (_sandbox, store) = store_in("tasks-board-drag-refusals");
        let project = store.project_create("前端", 0).unwrap();
        let project_id = project.projects[0].id.clone();
        let created = store
            .create("检查拒绝", Some(project_id.clone()), project.revision, 10)
            .unwrap();
        let task_id = created.tasks[0].id.clone();
        let bytes = fs::read(&store.path).unwrap();

        let noop = store
            .board_move_project(
                &task_id,
                BoardColumn::Queued,
                Some(project_id),
                created.revision,
                20,
            )
            .unwrap();
        assert_eq!(noop.revision, created.revision);
        assert_eq!(
            fs::read(&store.path).unwrap(),
            bytes,
            "同一项目是无写入 no-op"
        );

        for (requested_task, source, project, revision, code) in [
            (
                task_id.as_str(),
                BoardColumn::Queued,
                Some("missing-project".to_string()),
                created.revision,
                "target_not_found",
            ),
            (
                task_id.as_str(),
                BoardColumn::Running,
                Some("missing-project".to_string()),
                created.revision,
                "source_column_mismatch",
            ),
            (
                task_id.as_str(),
                BoardColumn::Queued,
                None,
                created.revision.saturating_sub(1),
                "stale_revision",
            ),
            (
                "missing-task",
                BoardColumn::Queued,
                None,
                created.revision,
                "not_found",
            ),
        ] {
            assert_eq!(
                store
                    .board_move_project(requested_task, source, project, revision, 30)
                    .unwrap_err()
                    .code,
                code
            );
            assert_eq!(
                fs::read(&store.path).unwrap(),
                bytes,
                "拒绝的拖放不得改写文件"
            );
        }

        let archived = store.archive(&task_id, true, created.revision, 40).unwrap();
        let archived_bytes = fs::read(&store.path).unwrap();
        assert_eq!(
            store
                .board_move_project(&task_id, BoardColumn::Queued, None, archived.revision, 50,)
                .unwrap_err()
                .code,
            "task_archived"
        );
        assert_eq!(fs::read(&store.path).unwrap(), archived_bytes);
    }

    #[test]
    fn task_board_status_drag_records_only_the_explicit_local_progress() {
        let (_sandbox, store) = store_in("tasks-board-status-drag-success");
        let created = store.create("本地进度拖放", None, 0, 10).unwrap();
        let task_id = created.tasks[0].id.clone();
        let running = store
            .record(
                &task_id,
                RunStatus::Running,
                None,
                None,
                created.revision,
                20,
            )
            .unwrap();
        let before = store.load().unwrap();
        let before_runs = serde_json::to_value(&before.runs).unwrap();

        let waiting = store
            .board_change_status(
                &task_id,
                BoardColumn::Running,
                BoardColumn::Attention,
                RunStatus::Waiting,
                Some(crate::tasks::AttentionKind::PlanApproval),
                before.revision,
                30,
            )
            .unwrap();

        assert_eq!(waiting.revision, before.revision + 1);
        assert_eq!(waiting.tasks[0].revision, before.tasks[0].revision + 1);
        assert_eq!(waiting.tasks[0].updated_ms, 30);
        assert_eq!(waiting.runs.len(), running.runs.len());
        assert_ne!(serde_json::to_value(&waiting.runs).unwrap(), before_runs);
        assert_eq!(waiting.runs[0].status, RunStatus::Waiting);
        assert_eq!(
            crate::tasks::board_column(&waiting, &waiting.tasks[0]),
            BoardColumn::Attention
        );
        assert_eq!(waiting.attentions.len(), 1);
        assert_eq!(
            waiting.attentions[0].kind,
            crate::tasks::AttentionKind::PlanApproval
        );
        assert!(!waiting.attentions[0].observed, "拖放只记录本地进度");
        assert_eq!(
            waiting.attentions[0].state,
            crate::tasks::AttentionState::Open
        );
        assert!(waiting.artifacts.is_empty());
        assert!(waiting.event_receipts.is_empty());
        assert!(waiting
            .runs
            .iter()
            .all(|run| run.status != RunStatus::Accepted));
        assert_eq!(store.load().unwrap().revision, waiting.revision);
    }

    #[test]
    fn task_board_status_drag_records_result_review_as_ready_with_a_local_gate() {
        let (_sandbox, store) = store_in("tasks-board-status-drag-review");
        let created = store.create("结果待验收", None, 0, 10).unwrap();
        let task_id = created.tasks[0].id.clone();
        let running = store
            .record(
                &task_id,
                RunStatus::Running,
                None,
                None,
                created.revision,
                20,
            )
            .unwrap();

        let reviewed = store
            .board_change_status(
                &task_id,
                BoardColumn::Running,
                BoardColumn::Attention,
                RunStatus::Ready,
                Some(crate::tasks::AttentionKind::ResultReview),
                running.revision,
                30,
            )
            .unwrap();

        assert_eq!(reviewed.runs[0].status, RunStatus::Ready);
        assert_eq!(
            crate::tasks::board_column(&reviewed, &reviewed.tasks[0]),
            BoardColumn::Attention
        );
        assert_eq!(
            reviewed.attentions[0].kind,
            crate::tasks::AttentionKind::ResultReview
        );
        assert!(!reviewed.attentions[0].observed);
    }

    #[test]
    fn task_board_status_drag_accepts_every_explicit_target_choice() {
        use crate::tasks::AttentionKind;

        let (_sandbox, store) = store_in("tasks-board-status-drag-valid-choices");
        let choices = [
            (
                BoardColumn::Queued,
                BoardColumn::Running,
                RunStatus::Running,
                None,
            ),
            (
                BoardColumn::Running,
                BoardColumn::Queued,
                RunStatus::Queued,
                None,
            ),
            (
                BoardColumn::Running,
                BoardColumn::Attention,
                RunStatus::Waiting,
                Some(AttentionKind::Answer),
            ),
            (
                BoardColumn::Running,
                BoardColumn::Attention,
                RunStatus::Waiting,
                Some(AttentionKind::PlanApproval),
            ),
            (
                BoardColumn::Running,
                BoardColumn::Attention,
                RunStatus::Waiting,
                Some(AttentionKind::Confirmation),
            ),
            (
                BoardColumn::Running,
                BoardColumn::Attention,
                RunStatus::Ready,
                Some(AttentionKind::ResultReview),
            ),
            (
                BoardColumn::Running,
                BoardColumn::Attention,
                RunStatus::Failed,
                None,
            ),
            (
                BoardColumn::Running,
                BoardColumn::Finished,
                RunStatus::Ready,
                None,
            ),
            (
                BoardColumn::Running,
                BoardColumn::Finished,
                RunStatus::Cancelled,
                None,
            ),
        ];
        let mut revision = 0;

        for (index, (source, target, status, kind)) in choices.into_iter().enumerate() {
            let created = store
                .create(
                    &format!("合法选择 {index}"),
                    None,
                    revision,
                    10 + index as i64,
                )
                .unwrap();
            let task_id = created.tasks.last().unwrap().id.clone();
            let before = if source == BoardColumn::Running {
                store
                    .record(
                        &task_id,
                        RunStatus::Running,
                        None,
                        None,
                        created.revision,
                        20 + index as i64,
                    )
                    .unwrap()
            } else {
                created
            };
            assert_eq!(
                crate::tasks::board_column(
                    &before,
                    before.tasks.iter().find(|task| task.id == task_id).unwrap()
                ),
                source,
                "fixture must start in the requested source column"
            );

            let next = store
                .board_change_status(
                    &task_id,
                    source,
                    target,
                    status,
                    kind,
                    before.revision,
                    30 + index as i64,
                )
                .unwrap();
            let task = next.tasks.iter().find(|task| task.id == task_id).unwrap();
            assert_eq!(crate::tasks::board_column(&next, task), target);
            let run = next
                .runs
                .iter()
                .find(|run| Some(run.id.as_str()) == task.current_run_id.as_deref())
                .unwrap();
            assert_eq!(run.status, status);
            assert_ne!(run.status, RunStatus::Accepted);
            if let Some(kind) = kind {
                let gate = next
                    .attentions
                    .iter()
                    .find(|attention| {
                        attention.run_id == run.id
                            && attention.state == crate::tasks::AttentionState::Open
                    })
                    .unwrap();
                assert_eq!(gate.kind, kind);
                assert!(!gate.observed, "所有人工关口均标为本地记录");
            }
            revision = next.revision;
        }
        assert_eq!(store.load().unwrap().revision, revision);
    }

    #[test]
    fn task_board_status_drag_noop_and_refusals_preserve_file_bytes() {
        let (_sandbox, store) = store_in("tasks-board-status-drag-refusals");
        let created = store.create("状态拖放拒绝", None, 0, 10).unwrap();
        let task_id = created.tasks[0].id.clone();
        let running = store
            .record(
                &task_id,
                RunStatus::Running,
                None,
                None,
                created.revision,
                20,
            )
            .unwrap();
        let bytes = fs::read(&store.path).unwrap();

        let noop = store
            .board_change_status(
                &task_id,
                BoardColumn::Running,
                BoardColumn::Running,
                RunStatus::Running,
                None,
                running.revision,
                30,
            )
            .unwrap();
        assert_eq!(noop.revision, running.revision);
        assert_eq!(fs::read(&store.path).unwrap(), bytes, "同列拖放不写文件");

        for (requested_task, source, target, status, kind, revision, code) in [
            (
                task_id.as_str(),
                BoardColumn::Running,
                BoardColumn::Finished,
                RunStatus::Running,
                None,
                running.revision,
                "invalid_input",
            ),
            (
                task_id.as_str(),
                BoardColumn::Running,
                BoardColumn::Attention,
                RunStatus::Waiting,
                Some(crate::tasks::AttentionKind::ResultReview),
                running.revision,
                "invalid_input",
            ),
            (
                task_id.as_str(),
                BoardColumn::Queued,
                BoardColumn::Finished,
                RunStatus::Ready,
                None,
                running.revision,
                "source_column_mismatch",
            ),
            (
                task_id.as_str(),
                BoardColumn::Running,
                BoardColumn::Finished,
                RunStatus::Ready,
                None,
                running.revision.saturating_sub(1),
                "stale_revision",
            ),
            (
                "missing-task",
                BoardColumn::Running,
                BoardColumn::Finished,
                RunStatus::Ready,
                None,
                running.revision,
                "not_found",
            ),
            (
                task_id.as_str(),
                BoardColumn::Running,
                BoardColumn::Finished,
                RunStatus::Accepted,
                None,
                running.revision,
                "invalid_input",
            ),
        ] {
            assert_eq!(
                store
                    .board_change_status(
                        requested_task,
                        source,
                        target,
                        status,
                        kind,
                        revision,
                        40,
                    )
                    .unwrap_err()
                    .code,
                code
            );
            assert_eq!(
                fs::read(&store.path).unwrap(),
                bytes,
                "拒绝的拖放不得改写文件"
            );
        }

        let archived = store.archive(&task_id, true, running.revision, 50).unwrap();
        let archived_bytes = fs::read(&store.path).unwrap();
        assert_eq!(
            store
                .board_change_status(
                    &task_id,
                    BoardColumn::Running,
                    BoardColumn::Finished,
                    RunStatus::Ready,
                    None,
                    archived.revision,
                    60,
                )
                .unwrap_err()
                .code,
            "task_archived"
        );
        assert_eq!(fs::read(&store.path).unwrap(), archived_bytes);
    }
}
