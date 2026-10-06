//! 进程终止与异常清理（对应 Swift `AgentCleaner` + `ProcessTerminator`）。
//!
//! **这是全仓唯一一个「会杀进程」的地方**，所以安全规则集中在这里，且拆成两层：
//!
//! - [`plan`] —— **纯函数**。输入两份进程表（扫描时刻 / 复核时刻）与异常列表，
//!   输出「对每个 pid 做什么」的一串决定。它不发任何信号，因此**每一条安全规则
//!   都能用构造出来的表离线断言**，不必在真机上等一个死锁进程发生。
//! - [`execute`] / [`verify`] —— 执行与复核。拿到 [`plan`] 已经算好的决定，
//!   一个信号都不自己决定发不发。
//!
//! 三条安全规则（都落在 `plan` 里，不散在调用方）：
//!
//! 1. **动手前按可执行身份复核。** 异常列表可能已经陈旧——扫完之后进程退出、
//!    PID 被系统复用给一个无关程序。带复核的 kill 会误杀，而**误杀整棵进程树
//!    的后果远比「漏杀一个真异常」严重**。身份对不上 ⇒ 立即放弃（[`PlanAction::PidReused`]）。
//! 2. **僵尸不算存活。** 进程已退出、只是父进程还没 `wait` 回收，此时探活照样
//!    返回「在」。把它算成「收到终止信号后仍在运行」是把复核做成假警报——僵尸既不
//!    占 CPU 也不占内存，而用户看到的是一句「杀不掉」。
//! 3. **「发了信号」与「进程真没了」是两件事。** 清理结果**只能**靠复核得出
//!    （[`verify`]），且只统计**确认退出**的那些进程的内存。
//!
//! 批量与逐条的分工：当前扫描只生成疑似死锁与孤儿；历史内存超限类型保留兼容。
//! 孤儿（`ppid == 1`）
//! **只允许逐条手动**——它与 launchd 刻意托管的常驻服务从 `ppid` 上分不开，
//! 批量误杀等于静默丢任务。孤儿还要求「10 分钟内有会话写入」这条活动佐证
//! （[`looks_orphan`]）。**宁可漏杀**是这一整块的定位。
//!
//! 进程事实一律取自 [`crate::procmon`] 的进程表（sysinfo），不另开 `ps` 查询：
//! 同一张表同时给出 pid / ppid / 可执行名 / 是否僵尸，少一次查询就少一个
//! 「两次查询之间进程已经变了」的口子。

use crate::procmon::ProcHit;
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// 终止后等多久再复核。SIGTERM 有 300ms 优雅期 + SIGKILL 兜底，0.8s 让两者都落地。
pub const RECHECK_DELAY: Duration = Duration::from_millis(800);
/// SIGTERM 到 SIGKILL 的优雅期
const TERM_GRACE: Duration = Duration::from_millis(300);
/// 孤儿佐证窗口：10 分钟内仍有会话写入的档案算「还活着」
pub const ORPHAN_EVIDENCE_WINDOW: Duration = Duration::from_secs(600);
/// pid <= 1 永不作为目标：0 是「任意进程组」，1 是 init / launchd
pub const MIN_TARGET_PID: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnomalyType {
    /// 父进程转为 launchd（`ppid == 1`）且无活动佐证
    Orphan,
    /// 持续异常高 CPU 且观测够久
    Hung,
    /// 内存超限
    Overweight,
}

impl AnomalyType {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Orphan => "孤儿进程",
            Self::Hung => "疑似死锁",
            Self::Overweight => "内存超限",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Anomaly {
    pub pid: u32,
    pub ppid: u32,
    pub profile_id: String,
    pub agent_name: String,
    /// **扫描时刻**记录的可执行身份（见 [`identity`]）。
    ///
    /// 它是「我们以为在杀谁」的凭据，复核时拿它与当前进程表比对。
    /// **空串 = 没记到身份**，而空身份在 [`plan`] 里一律判为不可动手——
    /// 宁可漏杀，也不做一次无复核的 kill。
    pub command_path: String,
    pub memory_bytes: u64,
    pub anomaly_type: AnomalyType,
    pub reason: String,
}

impl Anomaly {
    /// **是否可进入批量清理**。孤儿一律 `false`。
    pub fn batch_cleanable(&self) -> bool {
        self.anomaly_type != AnomalyType::Orphan
    }

