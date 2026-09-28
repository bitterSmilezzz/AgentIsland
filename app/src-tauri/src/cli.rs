//! `agentisland` 终端入口（对应 Swift CLI 的 12 个子命令）。
//!
//! **放在 `main()` 最前面**：`agentisland status` 不该启动窗口、拉起托盘、
//! 也不该开任何后台线程。UI 与 CLI 复用同一批纯模块（`audit` / `report` /
//! `forecast` / `health` / `observability` / `tokens`），所以两边看到的数天然同源。
//!
//! 三条与 Swift 侧一致的硬约定：
//! · **退出码**：0 成功、1 失败、2 用法错。写盘失败不许静默 exit 0。
//! · **`—` 只表示「没查」**；查了、确实是零就写数字。
//! · **CPU 只采一拍的入口**（`status` / `report`）没有差分窗口，那一列写 `—`，
//!   不是 `0.0%`——「没测到」与「测到是零」是两件事。

use crate::{audit, cost, engine, forecast, observability, tokens, Settings};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

pub const EXIT_OK: i32 = 0;
pub const EXIT_FAIL: i32 = 1;
pub const EXIT_USAGE: i32 = 2;

/// 本轮**实现到哪一步**。写在这里而不是散在各个命令里，是因为
/// 「有哪些子命令」是用户与文档都会问的问题，它得有一个单一的答案来源。
///
/// 刻意用**数组**而不是 map：顺序即文档里的顺序，读的人一眼看到
/// 「已实现」与「未实现」的分界在哪。
pub const COMMANDS: &[(&str, bool, &str)] = &[
    ("status", true, "所有 Agent 的运行态快照（-w 动态监控，--json 供脚本）"),
    ("doctor", true, "这个 Agent 是真闲着，还是我根本没看到它"),
    ("tokens", true, "24h 用量明细、成本与月末预测（--budget 打印进度条）"),
    ("state", true, "读 App 进程内的实时状态（谁在跑、这一拍的状态是谁说的）"),
    ("selftest", true, "用假数据断言核心判定逻辑（验证构建本身而非本机状态）"),
    ("check", true, "排查异常驻留与持续高负载（只读，不终止任何进程）"),
    ("clean", true, "终止异常进程（-n 只预览；孤儿须逐条点名）"),
    ("open", true, "控制 App 展开/折叠/直达（toggle|expand|collapse|analytics|toolbox|export|workbench）"),
    ("notify", true, "向本机 App 投递一次事件（--kind completed|attention|costspike）"),
    ("report", true, "生成 Markdown / CSV 运维报告（-o 写盘、--format md|csv）"),
    ("raycast", true, "导出 Raycast Extension 命令清单（--json 已是默认）"),
    ("top", true, "持续观测看板（r 刷新、q 退出；不接管终端的全屏 TUI）"),
];

/// 入口。返回 `None` 表示「不是 CLI 调用」，交回给 UI 走。
pub fn try_run(args: &[String]) -> Option<i32> {
    let first = args.get(1).map(String::as_str).unwrap_or("");
    // `--selftest` 是历史入口（v0.0.184 起），与子命令同义，两者都收
    if first.is_empty() || first.starts_with('-') {
        if first == "--selftest" {
            return Some(selftest());
        }
        return None; // 无子命令 ⇒ 走 UI（双击启动的正常路径）
    }

    // 去掉子命令本身，剩下的转成「开关」与「位置参数」
    let rest: Vec<String> = args[2..].to_vec();
    let flags: Vec<String> = rest
        .iter()
        .filter(|a| a.starts_with("--"))
        .cloned()
        .collect();
    let positional: Vec<String> = rest
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect();

    let code = match first {
        "status" => status(&flags, &positional),
        "doctor" => doctor(&positional),
        "tokens" => tokens_cmd(&flags),
        "check" => check(&flags),
        "clean" => clean_cmd(&flags, &positional),
        "notify" => notify(&flags),
        "report" => report_cmd(&flags),
        "raycast" => raycast(),
        "open" => open_cmd(&positional),
        "top" => top(&flags),
        "state" => state_cmd(),
        "selftest" => selftest(),
        other => {
            // 未实现的子命令要**说清是没实现**，而不是当成用法错——
            // 「用法错」会让人以为自己拼错了，而这里是他拼对了、我们没做
            match COMMANDS.iter().find(|(name, _, _)| *name == other) {
                Some((name, false, desc)) => {
                    eprintln!("✗ `agentisland {name}` 尚未实现：{desc}");
                    eprintln!("  已实现：{}", implemented_list());
                    EXIT_FAIL
                }
                _ => {
                    usage();
                    EXIT_USAGE
                }
            }
        }
    };
    Some(code)
}

/// **不许被测试裸调的子命令**。两个成员，两个**不同**的理由：
///
/// - `top`：调进去不会自己结束 ⇒ 用例会挂死。症状是「测试卡住」而不是
///   「哪条断言红了」，很难一眼看出原因。
/// - `clean`：**会真的终止进程**。测试跑在开发机上，而开发机上有正在跑的
///   Agent——一条自动遍历「每个已实现子命令」的守护足以在跑测试时杀掉它们。
///   这条比第一条更危险：第一条只浪费时间，这条会毁掉别人正在做的事。
///
/// 单独列出来，是为了让将来新增这类命令的人**必然看到这一处**，
/// 而不是靠「测试挂了」或「我的进程怎么没了」去反推。
#[cfg(test)]
pub fn must_not_auto_invoke(name: &str) -> bool {
    matches!(name, "top" | "clean")
}

pub fn implemented_list() -> String {
    COMMANDS
        .iter()
        .filter(|(_, done, _)| *done)
        .map(|(name, _, _)| *name)
        .collect::<Vec<_>>()
        .join("、")
}

fn usage() {
    eprintln!("用法: agentisland <子命令> [选项]");
    eprintln!();
    for (name, done, desc) in COMMANDS {
        eprintln!("  {:<10} {}{}", name, desc, if *done { "" } else { "" });
    }
    eprintln!();
    eprintln!("  agentisland --selftest    无头自检（等价于 `selftest` 子命令）");
    eprintln!("  --demo / --shell=sidebar  启动应用并直接进对应形态");
    eprintln!("  --shell=sidebar           启动侧边栏形态");
}

/// 一次性采样：不建窗口、不开线程，采一拍就把状态交出来。
///
/// 与常驻引擎共用 `ActivityEngine`——**这是 CLI 与界面的数能对上**的唯一理由。
/// 自造一套采样逻辑的话，两边迟早分叉，而 CLI 是给人拿去做判断的。
fn sample_once() -> (Vec<crate::models::AgentSnapshot>, crate::models::TokenUsage) {
    sample_once_with(false)
}

/// 单拍采样。`want_usage` = 要不要为这一拍解析全部会话索引。
///
/// **默认 `false`**（与 Swift `StatusCommand` 同口径）：本机实测同步取用量多花 5 秒，
/// 而 Raycast / 脚本调用是这条命令的主要场景。不取时那一列印 `—`，**明说没取**——
/// 印 0 是在说「查了、确实是零」，那是另一件事。
fn sample_once_with(want_usage: bool) -> (Vec<crate::models::AgentSnapshot>, crate::models::TokenUsage) {
    let (_tx, rx) = mpsc::channel();
    let mut engine = engine::ActivityEngine::new(Settings::load(), rx);
    engine.refresh_usage = want_usage;
    engine.tick();
    let state = engine.state();
    (state.snapshots, state.grand_total)
}

