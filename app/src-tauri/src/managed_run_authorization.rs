//! TASK-03 managed-run authorization predicates.
//!
//! This module is deliberately not connected to Tauri commands, persistence, or
//! an executor. `ProjectAuthorization` is a current in-memory policy supplied by
//! a future local authorization service. The service must only change it after
//! explicit user consent, and `VerifiedRunner` must only be created after the
//! runner launcher authenticates the exact local process/session. A project ID
//! or a caller-provided runner name is not proof of either.

use std::collections::{BTreeMap, BTreeSet};

/// Planning, execution, and review are separate project capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RunRole {
    Planner,
    Executor,
    Reviewer,
}

/// Actions checked against a run grant. External effects such as pushing Git,
/// publishing a release, or sending a notification are intentionally absent;
/// they require their own subsystem authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunAction {
    Plan,
    Execute,
    Review,
    StoreHistory,
}

/// The immutable project/task/run tuple. IDs are opaque stable identifiers, not
/// paths, user-visible titles, or remote identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunTarget {
    project_id: String,
    task_id: String,
    run_id: String,
}

impl RunTarget {
    pub(crate) fn new(project_id: &str, task_id: &str, run_id: &str) -> Result<Self, Refusal> {
        if !valid_id(project_id) || !valid_id(task_id) || !valid_id(run_id) {
            return Err(Refusal::InvalidIdentity);
        }
        Ok(Self {
            project_id: project_id.to_owned(),
            task_id: task_id.to_owned(),
            run_id: run_id.to_owned(),
        })
    }
}

/// Proof-like identity produced by the trusted local runner launcher. It is not
/// deserializable and contains no reusable credential. The generation fences a
/// restarted process even when it reuses the same stable runner ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedRunner {
    id: String,
    generation: u64,
}

impl VerifiedRunner {
    /// Call only after the local launcher has authenticated the process/session.
    /// No current production path calls this constructor; user input must never
    /// be passed here directly.
    #[allow(dead_code)]
    fn from_authenticated_launch(id: &str, generation: u64) -> Result<Self, Refusal> {
        if !valid_id(id) || generation == 0 {
            return Err(Refusal::InvalidIdentity);
        }
        Ok(Self {
            id: id.to_owned(),
            generation,
        })
    }
}

/// Current local authorization state for one project. It is not persisted in
/// `tasks.v1.json`; the durable store and the user-consent command are later
/// implementation slices. New projects begin closed and collect no history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectAuthorization {
    project_id: String,
    enabled: bool,
    authorization_version: u64,
    history_capture_enabled: bool,
    history_policy_version: u64,
    runners: BTreeMap<RunRole, BTreeSet<String>>,
}

impl ProjectAuthorization {
    pub(crate) fn new(project_id: &str) -> Result<Self, Refusal> {
        if !valid_id(project_id) {
            return Err(Refusal::InvalidIdentity);
        }
        Ok(Self {
            project_id: project_id.to_owned(),
            enabled: false,
            authorization_version: 1,
            history_capture_enabled: false,
            history_policy_version: 1,
            runners: BTreeMap::new(),
        })
    }

    /// Add a runner identity for one role. A newly granted runner invalidates
    /// every earlier run snapshot so old claims cannot silently inherit scope.
    pub(crate) fn grant_runner(
        &mut self,
        role: RunRole,
        runner: &VerifiedRunner,
    ) -> Result<(), Refusal> {
        if self
            .runners
            .get(&role)
            .is_some_and(|runners| runners.contains(&runner.id))
        {
            return Ok(());
        }
        let next = next_version(self.authorization_version)?;
        self.runners
            .entry(role)
            .or_default()
            .insert(runner.id.clone());
        self.authorization_version = next;
        Ok(())
    }

