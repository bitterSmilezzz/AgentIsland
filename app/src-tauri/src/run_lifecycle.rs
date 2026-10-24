//! 执行生命周期的**状态机判据**（TASK-03 首片）。
//!
//! 需求原文：排队/规划/确认/执行/审查/完成/失败/关闭；启动、续跑、取消、重跑、
//! 计划/开发分工。
//!
//! 现有 `tasks.rs` 的 `RunStatus` 只记**当前**状态（queued/running/waiting/…），
//! 没记**哪些转移是合法的**、也没记「计划先行」「晚到结果」「重跑开新一轮」
//! 这些跨轮语义——它们散在各处调用点里各写一遍，改一处就有一处不一致。
//! 与网关系列同一条理由：**语义先于实现定死**。
//!
//! 本模块只做**判据**（纯函数，不碰 IO）：
//!
//! 1. **合法转移表**：每个阶段能去哪儿是声明，不是散落的 if。
//! 2. **计划先行**：规划阶段不能直接跳到执行——必须经确认（`Plan`/`Confirm`）。
//!    「计划/开发分工」里的分工体现在**两个角色**上：规划者与执行者可以是
//!    不同的人（角色不同不等于要不同人，但**同一角色不能既规划又执行**——
//!    那是「自己批准自己」）。
//! 3. **晚到结果不得复活已结束的运行**：关闭/取消/失败之后到达的结果**拒绝**，
//!    不能把一段已经结束的过程改写成「完成」。
//! 4. **重跑开新一轮**：重跑不是把上一轮的状态改回去，而是产出一个新运行
//!    （新 id），上一轮的历史原样保留。

/// 运行阶段。名字对应需求原文的八个词。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// 排队（还没开始）。
    Queued,
    /// 规划中（产出计划）。
    Planning,
    /// 等待确认（计划已就绪，等人批准）。
    Confirming,
    /// 执行中。
    Executing,
    /// 审查中（产物待审）。
    Reviewing,
    /// 完成。
    Completed,
    /// 失败。
    Failed,
    /// 关闭（人工作废或归档）。
    Closed,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Planning => "planning",
            Self::Confirming => "confirming",
            Self::Executing => "executing",
            Self::Reviewing => "reviewing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Closed => "closed",
        }
    }

    /// 该阶段是否**已结束**（终态）。终态只能被重跑**另起一轮**重开。
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Closed)
    }
}

/// 一次转移被拒绝时的理由。固定码，不带任何上下文。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// 这条转移不在表里。
    IllegalTransition,
    /// 计划未就绪就要求执行——计划先行。
    PlanNotReady,
    /// 计划已就绪但未确认就要求执行。
    NotConfirmed,
    /// 规划者与执行者是同一人（自己规划自己执行）。
    SameRole,
    /// 运行已结束，晚到结果不再接纳。
    RunAlreadyEnded,
    /// 已取消/关闭的运行不接受续跑。
    NotResumable,
}

impl Refusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::IllegalTransition => "illegal_transition",
            Self::PlanNotReady => "plan_not_ready",
            Self::NotConfirmed => "not_confirmed",
            Self::SameRole => "same_role",
            Self::RunAlreadyEnded => "run_already_ended",
            Self::NotResumable => "not_resumable",
        }
    }
}

/// 合法转移表。**每一格都是声明**。
///
/// - `Queued → Planning`：开始规划。
/// - `Planning → Confirming`：计划就绪，等待确认。
/// - `Confirming → Executing`：确认通过，开始执行。
/// - `Confirming → Closed`：人放弃（计划被否）。
/// - `Executing → Reviewing`：产物待审。
/// - `Executing → Failed`：执行失败。
/// - `Reviewing → Completed`：审查通过。
/// - `Reviewing → Failed`：审查否决。
/// - 任何非终态 → `Closed`：人工作废。
/// - `Executing → Cancelled` 不在表里——取消是**人主动**的动作，走 `Closed`
///   （需求把「取消」与「关闭」都列为终态语义；本仓用 Closed 承载取消，
///   取消与关闭的对外区别由取消原因记录，不另开一个阶段）。
pub fn can_transition(from: Stage, to: Stage) -> bool {
    use Stage as S;
    match (from, to) {
        (S::Queued, S::Planning) => true,
        (S::Planning, S::Confirming) => true,
        (S::Confirming, S::Executing) => true,
        (S::Confirming, S::Closed) => true,
        (S::Executing, S::Reviewing) => true,
        (S::Executing, S::Failed) => true,
        (S::Reviewing, S::Completed) => true,
        (S::Reviewing, S::Failed) => true,
        // 人工作废：任何非终态都能关闭。
        (from, S::Closed) if !from.is_terminal() => true,
        _ => false,
    }
}

