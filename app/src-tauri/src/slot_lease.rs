//! **槽位租约**的判据（TASK-02 第四片 / TASK-09 收窄）。
//!
//! 需求原文：TASK-02「角色/运行时/模型/记忆，主控与工人分工」；
//! TASK-09「注册/心跳/能力/并发、自动/指定派发、离线恢复/抢占」。
//!
//! 背景：TASK-02 第二片（v0.0.415）给了四类资源的**配置形态判据**
//! （`agent_resources.rs`），其中「运行时并发槽位上限」用的是**裸计数**
//! （`runtime_usage: HashMap<String, usize>`）；TASK-09 首片（v0.0.381）给了
//! 多机器的**租约版本核对**（`machine_node.rs`），但两边的槽位口径**没有打通**：
//!
//! - 裸计数**不知道谁持有**：`release_runtime_slot("local")` 减 1，但槽位
//!   早已易主给另一个任务，减的是**别人的**占用 ⇒ 账实不符 ⇒ 并发超卖。
//! - 裸计数**没有版本**：迟到的一次释放（任务早该结束时网络抖动才送达）
//!   会把**新持有者**的槽位减掉，或把已释放的槽位减成负数（被 saturating
//!   掩盖，于是「多减了一次」看不出来）。
//!
//! 本模块给出一条**统一的槽位租约口径**（纯函数，不碰 IO、不起进程）：
//!
//! 1. **租约有版本**：每次占用递增版本；持有者必须**带着版本**才能释放。
//!    版本不符 ⇒ **拒绝且不动账**（迟到释放不许踢掉新持有者）。
//! 2. **释放只认当前持有者**：释放一个已易主的租约 ⇒ 拒绝（不是「减 1」）。
//! 3. **占用幂等**：同一任务重复占用同一运行时 ⇒ **不重复计数**
//!    （重复派发会让计数虚高 ⇒ 看起来满了，其实没满）。
//! 4. **超限拒绝且不动账**：占用超限 ⇒ 拒绝，占用数**保持原样**
//!    （拒绝路径绝不能留下半个副作用）。
//!
//! **为什么不是「计数 + 校验」的组合**：计数没有「谁持有」的信息，
//! 而「谁持有」正是迟到释放与脑裂防护的判据。租约版本**就是**那个信息。

use std::collections::HashMap;

/// 槽位租约的拒绝原因。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseRefusal {
    /// 运行时（或节点）不存在。
    UnknownRuntime,
    /// 槽位已满：并发上限不允许再占。
    SlotExhausted,
    /// 租约版本已过时：迟到释放 / 陈旧持有者。
    StaleLease,
    /// 该任务不在此运行时的租约表里（从未占用，或已被释放/易主）。
    NoLease,
}

impl LeaseRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownRuntime => "unknown_runtime",
            Self::SlotExhausted => "slot_exhausted",
            Self::StaleLease => "stale_lease",
            Self::NoLease => "no_lease",
        }
    }
}

/// 一个运行时的槽位配置与当前占用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotPool {
    /// 并发上限。
    pub max_concurrency: usize,
    /// 当前被占用的槽位数（= 租约条数）。
    used: usize,
    /// 任务 → 租约版本。**一条租约 = 一个占用槽位**。
    leases: HashMap<String, u64>,
    /// 版本计数器（单调递增，已发出的版本不复用）。
    version_counter: u64,
}

impl SlotPool {
    pub fn new(max_concurrency: usize) -> Self {
        SlotPool {
            max_concurrency,
            used: 0,
            leases: HashMap::new(),
            version_counter: 0,
        }
    }

    /// 当前占用数。
    pub fn used(&self) -> usize {
        self.used
    }

    /// 空闲槽位数。
    pub fn idle(&self) -> usize {
        self.max_concurrency.saturating_sub(self.used)
    }

    /// 某任务当前持有的租约版本（None = 未持有）。
    pub fn lease_version(&self, task_id: &str) -> Option<u64> {
        self.leases.get(task_id).copied()
    }

    /// 占用一个槽位。
    ///
    /// - **幂等**：同一任务重复占用 ⇒ 返回**已有的版本**，占用数不变
    ///   （重复派发不应把计数抬高）。
    /// - **超限拒绝**：占用数达到上限 ⇒ 拒绝，**占用数保持原样**。
    pub fn acquire(&mut self, task_id: &str) -> Result<u64, LeaseRefusal> {
        if let Some(version) = self.leases.get(task_id) {
            return Ok(*version);
        }
        if self.used >= self.max_concurrency {
            return Err(LeaseRefusal::SlotExhausted);
        }
        let version = self.next_version();
        self.leases.insert(task_id.to_string(), version);
        self.used += 1;
        Ok(version)
    }