    pub(crate) fn revoke_runner(&mut self, role: RunRole, runner_id: &str) -> Result<(), Refusal> {
        let Some(runners) = self.runners.get(&role) else {
            return Ok(());
        };
        if !runners.contains(runner_id) {
            return Ok(());
        }
        let next = next_version(self.authorization_version)?;
        self.runners
            .get_mut(&role)
            .expect("role checked above")
            .remove(runner_id);
        self.authorization_version = next;
        Ok(())
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) -> Result<(), Refusal> {
        if self.enabled == enabled {
            return Ok(());
        }
        let next = next_version(self.authorization_version)?;
        self.enabled = enabled;
        self.authorization_version = next;
        Ok(())
    }

    /// History opt-in has its own version, independent of permission to run.
    /// Enabling collection is possible only after the project has been enabled.
    pub(crate) fn set_history_capture_enabled(&mut self, enabled: bool) -> Result<(), Refusal> {
        if enabled && !self.enabled {
            return Err(Refusal::ProjectNotAuthorized);
        }
        if self.history_capture_enabled == enabled {
            return Ok(());
        }
        let next = next_version(self.history_policy_version)?;
        self.history_capture_enabled = enabled;
        self.history_policy_version = next;
        Ok(())
    }

    pub(crate) fn authorize_run(
        &self,
        target: RunTarget,
        role: RunRole,
        runner: &VerifiedRunner,
        claim_version: u64,
    ) -> Result<RunAuthorization, Refusal> {
        if target.project_id != self.project_id {
            return Err(Refusal::TargetMismatch);
        }
        if !self.enabled {
            return Err(Refusal::ProjectNotAuthorized);
        }
        if claim_version == 0 {
            return Err(Refusal::InvalidIdentity);
        }
        if !self
            .runners
            .get(&role)
            .is_some_and(|runners| runners.contains(&runner.id))
        {
            return Err(Refusal::RunnerNotAuthorized);
        }
        Ok(RunAuthorization {
            target,
            role,
            runner_id: runner.id.clone(),
            runner_generation: runner.generation,
            claim_version,
            authorization_version: self.authorization_version,
            history_policy_version: self.history_policy_version,
        })
    }
}

/// Immutable authorization snapshot bound to one managed run and one runner
/// incarnation. Revalidate it against current policy before every privileged
/// action and before accepting runner output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunAuthorization {
    target: RunTarget,
    role: RunRole,
    runner_id: String,
    runner_generation: u64,
    claim_version: u64,
    authorization_version: u64,
    history_policy_version: u64,
}