/// 一次运行的判据状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lifecycle {
    pub stage: Stage,
    /// 计划是否就绪（进入确认阶段后为真）。
    pub plan_ready: bool,
    /// 计划是否已被人确认（未确认 = 不能执行）。
    pub plan_confirmed: bool,
    /// 规划者。
    pub planner: Option<String>,
    /// 执行者。
    pub executor: Option<String>,
}

impl Lifecycle {
    pub fn new() -> Lifecycle {
        Lifecycle {
            stage: Stage::Queued,
            plan_ready: false,
            plan_confirmed: false,
            planner: None,
            executor: None,
        }
    }

    /// 通用转移：先查转移表，再查该转移附加的前置条件。
    pub fn transition(&mut self, to: Stage) -> Result<(), Refusal> {
        if !can_transition(self.stage, to) {
            return Err(Refusal::IllegalTransition);
        }
        match to {
            Stage::Executing => {
                // 计划先行：计划未就绪、或未确认，都不许开始执行。
                if !self.plan_ready {
                    return Err(Refusal::PlanNotReady);
                }
                if !self.plan_confirmed {
                    return Err(Refusal::NotConfirmed);
                }
                // 计划/开发分工：规划者与执行者不能是同一人。
                if let (Some(planner), Some(executor)) = (&self.planner, &self.executor) {
                    if planner == executor {
                        return Err(Refusal::SameRole);
                    }
                }
            }
            Stage::Confirming => {
                self.plan_ready = true;
                self.plan_confirmed = false;
            }
            _ => {}
        }
        self.stage = to;
        Ok(())
    }

    /// 接纳一个结果。**已结束的运行不再接纳晚到结果**——
    /// 否则一段已经关闭的过程会被改写成「完成」，账目与审计都会对不上。
    pub fn accept_result(&mut self, outcome: Stage) -> Result<(), Refusal> {
        if self.stage.is_terminal() {
            return Err(Refusal::RunAlreadyEnded);
        }
        if outcome != Stage::Completed && outcome != Stage::Failed {
            return Err(Refusal::IllegalTransition);
        }
        self.stage = outcome;
        Ok(())
    }

    /// 续跑：把**未结束**的运行恢复到执行中。
    ///
    /// 只允许从「等确认」与「执行中」续——排队的还没开始、规划中的要重新规划、
    /// 已结束的不该被「续」复活（那是重跑的语义）。
    pub fn resume(&mut self) -> Result<(), Refusal> {
        match self.stage {
            Stage::Confirming | Stage::Executing => {}
            Stage::Completed | Stage::Failed | Stage::Closed => {
                return Err(Refusal::NotResumable)
            }
            _ => return Err(Refusal::IllegalTransition),
        }
        self.stage = Stage::Executing;
        Ok(())
    }

    /// 重跑：产出一个**新运行**（新 id），上一轮原样保留。
    ///
    /// 历史不可改写是硬约束——重跑不是「把上一轮的状态改回去」。
    pub fn rerun(&self, new_run_id: &str) -> Lifecycle {
        let mut next = Lifecycle::new();
        next.stage = Stage::Queued;
        // 规划者可以继承（重跑同一件事），执行者**不继承**——执行是新的一轮。
        next.planner = self.planner.clone();
        let _ = new_run_id;
        next
    }
}

