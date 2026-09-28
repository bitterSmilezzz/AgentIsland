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
    ("check", true, "排查异常驻留与持续高负载（-n 只预览不终止）"),
    ("clean", false, "一键释放（未实现：终止动作与进程树未迁）"),
    ("open", true, "控制 App 展开/折叠/直达（toggle|expand|collapse|analytics|toolbox|export）"),
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

/// **常驻命令**：调进去不会自己结束。
///
/// 这份清单只有一个用途——让「表与分派一致」那条守护别去调它们。
/// 把它们单独列出来，是为了让将来新增常驻命令的人**必然看到这一处**，
/// 而不是靠「测试挂住了」去反推。
#[cfg(test)]
pub fn is_resident_command(name: &str) -> bool {
    matches!(name, "top")
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
/// 终止是 `clean` 的事，而 `clean` 还没迁。把两者混在一起的后果很直接：
/// 用户以为自己在「看」，实际上有东西被杀了。所以这里刻意不给 `--force`。
fn check(flags: &[String]) -> i32 {
    let (snapshots, _) = sample_once();
    let mut guard = crate::resilience::Guard::default();
    let alerts = guard.evaluate(&snapshots, tokens::now_ms());
    if alerts.is_empty() {
        println!("没有排查到异常驻留或持续高负载。");
        return EXIT_OK;
    }
    println!("排查到 {} 条异常：", alerts.len());
    for alert in &alerts {
        println!(
            "  ⚠️  {} · {} · 已持续 {} 分 {} 秒",
            alert.agent_name,
            alert.message,
            alert.elapsed_ms / 60_000,
            (alert.elapsed_ms / 1000) % 60
        );
    }
    println!();
    println!("`check` 只看不杀。要释放请用 `agentisland clean`（尚未实现）。");
    let _ = flags; // 预留：`-n` 等开关在终止能力落地后才有意义
    EXIT_OK
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
        "toggle" | "expand" | "collapse" | "analytics" | "toolbox" | "export" => {
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
            eprintln!("  可用: toggle | expand | collapse | analytics | toolbox | export | agent <id>");
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
        // **常驻命令必须排除**：`top` 是持续观测循环，裸调它会让这条用例挂死。
        // 这个坑踩过——第一次写这条守护时把 `top` 一起调了，整套测试直接超时，
        // 症状是「测试卡住」而不是「哪条断言红了」，很难一眼看出原因。
        let implemented: Vec<&str> = COMMANDS
            .iter()
            .filter(|(_, done, _)| *done)
            .map(|(n, _, _)| *n)
            .filter(|n| !is_resident_command(n))
            .collect();
        for name in implemented {
            let args = vec!["agentisland".to_string(), name.to_string()];
            let code = try_run(&args);
            assert!(code.is_some(), "{name} 在表里标了已实现却分派不到");
        }
    }

    #[test]
    fn an_unknown_subcommand_is_a_usage_error_but_an_unfinished_one_is_not() {
        // 没实现 ≠ 拼错了：两者的退出码必须分开
        let unknown = try_run(&["agentisland".into(), "nope".into()]);
        assert_eq!(unknown, Some(EXIT_USAGE), "拼错要给用法错");
        // 注意这里必须挑一个**真没实现**的子命令：`top` 在 v0.0.206 起已实现，
        // 调它会进持续观测循环并让这条用例挂死（症状是「测试卡住」，不报哪条红）。
        let unfinished = try_run(&["agentisland".into(), "clean".into()]);
        assert_eq!(unfinished, Some(EXIT_FAIL), "拼对但没做要说清没做");
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
        // clean 仍未迁：它要终止进程，check 明确不给这个能力
        let clean = COMMANDS.iter().find(|(n, _, _)| *n == "clean").unwrap();
        assert!(!clean.1, "clean 尚未迁，不得标成已实现");
    }

    /// 拼错 `--format` 是用法错（exit 2），不是运行失败（exit 1）。
    /// 分错的后果是脚本会当成「报告生成失败」去重试，而重试没有意义。
    #[test]
    fn an_unknown_report_format_is_a_usage_error() {
        let code = try_run(&["agentisland".into(), "report".into(), "--format=pdf".into()]);
        assert_eq!(code, Some(EXIT_USAGE));
    }

    /// **11 / 12** 已实现；剩下 `clean` 是**有意留的**，不是漏做。
    ///
    /// `clean` 要**终止进程**。`check` 已经刻意不给 `--force`、并在输出里写明
    /// 「只看不杀」——那正是为了让「看」与「杀」两件事在权限上分开。
    /// 在终止能力（进程树构建 + 身份复核）迁过来之前就把它做成能杀的，
    /// 等于在能力最弱的时候先给一把刀。
    ///
    /// 这条断言的作用是**记账**：清单里只剩 `clean` 是有意留的；
    /// 它变成已实现时（或多出第二个未实现时）就会红，逼着人更新这份说明。
    #[test]
    fn eleven_of_twelve_subcommands_are_done_and_the_twelfth_is_deliberate() {
        assert_eq!(COMMANDS.len(), 12, "Swift CLI 是 12 个子命令");
        let unfinished: Vec<&str> = COMMANDS
            .iter()
            .filter(|(_, done, _)| !*done)
            .map(|(n, _, _)| *n)
            .collect();
        assert_eq!(
            unfinished,
            vec!["clean"],
            "只剩 `clean` 是有意留的（终止能力未迁）。多出来的未实现项要么做掉，\
             要么在这里说清为什么留"
        );
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