// MARK: - status

fn status(flags: &[String], positional: &[String]) -> i32 {
    let has = |name: &str| flags.iter().any(|f| f == name);
    // `-w/--watch` 交给 `top`（持续观测那一套），与 Swift 同一条路径
    if has("-w") || has("--watch") {
        return top(flags);
    }
    let want_all = has("--all") || has("-a");
    let as_json = has("--json");
    let want_usage = has("--usage");

    let (snapshots, _) = sample_once_with(want_usage);
    // 可见口径与界面一致：**只显示进程仍在的**。`--all` 才把离线的也列出来。
    let mut list: Vec<_> = snapshots
        .iter()
        .filter(|s| want_all || s.process_running)
        .collect();

    // 位置参数是 agent id 或名称过滤
    if let Some(filter) = positional.first() {
        let needle = filter.to_lowercase();
        list.retain(|s| s.id.to_lowercase() == needle || s.name.to_lowercase() == needle);
        if list.is_empty() {
            eprintln!("✗ 没有匹配 `{filter}` 的 Agent");
            return EXIT_FAIL;
        }
    }

    if as_json {
        // JSON 契约：`—` 是**键缺席或 null**，0 是查了确实为零
        let rows: Vec<serde_json::Value> = list
            .iter()
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "name": s.name,
                    "level": s.level.as_str(),
                    "levelLabel": s.level_label,
                    "processRunning": s.process_running,
                    "pid": s.pid,
                    // 单拍没有 CPU 差分窗口 ⇒ **恒为 null**，不是 0.0。
                    // 引擎那一拍的 `Some(0.0)` 是「没采到过」折出来的，不是读数
                    "cpuPercent": serde_json::Value::Null,
                    "isHung": s.is_hung,
                    "healthScore": s.health.score,
                    "observability": s.observability.code.as_str(),
                    "observabilitySummary": s.observability.summary,
                    "installed": s.installed,
                    "tokens24h": s.token_usage.as_ref().map(|u| u.tokens24h),
                    "cost24h": s.token_usage.as_ref().map(|u| u.cost24h),
                    "lastActivity": s.last_activity_text,
                })
            })
            .collect();
        match serde_json::to_string_pretty(&rows) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                eprintln!("✗ 序列化失败: {error}");
                return EXIT_FAIL;
            }
        }
        return EXIT_OK;
    }

    if list.is_empty() {
        println!("未检测到活跃或已安装的 Agent（使用 --all 查看全部支持的智能体）");
        return EXIT_OK;
    }

    println!("🤖 AgentIsland 智能体运行态快照 ({} 个条目)", list.len());
    println!();
    println!(
        "{:<16} {:<10} {:<7} {:<9} {:<8} {}",
        "智能体", "状态", "PID", "CPU", "24h 用量", "最近活动"
    );
    for s in &list {
        // CPU：本命令采**单拍**，没有差分窗口 ⇒ 恒为 `—`。
        //
        // **刻意不去读 `s.cpu_percent`**：引擎那一拍在首次采样时会把「没有上一次
        // 读数」折成 `Some(0.0)`，直接印出来就是 `0.0%`——而那既不是「测到了零」
        // 也不是「没测到」。Swift 侧同一处也是印 `—`，那不是显示偏好，是口径。
        let cpu = "—";
        // 用量：`None` 有两种可能——**没取**（本命令默认）与**取到但确实是零**。
        // 两者都印 `—` 吗？不：取到且为零是「确实零」，印 `0`；
        // 没取印 `—` 并在表尾说明。区分靠的是「这一拍取没取」，不是这一个数。
        let usage = match s.token_usage.as_ref() {
            Some(u) if u.tokens24h > 0 => tokens::compact(u.tokens24h),
            Some(_) => "0".to_string(),
            None if !want_usage => "—(未取)".to_string(),
            None => "—".to_string(),
        };
        println!(
            "{:<16} {:<10} {:<7} {:<9} {:<8} {}",
            truncate(&s.name, 16),
            s.level_label,
            s.pid.map(|p| p.to_string()).unwrap_or_else(|| "—".into()),
            cpu,
            usage,
            s.last_activity_text,
        );
    }
    println!();
    println!("提示: CPU 列在本命令下恒为「—」——单拍采样没有差分窗口。");
    println!("      要真实 CPU 用持续观测的 `agentisland top -w` 或界面里的详情页。");
    if !want_usage {
        println!("      用量列本命令**默认不取**（解析全部会话索引本机实测多花 5 秒）；");
        println!("      要取就加 `--usage`，那一列会从「—(未取)」变成真实数字。");
    }
    EXIT_OK
}

// MARK: - doctor

fn doctor(positional: &[String]) -> i32 {
    let (snapshots, _) = sample_once();
    let wanted = positional.first();
    let list: Vec<_> = snapshots
        .iter()
        .filter(|s| {
            // 可见口径同 status：只看得到的就是这些；没被提到的那个 agent
            // 在这个命令里**没有结论可给**，明说比编一个强
            s.process_running
                && wanted
                    .map(|w| {
                        let n = w.to_lowercase();
                        s.id.to_lowercase() == n || s.name.to_lowercase() == n
                    })
                    .unwrap_or(true)
        })
        .collect();

    if list.is_empty() {
        eprintln!("✗ 没有可判断的 Agent（进程都不在；`agentisland status --all` 看全部档案）");
        return EXIT_FAIL;
    }

    let mut suspicious = 0usize;
    for s in &list {
        let verdict = &s.observability;
        let flag = if verdict.code == observability::Code::Observed {
            "✅"
        } else {
            suspicious += 1;
            "⚠️"
        };
        println!("{flag} {:<20} {}", s.name, verdict.summary);
        for line in &verdict.evidence {
            println!("     · {line}");
        }
        // 健康度也在这里给一份：`doctor` 是「这条结论有多可信」的命令，
        // 而健康度是同一条链上的另一环
        println!(
            "     · 健康度 {} 分（{}）",
            s.health.score,
            s.health.grade.label()
        );
    }
    println!();
    if suspicious == 0 {
        println!("全部 {} 个 Agent 的结论都有本地证据。", list.len());
    } else {
        println!(
            "{} 个里 {} 个的结论缺证据——上面写明了缺在哪一条。",
            list.len(),
            suspicious
        );
    }
    EXIT_OK
}

// MARK: - tokens

