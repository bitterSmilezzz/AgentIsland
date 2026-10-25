//! Chief（主控）与团队 Agent 的**编排判据**（TASK-02 首片）。
//!
//! 需求原文：目标拆解、分派、主题/watch/check-in、角色/运行时/模型/记忆，
//! 主控与工人分工。场景 token：需求拆解/去重；任务指派/主题 watch 幂等；
//! worker 失败；预算/停止；memory 污染与跨主题权限。
//!
//! 与网关系列、生命周期判据同一条理由：**语义先于实现定死**——编排规则散在
//! 调用点各写一遍，每加一个 worker 角色就多一套「谁来干、谁说了算」的隐式约定。
//!
//! 本模块只做**判据**（纯函数，不碰 IO、不起进程）：
//!
//! 1. **拆解**：一个目标拆成若干子任务；**同样的子任务不重复拆**（去重键是
//!    目标 + 子任务标题的规范化形式）。
//! 2. **分派**：主控指派工人；**同一主题的 watch 幂等**（同一 (主题, 工人)
//!    只 watch 一次——重复 watch 会让同一个事件被通知两遍）。
//! 3. **worker 失败**：失败的工人**不自动重派**——重派策略是人的决定
//!    （和「结果不越权批准」同源：编排层不替人做执行决定）。
//! 4. **预算与停止**：预算耗尽或收到停止 ⇒ **不再分派新的子任务**，
//!    已在跑的工人**不被打断**（停止是「不再接新活」，不是「杀掉在跑的」）。
//! 5. **记忆隔离**：记忆按主题分域；**跨主题读取被拒**（防止 A 主题的记忆
//!    污染 B 主题的上下文）。记忆污染比缺记忆危险得多。

/// 角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// 主控（Chief）：拆解目标、分派任务。
    Chief,
    /// 工人（Worker）：执行被分派的子任务。
    Worker,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chief => "chief",
            Self::Worker => "worker",
        }
    }
}

/// 一个子任务。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubTask {
    /// 所属主题（记忆与权限的分域单位）。
    pub topic: String,
    /// 标题（去重键的一部分）。
    pub title: String,
}

/// 拆解结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decomposition {
    /// 拆出了这些子任务（可能为空：目标已经达成时不必拆）。
    Tasks(Vec<SubTask>),
    /// 目标本身已经完成，无需拆解。
    AlreadyDone,
}

/// 子任务的**去重键**：主题 + 规范化标题。
///
/// 规范化：**去掉所有空白**。理由：中文标题里「修 登录页」与「修登录页」
/// 是同一件事（空格是排版产物），而英文里 "fix login" 与 "fixlogin" 不是——
/// 但把空白全去掉对英文会过度合并。取舍：本仓的子任务标题以中文为主，
/// 去空白是更贴近实际的归一；**大小写敏感**（大小写差异是不是同一件事，
/// 得由人判断，程序不该替人合并）。
pub fn dedup_key(task: &SubTask) -> String {
    let normalized: String = task
        .title
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    format!("{}\u{1f}{normalized}", task.topic)
}

/// 拆解：把目标拆成子任务并**去重**（同主题同标题只留一个）。
///
/// 输入顺序保留（作者写的顺序有意义）；去重保留**首次出现**的那条。
pub fn decompose(tasks: Vec<SubTask>) -> Decomposition {
    let mut seen: Vec<String> = Vec::new();
    let mut kept: Vec<SubTask> = Vec::new();
    for task in tasks {
        let key = dedup_key(&task);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        kept.push(task);
    }
    if kept.is_empty() {
        Decomposition::AlreadyDone
    } else {
        Decomposition::Tasks(kept)
    }
}

/// 预算的计量单位（需求原文：预算/停止；消掉裸数字语义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetUnit {
    /// Token 消耗（总数）。
    Tokens,
    /// 子任务分派次数（每派一个扣 1）。
    Tasks,
    /// 运行墙钟秒数。
    Seconds,
}

impl BudgetUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tokens => "tokens",
            Self::Tasks => "tasks",
            Self::Seconds => "seconds",
        }
    }
}

/// 带单位的预算账本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetLedger {
    pub unit: BudgetUnit,
    pub limit: u64,
    pub spent: u64,
}

