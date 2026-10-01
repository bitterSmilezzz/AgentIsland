use crate::models::AgentProfile;
use std::collections::HashMap;
use std::time::Instant;
use sysinfo::{ProcessesToUpdate, System};

/// 进程表快照 + CPU 差分（sysinfo 内部就是两拍 refresh 之间的差分）。
/// 第一拍没有窗口，CPU 返回「没测」（None），不谎报 0。
pub struct ProcessMonitor {
    sys: System,
    last_refresh: Option<Instant>,
    cpu_measured: bool,
    rows: Vec<ObservedProcess>,
}

struct ObservedProcess {
    hit: ProcHit,
    cmdline: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcHit {
    pub pid: u32,
    /// 父进程 PID（进程树要用它；`0` = 没有父，即系统级）
    pub ppid: u32,
    pub name: String,
    pub exe_path: String,
    pub memory: u64,
    pub cpu: Option<f64>,
    /// **僵尸**（已退出、父进程还没 `wait` 回收）。
    ///
    /// 进程表里必须带这一位，而不是让调用方另外去 `ps` 问一次：清干净了没
    /// 是个**判定**，而判定用到的两个事实（存活、是不是僵尸）得来自同一张表，
    /// 否则「问存活的那一拍」和「问僵尸的那一拍」之间进程可以已经变了。
    pub is_zombie: bool,
}

impl ProcessMonitor {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        let mut monitor = ProcessMonitor {
            sys,
            last_refresh: Some(Instant::now()),
            cpu_measured: false,
            rows: Vec::new(),
        };
        monitor.rebuild_rows();
        monitor
    }

    /// 刷新一拍。两次调用间隔即 CPU 差分窗口。
    pub fn refresh(&mut self) {
        self.cpu_measured = self.last_refresh.is_some_and(|at| at.elapsed() >= sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        self.sys.refresh_processes(ProcessesToUpdate::All, true);
        self.last_refresh = Some(Instant::now());
        self.rebuild_rows();
    }

    /// 按档案匹配进程（名字前缀族 + 命令行提示 + 路径排除）。
    pub fn match_profile(&self, profile: &AgentProfile) -> Vec<ProcHit> {
        self.rows.iter().filter(|row| profile_matches(profile, &row.hit.name, &row.hit.exe_path, &row.cmdline))
            .map(|row| row.hit.clone()).collect()
    }

    /// 同一拍的名字、路径和命令行只组装一次，各档案复用，避免逐档案重建整张进程表。
    fn rebuild_rows(&mut self) {
        self.rows = self.sys.processes().iter().map(|(pid, process)| {
            let raw = process.name().to_string_lossy().to_lowercase();
            ObservedProcess {
                cmdline: process.cmd().iter().map(|part| part.to_string_lossy()).collect::<Vec<_>>().join(" "),
                hit: ProcHit {
                    pid: pid.as_u32(),
                    ppid: process.parent().map(|parent| parent.as_u32()).unwrap_or(0),
                    name: raw.strip_suffix(".exe").unwrap_or(&raw).to_string(),
                    exe_path: process.exe().map(|path| path.to_string_lossy().into_owned()).unwrap_or_default(),
                    memory: process.memory(),
                    cpu: self.cpu_measured.then_some(process.cpu_usage() as f64),
                    is_zombie: process.status() == sysinfo::ProcessStatus::Zombie,
                },
            }
        }).collect();
    }

    /// 进程树读取与档案匹配共用本拍数据，PID 顺序固定。
    pub fn table(&self) -> Vec<ProcHit> {
        let mut table: Vec<_> = self.rows.iter().map(|row| row.hit.clone()).collect();
        table.sort_by_key(|hit| hit.pid);
        table
    }

}

/// 一个进程是否属于某个档案。**纯函数**：只看名字 / 可执行路径 / 命令行三个字符串。
///
/// 抽出来是为了让排除规则能被夹具逐条断言——自检里那几条「系统路径不该误报」
/// （`CursorUIViewService` 不该算 Cursor、`ssh-agent` 不该算任何档案）以前只能靠真机碰运气。
///
/// 三条规则与 Swift `ProcessMatcher` 同口径：
/// ① 名字**精确**匹配或「名字 + 空格」前缀（所以 `cursoruiviewservice` 不会命中 `Cursor`）；
/// ② 命令含提示词也算命中（npm 装的 CLI 跑在 node 里，进程名对不上）；
/// ③ 路径命中排除表就否决——即使名字对上了。
pub fn profile_matches(profile: &AgentProfile, name: &str, exe: &str, cmdline: &str) -> bool {
    if profile.process_names.is_empty() && profile.cmdline_hints.is_empty() {
        return false;
    }
    let lower = name.to_lowercase();
    // Windows 上 sysinfo 返回 "ZCode.exe"：匹配前去掉扩展名
    let name = lower.strip_suffix(".exe").unwrap_or(&lower);
    let name_hit = profile.process_names.iter().any(|want| {
        let want = want.to_lowercase();
        name == want || name.starts_with(&format!("{want} "))
    });
    let hint_hit = !profile.cmdline_hints.is_empty()
        && profile
            .cmdline_hints
            .iter()
            .any(|hint| cmdline.to_lowercase().contains(&hint.to_lowercase()));
    if !name_hit && !hint_hit {
        return false;
    }
    // 路径锚定：登记了 `path_contains` 就**必须**命中其中一条。
    // 这是「有则必须命中」而不是「命中则加分」——后者会让锚定形同虚设，
    // 而前者才是 Swift 侧注释里说的「防止两个变体互相误命中」。
    if !profile.path_contains.is_empty() {
        let lower_exe = exe.to_lowercase();
        if !profile
            .path_contains
            .iter()
            .any(|anchor| lower_exe.contains(&anchor.to_lowercase()))
        {
            return false;
        }
    }
    !profile
        .path_excludes
        .iter()
        .any(|exclude| exe.to_lowercase().contains(&exclude.to_lowercase()))
}

pub fn memory_text(bytes: u64) -> String {
    if bytes == 0 {
        return "—".into();
    }
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.1}G", mb / 1024.0)
    } else if mb < 1.0 {
        "<1M".into()
    } else {
        format!("{}M", mb as u64)
    }
}

/// 引擎用的快照缓存类型
pub type ProcTable = HashMap<u32, ProcHit>;

#[cfg(test)]
mod cpu_window_regressions {
    use super::*;
    #[test]
    fn one_process_snapshot_has_no_cpu_difference_window() {
        let monitor = ProcessMonitor::new();
        let table = monitor.table();
        assert!(!table.is_empty());
        assert!(table.iter().all(|hit| hit.cpu.is_none()), "首次采样不能伪报 CPU 0 或旧读数");
    }
}