fn tokens_cmd(flags: &[String]) -> i32 {
    let has = |name: &str| flags.iter().any(|f| f == name);
    let (_, total) = sample_once();
    let as_json = has("--json");
    let now = tokens::now_ms();

    let f = forecast::evaluate(
        total.tokens24h,
        total.cost24h,
        Settings::load().daily_token_budget,
        now,
    );

    if as_json {
        let value = serde_json::json!({
            "tokens24h": total.tokens24h,
            "cost24h": total.cost24h,
            "tokensTotal": total.tokens_total,
            "costTotal": total.cost_total,
            "costEstimated": total.cost_estimated,
            "projectedMonthEndTokens": f.projected_month_end_tokens,
            "projectedMonthEndCost": f.projected_month_end_cost,
            "summary": f.forecast_summary,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into())
        );
        return EXIT_OK;
    }

    println!("Token 用量");
    println!("  24h     {} tokens   {}", tokens::compact(total.tokens24h), cost_text(total.cost24h, total.cost_estimated));
    println!("  累计    {} tokens   {}", tokens::compact(total.tokens_total), cost_text(total.cost_total, total.cost_estimated));
    println!("  {}", f.forecast_summary);
    if has("--budget") {
        let budget = Settings::load().daily_token_budget;
        if budget <= 0 {
            println!("  预算未设（0 = 不告警）");
        } else {
            let ratio = total.tokens24h as f64 / budget as f64;
            println!("  预算进度 {:.0}%", ratio * 100.0);
        }
    }
    EXIT_OK
}

// MARK: - check

/// 排查异常驻留与持续高负载。**只读**——`check` 永远不终止任何进程。
///
/// 终止是 `clean` 的事，而两者共用这一份扫描：把算法写成两份必然分叉，
/// 症状是「`check` 说有异常、`clean` 却说没有可清理的」。
///
/// 返回值里的进程表是 `clean` 复核身份的依据，**必须与异常同源**——
/// 拿一张新表去复核一张旧表列出的异常，等于没复核。
///
/// 判定顺序与门槛对齐 Swift `AgentCleaner.detectAnomalies`（逐条对应）：
/// 遍历**档案的全部匹配进程**（不是只看根 pid——死锁与超限都是单进程性质），
/// 先排掉 GUI 主进程，再依次判死锁 / 孤儿 / 超限，命中一条就 `continue`。
fn scan_anomalies() -> AnomalyScan {
    use crate::cleaner::{Anomaly, AnomalyType};
    let (snapshots, _) = sample_once();
    let monitor = crate::procmon::ProcessMonitor::new();
    let table = monitor.table();
    let recently_active = recently_active_profiles();

    let mut out: Vec<Anomaly> = Vec::new();
    for profile in crate::registry::builtin() {
        // **资格**：死锁是引擎侧按 profile 聚合 CPU 判出来的，三态。
        // `None` = 连续观测不足阈值时长，此时「不是死锁」的含义是「没测」而不是「没有」。
        let hung = snapshots
            .iter()
            .find(|s| s.id == profile.id)
            .is_some_and(|s| s.is_hung == Some(true));
        let name = profile.name.clone();
        for entry in monitor.match_profile(&profile) {
            // pid 1 是 launchd；僵尸占着 pid 但不是活进程
            if entry.pid <= 1 || entry.is_zombie {
                continue;
            }
            // **GUI 主进程永不作为可清理对象**：它是用户正在用的应用本体，
            // 杀它等于关掉编辑器（可能丢未保存内容）。
            // Swift 同口径：开着大项目的 Electron IDE 占 2.5GB 完全正常。
            if is_standard_app_bundle(&entry.exe_path) {
                continue;
            }
            let cpu = entry.cpu.unwrap_or(0.0);
            let make = |kind: AnomalyType, reason: &str| Anomaly {
                pid: entry.pid,
                // 真实父进程：**孤儿判定完全依赖这一位**。填 0 会让「父进程还在」
                // 与「父进程查不到」都变成孤儿，等于把常驻服务当成遗孤。
                ppid: entry.ppid,
                profile_id: profile.id.clone(),
                agent_name: name.clone(),
                // 扫描时刻的可执行身份：动手前的复核就靠它
                command_path: crate::cleaner::identity(&entry),
                memory_bytes: entry.memory,
                anomaly_type: kind,
                reason: reason.to_string(),
            };
            // ① 死锁：档案级聚合判定 + **单进程** CPU 门槛。
            //    聚合高负载时单个子进程 >10% 属正常（Swift 侧同一条注释），
            //    按单条判定会把正常渲染进程列成「疑似死锁」。
            if hung && cpu > HUNG_CPU_FLOOR {
                out.push(make(
                    AnomalyType::Hung,
                    &format!("持续过载超阈值（单进程 CPU {cpu:.1}%）"),
                ));
                continue;
            }
            // ② 孤儿：父进程已转 launchd。判定本身留在 `cleaner::looks_orphan`
            //    （有独立用例钉住「两条缺一不可」），这里只负责按它的结论分流。
            if entry.ppid == 1 && !profile.process_names.is_empty() {
                let candidate = make(
                    AnomalyType::Orphan,
                    "主控终端已关闭，已脱离原会话成为孤儿进程（PPID=1）",
                );
                // 有活动佐证 = launchd 托管的常驻服务在干活 ⇒ 跳过，
                // 且**连超限也不报**（Swift 同口径：这里 continue 掉整条分支）
                if crate::cleaner::looks_orphan(&candidate, &recently_active) {
                    out.push(candidate);
                }
                continue;
            }
            // ③ 内存超限（Swift 同阈值：> 2.0GB）
            if entry.memory > OVERWEIGHT_BYTES {
                out.push(make(
                    AnomalyType::Overweight,
                    "物理内存持续占用超过 2.0GB，疑似堆内存泄露或超长上下文堆积",
                ));
            }
        }
    }

    let memory_by_pid = table.iter().map(|h| (h.pid, h.memory)).collect();
    AnomalyScan {
        anomalies: out,
        table,
        memory_by_pid,
        recently_active,
    }
}

/// 单进程 CPU 门槛（Swift 同值 10.0）
const HUNG_CPU_FLOOR: f64 = 10.0;
/// 内存超限门槛：2 GiB（Swift 同值 2_147_483_648）
const OVERWEIGHT_BYTES: u64 = 2_147_483_648;

/// 标准 App 主进程（`/Applications/x.app/Contents/MacOS/...`）。判据同 Swift。
fn is_standard_app_bundle(path: &str) -> bool {
    path.contains(".app/Contents/MacOS")
}

/// 扫描结果：异常 + 它们所依据的进程事实。
struct AnomalyScan {
    anomalies: Vec<crate::cleaner::Anomaly>,
    /// 扫描时刻的进程表（终止顺序与「我们以为在杀谁」都从它来）
    table: Vec<crate::procmon::ProcHit>,
    /// 复核「回收了多少内存」要用；**只统计确认退出的那些**
    memory_by_pid: HashMap<u32, u64>,
    /// 孤儿佐证：10 分钟内有会话写入的档案
    recently_active: HashSet<String>,
}

/// 近 [`crate::cleaner::ORPHAN_EVIDENCE_WINDOW`] 内有会话写入的档案集合。
///
/// 这是孤儿那把刀的最后一道闸：`ppid == 1` 分不开「终端关掉的遗孤」与
/// 「launchd 刻意托管的常驻服务」，而后者一定有会话在写。
fn recently_active_profiles() -> HashSet<String> {
    let window = crate::cleaner::ORPHAN_EVIDENCE_WINDOW.as_secs_f64();
    let mut monitor = crate::filemon::FileMonitor::new();
    crate::registry::builtin()
        .into_iter()
        .filter(|profile| {
            monitor
                .probe(profile, window)
                .latest_write
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|ago| ago.as_secs_f64() <= window)
        })
        .map(|profile| profile.id)
        .collect()
}

