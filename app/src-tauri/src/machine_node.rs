//! 多机器与执行节点的**注册、心跳与派发判据**（TASK-09 首片）。
//!
//! 需求原文：注册/心跳/能力/并发、自动/指定派发、离线恢复/抢占、可选云沙箱、
//! 睡眠/备份/恢复。场景 token：注册/心跳/撤销；并发限额/租约抢占；指定 offline 与自动 queue；
//! 重连不重复执行。
//!
//! 与 Chief 编排（TASK-02）同源，但粒度不同：TASK-02 管**逻辑角色与任务拆解**，
//! 本模块管**物理/云机器节点、槽位与心跳**。
//!
//! 本模块只做**判据**（纯函数，不碰真实网络、不起云沙箱）：
//!
//! 1. **机器注册与能力匹配**：机器声明支持的能力标签（如 "gpu" / "x86_64" / "arm64"）；
//!    任务指定所需能力，派发时**必须全匹配**才算候选节点。
//! 2. **心跳与状态判定**：定期心跳；超窗（`HEARTBEAT_TIMEOUT_MS`）判定为离线，
//!    离线节点不接新任务。
//! 3. **并发限额（槽位）**：每台机器有 `max_concurrency` 上限；满载节点不可派发。
//! 4. **派发策略（自动 vs 指定）**：
//!    - 指定机器：若目标离线或满载，按策略**排队（Queue）**或拒绝，绝不强塞给离线机器；
//!    - 自动派发：在满足能力的所有在线节点中选**当前负载最低**（空闲槽位最多）的；
//!      全部不可用时排队。
//! 5. **重连不重复执行（防脑裂）**：离线机器重连时，如果它之前认领的任务已在超时后
//!    被别人抢占/重新派发，旧机器**必须放弃**该任务——不得出现「两台机器执行同一个任务」。

use std::collections::{HashMap, HashSet};

/// 节点状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    Online,
    Offline,
    Draining, // 排水中：不再接新活，等当前活跑完
}

/// 机器节点的规格与状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineNode {
    pub id: String,
    pub label: String,
    /// 能力标签（如 "gpu", "macos", "rust-toolchain"）。
    pub capabilities: HashSet<String>,
    /// 最大并发槽位数。
    pub max_concurrency: usize,
    /// 当前已占用的槽位数。
    pub active_tasks: usize,
    /// 最后一次心跳时刻。
    pub last_heartbeat_ms: i64,
    /// 状态。
    pub status: NodeStatus,
}

impl MachineNode {
    /// 该节点当前是否允许接入新任务。
    pub fn can_accept(&self) -> bool {
        self.status == NodeStatus::Online && self.active_tasks < self.max_concurrency
    }

    /// 该节点是否满足任务的能力需求。
    pub fn satisfies(&self, required_capabilities: &[&str]) -> bool {
        required_capabilities
            .iter()
            .all(|cap| self.capabilities.contains(*cap))
    }

    /// 空闲槽位数。
    pub fn idle_slots(&self) -> usize {
        self.max_concurrency.saturating_sub(self.active_tasks)
    }
}

/// 默认心跳超时窗口：60 秒未心跳视作离线。
pub const HEARTBEAT_TIMEOUT_MS: i64 = 60 * 1000;

/// 派发决定的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchDecision {
    /// 派发给指定的机器。
    AssignTo(String),
    /// 当前没有满足条件的可用机器，任务进入排队。
    Queue,
    /// 指定的机器已被显式撤销或不存在。
    Rejected(&'static str),
}

/// 多机器节点注册与调度表（纯判据状态）。
#[derive(Debug, Default)]
pub struct NodeRegistry {
    nodes: HashMap<String, MachineNode>,
    /// 任务租约：task_id -> (assigned_machine_id, lease_version)
    leases: HashMap<String, (String, u64)>,
}