impl BudgetLedger {
    pub fn new(unit: BudgetUnit, limit: u64) -> Self {
        BudgetLedger { unit, limit, spent: 0 }
    }

    pub fn remaining(&self) -> u64 {
        self.limit.saturating_sub(self.spent)
    }

    pub fn is_exhausted(&self) -> bool {
        self.spent >= self.limit
    }

    pub fn record_spend(&mut self, amount: u64) {
        self.spent = self.spent.saturating_add(amount);
    }
}

/// 工人的 check-in 记录（最后活跃时刻）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckInRecord {
    pub worker: String,
    pub last_seen_ms: i64,
}

/// 默认 check-in 超时窗口：5 分钟（工人超过 5 分钟未签到判定失联）。
pub const CHECKIN_TIMEOUT_MS: i64 = 5 * 60 * 1000;

/// 编排状态。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Orchestration {
    /// 已被 watch 的 (主题, 工人) 组合。
    pub watched: Vec<(String, String)>,
    /// 已分派的子任务（按分派顺序）。
    pub assigned: Vec<SubTask>,
    /// 剩余预算（兼容旧字段；`None` = 未设预算 = 不限）。
    pub remaining_budget: Option<u64>,
    /// 带单位的显式预算账本。
    pub budget_ledger: Option<BudgetLedger>,
    /// 是否收到停止指令。
    pub stopped: bool,
    /// 发生的工人故障记录（待人工介入）。
    pub incidents: Vec<FailureIncident>,
    /// 工人 check-in 记录表（worker -> CheckInRecord）。
    pub checkins: std::collections::HashMap<String, CheckInRecord>,
}

impl Orchestration {
    pub fn new() -> Orchestration {
        Orchestration::default()
    }

    /// 设置带单位的预算账本。
    pub fn with_budget(mut self, unit: BudgetUnit, limit: u64) -> Self {
        self.budget_ledger = Some(BudgetLedger::new(unit, limit));
        self.remaining_budget = Some(limit);
        self
    }

    /// 工人定期签到（check-in）。更新最后活跃时刻。
    pub fn checkin(&mut self, worker: &str, now_ms: i64) {
        let entry = self
            .checkins
            .entry(worker.to_string())
            .or_insert_with(|| CheckInRecord {
                worker: worker.to_string(),
                last_seen_ms: now_ms,
            });
        entry.last_seen_ms = now_ms;
    }

    /// 判定某个工人是否**失联**（超过给定窗口未签到）。
    /// 未签到过的工人视作尚未就绪，不报失联。
    pub fn is_worker_stale(&self, worker: &str, now_ms: i64, timeout_ms: i64) -> bool {
        match self.checkins.get(worker) {
            Some(rec) => now_ms - rec.last_seen_ms >= timeout_ms,
            None => false,
        }
    }

    /// 是否还允许分派新的子任务。
    ///
    /// **停止与预算只挡新活，不打断在跑的工人**——杀掉在跑的工人可能留下
    /// 半成品状态，那比多接一个活更糟。
    pub fn accepts_new_work(&self) -> bool {
        if self.stopped {
            return false;
        }
        if let Some(ledger) = &self.budget_ledger {
            if ledger.is_exhausted() {
                return false;
            }
        }
        match self.remaining_budget {
            Some(0) => false,
            _ => true,
        }
    }

    /// 分派一个子任务给工人。
    ///
    /// 拒绝原因：已停止 / 预算耗尽 / 该子任务已分派过（重复分派会让同一个任务
    /// 被两个工人同时干）。
    pub fn assign(&mut self, task: SubTask, worker: &str) -> Result<(), Refusal> {
        if !self.accepts_new_work() {
            return Err(if self.stopped {
                Refusal::Stopped
            } else {
                Refusal::BudgetExhausted
            });
        }
        let key = dedup_key(&task);
        if self.assigned.iter().any(|existing| dedup_key(existing) == key) {
            return Err(Refusal::AlreadyAssigned);
        }
        self.assigned.push(task);
        Ok(())
    }

    /// 开始 watch 一个 (主题, 工人)。**幂等**：同一组合重复 watch 不重复登记。
    pub fn watch(&mut self, topic: &str, worker: &str) -> bool {
        let pair = (topic.to_string(), worker.to_string());
        if self.watched.contains(&pair) {
            return false;
        }
        self.watched.push(pair);
        true
    }

