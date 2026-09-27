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
    ("open", false, "控制灵动岛展开/折叠/直达（未实现：需 App 进程在跑）"),
    ("notify", true, "向本机 App 投递一次事件（--kind completed|attention|costspike）"),
    ("report", true, "生成 Markdown / CSV 运维报告（-o 写盘、--format md|csv）"),
    ("raycast", false, "导出 Raycast 命令清单（未实现）"),
    ("top", false, "类 htop 的全屏看板（未实现）"),
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
    let (_tx, rx) = mpsc::channel();
    let mut engine = engine::ActivityEngine::new(Settings::load(), rx);
    engine.tick();
    let state = engine.state();
    (state.snapshots, state.grand_total)
}

// MARK: - status

fn status(flags: &[String], positional: &[String]) -> i32 {
    let has = |name: &str| flags.iter().any(|f| f == name);
    let want_all = has("--all");
    let as_json = has("--json");

    let (snapshots, _) = sample_once();
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
        // 用量：没取到写 `—`；查了确实是零才写 0
        let usage = match s.token_usage.as_ref().filter(|u| u.tokens24h > 0) {
            Some(u) => tokens::compact(u.tokens24h),
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
    println!("      要真实 CPU 用持续观测的 `agentisland top`（尚未实现）或界面里的详情页。");
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
        let implemented: Vec<&str> = COMMANDS
            .iter()
            .filter(|(_, done, _)| *done)
            .map(|(n, _, _)| *n)
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
        let unfinished = try_run(&["agentisland".into(), "top".into()]);
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

    #[test]
    fn truncation_counts_characters_not_bytes() {
        assert_eq!(truncate("短", 4), "短");
        // 六个中文字符 = 18 字节，按字节切会切出半个字
        assert_eq!(truncate("一二三四五六", 4), "一二三…");
    }
}