impl NodeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册或更新一个节点。
    pub fn register(
        &mut self,
        id: &str,
        label: &str,
        capabilities: &[&str],
        max_concurrency: usize,
        now_ms: i64,
    ) {
        let caps: HashSet<String> = capabilities.iter().map(|s| s.to_string()).collect();
        let node = self.nodes.entry(id.to_string()).or_insert_with(|| MachineNode {
            id: id.to_string(),
            label: label.to_string(),
            capabilities: caps.clone(),
            max_concurrency: max_concurrency.max(1),
            active_tasks: 0,
            last_heartbeat_ms: now_ms,
            status: NodeStatus::Online,
        });
        node.label = label.to_string();
        node.capabilities = caps;
        node.max_concurrency = max_concurrency.max(1);
        node.last_heartbeat_ms = now_ms;
        node.status = NodeStatus::Online;
    }

    /// 心跳续约。
    pub fn heartbeat(&mut self, id: &str, now_ms: i64) -> bool {
        if let Some(node) = self.nodes.get_mut(id) {
            node.last_heartbeat_ms = now_ms;
            if node.status == NodeStatus::Offline {
                node.status = NodeStatus::Online; // 心跳恢复在线
            }
            true
        } else {
            false
        }
    }

    /// 刷新所有节点的心跳状态（根据当前时间扫超时节点）。
    pub fn sweep_heartbeats(&mut self, now_ms: i64) {
        for node in self.nodes.values_mut() {
            if node.status == NodeStatus::Online
                && now_ms - node.last_heartbeat_ms >= HEARTBEAT_TIMEOUT_MS
            {
                node.status = NodeStatus::Offline;
            }
        }
    }

    /// 自动派发：在满足能力的在线节点中选空闲槽位最多者。全满或全离线时排队。
    ///
    /// **同空闲槽位时按 id 字典序取定**：HashMap 的迭代顺序是不确定的，
    /// 用 `max_by_key` 会在并列时随机挑一个——同一批任务在两次运行里
    /// 可能派到不同机器，账目与复现都无从谈起。
    pub fn dispatch_auto(
        &mut self,
        task_id: &str,
        required_capabilities: &[&str],
    ) -> DispatchDecision {
        let mut candidate: Option<&MachineNode> = None;
        for node in self.nodes.values() {
            if !node.can_accept() || !node.satisfies(required_capabilities) {
                continue;
            }
            match candidate {
                None => candidate = Some(node),
                Some(best) => {
                    let better = (node.idle_slots(), std::cmp::Reverse(node.id.as_str()))
                        > (best.idle_slots(), std::cmp::Reverse(best.id.as_str()));
                    if better {
                        candidate = Some(node);
                    }
                }
            }
        }

        if let Some(target) = candidate {
            let target_id = target.id.clone();
            self.claim_slot(&target_id, task_id);
            DispatchDecision::AssignTo(target_id)
        } else {
            DispatchDecision::Queue
        }
    }

    /// 指定机器派发：目标在线且有槽位才派，否则排队或拒绝。
    pub fn dispatch_targeted(
        &mut self,
        task_id: &str,
        target_machine: &str,
        required_capabilities: &[&str],
    ) -> DispatchDecision {
        let Some(node) = self.nodes.get(target_machine) else {
            return DispatchDecision::Rejected("machine_not_found");
        };
        if !node.satisfies(required_capabilities) {
            return DispatchDecision::Rejected("capability_mismatch");
        }
        if node.can_accept() {
            let id = node.id.clone();
            self.claim_slot(&id, task_id);
            DispatchDecision::AssignTo(id)
        } else {
            // 指定的机器离线或满载 ⇒ 进入排队等待该机器恢复或有槽位
            DispatchDecision::Queue
        }
    }

    fn claim_slot(&mut self, machine_id: &str, task_id: &str) {
        if let Some(node) = self.nodes.get_mut(machine_id) {
            node.active_tasks += 1;
        }
        let current_version = self.leases.get(task_id).map(|(_, v)| *v).unwrap_or(0);
        self.leases.insert(task_id.to_string(), (machine_id.to_string(), current_version + 1));
    }

    /// 任务完成或在节点上终止，释放槽位。
    pub fn release_slot(&mut self, machine_id: &str, task_id: &str) -> bool {
        self.leases.remove(task_id);
        if let Some(node) = self.nodes.get_mut(machine_id) {
            node.active_tasks = node.active_tasks.saturating_sub(1);
            true
        } else {
            false
        }
    }

    /// 抢占 / 重新派发：将任务从机器 A 重新分派给机器 B（租约版本推进）。
    pub fn preempt_to(&mut self, task_id: &str, new_machine: &str) -> Result<u64, &'static str> {
        let Some((old_machine, old_version)) = self.leases.get(task_id).cloned() else {
            return Err("task_not_leased");
        };
        if let Some(node) = self.nodes.get_mut(&old_machine) {
            node.active_tasks = node.active_tasks.saturating_sub(1);
        }
        let Some(new_node) = self.nodes.get_mut(new_machine) else {
            return Err("new_machine_not_found");
        };
        if !new_node.can_accept() {
            return Err("new_machine_busy_or_offline");
        }
        new_node.active_tasks += 1;
        let new_version = old_version + 1;
        self.leases.insert(task_id.to_string(), (new_machine.to_string(), new_version));
        Ok(new_version)
    }

    /// 重连机器核对租约：旧机器回报其持有的租约版本，若版本已过时 ⇒ **拒绝继续执行**（防脑裂）。
    pub fn verify_lease(&self, task_id: &str, machine_id: &str, claimed_version: u64) -> bool {
        match self.leases.get(task_id) {
            Some((current_machine, current_version)) => {
                current_machine == machine_id && *current_version == claimed_version
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_heartbeat_and_timeout_sweep() {
        let mut reg = NodeRegistry::new();
        reg.register("node-1", "Mac Studio", &["arm64", "gpu"], 4, 1_000);
        assert_eq!(reg.nodes["node-1"].status, NodeStatus::Online);
        assert_eq!(reg.nodes["node-1"].idle_slots(), 4);

        // 30s 后心跳正常续约
        assert!(reg.heartbeat("node-1", 1_000 + 30_000));
        assert_eq!(reg.nodes["node-1"].status, NodeStatus::Online);

        // 61s 未心跳触发超时扫除 ⇒ 离线
        reg.sweep_heartbeats(1_000 + 30_000 + HEARTBEAT_TIMEOUT_MS + 1000);
        assert_eq!(reg.nodes["node-1"].status, NodeStatus::Offline);
        assert!(!reg.nodes["node-1"].can_accept(), "离线节点不得接新活");

        // 离线节点重新心跳 ⇒ 恢复在线
        assert!(reg.heartbeat("node-1", 1_000 + 120_000));
        assert_eq!(reg.nodes["node-1"].status, NodeStatus::Online);
    }

    #[test]
    fn auto_dispatch_selects_least_busy_matching_node_or_queues() {
        let mut reg = NodeRegistry::new();
        reg.register("node-a", "节点 A (gpu 2槽)", &["gpu"], 2, 1_000);
        reg.register("node-b", "节点 B (gpu 4槽)", &["gpu"], 4, 1_000);
        reg.register("node-c", "节点 C (cpu 4槽)", &["cpu"], 4, 1_000);

        // 派发 gpu 任务：node-b 空闲槽最多 (4 > 2)
        let d1 = reg.dispatch_auto("task-1", &["gpu"]);
        assert_eq!(d1, DispatchDecision::AssignTo("node-b".into()));
        assert_eq!(reg.nodes["node-b"].active_tasks, 1);

        // 派发 cpu 任务：只有 node-c 满足
        let d2 = reg.dispatch_auto("task-2", &["cpu"]);
        assert_eq!(d2, DispatchDecision::AssignTo("node-c".into()));

        // 要求不存在的能力 (fpga) ⇒ 排队
        let d3 = reg.dispatch_auto("task-3", &["fpga"]);
        assert_eq!(d3, DispatchDecision::Queue);
    }

    /// 同空闲槽位时按 id 字典序取定——**确定性优先**：HashMap 迭代序不确定，
    /// 并列时随机挑会让同一批任务在两次运行里派到不同机器（账目与复现都无从谈起）。
    #[test]
    fn auto_dispatch_breaks_ties_by_machine_id() {
        for _ in 0..5 {
            let mut reg = NodeRegistry::new();
            reg.register("node-b", "B", &["worker"], 2, 1_000);
            reg.register("node-a", "A", &["worker"], 2, 1_000);
            reg.register("node-c", "C", &["worker"], 2, 1_000);
            // 三台机器槽位完全相同 ⇒ 每次都应派给字典序最小者
            let decision = reg.dispatch_auto("t1", &["worker"]);
            assert_eq!(
                decision,
                DispatchDecision::AssignTo("node-a".into()),
                "并列时必须按 id 字典序取定"
            );
        }
    }

    #[test]
    fn concurrency_cap_blocks_dispatch_when_exhausted() {
        let mut reg = NodeRegistry::new();
        reg.register("solo", "单槽机", &["any"], 1, 1_000);

        assert_eq!(
            reg.dispatch_auto("t1", &["any"]),
            DispatchDecision::AssignTo("solo".into())
        );
        // 满载 ⇒ 排队
        assert_eq!(reg.dispatch_auto("t2", &["any"]), DispatchDecision::Queue);

        // 释放 t1
        assert!(reg.release_slot("solo", "t1"));
        assert_eq!(reg.nodes["solo"].idle_slots(), 1);

        // 此时可以派发 t2
        assert_eq!(
            reg.dispatch_auto("t2", &["any"]),
            DispatchDecision::AssignTo("solo".into())
        );
    }

    #[test]
    fn targeted_dispatch_to_offline_machine_queues_not_forces() {
        let mut reg = NodeRegistry::new();
        reg.register("remote-box", "远程节点", &["linux"], 2, 1_000);
        reg.sweep_heartbeats(1_000 + HEARTBEAT_TIMEOUT_MS + 5000); // 让其超时离线

        // 指定发给离线机器 ⇒ 排队等待，绝不强塞
        let decision = reg.dispatch_targeted("task-99", "remote-box", &["linux"]);
        assert_eq!(
            decision,
            DispatchDecision::Queue,
            "指定离线机器必须排队等待，不得强发"
        );
    }

    #[test]
    fn lease_preemption_and_reconnection_prevents_split_brain() {
        let mut reg = NodeRegistry::new();
        reg.register("m1", "机器 1", &["worker"], 2, 1_000);
        reg.register("m2", "机器 2", &["worker"], 2, 1_000);

        // t1 派给 m1，租约版本 1
        reg.dispatch_auto("t1", &["worker"]);
        assert!(reg.verify_lease("t1", "m1", 1));

        // m1 离线，系统将 t1 抢占重新派给 m2，版本升至 2
        let new_ver = reg.preempt_to("t1", "m2").unwrap();
        assert_eq!(new_ver, 2);

        // m2 当前持有有效租约 (版本 2)
        assert!(reg.verify_lease("t1", "m2", 2));

        // m1 此时网络恢复重连，试图汇报版本 1 的旧任务 ⇒ 租约失效，拒绝执行（防脑裂）
        assert!(
            !reg.verify_lease("t1", "m1", 1),
            "旧机器持有的陈旧租约必须被拒绝，防止两台机器执行同一任务"
        );
    }
}