    /// 消费预算（分派一次花一次；带单位账本同步更新）。
    pub fn spend(&mut self, amount: u64) {
        if let Some(ledger) = &mut self.budget_ledger {
            ledger.record_spend(amount);
            self.remaining_budget = Some(ledger.remaining());
        } else if let Some(remaining) = self.remaining_budget {
            self.remaining_budget = Some(remaining.saturating_sub(amount));
        }
    }

    /// 记忆读取：只允许读**同主题**的记忆。
    ///
    /// 跨主题读取被拒——那正是「memory 污染」的入口：A 主题里的结论被带进
    /// B 主题的上下文，看起来像 B 主题自己知道的事。
    pub fn read_memory<'a>(
        &self,
        topic: &str,
        memories: &'a [(String, String)],
    ) -> Result<Vec<&'a String>, Refusal> {
        let mut allowed = Vec::new();
        for (scope, content) in memories {
            if scope == topic {
                allowed.push(content);
            } else {
                return Err(Refusal::CrossTopicMemory);
            }
        }
        Ok(allowed)
    }

    /// 记录一个工人的失败。
    ///
    /// 失败的子任务**不自动重派**——自动重派等于编排层替人做决定，
    /// 而失败可能是环境坏了、指令有误或超出权限，重跑只会重复失败。
    /// 状态标记为待人工介入；子任务仍在 `assigned` 里，防止被并发重派。
    pub fn record_failure(&mut self, task: &SubTask, worker: &str, reason: &str) -> String {
        let key = dedup_key(task);
        let incident_id = format!("inc-{}", self.incidents.len() + 1);
        self.incidents.push(FailureIncident {
            incident_id: incident_id.clone(),
            task_key: key,
            worker: worker.to_string(),
            reason: reason.to_string(),
            reassigned: false,
        });
        incident_id
    }

    /// **人工介入重派**：必须显式引用故障单（`incident_id`）并指名新工人。
    ///
    /// 判据：
    /// - 故障单不存在 ⇒ 拒绝；
    /// - 故障单已重派过 ⇒ 拒绝（不许重复介入）；
    /// - 新工人与原失败工人相同 ⇒ 拒绝（原样重派给同一个工人毫无意义）；
    /// - 编排已停止或预算耗尽 ⇒ 拒绝（停止只挡新活，包括重派）。
    pub fn manual_reassign(
        &mut self,
        incident_id: &str,
        new_worker: &str,
    ) -> Result<SubTask, Refusal> {
        if !self.accepts_new_work() {
            return Err(if self.stopped {
                Refusal::Stopped
            } else {
                Refusal::BudgetExhausted
            });
        }
        let incident = self
            .incidents
            .iter_mut()
            .find(|i| i.incident_id == incident_id)
            .ok_or(Refusal::IncidentNotFound)?;

        if incident.reassigned {
            return Err(Refusal::AlreadyReassigned);
        }
        if incident.worker == new_worker {
            return Err(Refusal::SameWorkerReassign);
        }

        incident.reassigned = true;
        let task = self
            .assigned
            .iter()
            .find(|t| dedup_key(t) == incident.task_key)
            .cloned()
            .ok_or(Refusal::IncidentNotFound)?;

        Ok(task)
    }
}

/// 工人故障记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureIncident {
    pub incident_id: String,
    pub task_key: String,
    pub worker: String,
    pub reason: String,
    pub reassigned: bool,
}

/// 拒绝原因。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// 已收到停止指令。
    Stopped,
    /// 预算耗尽。
    BudgetExhausted,
    /// 该子任务已分派过。
    AlreadyAssigned,
    /// 跨主题读取记忆被拒。
    CrossTopicMemory,
    /// 人工介入引用的故障单不存在。
    IncidentNotFound,
    /// 该故障已重派过。
    AlreadyReassigned,
    /// 重派给同一个工人（无意义重试）。
    SameWorkerReassign,
}

