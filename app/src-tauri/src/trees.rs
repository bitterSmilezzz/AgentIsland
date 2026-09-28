//! 进程树构建（Swift `ProcessTreeInspector`）。
//!
//! 用途是详情页那句「谁在吃 CPU」：拿到 Agent 的根进程 PID，把派生工具与子进程摊成一棵树。
//! **纯函数**：给定根 PID 与一张进程表快照，结果只由输入决定——所以环、断链、重复
//! 挂载这些罕见情形都能用构造出来的表离线断言，不必去真机器上等它们发生。
//!
//! 与 Swift 的两处差异，都是有意的：
//! ① CPU 用 `Option`：Rust 的进程表第一拍**没有差分窗口**（sysinfo 靠两拍差值算 CPU），
//!    把它当 0 会把「没测」混进合计。Swift 的 `cpuPercent` 是 `Double`，无窗口时给 0。
//!    这里 `None` 表示「这一拍没有读数」，并且额外给出 `cpu_measured_count`——
//!    部分子进程没读数时合计是**偏低**的，读的人需要知道。
//! ② 防环用的是「全局已访问集合」而不是「每个分支自己的集合」：同一 PID 在表里出现两次
//!    （父进程重复）时只统计一次，否则合计会翻倍。

use crate::procmon::{memory_text, ProcHit};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub path: String,
    /// `None` = 本拍没有差分窗口（不是 0）
    pub cpu_percent: Option<f64>,
    pub memory_bytes: u64,
    /// 已排版好的两个数（`512M` / `12.3%`）。排版口径只有一处，别让界面各写一份；
    /// `None` 的 CPU 在这里写成「本拍无差分窗口」，而不是 0
    pub memory_text: String,
    pub cpu_text: String,
    pub children: Vec<TreeNode>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeReport {
    pub root_pid: u32,
    /// 各子进程 CPU 之和；`None` = 本拍一个读数都没有（**不是 0**）
    pub total_subprocess_cpu: Option<f64>,
    pub total_subprocess_memory: u64,
    pub subprocess_count: usize,
    /// 有多少子进程**这一拍**有 CPU 读数。小于 `subprocess_count` 时合计偏低
    pub cpu_measured_count: usize,
    pub total_subprocess_memory_text: String,
    pub total_subprocess_cpu_text: String,
    pub nodes: Vec<TreeNode>,
}

fn cpu_text(cpu: Option<f64>) -> String {
    match cpu {
        None => "—（本拍无差分窗口）".to_string(),
        Some(value) if value <= 0.05 => "0%".to_string(),
        Some(value) => format!("{value:.1}%"),
    }
}

fn empty_report(root_pid: u32) -> TreeReport {
    TreeReport {
        root_pid,
        total_subprocess_cpu: None,
        total_subprocess_memory: 0,
        subprocess_count: 0,
        cpu_measured_count: 0,
        total_subprocess_memory_text: memory_text(0),
        total_subprocess_cpu_text: cpu_text(None),
        nodes: Vec::new(),
    }
}

fn make_node(child: &ProcHit, children: Vec<TreeNode>) -> TreeNode {
    TreeNode {
        pid: child.pid,
        ppid: child.ppid,
        name: if child.name.is_empty() {
            "unknown".to_string()
        } else {
            child.name.clone()
        },
        path: child.exe_path.clone(),
        cpu_percent: child.cpu,
        memory_bytes: child.memory,
        memory_text: memory_text(child.memory),
        cpu_text: cpu_text(child.cpu),
        children,
    }
}

