//! 任务定时与触发的**调度判据**（TASK-08 首片）。
//!
//! 需求原文：一次/小时/日/周、本地时区/DST、重叠策略、暂停/取消、离线/额度、
//! 错过与一次重试可观察。
//!
//! 与网关系列同一条理由：**调度语义先于实现定死**——定时器的实现可以是系统
//! cron、tokio timer 或后台线程，但「上一轮没跑完该跳过还是排队」「错过了要不要补」
//! 属于业务口径，散在各处实现必然会漂。
//!
//! 本模块只做**判据**（纯函数，不碰系统时钟、不启动真实定时器）：
//!
//! 1. **四类调度规格**：`Once`（一次性）、`Hourly`（每小时第 M 分）、
//!    `Daily`（每天 H:M）、`Weekly`（每周几的 H:M）。
//! 2. **三类重叠策略**（`OverlapPolicy`）：
//!    - `Skip`：上一轮还在跑，本拍**跳过**并记入 `missed_count`（不堆积）；
//!    - `Queue`：排队（带上限 `MAX_QUEUED=5`，超限仍跳过）；
//!    - `Replace`：取消上一轮、启动新一轮。
//! 3. **错过与重试可观察**：`missed_count` 记录跳过次数；失败的触发
//!    **最多自动重试一次**（`retries_left: 0..=1`），第二次失败直接记入账本不循环。
//! 4. **暂停与取消**：`Paused` 冻结触发，不丢配置；`Cancelled` 是终态。

/// 调度规格。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleSpec {
    /// 一次性：在指定时刻触发一次。
    Once { target_ms: i64 },
    /// 每小时：在每小时的第 `minute` 分钟（0..=59）。
    Hourly { minute: u8 },
    /// 每天：在本地时间的 `hour:minute`（0..=23, 0..=59）。
    Daily { hour: u8, minute: u8 },
    /// 每周：在本地周几的 `hour:minute`（weekday 1..=7，1=周一）。
    Weekly { weekday: u8, hour: u8, minute: u8 },
}

/// 重叠策略（当上一轮尚未结束、新一拍到达时）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OverlapPolicy {
    /// 默认：跳过本拍，记录一次错过。
    #[default]
    Skip,
    /// 排队等待上一轮结束（有界）。
    Queue,
    /// 取消上一轮，启动新一轮。
    Replace,
}

/// 排队重叠的最大深度。
pub const MAX_QUEUED_OVERLAPS: usize = 5;

/// 调度项的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleState {
    Active,
    /// 人手动暂停（**只有人能解除**）。
    Paused,
    /// 额度熔断（额度恢复后可自动回到 Active）。
    BudgetExhausted,
    Cancelled,
    Finished, // Once 触发完成
}

/// 触发决定的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerAction {
    /// 立即启动一次运行。
    Fire,
    /// 上一轮还在跑，按策略跳过；missed 计数已累加。
    SkipOverlap,
    /// 上一轮还在跑，进入重叠排队。
    QueueOverlap,
    /// 取消上一轮并启动新一轮。
    ReplaceRunning,
    /// 处于暂停、取消或完成状态，不触发。
    Suppressed,
    /// 额度熔断：不触发（已在跑的**不打断**）。
    BudgetBlocked,
}

/// 一条定时触发任务的判据状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledTask {
    pub id: String,
    pub spec: ScheduleSpec,
    pub overlap_policy: OverlapPolicy,
    pub state: ScheduleState,
    /// 上一轮是否仍在执行中。
    pub is_running: bool,
    /// 排队的重叠触发次数（Queue 策略下使用）。
    pub queued_overlaps: usize,
    /// 累计错过的触发次数（可观察指标）。
    pub missed_count: u64,
    /// 累计成功触发次数。
    pub fire_count: u64,
    /// 剩余重试次数：每轮触发分配 1 次，失败重试后归 0。
    pub retries_left: u8,
    /// 额度上限（`None` = 未设额度 = 不限）。
    pub budget_limit: Option<u64>,
    /// 本周期已消耗的额度。
    pub budget_spent: u64,
}