    /// 列表身份：**必须带 `profileId`**——同一 pid 可能被两个 profile 同时匹配
    /// （GUI 主程序与其 CLI 子工具同名），只用「类型-pid」会撞 id。
    pub fn identity(&self) -> String {
        format!(
            "{}-{}-{}",
            self.profile_id,
            self.anomaly_type.label(),
            self.pid
        )
    }
}

/// 进程的可执行身份：优先整条路径，取不到就退回进程名。
///
/// 比对时**只看 basename**（见 [`same_identity`]），所以「优先路径」只是为了
/// 在复核失败时能把**两个**候选都印出来给人看。
pub fn identity(hit: &ProcHit) -> String {
    if hit.exe_path.is_empty() {
        hit.name.clone()
    } else {
        hit.exe_path.clone()
    }
}

pub fn find<'a>(table: &'a [ProcHit], pid: u32) -> Option<&'a ProcHit> {
    table.iter().find(|h| h.pid == pid)
}

fn basename(path: &str) -> &str {
    let trimmed = path.trim();
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

/// 两个身份是否指同一个程序。
///
/// **只比 basename**：brew 升级会让整条路径变掉，但同名仍是同一个程序；
/// 比整条路径会把每一次升级都变成「杀不掉」。
///
/// 任何一边为空 ⇒ `false`。空身份**不是**「无从比较所以放行」，
/// 而是「没记到是谁，不许动手」。
pub fn same_identity(expected: &str, current: &str) -> bool {
    let want = basename(expected);
    let got = basename(current);
    if want.is_empty() || got.is_empty() {
        return false;
    }
    want.eq_ignore_ascii_case(got)
}

/// 进程是否仍在。**僵尸算已退出**（见模块头规则 2）。
pub fn is_alive(table: &[ProcHit], pid: u32) -> bool {
    find(table, pid).is_some_and(|hit| !hit.is_zombie)
}

/// 终止顺序：**先子后父**，`root` 本身排在最后。
///
/// 父进程先死会让子进程被 init 收养、ppid 变成 1，于是再也认不出它是谁家的。
/// 遍历复用 [`crate::trees::build_tree`]——那份已经处理过环、脏数据与深层嵌套，
/// 这里再写一份遍历就是两份算法必然分叉。
pub fn kill_order(root: u32, table: &[ProcHit]) -> Vec<u32> {
    if root == 0 {
        return Vec::new();
    }
    // 先序遍历收集后代，再倒序 ⇒ 子孙先于父。
    fn walk(nodes: &[crate::trees::TreeNode], out: &mut Vec<u32>) {
        for node in nodes {
            out.push(node.pid);
            walk(&node.children, out);
        }
    }
    let mut descendants = Vec::new();
    walk(
        &crate::trees::build_tree(root, table).nodes,
        &mut descendants,
    );
    descendants.reverse();
    descendants.push(root);
    descendants
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanAction {
    /// 复核通过，可以发信号
    Signal,
    /// 复核时这个 pid 已是另一个程序 ⇒ 放弃
    PidReused,
    /// 没记到可执行身份 ⇒ 无法复核 ⇒ 放弃
    NoIdentity,
    /// 复核时进程已经不在（或已变僵尸）
    Gone,
    /// pid <= 1：0 是「任意进程组」、1 是 init / launchd
    ProtectedPid,
    /// 这是我们自己或本进程的祖先（终端）
    OwnProcess,
    /// 孤儿但没有活动佐证
    OrphanNoEvidence,
    /// 孤儿出现在批量清理里 ⇒ 必须逐条点名
    OrphanNeedsNaming,
}

impl PlanAction {
    /// 会不会真的发信号。只有这一个为真时 [`execute`] 才动手。
    pub fn signals(self) -> bool {
        matches!(self, Self::Signal)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Signal => "终止",
            Self::PidReused => "跳过：PID 已被复用",
            Self::NoIdentity => "跳过：没记到可执行身份，无法复核",
            Self::Gone => "跳过：进程已不在",
            Self::ProtectedPid => "跳过：受保护的 PID",
            Self::OwnProcess => "跳过：那是我们自己",
            Self::OrphanNoEvidence => "跳过：孤儿但没有活动佐证",
            Self::OrphanNeedsNaming => "跳过：孤儿须逐条点名",
        }
    }
}

/// 计划里的一步：对这个 pid 做什么。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillStep {
    pub pid: u32,
    /// 复核用的身份（扫描时刻记下的）
    pub identity: String,
    pub action: PlanAction,
    /// 归属哪条异常——树里派生的进程要能追回它是从哪儿来的
    pub anomaly: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KillPlan {
    pub steps: Vec<KillStep>,
}

impl KillPlan {
    /// 真正会发信号的那些 pid。
    pub fn signal_targets(&self) -> Vec<u32> {
        self.steps
            .iter()
            .filter(|s| s.action.signals())
            .map(|s| s.pid)
            .collect()
    }
}

/// 本进程及其全部祖先。**永远不作为目标**——杀掉自己的祖先就是杀掉终端。
pub fn self_and_ancestors() -> HashSet<u32> {
    let mut out = HashSet::new();
    // `libc::getpid` 给 i32；负值（理论上不该出现）一律当 0，那必然被 `pid <= 1` 挡掉
    let mut pid = unsafe { libc::getpid() }.max(0) as u32;
    // 上限 64：祖先链在脏数据下可能成环
    for _ in 0..64 {
        if pid <= 1 || !out.insert(pid) {
            break;
        }
        pid = (unsafe { libc::getppid() }).max(0) as u32;
    }
    out
}

/// 判孤儿：**`ppid == 1` 且没有活动佐证**。两条缺一不可。
///
/// `ppid == 1` 分不开「终端关掉的遗孤」与「launchd 刻意托管的常驻服务」；
/// 但后者**必有活动佐证**（10 分钟内有会话写入），所以佐证能把误杀挡掉。
/// 宁可漏杀——这正是本模块的定位。
pub fn looks_orphan(anomaly: &Anomaly, recently_active: &HashSet<String>) -> bool {
    anomaly.ppid == 1 && !recently_active.contains(&anomaly.profile_id)
}

/// 排一份终止计划。**纯函数，不发任何信号。**
///
/// `scanned` 是扫描时刻的进程表（异常里记的身份与它同源），
/// `fresh` 是动手前重新采的进程表（复核用）。两者不同表正是 PID 复用的检测窗口。
///
/// `batch` = 批量模式（不带位置参数）。批量模式下孤儿一律不排进计划。
pub fn plan(
    anomalies: &[Anomaly],
    scanned: &[ProcHit],
    fresh: &[ProcHit],
    own: &HashSet<u32>,
    recently_active: &HashSet<String>,
    batch: bool,
) -> KillPlan {
    let mut out = KillPlan::default();
    // 同一个 pid 只排一次步：两条异常共享子进程时（父被判死锁、子也单列一条）
    // 不重复发信号。保序去重——顺序本身就是终止顺序，不能为了去重而重排。
    let mut planned: HashSet<u32> = HashSet::new();
    for anomaly in anomalies {
        let label = anomaly.identity();
        let orphan = looks_orphan(anomaly, recently_active);
        if batch && anomaly.anomaly_type == AnomalyType::Orphan {
            out.steps.push(KillStep {
                pid: anomaly.pid,
                identity: anomaly.command_path.clone(),
                action: PlanAction::OrphanNeedsNaming,
                anomaly: label.clone(),
            });
            continue;
        }
        for pid in kill_order(anomaly.pid, scanned) {
            if !planned.insert(pid) {
                continue;
            }
            // **根进程用异常里记的那份身份**（`Anomaly.command_path`）：
            // 它才是「扫描时我们以为在杀谁」的凭据，也是界面上那条记录带来的。
            // 派生进程没有各自的异常记录，退回扫描表取——复核仍然要做（对 `fresh` 比），
            // 只是「以为在杀谁」这一侧的依据弱一档。
            let expected = if pid == anomaly.pid {
                anomaly.command_path.clone()
            } else {
                find(scanned, pid).map(identity).unwrap_or_default()
            };
            let action = if pid < MIN_TARGET_PID {
                PlanAction::ProtectedPid
            } else if own.contains(&pid) {
                PlanAction::OwnProcess
            } else if expected.is_empty() {
                // 没记到身份就没法复核——这一步不许「无从比较就放行」
                PlanAction::NoIdentity
            } else {
                match find(fresh, pid) {
                    None => PlanAction::Gone,
                    // 僵尸算已退出：它占着 pid 但不是活进程，发信号没有意义
                    Some(hit) if hit.is_zombie => PlanAction::Gone,
                    Some(hit) => {
                        if !same_identity(&expected, &identity(hit)) {
                            PlanAction::PidReused
                        } else if orphan && pid == anomaly.pid {
                            PlanAction::OrphanNoEvidence
                        } else {
                            PlanAction::Signal
                        }
                    }
                }
            };
            out.steps.push(KillStep {
                pid,
                identity: expected,
                action,
                anomaly: label.clone(),
            });
        }
    }
    out
}

/// 单个 pid 的信号投递结果。**身份复核不在这里**——它在 [`plan`] 层就做完了，
/// 所以这个枚举里没有「PID 被复用」：走到 [`terminate`] 的都是已经复核过的目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationOutcome {
    /// 进程树已终止
    Killed,
    /// 发了信号但复核时仍在跑
    StillRunning,
    /// 目标不可用（pid 太小 / 进程已不在）
    Unavailable,
}