/// 构建派生进程树。`root_pid == 0` 直接返回空树（Swift 同口径：调用方可能拿到 nil/0）。
///
/// **全程不递归**：进程树深度由外部数据决定，递归实现遇到深层嵌套会直接爆栈
/// （实测 5,000 层的链就能让测试进程 SIGABRT）。做法是两步都迭代：
/// ① 广度优先取出可达节点（层序天然「父在子前」）并累计合计；
/// ② 按层序**倒着**装配（子先于父），于是每个节点的子树已经就绪。
pub fn build_tree(root_pid: u32, table: &[ProcHit]) -> TreeReport {
    if root_pid == 0 {
        return empty_report(root_pid);
    }

    // 按父 PID 分组建索引：一次遍历，避免每个节点都扫全表
    let mut children_by_parent: HashMap<u32, Vec<usize>> = HashMap::new();
    for (index, entry) in table.iter().enumerate() {
        children_by_parent.entry(entry.ppid).or_default().push(index);
    }

    // 根进程先算「已访问」：表里若把根自己写成它的后代（脏数据），不该把根收进来
    let mut visited: HashSet<u32> = HashSet::new();
    visited.insert(root_pid);

    let mut totals = (0.0f64, false, 0usize, 0u64, 0usize); // cpu 合计、有无读数、读数条数、内存、节点数
    let mut children_of: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut order: Vec<usize> = Vec::new();
    let mut queue: VecDeque<u32> = VecDeque::new();
    queue.push_back(root_pid);
    while let Some(parent) = queue.pop_front() {
        let Some(kids) = children_by_parent.get(&parent) else {
            continue;
        };
        for index in kids {
            let child = &table[*index];
            if !visited.insert(child.pid) {
                // 环（PID 环）或同一进程被挂到两个父下：只算一次
                continue;
            }
            totals.4 += 1;
            totals.3 += child.memory;
            if let Some(value) = child.cpu {
                totals.0 += value;
                totals.1 = true;
                totals.2 += 1;
            }
            children_of.entry(parent).or_default().push(child.pid);
            order.push(*index);
            queue.push_back(child.pid);
        }
    }

    // 倒序装配：逆层序保证子节点先建好
    let mut built: HashMap<u32, TreeNode> = HashMap::new();
    for index in order.iter().rev() {
        let child = &table[*index];
        let children = children_of
            .get(&child.pid)
            .map(|pids| {
                pids.iter()
                    .filter_map(|pid| built.remove(pid))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        built.insert(child.pid, make_node(child, children));
    }
    let nodes = children_of
        .get(&root_pid)
        .map(|pids| {
            pids.iter()
                .filter_map(|pid| built.remove(pid))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    TreeReport {
        root_pid,
        total_subprocess_cpu: if totals.1 { Some(totals.0) } else { None },
        total_subprocess_memory: totals.3,
        subprocess_count: totals.4,
        cpu_measured_count: totals.2,
        total_subprocess_memory_text: memory_text(totals.3),
        total_subprocess_cpu_text: cpu_text(if totals.1 { Some(totals.0) } else { None }),
        nodes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(pid: u32, ppid: u32, name: &str, cpu: Option<f64>, memory: u64) -> ProcHit {
        ProcHit {
            pid,
            ppid,
            name: name.to_string(),
            exe_path: format!("/usr/local/bin/{name}"),
            memory,
            cpu,
            is_zombie: false,
        }
    }

    fn flat(report: &TreeReport) -> Vec<(u32, u32)> {
        fn walk(nodes: &[TreeNode], out: &mut Vec<(u32, u32)>) {
            for node in nodes {
                out.push((node.pid, node.ppid));
                walk(&node.children, out);
            }
        }
        let mut out = Vec::new();
        walk(&report.nodes, &mut out);
        out
    }

    #[test]
    fn a_root_without_children_is_an_empty_tree() {
        let table = vec![hit(1, 0, "init", Some(0.0), 100)];
        let report = build_tree(999, &table);
        assert_eq!(report.subprocess_count, 0);
        assert!(report.nodes.is_empty());
        assert_eq!(report.total_subprocess_cpu, None, "没有子进程 ⇒ 没有读数");
        assert_eq!(report.total_subprocess_memory, 0);
    }

    #[test]
    fn a_zero_or_negative_root_returns_an_empty_tree_instead_of_the_whole_table() {
        // root_pid = 0 时若不早退，表里所有 ppid=0 的进程都会被当成它的子进程——
        // 那等于把整机进程树当成某个 Agent 的派生工具
        let table = vec![hit(1, 0, "a", Some(1.0), 1), hit(2, 0, "b", Some(1.0), 1)];
        for root in [0u32] {
            let report = build_tree(root, &table);
            assert!(report.nodes.is_empty(), "root={root} 不该收任何节点");
            assert_eq!(report.subprocess_count, 0);
        }
    }

    #[test]
    fn a_multi_level_tree_is_ordered_and_totalled() {
        let table = vec![
            hit(10, 5, "npm", Some(2.0), 100),
            hit(11, 10, "node", Some(30.0), 200),
            hit(12, 11, "esbuild", Some(8.0), 300),
            hit(13, 5, "rg", Some(0.5), 400),
            hit(99, 1, "unrelated", Some(50.0), 9_999),
        ];
        let report = build_tree(5, &table);
        assert_eq!(report.subprocess_count, 4, "只数根的后代");
        assert_eq!(
            flat(&report),
            vec![(10, 5), (11, 10), (12, 11), (13, 5)],
            "先子后孙、同层按表序"
        );
        assert_eq!(report.total_subprocess_cpu, Some(40.5), "2 + 30 + 8 + 0.5");
        assert_eq!(report.cpu_measured_count, 4);
        assert_eq!(report.total_subprocess_memory, 1_000);
        assert_eq!(report.nodes[0].children[0].name, "node");
        assert_eq!(report.nodes[0].children[0].children[0].name, "esbuild");
    }

    #[test]
    fn a_pid_cycle_is_broken_without_double_counting_or_hanging() {
        // 脏数据里的环：12 的父是 11、11 的父是 12
        let table = vec![
            hit(11, 10, "a", Some(1.0), 10),
            hit(12, 11, "b", Some(2.0), 20),
            hit(11, 12, "a", Some(3.0), 30), // 同 PID 又挂到 12 下
        ];
        let report = build_tree(10, &table);
        assert_eq!(report.subprocess_count, 2, "同一个 PID 只能算一次");
        assert_eq!(report.total_subprocess_memory, 30, "10 + 20，重复那条不计");
        assert_eq!(flat(&report).len(), 2);
    }

    #[test]
    fn a_deep_spine_does_not_blow_the_stack() {
        // 进程树深度由外部数据决定：这里造 5,000 层，递归实现会爆栈
        let mut table = Vec::new();
        for i in 1..5_000u32 {
            table.push(hit(i + 1, i, "p", Some(0.1), 1));
        }
        let report = build_tree(1, &table);
        assert_eq!(report.subprocess_count, 4_999);
        assert_eq!(report.total_subprocess_memory, 4_999);
        // 走到最深一层确认结构真的建出来了
        let mut node = &report.nodes[0];
        let mut depth = 1;
        while let Some(next) = node.children.first() {
            node = next;
            depth += 1;
        }
        assert_eq!(depth, 4_999);
    }

    #[test]
    fn an_unmeasured_child_makes_the_cpu_total_absent_rather_than_zero() {
        // 第一拍没有差分窗口：CPU 全是 None。合计必须是「没有读数」而不是 0——
        // 写成 0 会让详情页显示「这个 Agent 的子进程没吃 CPU」，而真相是没测
        let table = vec![hit(2, 1, "a", None, 10), hit(3, 2, "b", None, 20)];
        let report = build_tree(1, &table);
        assert_eq!(report.total_subprocess_cpu, None);
        assert_eq!(report.cpu_measured_count, 0);
        assert_eq!(report.total_subprocess_cpu_text, "—（本拍无差分窗口）");
        assert_eq!(report.total_subprocess_memory, 30, "内存与 CPU 无关，照常合计");
    }

    #[test]
    fn a_partially_measured_tree_reports_how_many_children_were_counted() {
        // 部分子进程没读数 ⇒ 合计偏低；`cpu_measured_count` 让调用方说得出这件事
        let table = vec![
            hit(2, 1, "a", Some(2.0), 10),
            hit(3, 1, "b", None, 20),
            hit(4, 1, "c", Some(1.0), 30),
        ];
        let report = build_tree(1, &table);
        assert_eq!(report.total_subprocess_cpu, Some(3.0));
        assert_eq!(report.subprocess_count, 3);
        assert_eq!(report.cpu_measured_count, 2, "3 个里只有 2 个有读数");
    }

    #[test]
    fn an_empty_name_falls_back_to_unknown_and_the_text_rules_hold() {
        let table = vec![hit(2, 1, "", Some(0.01), 512 * 1024 * 1024)];
        let report = build_tree(1, &table);
        let node = &report.nodes[0];
        assert_eq!(node.name, "unknown", "空名字要回落而不是留空");
        assert_eq!(node.cpu_text, "0%", "低于 0.05% 写成 0%");
        assert_eq!(node.memory_text, "512M");
        assert_eq!(report.total_subprocess_memory_text, "512M");

        let busy = build_tree(1, &[hit(2, 1, "x", Some(12.34), 0)]);
        assert_eq!(busy.nodes[0].cpu_text, "12.3%");
        assert_eq!(busy.nodes[0].memory_text, "—", "0 字节是「没测到」而不是 0M");
    }

    #[test]
    fn the_root_itself_is_never_included_even_if_the_table_claims_it_is_a_child() {
        // 两种脏数据都要挡住：
        // ① 自己当自己的父（`pid == ppid == root`）
        // ② 一个环绕回根本身（5 → 6 → 5）——若不把根预先标为已访问，
        //    第二个环会让根作为 6 的子节点被收进树里
        let self_parent = vec![hit(5, 5, "root-as-own-child", Some(9.0), 999)];
        let report = build_tree(5, &self_parent);
        assert!(report.nodes.is_empty(), "根不该出现在自己的子树里");
        assert_eq!(report.total_subprocess_memory, 0);
        assert_eq!(report.subprocess_count, 0);

        let cycle_back_to_root = vec![
            hit(6, 5, "child", Some(1.0), 10),
            hit(5, 6, "root-again", Some(9.0), 999),
        ];
        let report = build_tree(5, &cycle_back_to_root);
        assert_eq!(report.subprocess_count, 1, "只该有 6 那一个子节点");
        assert_eq!(flat(&report), vec![(6, 5)]);
        assert_eq!(report.total_subprocess_memory, 10, "根的内存不该被算进子进程合计");
    }
}
