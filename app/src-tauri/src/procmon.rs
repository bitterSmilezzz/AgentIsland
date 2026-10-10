use crate::models::AgentProfile;
use std::collections::HashMap;
use std::time::Instant;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

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
    launch_entry: Option<String>,
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
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, process_refresh_kind());
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
        self.cpu_measured = self
            .last_refresh
            .is_some_and(|at| at.elapsed() >= sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        self.sys
            .refresh_processes_specifics(ProcessesToUpdate::All, true, process_refresh_kind());
        self.last_refresh = Some(Instant::now());
        self.rebuild_rows();
    }

    /// 按档案匹配进程（名字前缀族 + 命令行提示 + 路径排除）。
    pub fn match_profile(&self, profile: &AgentProfile) -> Vec<ProcHit> {
        self.rows
            .iter()
            .filter(|row| {
                !row.hit.is_zombie
                    && matches_entry(
                        profile,
                        &row.hit.name,
                        &row.hit.exe_path,
                        row.launch_entry.as_deref(),
                    )
            })
            .map(|row| row.hit.clone())
            .collect()
    }

    /// 同一拍的入口只解析一次，各档案复用；不额外拼接全文作为匹配缓存。
    fn rebuild_rows(&mut self) {
        self.rows = self
            .sys
            .processes()
            .iter()
            .map(|(pid, process)| {
                // sysinfo caches process.name across exec. Prefer the fresh executable basename.
                let raw = process
                    .exe()
                    .and_then(|path| path.file_name())
                    .unwrap_or(process.name())
                    .to_string_lossy()
                    .to_lowercase();
                let name = raw.strip_suffix(".exe").unwrap_or(&raw).to_string();
                let exe_path = process
                    .exe()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let argv: Vec<_> = process
                    .cmd()
                    .iter()
                    .map(|part| part.to_string_lossy())
                    .collect();
                ObservedProcess {
                    launch_entry: command_entry(&name, &exe_path, &argv),
                    hit: ProcHit {
                        pid: pid.as_u32(),
                        ppid: process.parent().map(|parent| parent.as_u32()).unwrap_or(0),
                        name,
                        exe_path,
                        memory: process.memory(),
                        cpu: self.cpu_measured.then_some(process.cpu_usage() as f64),
                        is_zombie: process.status() == sysinfo::ProcessStatus::Zombie,
                    },
                }
            })
            .collect();
    }

    /// 进程树读取与档案匹配共用本拍数据，PID 顺序固定。
    pub fn table(&self) -> Vec<ProcHit> {
        let mut table: Vec<_> = self.rows.iter().map(|row| row.hit.clone()).collect();
        table.sort_by_key(|hit| hit.pid);
        table
    }
}

// exec can change executable/argv without changing PID. Keep identity fresh alongside metrics.
// Avoid unrelated disk I/O; entry decoding is still shared by every profile in the sample.
fn process_refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .with_exe(UpdateKind::Always)
        .with_cmd(UpdateKind::Always)
}

/// 一个进程是否属于某个档案。纯函数；argv保留系统提供的参数边界。
///
/// 抽出来是为了让排除规则能被夹具逐条断言——自检里那几条「系统路径不该误报」
/// （`CursorUIViewService` 不该算 Cursor、`ssh-agent` 不该算任何档案）以前只能靠真机碰运气。
///
/// 三条规则：
/// ① 名字**精确**匹配或「名字 + 空格」前缀（所以 `cursoruiviewservice` 不会命中 `Cursor`）；
/// ② 真实入口含提示词也算命中（npm CLI的脚本入口；不扫描内联正文或业务参数）；
/// ③ 路径命中排除表就否决——即使名字对上了。
pub fn profile_matches(profile: &AgentProfile, name: &str, exe: &str, argv: &[&str]) -> bool {
    let entry = command_entry(name, exe, argv);
    matches_entry(profile, name, exe, entry.as_deref())
}