impl ScheduledTask {
    pub fn new(id: &str, spec: ScheduleSpec, overlap_policy: OverlapPolicy) -> Self {
        ScheduledTask {
            id: id.to_string(),
            spec,
            overlap_policy,
            state: ScheduleState::Active,
            is_running: false,
            queued_overlaps: 0,
            missed_count: 0,
            fire_count: 0,
            retries_left: 1,
            budget_limit: None,
            budget_spent: 0,
        }
    }

    /// 设置额度上限（`None` = 不限）。
    pub fn with_budget(mut self, limit: Option<u64>) -> Self {
        self.budget_limit = limit;
        self
    }

    /// 额度是否已耗尽。
    pub fn budget_exhausted(&self) -> bool {
        match self.budget_limit {
            Some(limit) => self.budget_spent >= limit,
            None => false,
        }
    }

    /// 消费额度（一次触发记一次）。**不打断在跑的运行**——
    /// 额度用尽只挡新活，打断在跑的只会留下半成品。
    pub fn spend_budget(&mut self, amount: u64) {
        self.budget_spent = self.budget_spent.saturating_add(amount);
        self.apply_budget_state();
    }

    /// 额度周期重置：把已消耗清零并**尝试解冻**。
    ///
    /// **手动暂停不会被额度重置解除**——暂停是人的决定，只有人能解除。
    pub fn reset_budget(&mut self) {
        self.budget_spent = 0;
        self.apply_budget_state();
    }

    /// 额度状态同步到调度状态。
    fn apply_budget_state(&mut self) {
        if self.state == ScheduleState::Paused || self.state == ScheduleState::Cancelled {
            return; // 手动暂停/取消优先，不被额度状态覆盖
        }
        if self.state == ScheduleState::Finished {
            return;
        }
        self.state = if self.budget_exhausted() {
            ScheduleState::BudgetExhausted
        } else {
            ScheduleState::Active
        };
    }

    /// 额度恢复（充值/额度提升）：若当前因额度熔断则回到 Active。
    pub fn grant_budget(&mut self, additional: u64) {
        self.budget_spent = self.budget_spent.saturating_sub(additional);
        self.apply_budget_state();
    }

    /// 触发拍到达时的判定。
    pub fn on_schedule_tick(&mut self) -> TriggerAction {
        match self.state {
            ScheduleState::Active => {}
            ScheduleState::BudgetExhausted => return TriggerAction::BudgetBlocked,
            _ => return TriggerAction::Suppressed,
        }
        if self.budget_exhausted() {
            self.state = ScheduleState::BudgetExhausted;
            return TriggerAction::BudgetBlocked;
        }

        if !self.is_running {
            self.is_running = true;
            self.fire_count += 1;
            self.retries_left = 1;
            if matches!(self.spec, ScheduleSpec::Once { .. }) {
                self.state = ScheduleState::Finished;
            }
            return TriggerAction::Fire;
        }

        // 上一轮仍在运行：按重叠策略分流
        match self.overlap_policy {
            OverlapPolicy::Skip => {
                self.missed_count += 1;
                TriggerAction::SkipOverlap
            }
            OverlapPolicy::Queue => {
                if self.queued_overlaps < MAX_QUEUED_OVERLAPS {
                    self.queued_overlaps += 1;
                    TriggerAction::QueueOverlap
                } else {
                    self.missed_count += 1; // 队列满了仍记错过
                    TriggerAction::SkipOverlap
                }
            }
            OverlapPolicy::Replace => {
                self.fire_count += 1;
                self.retries_left = 1;
                TriggerAction::ReplaceRunning
            }
        }
    }

    /// 上一轮运行结束通知。
    ///
    /// 若有排队的重叠触发，出队并立即触发下一次。返回是否启动了排队的运行。
    pub fn on_run_finished(&mut self, success: bool) -> bool {
        self.is_running = false;
        if !success && self.retries_left > 0 {
            // 失败自动重试一次
            self.retries_left = 0;
            self.is_running = true;
            return true;
        }
        if self.queued_overlaps > 0 {
            self.queued_overlaps -= 1;
            self.is_running = true;
            self.fire_count += 1;
            self.retries_left = 1;
            return true;
        }
        false
    }