#[cfg(unix)]
fn signal(pid: u32, sig: i32) -> bool {
    unsafe { libc::kill(pid as i32, sig) == 0 }
}

#[cfg(not(unix))]
fn signal(_pid: u32, _sig: i32) -> bool {
    false
}

/// 终止指定 pid：**先 SIGTERM，超过优雅期仍在就 SIGKILL**。
///
/// 计划层已经把身份、孤儿资格、保护 PID 都判完了，这里不再做决定——
/// 它只负责「发信号 → 等 → 必要时升级」这一段机械动作。
pub fn terminate(pid: u32, grace: Duration) -> TerminationOutcome {
    if pid < MIN_TARGET_PID {
        return TerminationOutcome::Unavailable;
    }
    if !signal(pid, libc::SIGTERM) {
        return TerminationOutcome::Unavailable;
    }
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        // `kill(pid, 0)` 探活；僵尸在这个信号下依然返回成功，所以它只用来判
        // 「要不要升级成 SIGKILL」，最终结论一律以 `verify` 为准
        if !signal(pid, 0) {
            return TerminationOutcome::Killed;
        }
    }
    if signal(pid, libc::SIGKILL) {
        TerminationOutcome::StillRunning
    } else {
        TerminationOutcome::Killed
    }
}

/// 执行计划：只对 [`PlanAction::Signal`] 的步骤发信号，返回实际动了的 pid。
pub fn execute(plan: &KillPlan) -> Vec<u32> {
    let mut out = Vec::new();
    for step in plan.steps.iter().filter(|s| s.action.signals()) {
        terminate(step.pid, TERM_GRACE);
        out.push(step.pid);
    }
    out
}