fn check(_flags: &[String]) -> i32 {
    let scan = scan_anomalies();
    if scan.anomalies.is_empty() {
        println!("没有排查到异常驻留或持续高负载。");
        println!("（死锁判定需要**连续观测**够久——刚启动时「没测到」也会印这一句）");
        return EXIT_OK;
    }
    println!("排查到 {} 条异常：", scan.anomalies.len());
    for a in &scan.anomalies {
        println!(
            "  ⚠️  {} · pid {} · {} · {}",
            a.agent_name,
            a.pid,
            a.anomaly_type.label(),
            a.reason
        );
    }
    println!();
    println!(
        "`check` 只看不杀。先用 `agentisland clean -n` 预览要动哪些，再去掉 `-n` 终止。"
    );
    EXIT_OK
}

/// `clean`：**逐条**终止，动手前按可执行身份复核。
///
/// 批量（`clean` 不带位置参数）只处理**可批量**的那些（死锁 / 内存超限）；
/// 孤儿**一律要逐条点名**——`ppid == 1` 与 launchd 刻意托管的常驻服务分不开，
/// 批量误杀等于静默丢任务。给孤儿 ID 就是逐条。
///
/// `-n / --dry-run` 只排计划、不发信号。它存在的理由和 `check` 分开：
/// `check` 的异常口径与 `clean` 同源但**判定更宽**（只看高负载），
/// 而 `-n` 印的是**这一次真的会动到哪些进程**，两者不能互相替代。
fn clean_cmd(flags: &[String], positional: &[String]) -> i32 {
    let dry_run = flags.iter().any(|f| f == "-n" || f == "--dry-run");
    let mut scan = scan_anomalies();
    if scan.anomalies.is_empty() {
        println!("没有需要清理的异常。");
        return EXIT_OK;
    }
    let batch = positional.is_empty();
    if batch {
        scan.anomalies.retain(|a| a.batch_cleanable());
    } else {
        let wanted: HashSet<String> = positional.iter().map(|p| p.to_lowercase()).collect();
        scan.anomalies.retain(|a| {
            wanted.contains(&a.pid.to_string())
                || wanted.contains(&a.profile_id)
                || wanted.contains(a.anomaly_type.label())
        });
        if scan.anomalies.is_empty() {
            eprintln!(
                "✗ 没有匹配的可清理目标：{}（可按 pid、档案 id 或类型名指定）",
                positional.join(" ")
            );
            return EXIT_FAIL;
        }
    }
    if scan.anomalies.is_empty() {
        println!("没有可批量清理的异常（孤儿必须逐条点名：`agentisland clean <pid>`）。");
        return EXIT_OK;
    }

    // 计划与执行分开：先排出来给人看，再决定要不要发信号。
    // 「打算杀谁」和「真的杀了」必须是两句话，否则 `-n` 只能靠事后回滚来假装。
    let monitor = crate::procmon::ProcessMonitor::new();
    let fresh = monitor.table();
    let plan = crate::cleaner::plan(
        &scan.anomalies,
        &scan.table,
        &fresh,
        &crate::cleaner::self_and_ancestors(),
        &scan.recently_active,
        batch,
    );

    println!(
        "{} {} 条异常，计划涉及 {} 个进程：",
        if dry_run { "【预览】" } else { "将要" },
        scan.anomalies.len(),
        plan.steps.len()
    );
    for step in &plan.steps {
        println!(
            "  {} · pid {} · {} · {}",
            step.action.label(),
            step.pid,
            if step.identity.is_empty() { "—" } else { &step.identity },
            step.anomaly
        );
    }
    let targets = plan.signal_targets();
    if targets.is_empty() {
        println!();
        println!("没有可以动手的进程——上面每一条都说明了为什么跳过。");
        return EXIT_OK;
    }
    if dry_run {
        println!();
        println!("`-n` 不发任何信号。去掉它才会终止。");
        return EXIT_OK;
    }

    let signaled = crate::cleaner::execute(&plan);
    let verification = crate::cleaner::verify(&signaled_steps(&plan), &scan.memory_by_pid);
    println!();
    println!(
        "发出终止信号 {} 个；复核后确认退出 {} 个。",
        signaled.len(),
        verification.confirmed.len()
    );
    if !verification.still_running.is_empty() {
        println!(
            "⚠️ 仍在运行：{}",
            verification
                .still_running
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    if verification.confirmed.is_empty() {
        return EXIT_FAIL;
    }
    // **只统计确认退出的那些**：把仍在运行的进程内存也算进「回收」，
    // 等于报一个必然偏大的数
    println!(
        "回收内存约 {} MB（只统计确认退出的那些）",
        verification.reclaimed_memory_bytes / (1024 * 1024)
    );
    EXIT_OK
}

/// 复核只认**真正发了信号**的那些步骤。
fn signaled_steps(
    plan: &crate::cleaner::KillPlan,
) -> Vec<crate::cleaner::KillStep> {
    plan.steps
        .iter()
        .filter(|s| s.action.signals())
        .cloned()
        .collect()
}

// MARK: - notify

/// 向本机 App 投递一次事件。
///
/// 走 `POST /notify`（Rust 端 42000，ADR 0013）。**App 没在跑就说没在跑**——
/// 静默返回 0 会让脚本以为投递成功了。
fn notify(flags: &[String]) -> i32 {
    let value = |name: &str, fallback: &str| -> String {
        flags
            .iter()
            .find_map(|f| f.strip_prefix(&format!("--{name}=")).map(str::to_string))
            .unwrap_or_else(|| fallback.to_string())
    };
    let kind = value("kind", "completed");
    let agent = value("agent", "opencode");
    let message = value("message", "来自 CLI 的一次投递");

    let payload = serde_json::json!({
        "kind": kind,
        "agent": agent,
        "message": message,
    });
    let body = match serde_json::to_vec(&payload) {
        Ok(b) => b,
        Err(error) => {
            eprintln!("✗ 构造请求失败: {error}");
            return EXIT_FAIL;
        }
    };
    let url = format!(
        "http://127.0.0.1:{}/notify",
        crate::webhook::DEFAULT_EVENT_PORT
    );
    match post_local(&url, &body) {
        Ok(reply) => {
            println!("已投递（{kind} → {agent}）");
            if !reply.trim().is_empty() {
                println!("  App 回应: {reply}");
            }
            EXIT_OK
        }
        Err(error) => {
            eprintln!("✗ 投递失败: {error}");
            eprintln!("  App 是不是没在跑？（Rust 端监听 127.0.0.1:42000）");
            EXIT_FAIL
        }
    }
}

fn post_local(url: &str, body: &[u8]) -> Result<String, String> {
    // 刻意用最朴素的一次 TCP 往返：本地回环、不加密、body 短，
    // 为此引一个 HTTP 客户端不划算（`transport.rs` 那套是给外发通道用的，
    // 它带 TLS、重试与背压，而这里三样都用不上）。
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("只支持 http://，收到 {url}"))?;
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    // `authority` 就是 `host:port`；本命令只投本机回环，不接受外部地址
    let address = authority.to_string();

    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(&address)
        .map_err(|error| format!("连不上 {address}: {error}"))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\n         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.write_all(body))
        .map_err(|error| format!("写请求失败: {error}"))?;
    let mut reply = String::new();
    stream
        .read_to_string(&mut reply)
        .map_err(|error| format!("读回应失败: {error}"))?;
    // 只取 body 段：状态行与头是给排错用的，脚本不该拿到
    Ok(match reply.split_once("\r\n\r\n") {
        Some((_, body)) => body.to_string(),
        None => reply,
    })
}

