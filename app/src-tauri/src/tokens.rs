use crate::models::{ModelUsage, TokenReport, TokenUsage};
use crate::session;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Token 用量监控：只读解析会话 JSONL 的结构化 usage 字段
/// （净消耗口径，不含缓存读取），文件指纹增量缓存。
pub struct TokenUsageMonitor {
    states: HashMap<String, FileState>,
}

#[derive(Default)]
struct FileState {
    parsed_len: u64,
    last_write: Option<SystemTime>,
    tokens_total: i64,
    cost_total: f64,
    /// (unix ms, model, tokens, cost)
    entries: Vec<(i64, String, i64, f64)>,
}

struct UsageLine {
    tokens: i64,
    cost: f64,
    model: String,
    ts_ms: i64,
}

impl TokenUsageMonitor {
    pub fn new() -> Self {
        TokenUsageMonitor { states: HashMap::new() }
    }

    pub fn monitor(&mut self, profile: &crate::models::AgentProfile) -> TokenReport {
        let cutoff24 = now_ms() - 24 * 3600 * 1000;
        let mut tokens24 = 0i64;
        let mut tokens_total = 0i64;
        let mut cost24 = 0f64;
        let mut cost_total = 0f64;
        let mut models: HashMap<String, (i64, f64)> = HashMap::new();

        for root in &profile.token_roots {
            if !Path::new(root).is_dir() {
                continue;
            }
            for entry in walkdir::WalkDir::new(root)
                .max_depth(4)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if !entry.file_type().is_file() {
                    continue;
                }
                if entry
                    .path()
                    .extension()
                    .map(|e| e.to_string_lossy() != "jsonl")
                    .unwrap_or(true)
                {
                    continue;
                }
                let path = entry.path().to_path_buf();
                let (t24, c24, tt, ct, m) = self.parse_file(&path, cutoff24);
                tokens24 += t24;
                cost24 += c24;
                tokens_total += tt;
                cost_total += ct;
                for (model, (tk, co)) in m {
                    let e = models.entry(model).or_insert((0, 0.0));
                    e.0 += tk;
                    e.1 += co;
                }
            }
        }

        let mut models24h: Vec<ModelUsage> = models
            .into_iter()
            .map(|(model, (tokens, cost))| ModelUsage { model, tokens, cost })
            .collect();
        models24h.sort_by(|a, b| b.tokens.cmp(&a.tokens));

        // 30 天逐小时桶
        let mut hourly: HashMap<i64, i64> = HashMap::new();
        let cutoff30 = now_ms() - 30 * 24 * 3600 * 1000;
        for st in self.states.values() {
            for (ts, _, tokens, _) in &st.entries {
                if *ts < cutoff30 || *tokens <= 0 {
                    continue;
                }
                let hour = ts - ts % 3_600_000;
                *hourly.entry(hour).or_insert(0) += tokens;
            }
        }
        let mut hourly30d: Vec<(i64, i64)> = hourly.into_iter().collect();
        hourly30d.sort_by_key(|kv| kv.0);

