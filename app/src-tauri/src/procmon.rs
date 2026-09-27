use crate::models::AgentProfile;
use std::collections::HashMap;
use std::time::Instant;
use sysinfo::{Process, ProcessesToUpdate, System};

/// 进程表快照 + CPU 差分（sysinfo 内部就是两拍 refresh 之间的差分）。
/// 第一拍没有窗口，CPU 返回「没测」（None），不谎报 0。
pub struct ProcessMonitor {
    sys: System,
    last_refresh: Option<Instant>,
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
}

impl ProcessMonitor {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_processes(ProcessesToUpdate::All, true);
        ProcessMonitor {
            sys,
            last_refresh: Some(Instant::now()),
        }
    }

    /// 刷新一拍。两次调用间隔即 CPU 差分窗口。
    pub fn refresh(&mut self) {
        self.sys.refresh_processes(ProcessesToUpdate::All, true);
        self.last_refresh = Some(Instant::now());
    }

    pub fn core_count(&self) -> usize {
        self.sys.cpus().len()
    }

    /// 按档案匹配进程（名字前缀族 + 命令行提示 + 路径排除）。
    pub fn match_profile(&self, profile: &AgentProfile) -> Vec<ProcHit> {
        let mut hits: Vec<ProcHit> = Vec::new();
        if profile.process_names.is_empty() && profile.cmdline_hints.is_empty() {
            return hits;
        }
        for (pid, proc_) in self.sys.processes() {
            let raw_name = proc_.name().to_string_lossy().to_string();
            // Windows 上 sysinfo 返回 "ZCode.exe"：匹配前去掉扩展名并统一小写
            let lower_raw = raw_name.to_lowercase();
            let name = lower_raw
                .strip_suffix(".exe")
                .unwrap_or(&lower_raw)
                .to_string();
            let exe = proc_
                .exe()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default();
            let cmdline = proc_
                .cmd()
                .iter()
                .map(|c| c.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();

            let name_hit = profile.process_names.iter().any(|want| {
                let want = want.to_lowercase();
                name == want || name.starts_with(&format!("{} ", want))
            });
            // npm 安装的 CLI 跑在 node.exe 里：命令行包含提示词即命中
            let hint_hit = !profile.cmdline_hints.is_empty()
                && profile
                    .cmdline_hints
                    .iter()
                    .any(|hint| cmdline.contains(&hint.to_lowercase()));
            if !name_hit && !hint_hit {
                continue;
            }
            if profile
                .path_excludes
                .iter()
                .any(|x| exe.to_lowercase().contains(&x.to_lowercase()))
            {
                continue;
            }
            let cpu = if self.last_refresh.is_some() {
                let c = proc_.cpu_usage() as f64;
                Some(c)
            } else {
                None
            };
            hits.push(ProcHit {
                pid: pid.as_u32(),
                ppid: proc_.parent().map(|p| p.as_u32()).unwrap_or(0),
                name,
                exe_path: exe,
                memory: proc_.memory(),
                cpu,
            });
        }
        hits
    }

    /// 整张进程表（不做档案匹配）。
    ///
    /// 进程树要用它：树是「谁派生谁」的结构，只给匹配到的那些进程建不出树来
    /// （中间往往夹着 npm / node 这类不属于任何档案的进程）。
    /// 名字归一化与 [`ProcessMonitor::match_profile`] 同一套（去 `.exe`、转小写）。
    pub fn table(&self) -> Vec<ProcHit> {
        let mut table: Vec<ProcHit> = Vec::with_capacity(self.sys.processes().len());
        for (pid, proc_) in self.sys.processes() {
            let raw = proc_.name().to_string_lossy().to_lowercase();
            let name = raw.strip_suffix(".exe").unwrap_or(&raw).to_string();
            let cpu = if self.last_refresh.is_some() {
                Some(proc_.cpu_usage() as f64)
            } else {
                None
            };
            table.push(ProcHit {
                pid: pid.as_u32(),
                ppid: proc_.parent().map(|p| p.as_u32()).unwrap_or(0),
                name,
                exe_path: proc_
                    .exe()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_default(),
                memory: proc_.memory(),
                cpu,
            });
        }
        // 顺序固定：sysinfo 的 HashMap 迭代顺序不保证，而树的同层顺序会反映到界面上
        table.sort_by_key(|hit| hit.pid);
        table
    }

    /// 供调试：全部进程名
    pub fn all_names(&self) -> Vec<String> {
        self.sys
            .processes()
            .values()
            .map(|p: &Process| p.name().to_string_lossy().to_string())
            .collect()
    }
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