    /// 释放一个槽位。**必须带版本**。
    ///
    /// - 版本不符（迟到释放 / 陈旧持有者）⇒ 拒绝，**账不动**；
    /// - 该任务没有租约 ⇒ 拒绝（`NoLease`）——释放一个不存在的占用
    ///   会凭空多出空闲槽位。
    pub fn release(&mut self, task_id: &str, version: u64) -> Result<(), LeaseRefusal> {
        match self.leases.get(task_id).copied() {
            None => Err(LeaseRefusal::NoLease),
            Some(current) if current != version => Err(LeaseRefusal::StaleLease),
            Some(_) => {
                self.leases.remove(task_id);
                self.used = self.used.saturating_sub(1);
                Ok(())
            }
        }
    }

    /// 把一个任务的租约**易主**给新的持有者（抢占 / 重新认领）。
    ///
    /// 占用数**不变**（同一个槽位换人），版本**递增**（旧持有者的版本立刻过时）。
    pub fn transfer(&mut self, task_id: &str, new_holder: &str) -> Result<u64, LeaseRefusal> {
        if !self.leases.contains_key(task_id) {
            return Err(LeaseRefusal::NoLease);
        }
        let version = self.next_version();
        self.leases.insert(task_id.to_string(), version);
        let _ = new_holder;
        Ok(version)
    }

    /// 下一个租约版本号。**单调递增**，不复用。
    ///
    /// 用一个表内计数器而不是「当前最大版本 + 1」：后者在租约被删干净后
    /// 会退回 1，于是「版本 1 的迟到释放」可能撞上「新持有者的版本 1」。
    fn next_version(&mut self) -> u64 {
        self.version_counter = self.version_counter.saturating_add(1);
        self.version_counter
    }
}

/// 运行时 → 槽位池。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlotRegistry {
    pools: HashMap<String, SlotPool>,
}

impl SlotRegistry {
    pub fn new() -> Self {
        SlotRegistry::default()
    }

    /// 登记一个运行时及其并发上限。**重登记不改已有租约**（上限下调时
    /// 已占用的槽位仍算占用，只是不能再新增）。
    pub fn register_runtime(&mut self, runtime: &str, max_concurrency: usize) {
        self.pools
            .entry(runtime.to_string())
            .or_insert_with(|| SlotPool::new(max_concurrency));
    }

    /// 登记一个运行时（覆盖上限）。
    pub fn set_max_concurrency(&mut self, runtime: &str, max_concurrency: usize) {
        self.pools
            .entry(runtime.to_string())
            .and_modify(|pool| pool.max_concurrency = max_concurrency)
            .or_insert_with(|| SlotPool::new(max_concurrency));
    }

    /// 占用某运行时的一个槽位。
    pub fn acquire(&mut self, runtime: &str, task_id: &str) -> Result<u64, LeaseRefusal> {
        let pool = self
            .pools
            .get_mut(runtime)
            .ok_or(LeaseRefusal::UnknownRuntime)?;
        pool.acquire(task_id)
    }

    /// 释放某运行时上的一个槽位（带版本核对）。
    pub fn release(
        &mut self,
        runtime: &str,
        task_id: &str,
        version: u64,
    ) -> Result<(), LeaseRefusal> {
        let pool = self
            .pools
            .get_mut(runtime)
            .ok_or(LeaseRefusal::UnknownRuntime)?;
        pool.release(task_id, version)
    }

    /// 把租约易主（抢占）。
    pub fn transfer(
        &mut self,
        runtime: &str,
        task_id: &str,
        new_holder: &str,
    ) -> Result<u64, LeaseRefusal> {
        let pool = self
            .pools
            .get_mut(runtime)
            .ok_or(LeaseRefusal::UnknownRuntime)?;
        pool.transfer(task_id, new_holder)
    }

    /// 当前占用数。
    pub fn used(&self, runtime: &str) -> usize {
        self.pools.get(runtime).map(|p| p.used()).unwrap_or(0)
    }