        TokenReport {
            usage: TokenUsage { tokens24h: tokens24, tokens_total, cost24h: cost24, cost_total },
            models24h: models24h.clone(),
            models_total: models24h,
            hourly30d,
        }
    }

    fn parse_file(
        &mut self,
        path: &PathBuf,
        cutoff24: i64,
    ) -> (i64, f64, i64, f64, HashMap<String, (i64, f64)>) {
        let key = path.to_string_lossy().to_string();
        let (start, last_write) = {
            let st = self.states.entry(key.clone()).or_default();
        let meta = fs::metadata(path);
        match meta {
            Ok(m) => {
                let lw = m.modified().unwrap_or(UNIX_EPOCH);
                let len = m.len();
                if len < st.parsed_len {
                    st.parsed_len = 0; // 文件被截断/重写：全量重读
                }
                (st.parsed_len, Some(lw))
            }
            Err(_) => (st.parsed_len, st.last_write),
        }
    };

        let mut collected: Vec<UsageLine> = Vec::new();
        let new_len = session::for_each_line(path, start, |line| {
            if line.len() < 8 {
                return;
            }
            if let Some(u) = parse_usage_line(line) {
                collected.push(u);
            }
        });

        // 汇总在借种作用域内完成，之后状态表可再整体访问
        let (tokens24, cost24, tokens_total, cost_total, models) = {
            let st = self.states.get_mut(&key).unwrap();
            if let Some(l) = new_len {
                st.parsed_len = l;
                st.last_write = last_write;
            }
            for u in collected {
                st.tokens_total += u.tokens;
                st.cost_total += u.cost;
                st.entries.push((u.ts_ms, u.model, u.tokens, u.cost));
            }
            // entries 有界：只留近 7 天
            let cutoff7 = now_ms() - 7 * 24 * 3600 * 1000;
            st.entries.retain(|(ts, _, _, _)| *ts >= cutoff7);
            let mut models: HashMap<String, (i64, f64)> = HashMap::new();
            let mut tokens24 = 0i64;
            let mut cost24 = 0f64;
            for (ts, model, tokens, cost) in &st.entries {
                if *ts >= cutoff24 {
                    tokens24 += tokens;
                    cost24 += cost;
                    let e = models.entry(model.clone()).or_insert((0, 0.0));
                    e.0 += tokens;
                    e.1 += cost;
                }
            }
            (tokens24, cost24, st.tokens_total, st.cost_total, models)
        };
        // 状态表有界
        if self.states.len() > 8000 {
            let keys: Vec<String> = self
                .states
                .iter()
                .take(1000)
                .map(|(k, _)| k.clone())
                .collect();
            for k in keys {
                self.states.remove(&k);
            }
        }
        (tokens24, cost24, tokens_total, cost_total, models)
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Claude: {"type":"assistant","timestamp":"...","message":{"model":"...","usage":{...}}}
/// Codex:  {"type":"event_msg","timestamp":"...","payload":{"type":"token_count","info":{"last_token_usage":{...}}}}
fn parse_usage_line(line: &str) -> Option<UsageLine> {
    let doc: Value = serde_json::from_str(line).ok()?;
    let obj = doc.as_object()?;

    let (usage, model, is_claude): (Value, Option<String>, bool) = if let Some(msg) = obj.get("message") {
        let u = msg.get("usage")?.clone();
        let m = msg.get("model").and_then(|v| v.as_str()).map(|s| s.to_string());
        (u, m, true)
    } else if let Some(pl) = obj.get("payload") {
        if pl.get("type").and_then(|v| v.as_str()) != Some("token_count") {
            return None;
        }
        let u = pl.get("info")?.get("last_token_usage")?.clone();
        (u, Some("gpt-5".to_string()), false)
    } else if let Some(resp) = obj.get("response") {
        // ZCode rollout：response.usage {inputTokens, outputTokens, cacheRead/WriteTokens}
        let u = resp.get("usage")?.clone();
        let m = obj
            .get("model")
            .and_then(|mv| mv.get("modelId"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        (u, m, false)
    } else {
        return None;
    };

    let get = |key: &str| usage.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
    let input = get("input_tokens").max(get("inputTokens"));
    let output = get("output_tokens").max(get("outputTokens"));
    let cache_read = if is_claude {
        get("cache_read_input_tokens")
    } else {
        get("cached_input_tokens").max(get("cacheReadTokens"))
    };
    let cache_write = if is_claude {
        get("cache_creation_input_tokens")
    } else {
        get("cacheWriteTokens")
    };
    // 净消耗：不含缓存读取
    let net = input + output + cache_write;
    if net <= 0 {
        return None;
    }

    let name = short_model_name(model.as_deref());
    let (in_price, out_price) = price_lookup(&name, is_claude);
    let cost = (input + cache_write) as f64 * in_price / 1_000_000.0
        + output as f64 * out_price / 1_000_000.0
        + cache_read as f64 * in_price * 0.1 / 1_000_000.0;

    let ts_ms = obj
        .get("timestamp")
        .or_else(|| obj.get("completedAt"))
        .and_then(|v| v.as_str())
        .and_then(parse_iso_ms)
        .unwrap_or_else(now_ms);

    Some(UsageLine { tokens: net, cost, model: name, ts_ms })
}

pub fn parse_iso_ms_pub(s: &str) -> Option<i64> {
    parse_iso_ms(s)
}

fn parse_iso_ms(s: &str) -> Option<i64> {
    // "2026-09-24T12:34:56.789Z" 或无毫秒/带时区
    let s = s.trim_end_matches('Z').trim_end_matches("+00:00");
    let (date, time) = s.split_once('T')?;
    let dp: Vec<i64> = date.split('-').filter_map(|x| x.parse().ok()).collect();
    if dp.len() != 3 {
        return None;
    }
    let time = time.split('.').next().unwrap_or(time);
    let tp: Vec<i64> = time.split(':').filter_map(|x| x.parse().ok()).collect();
    if tp.len() < 2 {
        return None;
    }
    // 一律按 UTC 计算（本地时区偏差只影响桶归属，不影响总量）
    Some(days_from_civil(dp[0], dp[1], dp[2]) * 86_400_000
        + tp[0] * 3_600_000
        + tp[1] * 60_000
        + tp.get(2).copied().unwrap_or(0) * 1_000)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn short_model_name(model: Option<&str>) -> String {
    let m = match model {
        Some(m) if !m.is_empty() => m.to_string(),
        _ => "unknown".to_string(),
    };
    match m.rfind('/') {
        Some(i) => m[i + 1..].to_string(),
        None => m,
    }
}

/// ($/M input, $/M output) — 与 Windows 端 PriceTable 同源
fn price_lookup(model: &str, is_claude: bool) -> (f64, f64) {
    let lower = model.to_lowercase();
    if lower.starts_with("opus") || lower.contains("opus") {
        (15.0, 75.0)
    } else if lower.starts_with("sonnet") || lower.contains("sonnet") {
        (3.0, 15.0)
    } else if lower.starts_with("haiku") || lower.contains("haiku") {
        (1.0, 5.0)
    } else if lower.contains("glm") {
        (0.55, 2.0) // GLM 系近似（$/M）
    } else if !is_claude {
        (1.25, 10.0) // GPT 系近似
    } else {
        (3.0, 15.0)
    }
}