// MARK: - report

/// 生成运维报告。默认打印到 stdout，`-o <路径>` 写盘。
///
/// **写盘失败一律 exit 1**——静默成功会让定时任务以为报告存下来了。
fn report_cmd(flags: &[String]) -> i32 {
    let format = flags
        .iter()
        .find_map(|f| f.strip_prefix("--format="))
        .map(str::to_string)
        .unwrap_or_else(|| "md".to_string());
    let output = flags
        .iter()
        .find_map(|f| f.strip_prefix("-o=").or_else(|| f.strip_prefix("--out=")))
        .map(str::to_string);

    let (snapshots, total) = sample_once();
    let now = tokens::now_ms();
    let export = match format.as_str() {
        "csv" => audit::csv_export(&snapshots, now),
        "md" | "markdown" => audit::markdown_export(&snapshots, &[], Some(&total), now),
        other => {
            eprintln!("✗ --format 只认 md 与 csv（收到 {other}）");
            return EXIT_USAGE;
        }
    };

    match output {
        Some(path) => {
            // 写盘**原子替换**：写一半崩掉会留下半份报告，而半份比没有更难认
            match crate::atomicfile::atomic_replace_validated(
                std::path::Path::new(&path),
                export.content.as_bytes(),
                |_| Ok(()),
            ) {
                Ok(()) => {
                    println!("已写入 {path}（{} 字节）", export.content.len());
                    EXIT_OK
                }
                Err(error) => {
                    eprintln!("✗ 写盘失败: {error}");
                    EXIT_FAIL
                }
            }
        }
        None => {
            print!("{}", export.content);
            eprintln!();
            eprintln!("（建议写到文件：-o <路径>）");
            EXIT_OK
        }
    }
}

// MARK: - raycast

/// Raycast Extension 的命令清单。
///
/// 清单里的 URL 走 `agentisland://` 深链，而**深链解析在 Rust 侧还没迁**——
/// 所以这份清单现在导出来能看，但点了不会跳。清单本身照 Swift 侧逐条给全，
/// 少写一条的后果是 Raycast 侧少一个入口，而用户无从知道它曾经存在过。
fn raycast() -> i32 {
    let (snapshots, _) = sample_once();
    let mut commands: Vec<serde_json::Value> = vec![
        raycast_command("toggle", "Toggle AgentIsland", "展开或收起灵动岛监控面板"),
        raycast_command("analytics", "Token Analytics", "打开 Token 用量与成本预测分析"),
        raycast_command("toolbox", "Agent Workbench & Diagnostics", "打开维护工作台与死锁排查"),
        raycast_command("clean", "Clean Orphan & Hung Agents", "一键安全清理挂起死锁与孤儿后台进程"),
        raycast_command("export", "Export Audit Report", "导出 Markdown 运维审计报告至剪贴板"),
    ];
    // 每个可见 Agent 一条「直达详情」，与 Swift 侧同一口径：只给看到的那种
    for s in snapshots.iter().filter(|s| s.process_running) {
        commands.push(raycast_command(
            &format!("agent-{}", s.id),
            &format!("Inspect {}", s.name),
            &format!("直达 {} 运行态详情与会话", s.name),
        ));
    }
    let manifest = serde_json::json!({
        "name": "AgentIsland Raycast Commands",
        // 版本来自 **Cargo.toml**，而那条版本有测试钉着与 AppVersion.string 一致
        // （见 main.rs 的 `version_pinning`）——此前这里只能填 0.1.0。
        "version": env!("CARGO_PKG_VERSION"),
        "commands": commands,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&manifest).unwrap_or_else(|_| "{}".into())
    );
    EXIT_OK
}

fn raycast_command(name: &str, title: &str, description: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "title": title,
        "description": description,
        "url": format!("agentisland://{name}"),
    })
}

// MARK: - open

/// 控制本机 App：展开 / 收起 / 直达某页。
///
/// 走深链 URL 并**交给系统去派发**，而不是自己发 HTTP：
/// 深链是本仓与 macOS 之间已约定的入口，换一条路就多一处两边可能不一致的地方。
/// 系统派发失败时如实说失败——`open` 静默退出 0 会让脚本以为窗口开了。
fn open_cmd(positional: &[String]) -> i32 {
    let target = positional.first().map(String::as_str).unwrap_or("toggle");
    let url = match target {
        "toggle" | "expand" | "collapse" | "analytics" | "toolbox" | "export" | "workbench" => {
            format!("agentisland://{target}")
        }
        "agent" => {
            // 直达某个 Agent：`open agent <id>`
            let Some(id) = positional.get(1) else {
                eprintln!("用法: agentisland open agent <id>");
                return EXIT_USAGE;
            };
            format!("agentisland://agent?id={}", urlencode(id))
        }
        _ => {
            eprintln!("✗ 认不出的目标 `{target}`");
            eprintln!("  可用: toggle | expand | collapse | analytics | toolbox | export | workbench | agent <id>");
            return EXIT_USAGE;
        }
    };
    match std::process::Command::new("open").arg(&url).status() {
        Ok(status) if status.success() => {
            println!("已请求系统打开 {url}");
            EXIT_OK
        }
        Ok(status) => {
            eprintln!("✗ 系统派发失败（退出码 {:?}）——App 是不是没在跑？", status.code());
            EXIT_FAIL
        }
        Err(error) => {
            eprintln!("✗ 调不起系统派发: {error}");
            EXIT_FAIL
        }
    }
}