/// 终止后的复核结论：与「发了信号」**分开**，因为那两件事不是一件。
///
/// `after` 是 [`RECHECK_DELAY`] 之后**重新采**的进程表。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CleanVerification {
    pub confirmed: Vec<u32>,
    pub still_running: Vec<u32>,
    /// **只统计确认退出的那些**——把仍在运行的进程内存也算进「回收」，
    /// 等于报一个必然偏大的数。
    pub reclaimed_memory_bytes: u64,
}

/// 复核终止结果。等 [`RECHECK_DELAY`] 让 SIGTERM / SIGKILL 都落地，再重新采一次表。
pub fn verify(
    signaled: &[KillStep],
    memory_by_pid: &std::collections::HashMap<u32, u64>,
) -> CleanVerification {
    std::thread::sleep(RECHECK_DELAY);
    let monitor = crate::procmon::ProcessMonitor::new();
    let table = monitor.table();
    verify_with(&table, signaled, memory_by_pid)
}
/// [`verify`] 的纯函数内核：给定复核时刻的表就算出结论（不睡、不采样）。
pub fn verify_with(
    table: &[ProcHit],
    signaled: &[KillStep],
    memory_by_pid: &std::collections::HashMap<u32, u64>,
) -> CleanVerification {
    let mut out = CleanVerification::default();
    for step in signaled {
        if step.pid < MIN_TARGET_PID {
            continue;
        }
        if is_alive(table, step.pid) {
            out.still_running.push(step.pid);
        } else {
            out.confirmed.push(step.pid);
            out.reclaimed_memory_bytes += memory_by_pid.get(&step.pid).copied().unwrap_or(0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(pid: u32, ppid: u32, name: &str) -> ProcHit {
        ProcHit {
            pid,
            ppid,
            name: name.to_string(),
            exe_path: format!("/opt/homebrew/bin/{name}"),
            memory: 1_000,
            cpu: Some(1.0),
            is_zombie: false,
        }
    }

    fn bare(pid: u32, ppid: u32, name: &str) -> ProcHit {
        ProcHit {
            exe_path: String::new(),
            ..hit(pid, ppid, name)
        }
    }

    fn anomaly(pid: u32, ppid: u32, path: &str, kind: AnomalyType) -> Anomaly {
        Anomaly {
            pid,
            ppid,
            profile_id: "codex".into(),
            agent_name: "Codex".into(),
            command_path: path.into(),
            memory_bytes: 1_000,
            anomaly_type: kind,
            reason: "test".into(),
        }
    }

    fn no_one() -> HashSet<u32> {
        HashSet::new()
    }

    fn none_active() -> HashSet<String> {
        HashSet::new()
    }

    // ── 身份比对 ───────────────────────────────────────────────

    #[test]
    fn identity_compares_basenames_so_a_homebrew_upgrade_is_not_a_mismatch() {
        assert!(same_identity(
            "/usr/local/bin/node",
            "/opt/homebrew/bin/node"
        ));
        assert!(same_identity("Node", "node"));
        // 路径升级不该让每一次 brew upgrade 都变成「杀不掉」
    }

    #[test]
    fn an_empty_identity_never_matches_because_cannot_verify_is_not_may_kill() {
        assert!(!same_identity("", "node"), "没记到身份 ⇒ 不许动手");
        assert!(!same_identity("node", ""), "现在查不到 ⇒ 也不许动手");
        assert!(!same_identity("", ""));
    }

    #[test]
    fn a_different_program_under_the_same_pid_is_a_mismatch() {
        assert!(!same_identity("/bin/node", "/usr/bin/postgres"));
    }

    // ── 终止顺序 ───────────────────────────────────────────────

    #[test]
    fn children_die_before_their_parent() {
        let table = vec![
            hit(10, 5, "npm"),
            hit(11, 10, "node"),
            hit(12, 11, "esbuild"),
            hit(13, 5, "rg"),
        ];
        let order = kill_order(5, &table);
        assert_eq!(order.last(), Some(&5), "根必须排最后");
        let pos = |pid: u32| order.iter().position(|p| *p == pid).unwrap();
        assert!(pos(12) < pos(11), "孙进程要先于子进程");
        assert!(pos(11) < pos(10), "子进程要先于父进程");
        assert!(pos(10) < pos(5));
    }

    #[test]
    fn a_leaf_target_is_just_itself() {
        let table = vec![hit(10, 5, "npm"), hit(11, 10, "node")];
        assert_eq!(kill_order(11, &table), vec![11]);
    }

    #[test]
    fn a_zero_root_yields_nothing_rather_than_the_whole_machine() {
        let table = vec![hit(1, 0, "init"), hit(2, 0, "launchd")];
        assert!(kill_order(0, &table).is_empty());
    }

    #[test]
    fn a_pid_cycle_does_not_hang_the_kill_order() {
        // 脏数据：12 的父是 11，而 11 又被挂到 12 下（成环）
        let table = vec![
            hit(10, 5, "root"),
            hit(11, 10, "a"),
            hit(12, 11, "b"),
            hit(11, 12, "a-again"),
        ];
        let order = kill_order(10, &table);
        assert!(
            order.contains(&11) && order.contains(&12),
            "环不该把后代从树里吃掉"
        );
        assert_eq!(order.last(), Some(&10), "根必须排最后");
        assert_eq!(
            order.iter().filter(|p| **p == 11).count(),
            1,
            "同一个 pid 只能出现一次"
        );
    }

    // ── 僵尸 ──────────────────────────────────────────────────

    #[test]
    fn a_zombie_is_not_alive() {
        let mut z = hit(77, 1, "node");
        z.is_zombie = true;
        assert!(!is_alive(&[z], 77), "僵尸占着 pid，但它不是活进程");
        assert!(is_alive(&[hit(77, 1, "node")], 77));
        assert!(!is_alive(&[], 77), "表里没有就是不在");
    }

    // ── 孤儿 ──────────────────────────────────────────────────

    #[test]
    fn an_orphan_needs_both_a_launchd_parent_and_no_activity_evidence() {
        let a = anomaly(50, 1, "/bin/node", AnomalyType::Orphan);
        assert!(looks_orphan(&a, &none_active()), "ppid=1 且没佐证 ⇒ 孤儿");
        let mut active = HashSet::new();
        active.insert("codex".to_string());
        assert!(
            !looks_orphan(&a, &active),
            "launchd 托管的常驻服务必有活动佐证，不能当孤儿杀"
        );
        let not_orphan = anomaly(50, 42, "/bin/node", AnomalyType::Orphan);
        assert!(
            !looks_orphan(&not_orphan, &none_active()),
            "父进程还在就不是孤儿"
        );
    }

    // ── 计划：安全规则逐条 ──────────────────────────────────────

    #[test]
    fn a_plan_that_matches_everywhere_signals_the_tree_children_first() {
        let table = vec![
            hit(10, 5, "npm"),
            hit(11, 10, "node"),
            hit(12, 11, "esbuild"),
        ];
        let a = anomaly(10, 5, "/opt/homebrew/bin/npm", AnomalyType::Hung);
        let p = plan(&[a], &table, &table, &no_one(), &none_active(), true);
        assert_eq!(p.signal_targets(), vec![12, 11, 10], "子孙先于父");
    }

    #[test]
    fn a_pid_reused_between_scan_and_kill_is_refused() {
        let scanned = vec![hit(10, 5, "npm")];
        // 复核时同一个 pid 已经是别的程序了
        let fresh = vec![hit(10, 5, "postgres")];
        let a = anomaly(10, 5, "/opt/homebrew/bin/npm", AnomalyType::Hung);
        let p = plan(&[a], &scanned, &fresh, &no_one(), &none_active(), true);
        assert!(p.signal_targets().is_empty(), "复核不过就不许动手");
        assert_eq!(p.steps[0].action, PlanAction::PidReused);
    }

    #[test]
    fn an_anomaly_without_a_recorded_identity_is_never_killed() {
        // 这是防「无复核的 kill」的那条：空身份不放行
        let table = vec![hit(10, 5, "npm")];
        let a = anomaly(10, 5, "", AnomalyType::Hung);
        let p = plan(&[a], &table, &table, &no_one(), &none_active(), true);
        assert!(p.signal_targets().is_empty());
        assert_eq!(p.steps[0].action, PlanAction::NoIdentity);
    }

    #[test]
    fn a_process_that_left_during_the_recheck_is_skipped_not_killed() {
        let scanned = vec![hit(10, 5, "npm")];
        let fresh: Vec<ProcHit> = Vec::new();
        let a = anomaly(10, 5, "/opt/homebrew/bin/npm", AnomalyType::Hung);
        let p = plan(&[a], &scanned, &fresh, &no_one(), &none_active(), true);
        assert_eq!(p.steps[0].action, PlanAction::Gone);
    }

    #[test]
    fn a_zombie_target_is_treated_as_already_gone() {
        let scanned = vec![hit(10, 5, "npm")];
        let mut fresh = vec![hit(10, 5, "npm")];
        fresh[0].is_zombie = true;
        let a = anomaly(10, 5, "/opt/homebrew/bin/npm", AnomalyType::Hung);
        let p = plan(&[a], &scanned, &fresh, &no_one(), &none_active(), true);
        assert_eq!(
            p.steps[0].action,
            PlanAction::Gone,
            "僵尸不占信号，也不占内存"
        );
    }

    #[test]
    fn pid_zero_and_one_are_never_targets() {
        let table = vec![hit(1, 0, "launchd")];
        let a = anomaly(1, 0, "/sbin/launchd", AnomalyType::Hung);
        let p = plan(&[a], &table, &table, &no_one(), &none_active(), true);
        assert_eq!(p.steps[0].action, PlanAction::ProtectedPid);
        assert!(p.signal_targets().is_empty());
    }

    #[test]
    fn we_never_signal_ourselves_or_our_terminal() {
        let table = vec![hit(4242, 400, "agentisland"), hit(400, 1, "zsh")];
        let a = anomaly(4242, 400, "/usr/local/bin/agentisland", AnomalyType::Hung);
        let mut own = HashSet::new();
        own.insert(4242);
        own.insert(400);
        let p = plan(&[a], &table, &table, &own, &none_active(), true);
        assert!(p.signal_targets().is_empty(), "杀掉自己的祖先就是杀掉终端");
        assert_eq!(p.steps[0].action, PlanAction::OwnProcess);
    }

    #[test]
    fn batch_mode_refuses_orphans_and_naming_one_explicitly_still_needs_evidence() {
        let scanned = vec![hit(50, 1, "node")];
        let a = anomaly(50, 1, "/opt/homebrew/bin/node", AnomalyType::Orphan);
        // 批量：拒
        let batch = plan(
            &[a.clone()],
            &scanned,
            &scanned,
            &no_one(),
            &none_active(),
            true,
        );
        assert!(batch.signal_targets().is_empty());
        assert_eq!(batch.steps[0].action, PlanAction::OrphanNeedsNaming);
        // 逐条点名：仍要有活动佐证，没有就还是不杀
        let named = plan(
            &[a.clone()],
            &scanned,
            &scanned,
            &no_one(),
            &none_active(),
            false,
        );
        assert_eq!(named.steps[0].action, PlanAction::OrphanNoEvidence);
        // 点名 + 有佐证 ⇒ 说明它其实是常驻服务，不当孤儿处理
        let mut active = HashSet::new();
        active.insert("codex".to_string());
        let ok = plan(&[a], &scanned, &scanned, &no_one(), &active, false);
        assert_eq!(ok.signal_targets(), vec![50]);
    }

    #[test]
    fn a_descendant_that_got_reused_blocks_only_itself_not_the_whole_tree() {
        // 子进程被复用 ⇒ 只跳过它自己；父进程仍按计划终止
        let scanned = vec![hit(10, 5, "npm"), hit(11, 10, "node")];
        let fresh = vec![hit(10, 5, "npm"), hit(11, 10, "curl")];
        let a = anomaly(10, 5, "/opt/homebrew/bin/npm", AnomalyType::Hung);
        let p = plan(&[a], &scanned, &fresh, &no_one(), &none_active(), true);
        assert_eq!(p.signal_targets(), vec![10], "父仍要收");
        let reused = p.steps.iter().find(|s| s.pid == 11).unwrap();
        assert_eq!(reused.action, PlanAction::PidReused);
    }

    /// 复核身份对**根**取异常里记的那一份、对**后代**取扫描表——
    /// 分界钉住，否则 `Anomaly.command_path` 会退化成没人读的装饰字段。
    #[test]
    fn the_root_is_verified_against_the_identity_the_anomaly_recorded() {
        // 异常里记的是 `/opt/homebrew/bin/npm`，扫描表里那个进程已经叫别的了
        let scanned = vec![hit(10, 5, "npm")];
        let fresh = vec![hit(10, 5, "postgres")];
        let a = anomaly(10, 5, "/usr/local/bin/npm", AnomalyType::Hung);
        let p = plan(&[a], &scanned, &fresh, &no_one(), &none_active(), true);
        assert_eq!(p.steps[0].action, PlanAction::PidReused);
        assert_eq!(
            p.steps[0].identity, "/usr/local/bin/npm",
            "复核基准必须是异常记录的那份，而不是现查扫描表"
        );
    }

    #[test]
    fn a_descendant_is_still_verified_against_the_fresh_table() {
        // 后代没有各自的异常记录 ⇒ 基准取扫描表，但仍要与 fresh 比
        let scanned = vec![hit(10, 5, "npm"), hit(11, 10, "node")];
        let fresh = vec![hit(10, 5, "npm"), hit(11, 10, "curl")];
        let a = anomaly(10, 5, "/opt/homebrew/bin/npm", AnomalyType::Hung);
        let p = plan(&[a], &scanned, &fresh, &no_one(), &none_active(), true);
        assert_eq!(p.signal_targets(), vec![10], "父仍要收");
        assert_eq!(
            p.steps.iter().find(|s| s.pid == 11).unwrap().action,
            PlanAction::PidReused,
            "后代也要复核"
        );
    }

    #[test]
    fn a_process_with_no_executable_path_falls_back_to_its_name() {
        let h = bare(10, 5, "node");
        assert_eq!(identity(&h), "node");
        assert!(same_identity("node", &identity(&h)));
    }

    // ── 复核 ──────────────────────────────────────────────────

    #[test]
    fn verification_counts_only_what_actually_left() {
        let signaled = vec![
            KillStep {
                pid: 10,
                identity: "node".into(),
                action: PlanAction::Signal,
                anomaly: "a".into(),
            },
            KillStep {
                pid: 11,
                identity: "node".into(),
                action: PlanAction::Signal,
                anomaly: "a".into(),
            },
        ];
        // 复核时：10 没了、11 还在
        let after = vec![hit(11, 5, "node")];
        let mut mem = std::collections::HashMap::new();
        mem.insert(10u32, 500u64);
        mem.insert(11u32, 900u64);
        let v = verify_with(&after, &signaled, &mem);
        assert_eq!(v.confirmed, vec![10]);
        assert_eq!(v.still_running, vec![11]);
        assert_eq!(
            v.reclaimed_memory_bytes, 500,
            "仍在运行的 900 不能算进回收，否则报一个必然偏大的数"
        );
    }

    #[test]
    fn a_zombie_after_termination_counts_as_gone() {
        let signaled = vec![KillStep {
            pid: 10,
            identity: "node".into(),
            action: PlanAction::Signal,
            anomaly: "a".into(),
        }];
        let mut z = hit(10, 1, "node");
        z.is_zombie = true;
        let v = verify_with(&[z], &signaled, &std::collections::HashMap::new());
        assert_eq!(v.confirmed, vec![10], "僵尸不在占内存，不该报「杀不掉」");
        assert!(v.still_running.is_empty());
    }

    #[test]
    fn a_tree_is_never_double_signalled_when_two_anomalies_share_a_child() {
        // 11 既是 10 的后代、又被单列成一条异常 ⇒ 不修的话会对它发两次信号
        let scanned = vec![hit(10, 5, "npm"), hit(11, 10, "node")];
        let a = anomaly(10, 5, "/opt/homebrew/bin/npm", AnomalyType::Hung);
        let b = anomaly(11, 10, "/opt/homebrew/bin/node", AnomalyType::Hung);
        let p = plan(&[a, b], &scanned, &scanned, &no_one(), &none_active(), true);
        let targets = p.signal_targets();
        let mut unique = targets.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            targets.len(),
            unique.len(),
            "同一个 pid 只能被发一次信号（实际排了 {targets:?}）"
        );
        assert_eq!(targets, vec![11, 10], "仍保持先子后父的终止顺序");
    }
}