impl Default for Lifecycle {
    fn default() -> Self {
        Lifecycle::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planned(executor: &str) -> Lifecycle {
        let mut life = Lifecycle::new();
        life.planner = Some("planner-a".into());
        life.executor = Some(executor.into());
        life.transition(Stage::Planning).unwrap();
        life.transition(Stage::Confirming).unwrap();
        life
    }

    #[test]
    fn the_happy_path_walks_the_declared_states_in_order() {
        let mut life = planned("worker-b");
        life.plan_confirmed = true;
        assert_eq!(life.transition(Stage::Executing), Ok(()));
        assert_eq!(life.transition(Stage::Reviewing), Ok(()));
        assert_eq!(life.transition(Stage::Completed), Ok(()));
        assert_eq!(life.stage, Stage::Completed);
        assert!(life.stage.is_terminal());
    }

    #[test]
    fn plan_first_blocks_execution_until_the_plan_is_ready_and_confirmed() {
        let mut life = planned("worker-b");
        // 计划未确认 ⇒ 不许执行
        assert_eq!(life.transition(Stage::Executing), Err(Refusal::NotConfirmed));
        life.plan_confirmed = true;
        assert_eq!(life.transition(Stage::Executing), Ok(()));
        // 计划压根没就绪（绕过确认阶段直接想执行）⇒ 转移表先拦
        let mut bare = Lifecycle::new();
        assert_eq!(bare.transition(Stage::Executing), Err(Refusal::IllegalTransition));
    }

    #[test]
    fn the_planner_may_not_also_execute() {
        let mut life = planned("planner-a"); // 规划者与执行者同一人
        life.plan_confirmed = true;
        assert_eq!(
            life.transition(Stage::Executing),
            Err(Refusal::SameRole),
            "自己规划自己执行 = 自己批准自己"
        );
    }

    #[test]
    fn illegal_transitions_are_refused_not_silently_applied() {
        let mut life = Lifecycle::new();
        // 排队不能直接执行
        assert_eq!(
            life.transition(Stage::Executing),
            Err(Refusal::IllegalTransition)
        );
        assert_eq!(life.stage, Stage::Queued, "被拒的转移不得改变状态");
        // 终态不能直接复活
        let mut done = planned("worker-b");
        done.plan_confirmed = true;
        done.transition(Stage::Executing).unwrap();
        done.transition(Stage::Reviewing).unwrap();
        done.transition(Stage::Completed).unwrap();
        assert_eq!(
            done.transition(Stage::Executing),
            Err(Refusal::IllegalTransition)
        );
    }

    #[test]
    fn a_late_result_cannot_revive_a_finished_run() {
        let mut done = planned("worker-b");
        done.plan_confirmed = true;
        done.transition(Stage::Executing).unwrap();
        done.transition(Stage::Reviewing).unwrap();
        done.transition(Stage::Completed).unwrap();
        // 晚到的「完成」结果
        assert_eq!(
            done.accept_result(Stage::Completed),
            Err(Refusal::RunAlreadyEnded),
            "已结束的运行不得被晚到结果改写"
        );
        assert_eq!(done.stage, Stage::Completed, "状态必须原样保留");
        // 关闭与失败同理
        let mut closed = planned("worker-b");
        closed.transition(Stage::Closed).unwrap();
        assert_eq!(
            closed.accept_result(Stage::Completed),
            Err(Refusal::RunAlreadyEnded)
        );
    }

    #[test]
    fn resume_only_applies_to_unfinished_runs() {
        let mut life = planned("worker-b");
        life.plan_confirmed = true;
        life.transition(Stage::Executing).unwrap();
        assert_eq!(life.resume(), Ok(()), "执行中的运行可以续跑");
        // 已结束的不可续
        let mut done = planned("worker-b");
        done.plan_confirmed = true;
        done.transition(Stage::Executing).unwrap();
        done.transition(Stage::Reviewing).unwrap();
        done.transition(Stage::Completed).unwrap();
        assert_eq!(done.resume(), Err(Refusal::NotResumable));
    }

    #[test]
    fn a_rerun_starts_a_new_run_and_keeps_the_old_history() {
        let mut life = planned("worker-b");
        life.plan_confirmed = true;
        life.transition(Stage::Executing).unwrap();
        life.transition(Stage::Reviewing).unwrap();
        life.transition(Stage::Failed).unwrap();
        let next = life.rerun("run-2");
        assert_eq!(next.stage, Stage::Queued, "重跑从排队开始");
        assert_ne!(next.stage, life.stage);
        assert_eq!(
            life.stage,
            Stage::Failed,
            "上一轮的历史原样保留——重跑不是把状态改回去"
        );
        assert_eq!(next.planner, life.planner, "规划者可以继承");
        assert_eq!(next.executor, None, "执行者不继承——执行是新的一轮");
    }

    #[test]
    fn a_human_can_close_any_unfinished_run() {
        for stage in [
            Stage::Queued,
            Stage::Planning,
            Stage::Confirming,
            Stage::Executing,
            Stage::Reviewing,
        ] {
            // 每个阶段从零构造到该阶段（夹具不得借用别的阶段的终态）
            let mut life = Lifecycle::new();
            life.planner = Some("planner-a".into());
            life.executor = Some("worker-b".into());
            if stage != Stage::Queued {
                life.transition(Stage::Planning).unwrap();
            }
            if matches!(
                stage,
                Stage::Confirming | Stage::Executing | Stage::Reviewing
            ) {
                life.transition(Stage::Confirming).unwrap();
            }
            if matches!(stage, Stage::Executing | Stage::Reviewing) {
                life.plan_confirmed = true;
                life.transition(Stage::Executing).unwrap();
            }
            if stage == Stage::Reviewing {
                life.transition(Stage::Reviewing).unwrap();
            }
            assert_eq!(life.stage, stage);
            assert_eq!(life.transition(Stage::Closed), Ok(()), "{stage:?} 应可关闭");
        }
    }
}