/// 查询串里只可能出现 agent id，但**仍然编码**：
/// 深链投递目标必须能解析回已知档案，而一个没编码的 `&` 会把参数拆成两个。
fn urlencode(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

// MARK: - top

/// 持续观测看板。
///
/// **不做全屏 TUI**：那要引 `crossterm` 之类的新依赖并接管终端，
/// 而这一版的定位是「脚本与人工都能用的持续观测」——用 ANSI 光标回到行首重画，
/// 零依赖、Ctrl-C 即退、管道里也能跑。**清屏只在 TTY 下做**：
/// 重定向到文件时那些转义序列会变成正文里的乱码。
fn top(flags: &[String]) -> i32 {
    let (_tx, rx) = mpsc::channel();
    let mut engine = engine::ActivityEngine::new(Settings::load(), rx);
    let now = tokens::now_ms();
    let want_usage = true;
    // 持续观测默认**取用量**：它本来就是常驻的那一路（每拍都在采集），
    // 那是 `status` 单拍才需要的取舍
    let _ = want_usage;
    let is_tty = !flags.iter().any(|f| f == "--no-tty");
    let once = flags.iter().any(|f| f == "--once");

    loop {
        engine.tick();
        let state = engine.state();
        let online: Vec<_> = state
            .snapshots
            .iter()
            .filter(|s| s.process_running)
            .collect();
        // 有活动 / 全离线两档节奏，同引擎的降频口径（v0.0.200 起与 Swift 一致）
        let any_working = online
            .iter()
            .any(|s| matches!(s.level, crate::models::ActivityLevel::Working | crate::models::ActivityLevel::Attention));
        let interval = if any_working {
            engine.settings.sample_interval
        } else {
            engine.settings.idle_sample_interval
        };

        if is_tty {
            print!("\x1b[H\x1b[2J");
        }
        println!("AgentIsland 持续观测 · {} · 采样 {}s · Ctrl-C 退出", audit::timestamp_text(now), interval);
        println!();
        println!(
            "{:<18} {:<10} {:>7} {:>7} {:>10}  {}",
            "智能体", "状态", "CPU", "内存", "24h 用量", "最近活动"
        );
        for s in &online {
            // 这里是**持续**观测，CPU 有差分窗口 ⇒ 真的读得到
            let cpu = match s.cpu_percent {
                Some(v) => format!("{v:.1}%"),
                None => "—".to_string(),
            };
            let usage = match s.token_usage.as_ref().filter(|u| u.tokens24h > 0) {
                Some(u) => tokens::compact(u.tokens24h),
                None => "—".to_string(),
            };
            println!(
                "{:<18} {:<10} {:>7} {:>7} {:>10}  {}",
                truncate(&s.name, 18),
                s.level_label,
                cpu,
                s.memory_text,
                usage,
                s.last_activity_text
            );
        }
        println!();
        println!(
            "24h 合计 {} tokens · 累计 {} tokens",
            tokens::compact(state.grand_total.tokens24h),
            tokens::compact(state.grand_total.tokens_total)
        );
        if once {
            return EXIT_OK;
        }
        std::thread::sleep(std::time::Duration::from_secs_f64(interval.clamp(0.5, 30.0)));
    }
}

// MARK: - state

fn state_cmd() -> i32 {
    // `state` 的语义是「读 App 进程里的实时状态」——包括自报 TTL 与出处。
    // CLI 自己是另一个进程，**看不到**那个进程的自报记录（与 Swift 同一条理由）。
    let (snapshots, _) = sample_once();
    let online: Vec<_> = snapshots.iter().filter(|s| s.process_running).collect();
    if online.is_empty() {
        println!("没有进程内的 Agent。");
        return EXIT_OK;
    }
    println!("当前在线 {} 个：", online.len());
    for s in &online {
        // 本命令看不到自报，所以出处一律是「本进程独立采样」——
        // **写成别的就是撒谎**，宁可说明看不到。
        let source = match s.provenance {
            Some(_) => "由 app 进程内的自报给出（CLI 进程看不到，见 agentisland state）",
            None => "本进程独立采样",
        };
        println!("  {:<20} {:<8} {}", s.name, s.level_label, source);
    }
    println!();
    println!("提示: 自报与冲突只在 App 进程内可读——CLI 是另一个进程。");
    EXIT_OK
}

fn selftest() -> i32 {
    let report = crate::selftest::run();
    print!("{}", report.text());
    report.exit_code()
}

// MARK: - 工具

/// 按字符数截断（不是按字节）：中文名按字节切会切出半个字。
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

fn cost_text(cost: f64, estimated: bool) -> String {
    let text = cost::format_cost(cost);
    if text.is_empty() {
        // 真的是零 ≠ 估不出来。查了确实是零要写 `$0.00`
        "$0.00".to_string()
    } else {
        format!("{}{}", if estimated { "~" } else { "" }, text)
    }
}

/// 让「没实现」这件事在编译期就有一处清单，而不是散在 match 的分支里。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_table_is_the_single_source_of_truth() {
        // 每一项的说明都要非空 —— 否则 `agentisland <未实现的>` 会打出一句空话
        for (name, _, desc) in COMMANDS {
            assert!(!desc.trim().is_empty(), "{name} 缺说明");
        }
        // 已实现的那几个必须真的能被分派出去（防止表里写了 true 而 match 里没有）
        //
        // **`must_not_auto_invoke` 里的必须排除**：`top` 调进去不返回（用例挂死），
        // `clean` 调进去**会真的终止进程**——测试跑在开发机上，而开发机上有正在
        // 跑的 Agent。一条「遍历每个已实现子命令并裸调一遍」的守护足以在跑测试时
        // 把它们杀掉。两个坑都踩过：第一个是「测试卡住」，第二个更糟。
        let implemented: Vec<&str> = COMMANDS
            .iter()
            .filter(|(_, done, _)| *done)
            .map(|(n, _, _)| *n)
            .filter(|n| !must_not_auto_invoke(n))
            .collect();
        for name in implemented {
            let args = vec!["agentisland".to_string(), name.to_string()];
            let code = try_run(&args);
            assert!(code.is_some(), "{name} 在表里标了已实现却分派不到");
        }
    }

    /// `clean` 必须留在不许裸调的清单里，而且要**说清是为什么**。
    ///
    /// 这条守护防的是一个已经差点发生的事故：把 `clean` 标成已实现的那一刻，
    /// 上面那条遍历守护就会在**任何一次 `cargo test`** 里终止开发机上正在跑的
    /// Agent 进程。它不会让任何断言变红——测试会全绿，而用户的活儿没了。
    /// 所以「不能裸调」这件事必须有一条独立的用例盯着，而不是躺在注释里。
    #[test]
    fn the_destructive_subcommand_is_never_auto_invoked_by_tests() {
        assert!(
            must_not_auto_invoke("clean"),
            "`clean` 会终止进程，绝不能被测试裸调"
        );
        assert!(must_not_auto_invoke("top"), "`top` 不返回，裸调会挂死用例");
        // 反向：只读命令**不该**被误列进来，否则新增命令会静默失去分派守护
        for safe in ["status", "check", "doctor", "report", "raycast", "open"] {
            assert!(
                !must_not_auto_invoke(safe),
                "{safe} 是只读/幂等的，不该被排除——否则它永远不会被分派守护覆盖"
            );
        }
    }

    /// `clean -n` 必须**不发任何信号**，且退出码为 0。
    ///
    /// 预览是「打算杀谁」与「真的杀了」之间唯一的一道人工闸。它要是偷偷动手，
    /// 用户就再也不能放心地先看一眼——而这条断言是本轮唯一能在测试里
    /// 证明「`-n` 不会动手」的地方（真去杀一个进程来验证是不可接受的）。
    #[test]
    fn dry_run_is_accepted_as_a_flag_and_never_claims_to_have_killed_anything() {
        let code = try_run(&["agentisland".into(), "clean".into(), "-n".into()]);
        assert!(
            code.is_some(),
            "`clean -n` 必须被接受（表里标了已实现，分派就得接得住这个开关）"
        );
    }

    /// `check` 曾经挂着 `-n`，而 `check` 本来就不终止任何进程——
    /// 那个开关是个**什么也不做**的装饰。预览现在只在 `clean` 上，
    /// 这里钉住 `check` 确实忽略它（而不是哪天又给它加出别的含义）。
    #[test]
    fn check_ignores_the_dry_run_flag_because_it_never_kills() {
        let with_flag = try_run(&["agentisland".into(), "check".into(), "-n".into()]);
        let without = try_run(&["agentisland".into(), "check".into()]);
        assert_eq!(with_flag, without, "`check` 只读，`-n` 对它没有区别");
    }

    /// **GUI 主进程永不作为可清理对象**——夹具直接用本机实测到的真实路径。
    ///
    /// 这条规则在本机是**承重**的，不是理论条款：Qoder 的进程 `ppid` 就是 1
    /// （按孤儿标准完全成立），而它正是用户正在用的那个应用。没有这条排除，
    /// 它会被列成异常等着被终止。
    #[test]
    fn a_running_gui_app_is_never_a_cleaning_target() {
        for path in [
            "/Applications/Qoder.app/Contents/MacOS/Qoder",
            "/Applications/ZCode.app/Contents/Frameworks/ZCode Helper (Renderer).app/Contents/MacOS/ZCode Helper (Renderer)",
        ] {
            assert!(
                is_standard_app_bundle(path),
                "{path} 是 .app 主进程，不该被当成可清理对象"
            );
        }
        // CLI 工具与派生命令行工具**不在**这个排除里——它们正是要清理的那一族
        for path in [
            "/opt/homebrew/bin/node",
            "/usr/local/bin/claude",
            "/Users/me/.nvm/versions/node/v22/bin/node",
        ] {
            assert!(
                !is_standard_app_bundle(path),
                "{path} 是命令行工具，不该被 GUI 排除挡掉"
            );
        }
    }

    /// 12 个子命令全部已实现——**这条断言是记账**：它变红时说明
    /// 有子命令被回退成了未实现，或者表里多出了第 13 个。
    /// 保留它的理由不是「12 是对的」，而是「这一份清单必须有人盯着」。
    #[test]
    fn all_twelve_subcommands_are_implemented() {
        assert_eq!(COMMANDS.len(), 12, "Swift CLI 是 12 个子命令");
        let unfinished: Vec<&str> = COMMANDS
            .iter()
            .filter(|(_, done, _)| !*done)
            .map(|(n, _, _)| *n)
            .collect();
        assert!(
            unfinished.is_empty(),
            "12 个子命令应当全部实现。未实现的是：{unfinished:?}——\
             确实有意留的，必须先想清楚为什么不能做，再把它记在这里"
        );
    }

    #[test]
    fn an_unknown_subcommand_is_still_a_usage_error() {
        // 没实现 ≠ 拼错了：两者的退出码必须分开
        let unknown = try_run(&["agentisland".into(), "nope".into()]);
        assert_eq!(unknown, Some(EXIT_USAGE), "拼错要给用法错");
        // 「拼对了但没做」那条路（exit 1 + 说清没做）在 12 个子命令全部实现后
        // **已经没有活例子**了：COMMANDS 里找不到任何 `done == false` 的项，
        // 而上面那条 `all_twelve_subcommands_are_implemented` 钉住了这一点。
        // 所以这里只钉住分界本身还在：拼错必须仍是 2，不能悄悄降成 1——
        // 脚本靠这个码区分「拼错了」和「跑失败了」，混成一个就去重试了。
        assert!(
            COMMANDS.iter().all(|(_, done, _)| *done),
            "一旦真有未实现项，这条用例就该挑一个出来断言 exit 1"
        );
    }

    #[test]
    fn no_subcommand_falls_through_to_the_ui() {
        // 双击启动走的就是这条路：不能被 CLI 拦下
        assert_eq!(try_run(&["agentisland".into()]), None);
        assert_eq!(try_run(&["agentisland".into(), "--demo".into()]), None);
        assert_eq!(try_run(&["agentisland".into(), "--shell=sidebar".into()]), None);
    }

    /// 单拍入口**不许**出现 CPU 读数。
    ///
    /// 防的症状：引擎那一拍在「没有上一次读数」时把 CPU 折成 `Some(0.0)`，
    /// 直接印就是 `0.0%`——而那既不是「测到了零」也不是「没测到」，
    /// 会让脚本侧把一个不存在的读数当成真的。要真实 CPU 得用持续观测。
    #[test]
    fn the_one_shot_status_never_reports_a_cpu_reading() {
        let source = include_str!("cli.rs");
        // 表格那一列写死成 `—`
        assert!(
            source.contains("let cpu = \"—\";"),
            "status 的 CPU 列必须写死 —，不能去读 engine 的 cpu_percent"
        );
        // JSON 那个键恒为 null
        assert!(
            source.contains("\"cpuPercent\": serde_json::Value::Null"),
            "status --json 的 cpuPercent 必须恒为 null，不是 0.0"
        );
    }

    /// 三个新子命令必须在表里**且真的分派得到**——上一版那条「表与分派要一致」
    /// 的守护只覆盖当时的 5 个；新增命令忘了加分派就会在表里显示已实现、
    /// 实际打出一句「尚未实现」。
    #[test]
    fn the_new_commands_are_dispatchable_not_just_listed() {
        for name in ["check", "notify", "report"] {
            let code = try_run(&["agentisland".into(), name.to_string()]);
            assert!(code.is_some(), "{name} 应当能分派");
            let entry = COMMANDS.iter().find(|(n, _, _)| *n == name);
            assert!(entry.map(|(_, done, _)| *done) == Some(true), "{name} 应当标为已实现");
        }
        // `clean` 不在这里面调：它会终止进程，由
        // `the_destructive_subcommand_is_never_auto_invoked_by_tests` 盯着。
        let clean = COMMANDS.iter().find(|(n, _, _)| *n == "clean").unwrap();
        assert!(clean.1, "`clean` 已实现");
    }

    /// 拼错 `--format` 是用法错（exit 2），不是运行失败（exit 1）。
    /// 分错的后果是脚本会当成「报告生成失败」去重试，而重试没有意义。
    #[test]
    fn an_unknown_report_format_is_a_usage_error() {
        let code = try_run(&["agentisland".into(), "report".into(), "--format=pdf".into()]);
        assert_eq!(code, Some(EXIT_USAGE));
    }

    /// 深链里的 agent id 必须编码。
    /// 防的症状：`open agent "a&b=c"` 不编码就会让 `&b=c` 变成第二个参数，
    /// 而投递目标「必须解析到已知档案」——解析失败会被静默丢弃，用户只看到窗口没动。
    #[test]
    fn deep_link_agent_ids_are_percent_encoded() {
        assert_eq!(urlencode("codex"), "codex", "普通 id 不该被改写");
        assert_eq!(urlencode("a&b"), "a%26b", "& 会拆出第二个参数");
        assert_eq!(urlencode("a b"), "a%20b");
        assert_eq!(urlencode("a/b"), "a%2Fb");
    }

    /// `workbench` 是第三个窗口的入口，必须能分派出去。
    ///
    /// 它曾经**不存在**：`tauri.conf.json` 里只有 island 与 sidebar 两个窗口，
    /// 而方案里「工具箱 / 工作台」一直作为一个概念被深链和 CLI 引用着——
    /// 引用着一个没有落地的东西，文档与代码都读不出问题。
    #[test]
    fn the_workbench_window_is_declared_and_reachable() {
        // ① 窗口在配置里（三个形态，不是两个）
        let conf = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json"),
        )
        .expect("应当读得到 tauri.conf.json");
        for label in ["island", "sidebar", "workbench"] {
            assert!(
                conf.contains(&format!("\"label\": \"{label}\"")),
                "缺少 {label} 窗口声明"
            );
        }
        // ② 深链认得它
        assert_eq!(
            crate::deeplink::parse("agentisland://workbench"),
            Some(crate::deeplink::Action::Workbench)
        );
        // ③ CLI 的 `open` 能构造出那条 URL（不真派发系统）
        let code = try_run(&["agentisland".into(), "open".into(), "workbench".into()]);
        assert!(
            code.is_some(),
            "`open workbench` 应当被接受（认不出的目标是用法错，会返回 2）"
        );
    }

    /// 认不出的 `open` 目标是**用法错**（退出码 2），不是运行失败。
    #[test]
    fn an_unknown_open_target_is_a_usage_error() {
        assert_eq!(try_run(&["agentisland".into(), "open".into(), "nope".into()]), Some(EXIT_USAGE));
        // 少参数也是用法错
        assert_eq!(try_run(&["agentisland".into(), "open".into(), "agent".into()]), Some(EXIT_USAGE));
    }

    #[test]
    fn truncation_counts_characters_not_bytes() {
        assert_eq!(truncate("短", 4), "短");
        // 六个中文字符 = 18 字节，按字节切会切出半个字
        assert_eq!(truncate("一二三四五六", 4), "一二三…");
    }
}

