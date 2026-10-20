//! Explicit, bounded core measurements. No identifiers or transcript content.
use serde::Serialize;
#[derive(Default, Clone, Serialize)]
pub struct Cache {
    pub files: usize,
    pub entries: usize,
    pub entry_capacity: usize,
    pub dedup_keys: usize,
    pub dedup_capacity: usize,
    /// 当轮跨文件归并索引的键数。与 `dedup_keys` 分开：后者逐文件求和、随折入清空，
    /// 前者是跨文件作用域、每轮重建。二者不是同一件事，不能合并成一个数字。
    pub cross_file_keys: usize,
    /// Conservative cache capacity charge, not allocator usage or footprint.
    pub retained_charge_bytes: usize,
    pub retained_budget_bytes: usize,
}
#[derive(Default, Clone, Serialize)]
pub struct FileCache {
    pub roots: usize,
    pub files: usize,
    pub file_capacity: usize,
    pub cached_candidates: usize,
}
#[derive(Default, Clone, Serialize)]
pub struct Tick {
    pub demo: bool,
    pub total_us: u64,
    pub process_us: u64,
    pub installed_us: u64,
    pub files_us: u64,
    pub sessions_us: u64,
    pub tokens_us: u64,
    pub other_us: u64,
    pub cache_stats_us: u64,
    pub profiles: usize,
    pub candidates: usize,
    pub file_cache: FileCache,
    pub token_cache: Cache,
}
/// 显式诊断入口：不建 WebView、不订阅 hook、不主动保存设置（仅损坏留档时可能重命名）、不外发。
/// 与其它无头入口共用 `headless` 分派，因此它的日志同样默认静默。
pub(crate) fn cli(args: &[String]) -> Option<i32> {
    if !args.iter().any(|a| a == "--memory-core-probe") {
        return None;
    }
    // 返回码而不是自己 exit：进程退出由统一分派器负责，否则这里会成为
    // 又一个「绕过 headless 入口」的旁门。
    let options = match Options::parse(args) {
        Ok(value) => value,
        Err(reason) => {
            eprintln!("{reason}");
            return Some(2);
        }
    };
    if args.iter().any(|a| a == "--process-only") {
        let mut monitor = crate::procmon::ProcessMonitor::new();
        for sample in 0..options.samples {
            let start = std::time::Instant::now();
            monitor.refresh();
            println!(
                "{}",
                serde_json::json!({"schema_version":1,"sample":sample+1,"recorded_ms":crate::tokens::now_ms(),"mode":"process_only","process_us":elapsed(Some(start))})
            );
            if sample + 1 < options.samples {
                std::thread::sleep(std::time::Duration::from_millis(options.interval_ms));
            }
        }
        return Some(0);
    }
    let (_tx, rx) = std::sync::mpsc::channel();
    let mut settings = crate::settings::Settings::load();
    settings.remote_policy.master_enabled = false;
    let mut engine = crate::engine::ActivityEngine::new(settings, rx);
    engine.refresh_usage = !args.iter().any(|a| a == "--without-usage");
    engine.enable_resource_diagnostics();
    for sample in 0..options.samples {
        engine.tick();
        let reclaim = !args.iter().any(|a| a == "--retain-allocator-pages");
        let start = std::time::Instant::now();
        if reclaim {
            crate::memory::reclaim_idle_pages();
        }
        let reclaim_us = elapsed(Some(start));
        println!(
            "{}",
            serde_json::json!({"schema_version":1,"sample":sample+1,"recorded_ms":crate::tokens::now_ms(),"mode":"core","usage_enabled":engine.refresh_usage,"reclaim_enabled":reclaim,"reclaim_us":reclaim_us,"tick":engine.resource_diagnostics()})
        );
        if sample + 1 < options.samples {
            std::thread::sleep(std::time::Duration::from_millis(options.interval_ms));
        }
    }
    Some(0)
}

pub fn elapsed(start: Option<std::time::Instant>) -> u64 {
    start
        .map(|s| s.elapsed().as_micros().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}
pub struct Options {
    pub samples: usize,
    pub interval_ms: u64,
}
impl Options {
    /// Validate before settings, credential access, engine creation or sampling.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut samples = None;
        let mut interval = None;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--memory-core-probe"
                | "--process-only"
                | "--without-usage"
                | "--retain-allocator-pages" => {}
                "--probe-samples" | "--probe-interval-ms" => {
                    let option = args[i].as_str();
                    i += 1;
                    let value = args.get(i).ok_or("诊断参数缺少值")?;
                    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                        return Err("诊断参数需要正整数".into());
                    }
                    let number = value.parse::<u64>().map_err(|_| "诊断参数超出范围")?;
                    if option == "--probe-samples" {
                        if samples.is_some() || !(1..=900).contains(&number) {
                            return Err("诊断样本需要1到900，不能重复指定".into());
                        }
                        samples = Some(number as usize);
                    } else {
                        if interval.is_some() || !(100..=60_000).contains(&number) {
                            return Err("诊断间隔需要100到60000毫秒，不能重复指定".into());
                        }
                        interval = Some(number);
                    }
                }
                _ => return Err("不支持的核心诊断参数".into()),
            }
            i += 1;
        }
        Ok(Self {
            samples: samples.unwrap_or(20),
            interval_ms: interval.unwrap_or(2000),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn args(extra: &[&str]) -> Vec<String> {
        [vec!["agentisland", "--memory-core-probe"], extra.to_vec()]
            .concat()
            .into_iter()
            .map(String::from)
            .collect()
    }
    #[test]
    fn diagnostic_options_are_bounded_and_unambiguous() {
        let defaults = Options::parse(&args(&[])).unwrap();
        assert_eq!(defaults.samples, 20);
        assert_eq!(defaults.interval_ms, 2000);
        let custom = Options::parse(&args(&[
            "--probe-samples",
            "1",
            "--probe-interval-ms",
            "100",
        ]))
        .unwrap();
        assert_eq!(custom.samples, 1);
        assert_eq!(custom.interval_ms, 100);
        for invalid in [
            vec!["--probe-samples"],
            vec!["--probe-samples", "0"],
            vec!["--probe-samples", "901"],
            vec!["--probe-samples", "-1"],
            vec!["--probe-interval-ms", "99"],
            vec!["--probe-interval-ms", "60001"],
            vec!["--probe-samples", "1.5"],
            vec!["--probe-samples", "1", "--probe-samples", "2"],
            vec!["--unknown"],
        ] {
            assert!(Options::parse(&args(&invalid)).is_err());
        }
        assert!(Options::parse(&args(&[
            "--probe-samples",
            "900",
            "--probe-interval-ms",
            "60000"
        ]))
        .is_ok());
    }
}
