//! 无头自检（Swift `Selftest`）：在进程内跑一遍核心判定的断言，零 UI。
//!
//! 与单元测试的**分工**：单元测试跑在开发机上、证明「代码按写的做」；
//! 自检跑在**用户的机器上**、回答「这台机器上这套判定还成立吗」——
//! 所以它只用合成输入（不碰用户的会话目录），唯一碰文件系统的那条也只写自己的临时目录。
//! 这也是为什么它必须随时能在任何一台机器上全绿：有一条红就说明建出来的包有问题，
//! 而不是「这台机器刚好有什么」。
//!
//! 每条检查都给一句**能读懂的话**（`30s 前` 具体等于什么、`—` 与 `刚刚` 的区别），
//! 而不是「check #7 failed」——自检的输出就是给人看的诊断。

use crate::engine::ActivityEngine;
use crate::filemon::{time_ago_text, FileActivityResult};
use crate::models::{ActivityLevel, AgentProfile};
use crate::session::{self, Signal};
use crate::settings::Settings;

/// 一条检查的结果
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    pub passed: bool,
    /// 失败时给出实际值（成功时为 None，避免噪声）
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub passed: usize,
    pub failed: usize,
    pub checks: Vec<Check>,
}

impl Report {
    /// 进程退出码：全绿 0、有失败 1（Swift `Selftest.run` 同约定）
    pub fn exit_code(&self) -> i32 {
        if self.failed == 0 {
            0
        } else {
            1
        }
    }

    /// 给人看的多行输出（CLI/日志/界面都用它）
    pub fn text(&self) -> String {
        let mut out = String::from("AgentIsland selftest — 核心逻辑断言\n");
        for check in &self.checks {
            if check.passed {
                out.push_str(&format!("  ✅ {}\n", check.name));
            } else {
                out.push_str(&format!(
                    "  ❌ {}{}\n",
                    check.name,
                    check
                        .detail
                        .as_deref()
                        .map(|d| format!("（实际：{d}）"))
                        .unwrap_or_default()
                ));
            }
        }
        out.push_str(if self.failed == 0 {
            "✅ 全部通过\n"
        } else {
            "❌ 有检查未通过\n"
        });
        out
    }
}

struct Runner {
    checks: Vec<Check>,
}

impl Runner {
    fn new() -> Self {
        Runner { checks: Vec::new() }
    }

    fn check(&mut self, name: impl Into<String>, condition: bool, actual: Option<String>) {
        let passed = condition;
        self.checks.push(Check {
            name: name.into(),
            passed,
            detail: if passed { None } else { actual },
        });
    }

    fn finish(self) -> Report {
        let passed = self.checks.iter().filter(|c| c.passed).count();
        Report {
            passed,
            failed: self.checks.len() - passed,
            checks: self.checks,
        }
    }
}

/// 合成一台「引擎」：不碰真实系统，只驱动判定那一层
struct Harness {
    engine: ActivityEngine,
    profile: AgentProfile,
}

impl Harness {
    fn new(profile_id: &str) -> Option<Self> {
        let profile = crate::registry::builtin().into_iter().find(|p| p.id == profile_id)?;
        let (_tx, rx) = std::sync::mpsc::channel();
        Some(Harness {
            engine: ActivityEngine::new(Settings::default(), rx),
            profile,
        })
    }

    fn decide(
        &mut self,
        now: i64,
        running: bool,
        cpu: Option<f64>,
        signal: Option<Signal>,
        latest_write: Option<std::time::SystemTime>,
    ) -> ActivityLevel {
        self.engine.decide_level(
            &self.profile,
            now,
            running,
            cpu,
            self.engine.settings.cpu_threshold,
            60.0,
            10.0,
            &FileActivityResult {
                latest_write,
                latest_file: None,
            },
            &session::SessionProbe {
                signal,
                subagent_count: 0,
            },
            None,
        )
    }
}

fn write_at(now: i64, ago_ms: i64) -> Option<std::time::SystemTime> {
    let millis = now - ago_ms;
    if millis <= 0 {
        return None;
    }
    Some(std::time::UNIX_EPOCH + std::time::Duration::from_millis(millis as u64))
}

