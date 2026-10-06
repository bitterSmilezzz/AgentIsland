//! Explicit, bounded core measurements. No identifiers or transcript content.
use serde::Serialize;
#[derive(Default, Clone, Serialize)]
pub struct Cache {
    pub files: usize,
    pub entries: usize,
    pub entry_capacity: usize,
    pub dedup_keys: usize,
    pub dedup_capacity: usize,
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