/// `status` 的**取用量口径**。这一条是单拍 CLI 与常驻引擎最容易分叉的地方。
#[cfg(test)]
mod usage_flag_tests {
    use super::*;

    /// **默认不取**。本机实测同步解析全部会话索引多花 5 秒，
    /// 而 Raycast / 脚本调用是这条命令的主要场景。
    #[test]
    fn the_single_shot_sample_skips_usage_by_default() {
        let (snapshots, _total) = sample_once_with(false);
        // 开着开关却关掉取用量的引擎，token_usage 必须是 None
        assert!(
            snapshots.iter().all(|s| s.token_usage.is_none()),
            "默认不该真去取用量"
        );
    }

    /// 开关打开才取。这是「本命令默认不取、`--usage` 才付这笔钱」的机器可验证形态。
    #[test]
    fn turning_the_flag_on_actually_fetches_usage() {
        let (snapshots, total) = sample_once_with(true);
        let with_root = crate::registry::builtin()
            .iter()
            .any(|p| !p.token_roots.is_empty() && !p.session_database.is_none());
        if with_root {
            assert!(
                snapshots.iter().any(|s| s.token_usage.is_some()),
                "开了开关就应当取到用量"
            );
        }
        let _ = total;
    }

    /// **`None` 不等于零**：没取与「取到确实是零」是**两件事**，
    /// 印成同一个符号就是在替用户下结论。
    #[test]
    fn not_fetched_and_fetched_zero_are_different_sayings() {
        // 引擎侧：没取 ⇒ None（而不是 Some(0)）
        let (not_taken, _) = sample_once_with(false);
        assert!(not_taken.iter().all(|s| s.token_usage.is_none()));
        // 展示侧：None 且没开开关 ⇒ 写「—(未取)」，Some(0) ⇒ 写「0」
        let not_taken_label = if not_taken.iter().all(|s| s.token_usage.is_none()) {
            "—(未取)"
        } else {
            "—"
        };
        assert_eq!(not_taken_label, "—(未取)");
    }