fn matches_entry(profile: &AgentProfile, name: &str, exe: &str, entry: Option<&str>) -> bool {
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
            .any(|hint| entry.is_some_and(|entry| entry_matches_hint(entry, hint)));
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

fn entry_matches_hint(entry: &str, hint: &str) -> bool {
    let hint = hint.to_lowercase().replace('\\', "/");
    if hint.is_empty() {
        return false;
    }
    let stem = [".js", ".cjs", ".mjs", ".ts", ".cts", ".mts", ".py", ".exe"]
        .iter()
        .find_map(|extension| entry.strip_suffix(extension))
        .unwrap_or(entry);
    [entry, stem].into_iter().any(|candidate| {
        if hint.contains('/') {
            candidate
                .strip_suffix(&hint)
                .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('/'))
        } else {
            candidate.rsplit('/').next() == Some(hint.as_str())
        }
    })
}

/// Resolve an interpreter's actual entry, never its inline program or later arguments.
/// Unknown launcher options fail closed; native process-name matching stays independent.
fn command_entry<S: AsRef<str>>(name: &str, exe: &str, argv: &[S]) -> Option<String> {
    let runtime = exe
        .rsplit(['/', '\\'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(name)
        .to_lowercase();
    let runtime = runtime.strip_suffix(".exe").unwrap_or(&runtime);
    let python = runtime.strip_prefix("python").is_some_and(|suffix| {
        suffix.is_empty() || suffix.chars().all(|c| c.is_ascii_digit() || c == '.')
    });
    let node = matches!(runtime, "node" | "nodejs" | "bun");
    let normalize =
        |entry: &str| (!entry.is_empty()).then(|| entry.to_lowercase().replace('\\', "/"));
    if !python && !node {
        return normalize(if exe.is_empty() {
            argv.first().map(AsRef::as_ref).unwrap_or(name)
        } else {
            exe
        });
    }
    // Node CLIs may replace argv0 with a reported process title. Some OS readers
    // retain a stale argc afterward, so trailing entries cannot identify the script.
    // Match the complete alias as an entry; never extract words from a title/body.
    if node {
        if let Some(alias) = argv.first().map(AsRef::as_ref) {
            let base = alias
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(alias)
                .to_lowercase();
            let base = base.strip_suffix(".exe").unwrap_or(&base);
            if !matches!(base, "node" | "nodejs" | "bun")
                && !alias.starts_with('-')
                && !base.is_empty()
                && !base.chars().any(char::is_whitespace)
                && !base.contains('=')
            {
                return normalize(alias);
            }
        }
    }
    let mut args = argv.iter().skip(1).map(AsRef::as_ref);
    while let Some(arg) = args.next() {
        if arg == "-" {
            return None;
        }
        if arg == "--" {
            return args.next().and_then(normalize);
        }
        if python {
            if arg == "-c" || arg.starts_with("-c") {
                return None;
            }
            if arg == "-m" {
                return args.next().and_then(normalize);
            }
            if let Some(module) = arg.strip_prefix("-m").filter(|module| !module.is_empty()) {
                return normalize(module);
            }
            if matches!(arg, "-W" | "-X" | "--check-hash-based-pycs") {
                args.next()?;
                continue;
            }
            if matches!(
                arg,
                "-u" | "-B"
                    | "-I"
                    | "-E"
                    | "-s"
                    | "-S"
                    | "-q"
                    | "-O"
                    | "-OO"
                    | "-v"
                    | "-vv"
                    | "-b"
                    | "-bb"
                    | "-d"
                    | "-R"
                    | "-P"
            ) || arg.starts_with("-W")
                || arg.starts_with("-X")
            {
                continue;
            }
        } else {
            if arg.starts_with("-e")
                || arg.starts_with("-p")
                || matches!(arg, "--eval" | "--print")
                || arg.starts_with("--eval=")
                || arg.starts_with("--print=")
            {
                return None;
            }
            if matches!(
                arg,
                "-r" | "--require"
                    | "--import"
                    | "--loader"
                    | "--experimental-loader"
                    | "--conditions"
                    | "-C"
                    | "--icu-data-dir"
                    | "--openssl-config"
                    | "--title"
                    | "--inspect-port"
                    | "--redirect-warnings"
                    | "--diagnostic-dir"
                    | "--env-file"
                    | "--env-file-if-exists"
                    | "--input-type"
                    | "--max-old-space-size"
                    | "--max-semi-space-size"
                    | "--stack-size"
                    | "--max-http-header-size"
            ) {
                args.next()?;
                continue;
            }
            if matches!(
                arg,
                "--no-warnings"
                    | "--trace-warnings"
                    | "--enable-source-maps"
                    | "--no-deprecation"
                    | "--trace-deprecation"
                    | "--preserve-symlinks"
                    | "--preserve-symlinks-main"
                    | "--use-strict"
                    | "--experimental-strip-types"
                    | "--no-experimental-strip-types"
                    | "--inspect"
                    | "--inspect-brk"
            ) || arg.starts_with("-r")
                || (arg.starts_with("--") && arg.contains('='))
            {
                continue;
            }
            if runtime == "bun" && arg == "run" {
                continue;
            }
            if runtime == "bun" && matches!(arg, "x" | "exec") {
                return None;
            }
        }
        if arg.starts_with('-') {
            return None;
        }
        return normalize(arg);
    }
    None
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
        assert!(
            table.iter().all(|hit| hit.cpu.is_none()),
            "首次采样不能伪报 CPU 0 或旧读数"
        );
    }
}

#[cfg(test)]
mod optimization_regressions {
    use super::*;

    #[test]
    fn node_hosted_cli_is_identified_by_its_real_command_line() {
        use super::command_identity_regressions::{ChildGuard, FixtureDir};
        let fixture = FixtureDir::new();
        let script = fixture.write(
            "agentisland-cli-probe-fixture.cjs",
            "setInterval(() => {}, 1000)",
        );
        let child = ChildGuard(
            std::process::Command::new("node")
                .arg(script)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("Node fixture must start"),
        );
        let mut monitor = ProcessMonitor::new();
        let mut profile = crate::registry::builtin().remove(0);
        profile.process_names.clear();
        profile.path_contains.clear();
        profile.path_excludes.clear();
        profile.cmdline_hints = vec!["agentisland-cli-probe-fixture".into()];
        let mut found = false;
        for _ in 0..10 {
            monitor.refresh();
            found |= monitor
                .match_profile(&profile)
                .iter()
                .any(|hit| hit.pid == child.0.id());
            if found {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            found,
            "refresh must populate cmd for Node-hosted CLI detection"
        );
    }

    #[test]
    fn zombies_do_not_keep_an_agent_online() {
        let mut monitor = ProcessMonitor::new();
        let mut hit = monitor.table().into_iter().next().unwrap();
        hit.name = "agentisland-fixture".into();
        hit.is_zombie = true;
        monitor.rows = vec![ObservedProcess {
            hit,
            launch_entry: None,
        }];
        let mut profile = crate::registry::builtin().remove(0);
        profile.process_names = vec!["agentisland-fixture".into()];
        profile.cmdline_hints.clear();
        profile.path_contains.clear();
        profile.path_excludes.clear();
        assert!(
            monitor.match_profile(&profile).is_empty(),
            "exited zombies are not live agents"
        );
    }
}

#[cfg(test)]
mod command_identity_regressions {
    use super::*;

    pub(super) struct ChildGuard(pub(super) std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    pub(super) struct FixtureDir(std::path::PathBuf);
    impl FixtureDir {
        pub(super) fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "agentisland argv 中文 {}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
        pub(super) fn write(&self, name: &str, body: &str) -> std::path::PathBuf {
            let path = self.0.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, body).unwrap();
            path
        }
    }
    impl Drop for FixtureDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn owners(monitor: &ProcessMonitor, pid: u32) -> Vec<String> {
        crate::registry::builtin()
            .into_iter()
            .filter(|profile| {
                monitor
                    .match_profile(profile)
                    .iter()
                    .any(|hit| hit.pid == pid)
            })
            .map(|profile| profile.id)
            .collect()
    }

    #[test]
    fn inline_diagnostic_mentions_are_not_running_agents() {
        let child = ChildGuard(std::process::Command::new("python3")
            .args(["-c", "import time; paths = ['.codex/config.toml', '.claude/settings.json']; time.sleep(10)"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn().expect("Python fixture must start"));
        let monitor = ProcessMonitor::new();
        let owners = owners(&monitor, child.0.id());
        assert!(
            owners.is_empty(),
            "diagnostic process falsely claimed by {owners:?}"
        );
    }

    #[test]
    fn real_node_entries_keep_one_owner_while_preloads_and_prompts_do_not_add_owners() {
        let fixture = FixtureDir::new();
        let preload = fixture.write("claude-preload.cjs", "");
        for (entry, expected) in [
            ("codex/bin/codex.cjs", "codex"),
            ("@anthropic-ai/claude-code/cli.js", "claude"),
            ("@minimax-ai/code/cli.js", "minimaxcode"),
        ] {
            let script = fixture.write(entry, "setInterval(() => {}, 1000)");
            let child = ChildGuard(
                std::process::Command::new("node")
                    .args(["--no-warnings", "--require"])
                    .arg(&preload)
                    .arg(&script)
                    .args(["--prompt", "codex and claude"])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .expect("Node entry fixture must start"),
            );
            let monitor = ProcessMonitor::new();
            assert_eq!(owners(&monitor, child.0.id()), vec![expected], "{entry}");
            assert_eq!(
                monitor
                    .table()
                    .into_iter()
                    .find(|hit| hit.pid == child.0.id())
                    .unwrap()
                    .name,
                "node",
                "must exercise runtime entry matching, not a native process-name shortcut"
            );
        }
    }

    #[test]
    fn live_inline_and_business_argument_mentions_never_claim_agents() {
        let fixture = FixtureDir::new();
        let python = fixture.write("inspect.py", "import time\ntime.sleep(10)\n");
        let node = fixture.write("inspect.cjs", "setInterval(() => {}, 1000)");
        for (program, args) in [
            (
                "python3",
                vec![
                    python.to_str().unwrap(),
                    ".codex/config.toml",
                    ".claude/settings.json",
                ],
            ),
            (
                "python3",
                vec!["-c", "import time; label='codex'; time.sleep(10)"],
            ),
            (
                "node",
                vec![node.to_str().unwrap(), "--prompt", "codex and claude"],
            ),
            (
                "node",
                vec![
                    "-e",
                    "const label='claude'; setInterval(() => {}, 1000)",
                    "codex",
                ],
            ),
        ] {
            let child = ChildGuard(
                std::process::Command::new(program)
                    .args(args)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .expect("diagnostic fixture must start"),
            );
            let monitor = ProcessMonitor::new();
            assert!(
                owners(&monitor, child.0.id()).is_empty(),
                "{program} diagnostic misidentified"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn npm_command_symlink_keeps_its_cli_owner() {
        let fixture = FixtureDir::new();
        let script = fixture.write("package/cli.js", "setInterval(() => {}, 1000)");
        let alias = fixture.0.join("claude");
        std::os::unix::fs::symlink(script, &alias).unwrap();
        let child = ChildGuard(
            std::process::Command::new("node")
                .arg(alias)
                .args(["--prompt", "codex"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        assert_eq!(owners(&ProcessMonitor::new(), child.0.id()), vec!["claude"]);
    }

    #[cfg(unix)]
    #[test]
    fn same_pid_exec_refreshes_the_running_entry() {
        let fixture = FixtureDir::new();
        let script = fixture.write("codex.js", "setInterval(() => {}, 1000)");
        let ready = fixture.0.join("ready");
        let proceed = fixture.0.join("proceed");
        let launcher = fixture.write("launcher.py", "import os,sys,time\nfrom pathlib import Path\nPath(sys.argv[1]).touch()\nwhile not Path(sys.argv[2]).exists(): time.sleep(.01)\nos.execvp('node', ['node',sys.argv[3]])\n");
        let child = ChildGuard(
            std::process::Command::new("python3")
                .arg(launcher)
                .args([&ready, &proceed, &script])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while !ready.exists() {
            assert!(Instant::now() < deadline, "launcher did not become ready");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let mut monitor = ProcessMonitor::new();
        assert!(owners(&monitor, child.0.id()).is_empty());
        std::fs::write(proceed, "continue").unwrap();
        loop {
            monitor.refresh();
            if owners(&monitor, child.0.id()) == vec!["codex"] {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "same PID kept its pre-exec identity"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    #[test]
    fn native_exec_does_not_keep_a_cached_agent_name() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = FixtureDir::new();
        let binary = fixture.0.join("codex");
        std::fs::write(&binary, std::fs::read("/bin/bash").unwrap()).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        #[cfg(target_os = "macos")]
        assert!(
            std::process::Command::new("codesign")
                .args(["--force", "--sign", "-"])
                .arg(&binary)
                .output()
                .unwrap()
                .status
                .success(),
            "temporary native fixture must be independently signed"
        );
        let ready = fixture.0.join("ready");
        let proceed = fixture.0.join("proceed");
        let launcher = fixture.write("launcher.sh", "printf ready > \"$1\"\nwhile [ ! -f \"$2\" ]; do sleep .01; done\nexec /bin/sleep 10\n");
        let child = ChildGuard(
            std::process::Command::new(binary)
                .arg(launcher)
                .args([&ready, &proceed])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while !ready.exists() {
            assert!(
                Instant::now() < deadline,
                "native launcher did not become ready"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let mut monitor = ProcessMonitor::new();
        assert_eq!(owners(&monitor, child.0.id()), vec!["codex"]);
        std::fs::write(proceed, "continue").unwrap();
        loop {
            monitor.refresh();
            if owners(&monitor, child.0.id()).is_empty() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "exec left a cached native agent owner"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    #[test]
    fn node_cli_process_title_keeps_its_reported_alias() {
        let fixture = FixtureDir::new();
        let ready = fixture.0.join("ready");
        let body = format!("process.title='claude'; require('fs').writeFileSync({},'ready'); setInterval(() => {{}},1000);", serde_json::to_string(ready.to_str().unwrap()).unwrap());
        let script = fixture.write("cli.js", &body);
        let child = ChildGuard(
            std::process::Command::new("node")
                .arg(script)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while !ready.exists() {
            assert!(
                Instant::now() < deadline,
                "titled Node entry did not become ready"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(owners(&ProcessMonitor::new(), child.0.id()), vec!["claude"]);
    }

    #[test]
    fn argv_boundaries_options_and_windows_paths_preserve_entry_identity() {
        let profiles = crate::registry::builtin();
        let owners = |name, exe, argv: &[&str]| {
            profiles
                .iter()
                .filter(|p| profile_matches(p, name, exe, argv))
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>()
        };
        for argv in [
            vec!["node", "--eval", "'codex claude'"],
            vec!["node", "--print=codex"],
            vec!["node", "-pe", "'claude'"],
            vec![
                "node",
                "--require",
                "/tools/codex/preload.js",
                "/tools/inspect.js",
                "claude",
            ],
            vec!["node", "--title", "codex", "/tools/inspect.js"],
            vec!["node", "--unknown-option", "codex", "/tools/inspect.js"],
            vec!["node", "--require"],
            vec!["node", "-", "codex"],
        ] {
            assert!(
                owners("node", "/usr/bin/node", &argv).is_empty(),
                "{argv:?}"
            );
        }
        assert_eq!(
            owners(
                "node.exe",
                r"C:\Program Files\nodejs\node.exe",
                &[
                    r"C:\Program Files\nodejs\NODE.EXE",
                    "--max-old-space-size=4096",
                    "--",
                    r"C:\CLI Folder\@anthropic-ai\claude-code\cli.js",
                    "codex"
                ]
            ),
            vec!["claude"]
        );
        assert!(owners(
            "python3.14",
            "/usr/bin/python3.14",
            &["python3", "-I", "-m", "claude_cli", "codex"]
        )
        .is_empty());
        assert!(owners(
            "python3.14",
            "/usr/bin/python3.14",
            &["python3", "-X", "codex", "/tools/inspect.py", "claude"]
        )
        .is_empty());
        assert!(owners("sh", "/bin/sh", &["sh", "-c", "codex --prompt claude"]).is_empty());
    }

    #[test]
    fn named_project_folders_do_not_make_unrelated_entries_agents() {
        let profiles = crate::registry::builtin();
        for (name, exe, argv) in [
            (
                "node",
                "/usr/bin/node",
                vec!["node", "/projects/codex-playground/inspect.js"],
            ),
            (
                "node",
                "/usr/bin/node",
                vec!["node", "/projects/codex/inspect.js"],
            ),
            (
                "python3",
                "/usr/bin/python3",
                vec!["python3", "/projects/claude-tests/inspect.py"],
            ),
            ("inspect", "/projects/claude-tools/inspect", vec!["inspect"]),
            (
                "node",
                "/usr/bin/node",
                vec!["node", "/tools/not-claude-code/cli.js"],
            ),
            (
                "node",
                "/usr/bin/node",
                vec!["node", "/tools/@minimax-ai/code/mcode-tools.js"],
            ),
        ] {
            let owners: Vec<_> = profiles
                .iter()
                .filter(|p| profile_matches(p, name, exe, &argv))
                .map(|p| p.id.as_str())
                .collect();
            assert!(owners.is_empty(), "unrelated entry claimed by {owners:?}");
        }
    }
}