    /// 空闲槽位数。
    pub fn idle(&self, runtime: &str) -> usize {
        self.pools.get(runtime).map(|p| p.idle()).unwrap_or(0)
    }

    /// 某任务在某运行时持有的租约版本。
    pub fn lease_version(&self, runtime: &str, task_id: &str) -> Option<u64> {
        self.pools.get(runtime)?.lease_version(task_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> SlotRegistry {
        let mut reg = SlotRegistry::new();
        reg.register_runtime("local", 2);
        reg
    }

    #[test]
    fn acquire_then_release_returns_the_slot() {
        let mut reg = registry();
        let version = reg.acquire("local", "task-1").unwrap();
        assert_eq!(reg.used("local"), 1);
        assert_eq!(reg.idle("local"), 1);
        reg.release("local", "task-1", version).unwrap();
        assert_eq!(reg.used("local"), 0);
        assert_eq!(reg.idle("local"), 2);
    }

    #[test]
    fn repeated_acquire_is_idempotent_and_does_not_inflate_the_count() {
        let mut reg = registry();
        let first = reg.acquire("local", "task-1").unwrap();
        let second = reg.acquire("local", "task-1").unwrap();
        assert_eq!(first, second, "重复占用同一任务 ⇒ 版本不变");
        assert_eq!(reg.used("local"), 1, "重复占用不得把计数抬高");
    }

    #[test]
    fn exceeding_the_cap_is_refused_and_leaves_the_count_untouched() {
        let mut reg = registry();
        reg.acquire("local", "task-1").unwrap();
        reg.acquire("local", "task-2").unwrap();
        assert_eq!(reg.acquire("local", "task-3"), Err(LeaseRefusal::SlotExhausted));
        assert_eq!(reg.used("local"), 2, "拒绝路径不得留下半个副作用");
        assert_eq!(reg.idle("local"), 0);
    }

    /// **迟到释放不得踢掉新持有者**——这正是裸计数口径的真实缺陷：
    /// 旧机器迟到的释放会把新持有者的槽位减掉，于是并发超卖。
    #[test]
    fn a_late_release_cannot_take_the_new_holder_slot() {
        let mut reg = registry();
        let old_version = reg.acquire("local", "task-1").unwrap();
        // 抢占：任务易主给新持有者，版本推进
        let new_version = reg.transfer("local", "task-1", "worker-b").unwrap();
        assert_ne!(old_version, new_version);

        // 旧持有者迟到的释放 ⇒ 拒绝，且账不动
        assert_eq!(
            reg.release("local", "task-1", old_version),
            Err(LeaseRefusal::StaleLease)
        );
        assert_eq!(reg.used("local"), 1, "迟到释放不得减掉新持有者的占用");
        assert_eq!(reg.idle("local"), 1);

        // 新持有者释放 ⇒ 成功
        reg.release("local", "task-1", new_version).unwrap();
        assert_eq!(reg.used("local"), 0);
    }

    #[test]
    fn releasing_a_task_without_a_lease_is_refused() {
        let mut reg = registry();
        assert_eq!(
            reg.release("local", "never-acquired", 1),
            Err(LeaseRefusal::NoLease)
        );
        assert_eq!(reg.used("local"), 0);
        assert_eq!(reg.idle("local"), 2, "凭空释放不得多出空闲槽位");
    }

    #[test]
    fn an_unknown_runtime_is_refused_on_both_paths() {
        let mut reg = registry();
        assert_eq!(
            reg.acquire("nope", "task-1"),
            Err(LeaseRefusal::UnknownRuntime)
        );
        assert_eq!(
            reg.release("nope", "task-1", 1),
            Err(LeaseRefusal::UnknownRuntime)
        );
    }

    /// 版本号**不复用**：租约被释放干净后再占用，拿到的不是旧的版本 1。
    /// 否则「旧版本 1 的迟到释放」会撞上「新持有者的版本 1」。
    #[test]
    fn lease_versions_are_never_reused() {
        let mut reg = registry();
        let first = reg.acquire("local", "task-1").unwrap();
        reg.release("local", "task-1", first).unwrap();
        let second = reg.acquire("local", "task-2").unwrap();
        assert_ne!(
            first, second,
            "版本必须单调递增：旧版本 1 的迟到释放不能撞上新版本 1"
        );
        assert_eq!(
            reg.release("local", "task-2", first),
            Err(LeaseRefusal::StaleLease)
        );
    }
}