    /// 暂停触发（不改变规格，不丢计数）。
    ///
    /// **手动暂停优先于额度状态**：额度恢复/重置都不会解除它——暂停是人的决定。
    pub fn pause(&mut self) {
        if self.state == ScheduleState::Active
            || self.state == ScheduleState::BudgetExhausted
        {
            self.state = ScheduleState::Paused;
        }
    }

    /// 恢复触发（只有人能解除手动暂停）。
    pub fn resume(&mut self) {
        if self.state == ScheduleState::Paused {
            self.state = if self.budget_exhausted() {
                ScheduleState::BudgetExhausted
            } else {
                ScheduleState::Active
            };
        }
    }

    /// 取消调度（终态）。
    pub fn cancel(&mut self) {
        self.state = ScheduleState::Cancelled;
        self.queued_overlaps = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn once_task_fires_and_transitions_to_finished() {
        let mut task = ScheduledTask::new(
            "t-once",
            ScheduleSpec::Once { target_ms: 1_000 },
            OverlapPolicy::Skip,
        );
        assert_eq!(task.on_schedule_tick(), TriggerAction::Fire);
        assert_eq!(task.state, ScheduleState::Finished);
        assert_eq!(task.fire_count, 1);
        // 完成后再次拍触发 ⇒ Suppressed
        assert_eq!(task.on_schedule_tick(), TriggerAction::Suppressed);
    }

    #[test]
    fn skip_policy_increments_missed_count_on_overlap() {
        let mut task = ScheduledTask::new(
            "t-hourly",
            ScheduleSpec::Hourly { minute: 0 },
            OverlapPolicy::Skip,
        );
        // 第一拍启动
        assert_eq!(task.on_schedule_tick(), TriggerAction::Fire);
        assert!(task.is_running);
        assert_eq!(task.missed_count, 0);

        // 上一轮未结束，第二拍到达 ⇒ 跳过并记错过
        assert_eq!(task.on_schedule_tick(), TriggerAction::SkipOverlap);
        assert_eq!(task.missed_count, 1);
        assert_eq!(task.on_schedule_tick(), TriggerAction::SkipOverlap);
        assert_eq!(task.missed_count, 2);

        // 上一轮成功结束
        task.on_run_finished(true);
        assert!(!task.is_running);

        // 下一拍照常启动
        assert_eq!(task.on_schedule_tick(), TriggerAction::Fire);
    }

    #[test]
    fn queue_policy_buffers_overlaps_up_to_cap() {
        let mut task = ScheduledTask::new(
            "t-queue",
            ScheduleSpec::Daily { hour: 9, minute: 0 },
            OverlapPolicy::Queue,
        );
        assert_eq!(task.on_schedule_tick(), TriggerAction::Fire);

        // 排 5 次（满额）
        for _ in 0..MAX_QUEUED_OVERLAPS {
            assert_eq!(task.on_schedule_tick(), TriggerAction::QueueOverlap);
        }
        assert_eq!(task.queued_overlaps, MAX_QUEUED_OVERLAPS);
        assert_eq!(task.missed_count, 0);

        // 第 6 次超限 ⇒ 降级为跳过并记错过
        assert_eq!(task.on_schedule_tick(), TriggerAction::SkipOverlap);
        assert_eq!(task.missed_count, 1);

        // 上一轮结束 ⇒ 自动出队一条并启动
        assert!(task.on_run_finished(true));
        assert!(task.is_running);
        assert_eq!(task.queued_overlaps, MAX_QUEUED_OVERLAPS - 1);
    }

    #[test]
    fn replace_policy_cancels_running_and_fires_new() {
        let mut task = ScheduledTask::new(
            "t-replace",
            ScheduleSpec::Hourly { minute: 30 },
            OverlapPolicy::Replace,
        );
        assert_eq!(task.on_schedule_tick(), TriggerAction::Fire);
        // 新拍到达 ⇒ 替换
        assert_eq!(task.on_schedule_tick(), TriggerAction::ReplaceRunning);
        assert_eq!(task.fire_count, 2);
    }

    #[test]
    fn failed_run_retries_exactly_once() {
        let mut task = ScheduledTask::new(
            "t-retry",
            ScheduleSpec::Daily { hour: 0, minute: 0 },
            OverlapPolicy::Skip,
        );
        task.on_schedule_tick();

        // 第一次失败 ⇒ 自动重试（返回 true 且仍在 running）
        assert!(task.on_run_finished(false));
        assert!(task.is_running);
        assert_eq!(task.retries_left, 0);

        // 重试再次失败 ⇒ 不再重试（返回 false，running 变为 false）
        assert!(!task.on_run_finished(false));
        assert!(!task.is_running);
    }

    #[test]
    fn an_exhausted_budget_blocks_new_ticks_but_does_not_interrupt_a_running_one() {
        let mut task = ScheduledTask::new(
            "t-budget",
            ScheduleSpec::Hourly { minute: 0 },
            OverlapPolicy::Skip,
        )
        .with_budget(Some(2));

        // 两轮触发消耗完额度（每轮结束后再触发下一轮——重叠策略下上一轮未结束时
        // 本就是 SkipOverlap，不是 Fire）
        assert_eq!(task.on_schedule_tick(), TriggerAction::Fire);
        task.spend_budget(1);
        task.on_run_finished(true);
        assert_eq!(task.on_schedule_tick(), TriggerAction::Fire);
        task.spend_budget(1);
        assert!(task.budget_exhausted());
        // **第二轮此刻仍在跑**（不调 on_run_finished）——下面要断言的就是
        // 「熔断不打断在跑的那次」。

        // 额度耗尽 ⇒ 不再触发新活，但**在跑的那次不受影响**
        assert!(task.is_running, "额度熔断不得打断在跑的运行");
        assert_eq!(task.on_schedule_tick(), TriggerAction::BudgetBlocked);
        assert_eq!(task.state, ScheduleState::BudgetExhausted);
        assert!(task.is_running, "熔断后仍在跑的那次继续跑完");
    }

    #[test]
    fn budget_recovery_unfreezes_but_never_unpauses_a_manual_pause() {
        let mut task = ScheduledTask::new(
            "t-recover",
            ScheduleSpec::Daily { hour: 9, minute: 0 },
            OverlapPolicy::Skip,
        )
        .with_budget(Some(1));
        task.on_schedule_tick();
        task.spend_budget(1);
        task.on_run_finished(true);
        assert_eq!(task.on_schedule_tick(), TriggerAction::BudgetBlocked);

        // 额度恢复 ⇒ 解冻
        task.grant_budget(1);
        assert_eq!(task.state, ScheduleState::Active);
        assert_eq!(task.on_schedule_tick(), TriggerAction::Fire);
        task.on_run_finished(true);

        // 手动暂停后，额度重置**不解除**暂停（暂停是人的决定）
        task.pause();
        task.spend_budget(1);
        task.reset_budget();
        assert_eq!(
            task.state,
            ScheduleState::Paused,
            "额度重置不得解除手动暂停——只有人能解除"
        );
        assert_eq!(task.on_schedule_tick(), TriggerAction::Suppressed);

        // 人自己恢复 ⇒ 触发
        task.resume();
        assert_eq!(task.state, ScheduleState::Active);
        assert_eq!(task.on_schedule_tick(), TriggerAction::Fire);
    }

    #[test]
    fn pause_and_resume_freeze_ticks_without_losing_spec() {
        let mut task = ScheduledTask::new(
            "t-pause",
            ScheduleSpec::Hourly { minute: 15 },
            OverlapPolicy::Skip,
        );
        task.pause();
        assert_eq!(task.on_schedule_tick(), TriggerAction::Suppressed);

        task.resume();
        assert_eq!(task.on_schedule_tick(), TriggerAction::Fire);

        task.cancel();
        assert_eq!(task.on_schedule_tick(), TriggerAction::Suppressed);
    }
}