impl RunAuthorization {
    pub(crate) fn authorize_action(
        &self,
        current: &ProjectAuthorization,
        target: &RunTarget,
        runner: &VerifiedRunner,
        claim_version: u64,
        action: RunAction,
    ) -> Result<(), Refusal> {
        if target != &self.target || target.project_id != current.project_id {
            return Err(Refusal::TargetMismatch);
        }
        if !current.enabled || current.authorization_version != self.authorization_version {
            return Err(Refusal::AuthorizationChanged);
        }
        if runner.id != self.runner_id || runner.generation != self.runner_generation {
            return Err(Refusal::StaleRunnerIdentity);
        }
        if claim_version != self.claim_version {
            return Err(Refusal::StaleExecutionClaim);
        }
        if !current
            .runners
            .get(&self.role)
            .is_some_and(|runners| runners.contains(&self.runner_id))
        {
            return Err(Refusal::RunnerNotAuthorized);
        }
        match action {
            RunAction::Plan if self.role == RunRole::Planner => Ok(()),
            RunAction::Execute if self.role == RunRole::Executor => Ok(()),
            RunAction::Review if self.role == RunRole::Reviewer => Ok(()),
            RunAction::StoreHistory => {
                if !current.history_capture_enabled
                    || current.history_policy_version != self.history_policy_version
                {
                    return Err(Refusal::HistoryCaptureDisabledOrChanged);
                }
                Ok(())
            }
            _ => Err(Refusal::ActionNotAuthorized),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    InvalidIdentity,
    ProjectNotAuthorized,
    RunnerNotAuthorized,
    TargetMismatch,
    AuthorizationChanged,
    HistoryCaptureDisabledOrChanged,
    StaleRunnerIdentity,
    StaleExecutionClaim,
    ActionNotAuthorized,
    VersionExhausted,
}

impl Refusal {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::InvalidIdentity => "invalid_identity",
            Self::ProjectNotAuthorized => "project_not_authorized",
            Self::RunnerNotAuthorized => "runner_not_authorized",
            Self::TargetMismatch => "run_target_mismatch",
            Self::AuthorizationChanged => "authorization_changed",
            Self::HistoryCaptureDisabledOrChanged => "history_policy_changed",
            Self::StaleRunnerIdentity => "stale_runner_identity",
            Self::StaleExecutionClaim => "stale_execution_claim",
            Self::ActionNotAuthorized => "action_not_authorized",
            Self::VersionExhausted => "version_exhausted",
        }
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
}

fn next_version(current: u64) -> Result<u64, Refusal> {
    current.checked_add(1).ok_or(Refusal::VersionExhausted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runner(id: &str, generation: u64) -> VerifiedRunner {
        VerifiedRunner::from_authenticated_launch(id, generation).unwrap()
    }

    fn target(project: &str, task: &str, run: &str) -> RunTarget {
        RunTarget::new(project, task, run).unwrap()
    }

    fn enabled_project(role: RunRole, runner: &VerifiedRunner) -> ProjectAuthorization {
        let mut project = ProjectAuthorization::new("project-1").unwrap();
        project.grant_runner(role, runner).unwrap();
        project.set_enabled(true).unwrap();
        project
    }

    #[test]
    fn new_projects_deny_all_runs_and_history_by_default() {
        let runner = runner("runner-1", 1);
        let project = ProjectAuthorization::new("project-1").unwrap();
        assert!(!project.enabled);
        assert!(!project.history_capture_enabled);
        assert_eq!(
            project.authorize_run(
                target("project-1", "task-1", "run-1"),
                RunRole::Planner,
                &runner,
                1,
            ),
            Err(Refusal::ProjectNotAuthorized)
        );
    }

    #[test]
    fn only_the_granted_runner_and_role_can_bind_and_act_on_a_run() {
        let planner = runner("planner-1", 2);
        let other = runner("planner-2", 1);
        let mut project = enabled_project(RunRole::Planner, &planner);
        let run_target = target("project-1", "task-1", "run-1");
        let grant = project
            .authorize_run(run_target.clone(), RunRole::Planner, &planner, 7)
            .unwrap();

        assert_eq!(
            project.authorize_run(run_target.clone(), RunRole::Executor, &planner, 7,),
            Err(Refusal::RunnerNotAuthorized)
        );
        assert_eq!(
            project.authorize_run(run_target.clone(), RunRole::Planner, &other, 7,),
            Err(Refusal::RunnerNotAuthorized)
        );
        assert_eq!(
            grant.authorize_action(&project, &run_target, &planner, 7, RunAction::Plan,),
            Ok(())
        );
        assert_eq!(
            grant.authorize_action(&project, &run_target, &planner, 7, RunAction::Execute,),
            Err(Refusal::ActionNotAuthorized)
        );
    }

    #[test]
    fn grants_are_bound_to_the_exact_project_task_and_run() {
        let planner = runner("planner-1", 1);
        let project = enabled_project(RunRole::Planner, &planner);
        let bound = target("project-1", "task-1", "run-1");
        let grant = project
            .authorize_run(bound, RunRole::Planner, &planner, 1)
            .unwrap();

        for substituted in [
            target("project-2", "task-1", "run-1"),
            target("project-1", "task-2", "run-1"),
            target("project-1", "task-1", "run-2"),
        ] {
            assert_eq!(
                grant.authorize_action(&project, &substituted, &planner, 1, RunAction::Plan,),
                Err(Refusal::TargetMismatch)
            );
        }
    }

    #[test]
    fn revocation_and_scope_changes_invalidate_existing_run_grants() {
        let planner = runner("planner-1", 1);
        let mut project = enabled_project(RunRole::Planner, &planner);
        let run_target = target("project-1", "task-1", "run-1");
        let grant = project
            .authorize_run(run_target.clone(), RunRole::Planner, &planner, 1)
            .unwrap();

        project
            .revoke_runner(RunRole::Planner, "planner-1")
            .unwrap();
        assert_eq!(
            grant.authorize_action(&project, &run_target, &planner, 1, RunAction::Plan,),
            Err(Refusal::AuthorizationChanged)
        );

        project.grant_runner(RunRole::Planner, &planner).unwrap();
        let current = project
            .authorize_run(run_target.clone(), RunRole::Planner, &planner, 2)
            .unwrap();
        project.set_enabled(false).unwrap();
        assert_eq!(
            current.authorize_action(&project, &run_target, &planner, 2, RunAction::Plan,),
            Err(Refusal::AuthorizationChanged)
        );
    }

    #[test]
    fn history_requires_explicit_opt_in_and_its_own_fresh_policy_version() {
        let planner = runner("planner-1", 1);
        let mut project = enabled_project(RunRole::Planner, &planner);
        let run_target = target("project-1", "task-1", "run-1");
        let before_opt_in = project
            .authorize_run(run_target.clone(), RunRole::Planner, &planner, 3)
            .unwrap();
        assert_eq!(
            before_opt_in.authorize_action(
                &project,
                &run_target,
                &planner,
                3,
                RunAction::StoreHistory,
            ),
            Err(Refusal::HistoryCaptureDisabledOrChanged)
        );

        project.set_history_capture_enabled(true).unwrap();
        let opted_in = project
            .authorize_run(run_target.clone(), RunRole::Planner, &planner, 4)
            .unwrap();
        assert_eq!(
            opted_in.authorize_action(&project, &run_target, &planner, 4, RunAction::StoreHistory,),
            Ok(())
        );
        project.set_history_capture_enabled(false).unwrap();
        assert_eq!(
            opted_in.authorize_action(&project, &run_target, &planner, 4, RunAction::StoreHistory,),
            Err(Refusal::HistoryCaptureDisabledOrChanged)
        );
        assert_eq!(
            opted_in.authorize_action(&project, &run_target, &planner, 4, RunAction::Plan,),
            Ok(()),
            "history opt-in is independent from permission to plan"
        );
    }

    #[test]
    fn runner_restarts_and_reclaimed_claims_fence_late_actions() {
        let old_runner = runner("executor-1", 1);
        let restarted_runner = runner("executor-1", 2);
        let project = enabled_project(RunRole::Executor, &old_runner);
        let run_target = target("project-1", "task-1", "run-1");
        let grant = project
            .authorize_run(run_target.clone(), RunRole::Executor, &old_runner, 9)
            .unwrap();

        assert_eq!(
            grant.authorize_action(
                &project,
                &run_target,
                &restarted_runner,
                9,
                RunAction::Execute,
            ),
            Err(Refusal::StaleRunnerIdentity)
        );
        assert_eq!(
            grant.authorize_action(&project, &run_target, &old_runner, 10, RunAction::Execute,),
            Err(Refusal::StaleExecutionClaim)
        );
    }

    #[test]
    fn identifiers_and_versions_fail_closed_without_wrapping() {
        assert_eq!(
            RunTarget::new("project/one", "task-1", "run-1"),
            Err(Refusal::InvalidIdentity)
        );
        assert_eq!(
            VerifiedRunner::from_authenticated_launch("runner-1", 0),
            Err(Refusal::InvalidIdentity)
        );

        let runner = runner("runner-1", 1);
        let mut project = enabled_project(RunRole::Planner, &runner);
        project.authorization_version = u64::MAX;
        let before_grant = project.clone();
        assert_eq!(
            project.grant_runner(RunRole::Executor, &runner),
            Err(Refusal::VersionExhausted)
        );
        assert_eq!(project, before_grant, "failed grant must not mutate policy");
        assert_eq!(project.set_enabled(false), Err(Refusal::VersionExhausted));
        assert!(
            project.enabled,
            "failed version bump must leave policy unchanged"
        );

        project.history_policy_version = u64::MAX;
        assert_eq!(
            project.set_history_capture_enabled(true),
            Err(Refusal::VersionExhausted)
        );
        assert!(
            !project.history_capture_enabled,
            "failed policy version bump must leave history disabled"
        );
    }
}
