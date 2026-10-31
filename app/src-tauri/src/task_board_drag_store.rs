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
}