/// 跑完全部检查。**只读用户数据之外的东西**：唯一碰文件系统的那条写自己的临时目录并删掉。
pub fn run() -> Report {
    let mut runner = Runner::new();
    let now = crate::tokens::now_ms();

    // 1. 进程不在 ⇒ offline
    if let Some(mut harness) = Harness::new("dim") {
        let level = harness.decide(now, false, None, None, None);
        runner.check(
            "无进程时判为 offline",
            level == ActivityLevel::Offline,
            Some(format!("{level:?}")),
        );
    } else {
        runner.check("内置档案 dim 存在", false, Some("注册表里找不到 dim".into()));
    }

    // 2. 进程在 + 窗口内有写入 ⇒ working
    if let Some(mut harness) = Harness::new("dim") {
        let level = harness.decide(now, true, Some(0.0), None, write_at(now, 5_000));
        runner.check(
            "进程在且 5 秒内有写入 ⇒ working",
            level == ActivityLevel::Working,
            Some(format!("{level:?}")),
        );

        // 3. 进程在 + 超窗口无写入 + CPU 0 ⇒ idle（两个信号都不命中）
        let level = harness.decide(now, true, Some(0.0), None, write_at(now, 300_000));
        runner.check(
            "进程在、超窗口无写入、CPU 0 ⇒ idle",
            level == ActivityLevel::Idle,
            Some(format!("{level:?}")),
        );

        // 4. 进程在 + CPU 高但无写入 ⇒ working（信号之二）
        let level = harness.decide(now, true, Some(25.0), None, write_at(now, 300_000));
        runner.check(
            "CPU 高而无写入 ⇒ working（靠 CPU 那一路）",
            level == ActivityLevel::Working,
            Some(format!("{level:?}")),
        );

        // 5. 新写入出现 ⇒ 从 idle 升级为 working（同一台引擎连续两拍）
        let _ = harness.decide(now, true, Some(0.0), None, write_at(now, 300_000));
        let level = harness.decide(now + 2_000, true, Some(0.0), None, Some(std::time::SystemTime::now()));
        runner.check(
            "新写入出现后从 idle 升级为 working",
            level == ActivityLevel::Working,
            Some(format!("{level:?}")),
        );
    }

    // 6. 「最近活动」文案（口径与 Swift `formatAgo` 逐字相同）
    for (seconds, expected) in [
        (2.0, "刚刚"),
        (30.0, "30s 前"),
        (120.0, "2m 前"),
        (7_200.0, "2h 前"),
    ] {
        let got = time_ago_text(Some(seconds));
        runner.check(
            format!("活动文案 {seconds:.0}s ⇒ {expected}"),
            got == expected,
            Some(got),
        );
    }
    // 「没有活动信号」与「刚刚活动过」是两件事
    let none_text = time_ago_text(None);
    runner.check("没有活动信号 ⇒ —（不是「刚刚」）", none_text == "—", Some(none_text));

    // 7. 注册表完整性
    let profiles = crate::registry::builtin();
    let ids: Vec<&str> = profiles.iter().map(|p| p.id.as_str()).collect();
    let unique = {
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        sorted.len() == ids.len()
    };
    runner.check(
        "内置档案 id 唯一",
        unique,
        Some(format!("{} 个 id、{} 个唯一", ids.len(), {
            let mut s = ids.clone();
            s.sort_unstable();
            s.dedup();
            s.len()
        })),
    );
    runner.check("注册表含 dim", ids.contains(&"dim"), Some(format!("{ids:?}")));
    runner.check(
        "内置档案数量 ≥ 12",
        profiles.len() >= 12,
        Some(format!("{}", profiles.len())),
    );

    // 8. 进程匹配：命中与**不误报**都要验（排除规则以前只能靠真机碰运气）
    let cursor = profiles.iter().find(|p| p.id == "cursor").cloned();
    let dim = profiles.iter().find(|p| p.id == "dim").cloned();
    if let (Some(cursor), Some(dim)) = (cursor, dim) {
        let service = "/System/Library/PrivateFrameworks/TextInputUIMacHelper.framework/\
                       Versions/A/XPCServices/CursorUIViewService.xpc/Contents/MacOS/CursorUIViewService";
        runner.check(
            "CursorUIViewService 不该被当成 Cursor",
            !crate::procmon::profile_matches(&cursor, "CursorUIViewService", service, ""),
            Some("误报了".into()),
        );
        runner.check(
            "用户路径的 DimAgent 应命中 dim",
            crate::procmon::profile_matches(
                &dim,
                "DimAgent",
                "/Applications/DimAgent.app/Contents/MacOS/DimAgent",
                "",
            ),
            Some("没命中".into()),
        );
        runner.check(
            "ssh-agent 不该被任何档案命中",
            !crate::procmon::profile_matches(&dim, "ssh-agent", "/usr/sbin/ssh-agent", ""),
            Some("误报了".into()),
        );
        // npm 装的 CLI 跑在 node 里：进程名对不上，但命令行里有提示词
        let mut hinted = dim.clone();
        hinted.process_names = vec!["nothing-matches-this".into()];
        hinted.cmdline_hints = vec!["dimcode".into()];
        runner.check(
            "命令行提示词也能命中（npm 装的 CLI 跑在 node 里）",
            crate::procmon::profile_matches(
                &hinted,
                "node",
                "/usr/local/bin/node",
                "node /usr/local/lib/node_modules/dimcode/cli.js",
            ),
            Some("没命中".into()),
        );
        // 路径排除表：名字对上了也要否决
        let mut excluded = dim.clone();
        excluded.process_names = vec!["DimAgent".into()];
        excluded.path_excludes = vec!["/System/".into()];
        runner.check(
            "路径排除表命中时即使名字对上也要否决",
            !crate::procmon::profile_matches(
                &excluded,
                "DimAgent",
                "/System/Library/CoreServices/DimAgent",
                "",
            ),
            Some("误报了".into()),
        );
    } else {
        runner.check("cursor / dim 档案存在", false, Some("注册表缺档案".into()));
    }

    // 9. 文件活动：真实临时目录里写一个会话文件，确认「最新写入」读得到。
    //    这是唯一碰文件系统的一条，且只碰自己的临时目录。
    match temp_probe() {
        Ok(ok) => runner.check("写入后能读到最新活动时间", ok, Some("读不到".into())),
        Err(reason) => runner.check("写入后能读到最新活动时间", false, Some(reason)),
    }

    // 10. 卡死三态：观测不足时必须说「判不出」，不能宣布健康（本仓最容易被写坏的一条）
    let unqualified = crate::health::is_hung(None, None, now);
    runner.check(
        "从未观测到运行 ⇒ 卡死判为「判不出」而不是「没有卡死」",
        unqualified.is_none(),
        Some(format!("{unqualified:?}")),
    );
    // 观测了但不足阈值：同样必须是「判不出」
    let too_short = crate::health::is_hung(Some(now - 60_000), None, now);
    runner.check(
        "观测窗口不足阈值 ⇒ 仍然是「判不出」",
        too_short.is_none(),
        Some(format!("{too_short:?}")),
    );
    // 观测够久 + 持续高 CPU ⇒ 明确判成卡死
    let runaway = crate::health::is_hung(Some(now - 600_000), Some(now - 600_000), now);
    runner.check(
        "连续 10 分钟高 CPU ⇒ 明确判成卡死",
        runaway == Some(true),
        Some(format!("{runaway:?}")),
    );

    // 11. 任务效能统计：合计 / 平均 / 最长 / 排版口径
    {
        let mut tracker = crate::duration::TaskDurationTracker::new();
        tracker.record("selftest", 10.0, now - 1_000);
        tracker.record("selftest", 20.0, now - 2_000);
        // 窗口外的那次不算：否则「过去 24 小时」这句话就是假的
        tracker.record("selftest", 999.0, now - crate::duration::DEFAULT_WINDOW_MS - 1);
        let stats = tracker.stats("selftest", crate::duration::DEFAULT_WINDOW_MS, now);
        runner.check(
            "任务效能：两次记录 ⇒ 合计 30 秒、平均 15 秒、最长 20 秒（窗口外不算）",
            stats.task_count == 2
                && (stats.total_work_time - 30.0).abs() < 1e-9
                && (stats.average_duration - 15.0).abs() < 1e-9
                && (stats.max_duration - 20.0).abs() < 1e-9,
            Some(format!("{stats:?}")),
        );
        let empty = tracker.stats("nobody", crate::duration::DEFAULT_WINDOW_MS, now);
        runner.check(
            "没有任务样本时平均时长写 —（不是 0秒/次）",
            empty.formatted_average_duration == "—" && empty.formatted_total_time == "0秒",
            Some(format!("{empty:?}")),
        );
    }

    // 12. 待办：增删改查往返 + 坏文件降级（都在临时目录里做，不碰用户的清单）
    {
        let dir = std::env::temp_dir().join(format!(
            "agentisland-selftest-todos-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::create_dir_all(&dir);
        let store = crate::todos::TodoStore::new(dir.clone());
        let added = store.add("自检加一条", 1_000).is_ok() && store.add("再加一条", 2_000).is_ok();
        let pending_after_add = store.snapshot(0).pending;
        let toggled = store.toggle("1").map(|t| t.pending).unwrap_or(9) == 1;
        let removed = store.remove("2").map(|t| t.items.len()).unwrap_or(9) == 1;
        let cleared = store.clear_done().map(|t| t.items.is_empty()).unwrap_or(false);
        runner.check(
            "待办：加两条 ⇒ 未完成 2；勾一条 ⇒ 1；删一条 ⇒ 剩 1；清已完成 ⇒ 空",
            added && pending_after_add == 2 && toggled && removed && cleared,
            Some(format!(
                "added={added} pending={pending_after_add} toggled={toggled} removed={removed} cleared={cleared}"
            )),
        );

        // 坏文件：降级为空清单、留档、且之后还能继续用
        let broken_dir = dir.join("broken");
        let _ = std::fs::create_dir_all(&broken_dir);
        let broken = crate::todos::TodoStore::new(broken_dir.clone());
        let _ = std::fs::write(broken_dir.join("todos.json"), "{ 这不是 JSON");
        let snapshot = broken.snapshot(4_242);
        let stashed = snapshot.broken_backup.as_deref() == Some("todos.json.broken-4242");
        let recovered = broken.add("恢复之后", 5_000).map(|t| t.items.len()).unwrap_or(9) == 1;
        runner.check(
            "待办：清单文件读坏 ⇒ 显示空清单、原文件留档、之后还能继续加",
            snapshot.items.is_empty() && stashed && recovered,
            Some(format!(
                "items={} stash={:?} recovered={recovered}",
                snapshot.items.len(),
                snapshot.broken_backup
            )),
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 装机探测：「没核实」不得降级成「没装」。
    // 扫描器在这里注入，断言不依赖这台机器装了什么——否则「这台机器刚好没装 X」
    // 就会让自检在别人的机器上红掉，而红的原因与这个包无关。
    {
        use crate::installed::{BundleScanResult, InstalledApps};
        use std::sync::Arc;

        let blind = |ids: &[&str]| {
            let ids: std::collections::HashSet<String> =
                ids.iter().map(|s| (*s).into()).collect();
            Arc::new(move || BundleScanResult {
                ids: ids.clone(),
                // GUI 档案在没有应用包证据的平台上永远拿不到否定结论
                can_prove_absence: false,
            })
        };
        let cli = |names: &[&str]| {
            let names: std::collections::HashSet<String> =
                names.iter().map(|s| (*s).into()).collect();
            Arc::new(move |_wanted: &[String]| names.clone())
        };

        let mut cold = InstalledApps::with_scanners(vec!["codex".into()], cli(&[]), blind(&[]));
        let profile = |id: &str, process: &str, bundle: &str| AgentProfile {
            id: id.into(),
            name: id.into(),
            glyph: String::new(),
            emoji: String::new(),
            process_names: vec![process.into()],
            bundle_ids: vec![bundle.into()],
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: vec![],
            token_roots: vec![],
            session_database: None,
            category: "assistant".into(),
        };
        let cold_state = cold.is_installed(&profile("codex", "codex", "com.openai.codex"));

        cold = InstalledApps::with_scanners(
            vec!["codex".into()],
            cli(&["codex"]),
            blind(&["com.openai.codex"]),
        );
        cold.refresh();
        let warm_state = cold.is_installed(&profile("codex", "codex", "com.openai.codex"));

        let mut unknown_gui =
            InstalledApps::with_scanners(vec!["code".into()], cli(&[]), blind(&[]));
        unknown_gui.refresh();
        let gui = unknown_gui.is_installed(&profile("vscode", "Code", "com.microsoft.VSCode"));

        runner.check(
            "装机探测：缓存未热给「未核实」、命中才给「已安装」、缺应用包证据不判「未安装」",
            cold_state == None && warm_state == Some(true) && gui == None,
            Some(format!("cold={cold_state:?} warm={warm_state:?} gui={gui:?}")),
        );
    }

    runner.finish()
}

/// 临时目录里写一个 `probe.jsonl`，再用真实的目录扫描读回它的修改时间
fn temp_probe() -> Result<bool, String> {
    // 目录名必须**每次调用唯一**：自检会被并行调用（两个用例同时跑 `run()`），
    // 用进程号命名会让一个调用把另一个正在探测的目录删掉——而失败表现为
    // 「写入后读不到最新活动时间」，看起来像文件扫描坏了。
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "agentisland-selftest-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).map_err(|e| format!("建临时目录失败：{e}"))?;
    let file = dir.join("probe.jsonl");
    std::fs::write(&file, b"{\"selftest\":true}\n").map_err(|e| format!("写探针失败：{e}"))?;

    let mut monitor = crate::filemon::FileMonitor::new();
    let mut profile = crate::registry::builtin()
        .into_iter()
        .find(|p| p.id == "dim")
        .ok_or_else(|| "注册表缺 dim".to_string())?;
    profile.session_dirs = vec![dir.to_string_lossy().to_string()];
    let result = monitor.probe(&profile);

    // 自检不留下任何东西：临时目录用完就删（删失败不算检查失败，只是不干净）
    let _ = std::fs::remove_dir_all(&dir);
    Ok(result.latest_write.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 自检必须**在开发机上也全绿**：有一条红就说明包有问题。
    /// 这条用例同时是「所有检查项都还在跑」的守护——检查被删掉会在这里显形。
    #[test]
    fn the_selftest_passes_on_a_healthy_machine() {
        let report = run();
        // 自检存在的意义就是给人看，所以这里把它打出来（`--nocapture` 可见）
        println!("{}", report.text());
        assert_eq!(
            report.failed,
            0,
            "自检不该在健康机器上失败：\n{}",
            report.text()
        );
        // 这个下限的作用是「检查项被删掉时显形」，不是为了好看
        assert!(report.passed >= 26, "检查项少了：{}", report.passed);
        assert_eq!(report.exit_code(), 0);
    }

    #[test]
    fn the_report_is_readable_and_every_check_says_what_it_checked() {
        let report = run();
        let text = report.text();
        assert!(text.starts_with("AgentIsland selftest"), "{text}");
        assert!(text.ends_with("全部通过\n"), "{text}");
        for check in &report.checks {
            assert!(!check.name.trim().is_empty(), "检查项必须有名字");
            assert!(
                check.detail.is_none(),
                "通过的检查不该带 detail（噪声）：{}",
                check.name
            );
        }
        // 失败时才有 detail，且 exit code 变化
        let failing = Report {
            passed: 0,
            failed: 1,
            checks: vec![Check {
                name: "示例".into(),
                passed: false,
                detail: Some("实际值".into()),
            }],
        };
        assert_eq!(failing.exit_code(), 1);
        assert!(failing.text().contains("（实际：实际值）"), "{}", failing.text());
    }

    #[test]
    fn the_ago_text_matches_the_reference_including_the_nil_case() {
        assert_eq!(time_ago_text(Some(2.0)), "刚刚");
        assert_eq!(time_ago_text(Some(4.4)), "刚刚");
        assert_eq!(time_ago_text(Some(5.0)), "5s 前", "阈值是 5 秒：四舍五入到 5 就不再是「刚刚」");
        assert_eq!(
            time_ago_text(Some(9.0)),
            "9s 前",
            "旧实现把「刚刚」的阈值放在 10 秒，9 秒会显示「刚刚」——参考实现不是这样"
        );
        assert_eq!(time_ago_text(Some(30.0)), "30s 前");
        assert_eq!(time_ago_text(Some(120.0)), "2m 前");
        assert_eq!(time_ago_text(Some(7_200.0)), "2h 前");
        assert_eq!(time_ago_text(Some(200_000.0)), "55h 前", "没有「天」这一档，与参考实现一致");
        assert_eq!(time_ago_text(None), "—");
        assert_eq!(time_ago_text(Some(-5.0)), "刚刚", "负数来自时钟回拨，按 0 处理");
    }
}