impl Refusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::BudgetExhausted => "budget_exhausted",
            Self::AlreadyAssigned => "already_assigned",
            Self::CrossTopicMemory => "cross_topic_memory",
            Self::IncidentNotFound => "incident_not_found",
            Self::AlreadyReassigned => "already_reassigned",
            Self::SameWorkerReassign => "same_worker_reassign",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(topic: &str, title: &str) -> SubTask {
        SubTask {
            topic: topic.into(),
            title: title.into(),
        }
    }

    #[test]
    fn decomposition_deduplicates_the_same_subtask_and_keeps_the_first() {
        let result = decompose(vec![
            task("web", "修登录页"),
            task("db", "改迁移脚本"),
            task("web", "修 登录页"), // 折叠空白后与第一条同键
        ]);
        match result {
            Decomposition::Tasks(tasks) => {
                assert_eq!(tasks.len(), 2, "同主题同标题只留一条");
                assert_eq!(tasks[0].title, "修登录页", "保留首次出现的那条");
                assert_eq!(tasks[1].topic, "db");
            }
            other => panic!("应当拆出子任务，实际 {other:?}"),
        }
        // 不同主题的同名任务是两件事
        let result = decompose(vec![task("web", "迁移"), task("db", "迁移")]);
        assert!(matches!(result, Decomposition::Tasks(t) if t.len() == 2));
    }

    #[test]
    fn an_already_done_goal_is_not_decomposed_into_nothing() {
        assert_eq!(decompose(vec![]), Decomposition::AlreadyDone);
    }

    #[test]
    fn assignment_refuses_a_duplicate_task_but_allows_a_different_one() {
        let mut orchestration = Orchestration::new();
        assert_eq!(
            orchestration.assign(task("web", "修登录页"), "worker-1"),
            Ok(())
        );
        assert_eq!(
            orchestration.assign(task("web", "修登录页"), "worker-2"),
            Err(Refusal::AlreadyAssigned),
            "同一个任务不能分给两个工人"
        );
        assert_eq!(
            orchestration.assign(task("web", "修注册页"), "worker-2"),
            Ok(())
        );
        assert_eq!(orchestration.assigned.len(), 2);
    }

    #[test]
    fn watching_the_same_topic_worker_pair_is_idempotent() {
        let mut orchestration = Orchestration::new();
        assert!(orchestration.watch("web", "worker-1"), "首次 watch 登记");
        assert!(
            !orchestration.watch("web", "worker-1"),
            "同一 (主题, 工人) 重复 watch 不再登记——否则同一事件会被通知两遍"
        );
        assert!(orchestration.watch("web", "worker-2"), "不同工人各自 watch");
        assert!(orchestration.watch("db", "worker-1"), "不同主题各自 watch");
        assert_eq!(orchestration.watched.len(), 3);
    }

    #[test]
    fn a_stopped_or_exhausted_orchestration_accepts_no_new_work() {
        let mut stopped = Orchestration::new();
        stopped.stopped = true;
        assert_eq!(
            stopped.assign(task("web", "任务"), "worker-1"),
            Err(Refusal::Stopped)
        );

        let mut exhausted = Orchestration::new();
        exhausted.remaining_budget = Some(0);
        assert_eq!(
            exhausted.assign(task("web", "任务"), "worker-1"),
            Err(Refusal::BudgetExhausted)
        );

        // 未设预算 = 不限（不是「预算为 0」）
        let mut unlimited = Orchestration::new();
        assert!(unlimited.accepts_new_work());
        unlimited.spend(1_000);
        assert!(unlimited.accepts_new_work(), "未设预算就不该因花费而停");
    }

    #[test]
    fn memory_is_readable_only_within_its_topic() {
        let orchestration = Orchestration::new();
        let memories = vec![
            ("web".to_string(), "登录页用的是 OAuth".to_string()),
            ("db".to_string(), "迁移脚本要改".to_string()),
        ];
        // 跨主题读取被拒——那正是 memory 污染的入口
        assert_eq!(
            orchestration.read_memory("web", &memories),
            Err(Refusal::CrossTopicMemory),
            "读了别的主题的记忆 ⇒ 污染"
        );
        // 只放同主题的记忆时读得到
        let same_topic = vec![("web".to_string(), "登录页用的是 OAuth".to_string())];
        let read = orchestration.read_memory("web", &same_topic).unwrap();
        assert_eq!(read.len(), 1);
    }

    #[test]
    fn worker_failure_does_not_auto_reassign_and_requires_manual_intervention() {
        let mut orch = Orchestration::new();
        let sub = task("web", "复杂重构");
        orch.assign(sub.clone(), "worker-1").unwrap();

        // worker-1 失败：留单，**不自动重派**
        let inc_id = orch.record_failure(&sub, "worker-1", "out of memory");
        assert_eq!(orch.incidents.len(), 1);
        assert!(!orch.incidents[0].reassigned);

        // 重派给原失败工人 ⇒ 拒绝（原样重试无意义）
        assert_eq!(
            orch.manual_reassign(&inc_id, "worker-1"),
            Err(Refusal::SameWorkerReassign)
        );

        // 人工指定新工人 worker-2 ⇒ 成功，返回被重派的任务
        let reassigned = orch.manual_reassign(&inc_id, "worker-2").unwrap();
        assert_eq!(reassigned.title, "复杂重构");
        assert!(orch.incidents[0].reassigned);

        // 重复介入同一单 ⇒ 拒绝
        assert_eq!(
            orch.manual_reassign(&inc_id, "worker-3"),
            Err(Refusal::AlreadyReassigned)
        );
    }

    #[test]
    fn manual_reassignment_respects_stopped_and_budget_guards() {
        let mut orch = Orchestration::new();
        let sub = task("web", "任务");
        orch.assign(sub.clone(), "worker-1").unwrap();
        let inc_id = orch.record_failure(&sub, "worker-1", "network timeout");

        // 停止状态下人工重派也必须被挡住（停止 = 不出新活，包括重派）
        orch.stopped = true;
        assert_eq!(
            orch.manual_reassign(&inc_id, "worker-2"),
            Err(Refusal::Stopped)
        );
    }

    #[test]
    fn budget_ledger_tracks_units_and_blocks_on_exhaustion() {
        let mut orch = Orchestration::new().with_budget(BudgetUnit::Tasks, 2);
        assert_eq!(orch.budget_ledger.as_ref().unwrap().unit.as_str(), "tasks");
        assert_eq!(orch.budget_ledger.as_ref().unwrap().remaining(), 2);
        assert!(orch.accepts_new_work());

        // 分派第 1 个任务
        orch.assign(task("web", "任务-1"), "worker-1").unwrap();
        orch.spend(1);
        assert_eq!(orch.budget_ledger.as_ref().unwrap().remaining(), 1);
        assert!(orch.accepts_new_work());

        // 分派第 2 个任务，额度耗尽
        orch.assign(task("web", "任务-2"), "worker-2").unwrap();
        orch.spend(1);
        assert_eq!(orch.budget_ledger.as_ref().unwrap().remaining(), 0);
        assert!(orch.budget_ledger.as_ref().unwrap().is_exhausted());
        assert!(!orch.accepts_new_work(), "额度耗尽后不再接受新任务");

        // 试图分派第 3 个任务 ⇒ 拒绝
        assert_eq!(
            orch.assign(task("web", "任务-3"), "worker-3"),
            Err(Refusal::BudgetExhausted)
        );
    }

    #[test]
    fn worker_checkin_tracks_freshness_and_detects_staleness() {
        let mut orch = Orchestration::new();
        // 刚 check-in 的工人是新鲜的
        orch.checkin("worker-1", 1_000);
        assert!(!orch.is_worker_stale("worker-1", 1_000 + 60_000, CHECKIN_TIMEOUT_MS));

        // 超过窗口（5 分钟）未签到 ⇒ 判定失联
        assert!(orch.is_worker_stale(
            "worker-1",
            1_000 + CHECKIN_TIMEOUT_MS + 1000,
            CHECKIN_TIMEOUT_MS
        ));

        // 重新签到恢复
        orch.checkin("worker-1", 1_000 + CHECKIN_TIMEOUT_MS + 2000);
        assert!(!orch.is_worker_stale(
            "worker-1",
            1_000 + CHECKIN_TIMEOUT_MS + 3000,
            CHECKIN_TIMEOUT_MS
        ));

        // 从未签到过的工人不判定失联（尚未就绪）
        assert!(!orch.is_worker_stale("worker-new", 5_000, CHECKIN_TIMEOUT_MS));
    }
}