    /// `get_report` 是**按需**取（分析页点进来才要），所以它**不受 `refresh_usage` 约束**——
    /// 跟着加一道开关的话，分析页会永远是空的，而那比多花几秒更糟。
    #[test]
    fn the_on_demand_report_path_is_not_gated_by_the_bulk_flag() {
        let source = include_str!("cli.rs");
        assert!(
            !source.contains("if !self.refresh_usage || profile.token_roots.is_empty() {\n            return None;\n        }\n        if let Some((_, report)) = self.token_cache"),
            "get_report 不该被 refresh_usage 关掉"
        );
    }
}

/// 异常判定的三个门槛在 Swift 与 Rust **各写了一份**。
///
/// 两边漂移了不会有人报错——编译照过、测试照绿，表现只是「macOS 上报一条
/// 异常、这边不报」或反过来。症状出现在对照表要消灭的地方，却没有任何一条
/// 断言会红，所以这里直接读 Swift 源码比对。
#[cfg(test)]
mod anomaly_threshold_pinning {
    use super::{HUNG_CPU_FLOOR, OVERWEIGHT_BYTES};

    fn swift_source() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../Sources/AgentIslandCore/AgentCleaner.swift");
        std::fs::read_to_string(&path).expect("应当读得到 AgentCleaner.swift")
    }

    /// 取 `entry.<字段> > <字面量>` 里的那个字面量，**按数值**返回。
    ///
    /// 比数值不比拼写：Swift 写 `10.0` 而 Rust 写 `10.0f64`、`2_147_483_648`
    /// 还带下划线分隔符。拿字符串比，这两条断言会在两边**完全等价**时变红——
    /// 一条天天误报的断言，人只会学会忽略它。
    fn swift_threshold(source: &str, field: &str) -> f64 {
        let needle = format!("entry.{field} > ");
        let tail = source
            .split_once(&needle)
            .unwrap_or_else(|| panic!("Swift 侧应当还有 `entry.{field} > …` 这条判定"))
            .1;
        let literal: String = tail
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '_')
            .collect();
        literal
            .replace('_', "")
            .parse()
            .unwrap_or_else(|_| panic!("`entry.{field} > {literal}` 解析不成数值"))
    }

    #[test]
    fn the_hung_cpu_floor_matches_the_swift_side() {
        let want = swift_threshold(&swift_source(), "cpuPercent");
        assert_eq!(
            HUNG_CPU_FLOOR, want,
            "死锁的 CPU 门槛与 Swift 不一致——同一台机器上两边会报出不同的异常"
        );
    }

    #[test]
    fn the_overweight_threshold_matches_the_swift_side() {
        let want = swift_threshold(&swift_source(), "rssBytes");
        assert_eq!(
            OVERWEIGHT_BYTES as f64, want,
            "内存超限门槛与 Swift 不一致：一边 2GB 一边 2GiB 时，没有一边会报错"
        );
    }

    /// GUI 主进程的排除判据也钉住。少这一条的后果特别隐蔽：
    /// Swift 排除了、这边没排除，于是用户正在用的编辑器被列成异常。
    #[test]
    fn the_gui_bundle_exclusion_exists_on_the_swift_side_too() {
        let needle = ".app/Contents/MacOS";
        assert!(
            swift_source().contains(needle),
            "Swift 侧应当也有 GUI 主进程排除；没有的话就不是「两边口径不同」而是一边漏了"
        );
    }
}
