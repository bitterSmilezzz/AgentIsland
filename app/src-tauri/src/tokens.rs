use crate::cost;
use crate::models::{ModelUsage, SessionSchema, TokenReport, TokenUsage};
use crate::session;
use crate::sqlite;
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
        // 成本是估的还是记录的（见 models.rs 的 `cost_estimated`）。**逐来源记账**：
        // JSONL 的成本一律来自 `cost::estimate_cost`（估的）；SQLite 的成本是库里的
        // 记录值，不置这个标记。上一版靠「非零即估价」这条等价关系，SQLite 一进来就不成立了。
        let mut cost_estimated = false;
        // (tokens, cost, 该模型是否含估价)
        let mut models: HashMap<String, (i64, f64, bool)> = HashMap::new();

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
                // JSONL 方言不带记录成本：这里的成本全部是 `cost::estimate_cost` 估出来的
                if c24 > 0.0 || ct > 0.0 {
                    cost_estimated = true;
                }
                for (model, (tk, co)) in m {
                    merge_model(&mut models, model, tk, co, true);
                }
            }
        }

        // ── SQLite 源：与 JSONL 并列的第二类明细源 ─────────────────────────────
        // 位置与方言全部来自档案声明（ADR 0004），这里一个路径字面量都不出现。
        if let Some(database) = &profile.session_database {
            let part = match database.schema {
                SessionSchema::OpenCode => query_open_code(&database.path, cutoff24),
                SessionSchema::DimTasks => query_dim_tasks(&database.path, cutoff24),
                // 状态索引只回答「最新一条状态」，**不含 token**——不是缺数据，是没这个概念
                SessionSchema::StatusIndex => None,
            };
            if let Some(part) = part {
                tokens24 += part.tokens24;
                tokens_total += part.tokens_total;
                cost24 += part.cost24;
                cost_total += part.cost_total;
                if part.cost_estimated && (part.cost24 > 0.0 || part.cost_total > 0.0) {
                    cost_estimated = true;
                }
                for model in part.models {
                    merge_model(
                        &mut models,
                        model.model,
                        model.tokens,
                        model.cost,
                        model.cost_estimated,
                    );
                }
            }
        }

        let mut models24h: Vec<ModelUsage> = models
            .into_iter()
            .map(|(model, (tokens, cost, estimated))| ModelUsage {
                model,
                tokens,
                cost,
                // 这个模型的花费是估的 ⟺ 成本非零，且其中有估价那一份参与
                cost_estimated: cost > 0.0 && estimated,
            })
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
            usage: TokenUsage {
                tokens24h: tokens24,
                tokens_total,
                cost24h: cost24,
                cost_total,
                cost_estimated,
            },
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
    let cache_write = if is_claude {
        get("cache_creation_input_tokens")
    } else {
        get("cacheWriteTokens")
    };
    // 净消耗：不含缓存读取（与 Swift StructuredTokenUsageIndex 同口径）。
    // 缓存读取也不进估算——它按另一档单独计价，混进来只会把成本算高。
    let net = input + output + cache_write;
    if net <= 0 {
        return None;
    }

    // 成本口径与 Swift `TokenCostEstimator` 同源：3:1 混合单价 × 总 token。
    // 估不出来（模型不在费率表里）就是 0，**不编价**——旧实现按分量分别计价，
    // 并给没见过的模型兜底 (1.25,10.0)，于是两台实现同日同量会算出两个数。
    let name = short_model_name(model.as_deref());
    let cost = model
        .as_deref()
        .and_then(|m| cost::estimate_cost(m, net))
        .unwrap_or(0.0);

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

// ── SQLite 明细源 ────────────────────────────────────────────────────────────

/// 一个 SQLite 源贡献的用量。
/// SQLite 的成本是**记录值**（不是估的），所以 `cost_estimated` 恒为 false——
/// 留这个字段是为了让「记录的 vs 估的」在同一结构里说清，而不是靠调用方记规矩。
#[derive(Default, Debug, Clone)]
struct UsagePart {
    tokens24: i64,
    tokens_total: i64,
    cost24: f64,
    cost_total: f64,
    cost_estimated: bool,
    models: Vec<ModelUsage>,
}

/// 把一份用量并进模型表。`estimated` 按位或——同名模型可能两个来源都有。
fn merge_model(
    models: &mut HashMap<String, (i64, f64, bool)>,
    model: String,
    tokens: i64,
    cost: f64,
    estimated: bool,
) {
    let entry = models.entry(model).or_insert((0, 0.0, false));
    entry.0 += tokens;
    entry.1 += cost;
    entry.2 = entry.2 || (estimated && cost > 0.0);
}

/// 列 → i64。**必须容忍 SQLite 的动态类型**：列里只要有一行是 REAL，`SUM` 的整体结果
/// 就变成浮点，直接 `get::<i64>` 会当场失败、让整份统计静默消失（Swift 侧同样「经 Double
/// 中转」，注释在 `tokenColumn`）。浮点→整型走 Rust 的饱和转换，越界不会 panic。
fn column_i64(row: &rusqlite::Row<'_>, index: usize) -> i64 {
    match row.get_ref(index) {
        Ok(rusqlite::types::ValueRef::Integer(value)) => value,
        Ok(rusqlite::types::ValueRef::Real(value)) => value as i64,
        Ok(rusqlite::types::ValueRef::Text(bytes)) => {
            String::from_utf8_lossy(bytes).trim().parse().unwrap_or(0)
        }
        _ => 0,
    }
}

/// 列 → f64，同 `column_i64` 的容忍口径；**非有限值归 0** 而不是往下游传。
fn column_f64(row: &rusqlite::Row<'_>, index: usize) -> f64 {
    let value = match row.get_ref(index) {
        Ok(rusqlite::types::ValueRef::Integer(value)) => value as f64,
        Ok(rusqlite::types::ValueRef::Real(value)) => value,
        Ok(rusqlite::types::ValueRef::Text(bytes)) => {
            String::from_utf8_lossy(bytes).trim().parse().unwrap_or(0.0)
        }
        _ => 0.0,
    };
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

/// 模型拆分。SQLite 的成本是记录值，故一律 `cost_estimated: false`；
/// 模型名为 NULL 时落 `unknown`（与 JSONL 侧 `short_model_name` 同一个记号）。
fn collect_models(connection: &rusqlite::Connection, sql: &str) -> Vec<ModelUsage> {
    let Ok(mut statement) = connection.prepare(sql) else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([], |row| {
        let model = row
            .get::<_, Option<String>>(0)?
            .unwrap_or_else(|| "unknown".to_string());
        Ok(ModelUsage {
            model,
            tokens: column_i64(row, 1),
            cost: column_f64(row, 2),
            cost_estimated: false,
        })
    }) else {
        return Vec::new();
    };
    rows.filter_map(|row| row.ok()).collect()
}

/// OpenCode 方言（含同表 fork，如小米 MiMo Code）：`message.data` 是 JSON。
/// 净 token = input + output + reasoning（**cache.read 不参与**），只算
/// `role='assistant'` 的行。与 Swift 的两条查询逐字段同口径。
fn query_open_code(path: &str, cutoff24: i64) -> Option<UsagePart> {
    let connection = sqlite::open_readonly(path).ok()?;
    let (tokens_total, cost_total, tokens24, cost24) = connection
        .query_row(
            "SELECT COALESCE(SUM(t),0), COALESCE(SUM(c),0), \
                    COALESCE(SUM(CASE WHEN time_created >= ?1 THEN t ELSE 0 END),0), \
                    COALESCE(SUM(CASE WHEN time_created >= ?1 THEN c ELSE 0 END),0) \
             FROM ( \
                 SELECT time_created, \
                        COALESCE(json_extract(data,'$.cost'),0) AS c, \
                        COALESCE(json_extract(data,'$.tokens.input'),0) \
                          + COALESCE(json_extract(data,'$.tokens.output'),0) \
                          + COALESCE(json_extract(data,'$.tokens.reasoning'),0) AS t \
                 FROM message \
                 WHERE json_extract(data,'$.role')='assistant' \
             )",
            [cutoff24],
            |row| {
                Ok((
                    column_i64(row, 0),
                    column_f64(row, 1),
                    column_i64(row, 2),
                    column_f64(row, 3),
                ))
            },
        )
        .ok()?;
    let models = collect_models(
        &connection,
        "SELECT json_extract(data,'$.modelID') AS model, \
                COALESCE(SUM(COALESCE(json_extract(data,'$.tokens.input'),0) \
                  + COALESCE(json_extract(data,'$.tokens.output'),0) \
                  + COALESCE(json_extract(data,'$.tokens.reasoning'),0)),0), \
                COALESCE(SUM(json_extract(data,'$.cost')),0) \
         FROM message WHERE json_extract(data,'$.role')='assistant' \
         GROUP BY 1 ORDER BY 2 DESC",
    );
    Some(UsagePart {
        tokens24,
        tokens_total,
        cost24,
        cost_total,
        cost_estimated: false,
        models,
    })
}

/// DimAgent 方言：`usage_ledger`。净 token = (promptTokens − cacheReadTokens，**下限 0**)
/// + completionTokens——`promptTokens` 含缓存命中部分，直接相加会把同一批 token 计两遍。
/// `createdAt` 是 ISO8601 字符串，SQL 里靠**字典序**当时间比较，所以下界必须按同一个
/// 形状生成（带毫秒，见 `iso_utc_from_ms`）。
fn query_dim_tasks(path: &str, cutoff24: i64) -> Option<UsagePart> {
    let connection = sqlite::open_readonly(path).ok()?;
    let cutoff = iso_utc_from_ms(cutoff24);
    const NET: &str = "MAX(COALESCE(json_extract(usage,'$.promptTokens'),0) \
                       - COALESCE(json_extract(usage,'$.cacheReadTokens'),0), 0) \
                       + COALESCE(json_extract(usage,'$.completionTokens'),0)";
    let aggregate = format!(
        "SELECT COALESCE(SUM(t),0), COALESCE(SUM(c),0), \
                COALESCE(SUM(CASE WHEN createdAt >= ?1 THEN t ELSE 0 END),0), \
                COALESCE(SUM(CASE WHEN createdAt >= ?1 THEN c ELSE 0 END),0) \
         FROM (SELECT createdAt, COALESCE(cost,0) AS c, {NET} AS t FROM usage_ledger)"
    );
    let (tokens_total, cost_total, tokens24, cost24) = connection
        .query_row(&aggregate, [cutoff.as_str()], |row| {
            Ok((
                column_i64(row, 0),
                column_f64(row, 1),
                column_i64(row, 2),
                column_f64(row, 3),
            ))
        })
        .ok()?;
    let models = collect_models(
        &connection,
        &format!(
            "SELECT modelId, COALESCE(SUM({NET}),0), COALESCE(SUM(cost),0) \
             FROM usage_ledger GROUP BY modelId ORDER BY 2 DESC"
        ),
    );
    Some(UsagePart {
        tokens24,
        tokens_total,
        cost24,
        cost_total,
        cost_estimated: false,
        models,
    })
}

/// unix 毫秒 → `YYYY-MM-DDTHH:MM:SS.mmmZ`。
/// 与 Swift `isoFormatter` 同形（`.withInternetDateTime, .withFractionalSeconds`，带毫秒）：
/// dim 的 `createdAt` 就是这种字符串，比较靠字典序，形状不一致会静默漏掉整段窗口。
fn iso_utc_from_ms(ms: i64) -> String {
    let seconds = ms.div_euclid(1_000);
    let millis = ms.rem_euclid(1_000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let secs_of_day = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        secs_of_day / 3_600,
        (secs_of_day % 3_600) / 60,
        secs_of_day % 60
    )
}

/// Howard Hinnant 的 `civil_from_days`（本文件 `days_from_civil` 的逆）
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (if month <= 2 { year + 1 } else { year }, month, day)
}

// 费率表已迁到 `crate::cost`（与 Swift `TokenCostEstimator` 同表）。此前这里有一份
// 5 档粗分档的 `price_lookup`——同一件事两份算法，正是 ADR 0010 明令不许的。

#[cfg(test)]
mod tests {
    use super::*;

    /// Claude 方言的一行：3000 输入 + 1000 输出 = 净 4000 tokens
    fn claude_line(model: &str) -> String {
        format!(
            r#"{{"message":{{"model":"{model}","usage":{{"input_tokens":3000,"output_tokens":1000}}}}}}"#
        )
    }

    #[test]
    fn cost_comes_from_the_shared_estimator() {
        let parsed = parse_usage_line(&claude_line("claude-3-7-sonnet")).expect("应解析出用量");
        assert_eq!(parsed.tokens, 4000);
        // 3:1 混合价 = (3*3 + 15)/4 = 6.0 美元/百万 → 4000 tokens = $0.024
        assert!((parsed.cost - 0.024).abs() < 1e-12, "cost={}", parsed.cost);
    }

    #[test]
    fn unknown_model_gets_no_invented_cost() {
        // 旧实现对任何非 claude 模型兜底 (1.25,10.0)，于是编出一个成本
        let parsed = parse_usage_line(&claude_line("some-local-finetune")).expect("应解析出用量");
        assert_eq!(parsed.cost, 0.0, "费率表里没有的模型不许编成本");
        assert_eq!(parsed.tokens, 4000, "用量照记，只是不报钱数");
    }

    #[test]
    fn glm_is_not_priced_on_either_side() {
        // 23 号对照表点名过：旧 Rust 给 glm 编了 (0.55,2.0)，而 Swift 表里没有这一条
        let parsed = parse_usage_line(&claude_line("glm-4")).expect("应解析出用量");
        assert_eq!(parsed.cost, 0.0);
    }

    // ── SQLite 明细源 ────────────────────────────────────────────────────────

    fn temp_db(name: &str) -> String {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("agentisland-tokens-{}-{stamp}-{name}", std::process::id()))
            .to_string_lossy()
            .to_string()
    }

    fn seed(path: &str, statements: &[&str]) {
        let connection = rusqlite::Connection::open(path).unwrap();
        for sql in statements {
            connection.execute_batch(sql).unwrap();
        }
    }

    #[test]
    fn iso_formatter_matches_the_shape_swift_uses_for_dim_comparisons() {
        // dim 的 createdAt 是带毫秒的 ISO8601，SQL 里靠字典序比较——形状必须一致，
        // 否则窗口会静默错位（少一天/多一天都不会报错）
        assert_eq!(iso_utc_from_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_utc_from_ms(1_000), "1970-01-01T00:00:01.000Z");
        assert_eq!(iso_utc_from_ms(-1), "1969-12-31T23:59:59.999Z");
        // 与自家的 `parse_iso_ms` 往返：注意它**不解析小数秒**（既有行为），
        // 所以往返后毫秒位是 000；本函数则一律输出三位小数，与 Swift 带毫秒的
        // isoFormatter 同形——dim 的 createdAt 靠字典序比较，形状必须一致。
        let sample = "2026-09-27T02:43:12Z";
        let ms = parse_iso_ms(sample).expect("应能解析");
        assert_eq!(iso_utc_from_ms(ms), "2026-09-27T02:43:12.000Z");
        assert_eq!(parse_iso_ms(&iso_utc_from_ms(ms)), Some(ms), "秒级必须往返一致");
        // 闰年与月末边界（civil_from_days 是自写算法，这里钉住两处易错点）
        assert_eq!(iso_utc_from_ms(parse_iso_ms("2024-02-29T12:00:00Z").unwrap()),
                   "2024-02-29T12:00:00.000Z");
        assert_eq!(iso_utc_from_ms(parse_iso_ms("2026-12-31T23:59:59Z").unwrap()),
                   "2026-12-31T23:59:59.000Z");
    }

    #[test]
    fn open_code_source_applies_swift_net_rule_and_role_filter() {
        let path = temp_db("oc.db");
        seed(&path, &[
            "CREATE TABLE message (session_id TEXT, data TEXT, time_created INTEGER)",
            // assistant：10 + 20 + 5 = 35；cache.read 999 **不计**
            r#"INSERT INTO message VALUES ('s1', '{"role":"assistant","modelID":"oc1","tokens":{"input":10,"output":20,"reasoning":5,"cache":{"read":999}},"cost":0.5}', 1000)"#,
            // user 行整条不计（五万 token 也不能进账）
            r#"INSERT INTO message VALUES ('s1', '{"role":"user","modelID":"oc1","tokens":{"input":50000},"cost":9.9}', 1000)"#,
            // 24h 窗口外的 assistant 行：只进累计
            r#"INSERT INTO message VALUES ('s2', '{"role":"assistant","modelID":"oc2","tokens":{"input":200,"output":0,"reasoning":0},"cost":2.0}', 0)"#,
        ]);
        let part = query_open_code(&path, 500).expect("正常库应读到");
        assert_eq!(part.tokens_total, 235, "累计 = 35 + 200;user 行不计");
        assert_eq!(part.tokens24, 35, "24h 只算窗口内的 assistant 行");
        assert!((part.cost_total - 2.5).abs() < 1e-9, "cost={}", part.cost_total);
        assert!((part.cost24 - 0.5).abs() < 1e-9);
        assert!(!part.cost_estimated, "SQLite 的成本是记录值，不是估价");
        assert_eq!(part.models.len(), 2);
        assert_eq!(part.models[0].model, "oc2", "按 token 降序");
        assert_eq!(part.models[0].tokens, 200);
        let oc1 = part.models.iter().find(|m| m.model == "oc1").expect("应有 oc1");
        assert_eq!(oc1.tokens, 35);
        assert!(!oc1.cost_estimated);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn dim_source_subtracts_cache_reads_and_floors_at_zero() {
        let path = temp_db("dim.sqlite");
        seed(&path, &[
            "CREATE TABLE usage_ledger (createdAt TEXT, modelId TEXT, usage TEXT, cost REAL, sessionId TEXT)",
            // (100 - 40) + 50 = 110
            r#"INSERT INTO usage_ledger VALUES ('2026-09-27T00:00:00.000Z', 'm1', '{"promptTokens":100,"cacheReadTokens":40,"completionTokens":50}', 0.10, 's')"#,
            // 缓存命中比 prompt 还多：max(1200-1000,0) = 0，**不许变成负数**
            r#"INSERT INTO usage_ledger VALUES ('2026-09-27T00:00:00.000Z', 'm1', '{"promptTokens":1000,"cacheReadTokens":1200,"completionTokens":0}', 0.05, 's')"#,
            // 窗口外：只进累计
            r#"INSERT INTO usage_ledger VALUES ('2020-01-01T00:00:00.000Z', 'm2', '{"promptTokens":500,"completionTokens":0}', 1.00, 's')"#,
        ]);
        let cutoff = parse_iso_ms("2026-01-01T00:00:00.000Z").unwrap();
        let part = query_dim_tasks(&path, cutoff).expect("正常库应读到");
        assert_eq!(part.tokens_total, 610, "110 + 0 + 500");
        assert_eq!(part.tokens24, 110, "窗口内两行：110 + 0");
        assert!(!part.cost_estimated);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn real_typed_token_columns_do_not_erase_the_whole_statistic() {
        // 一列里只要有一行是 REAL，SUM 就整体变浮点。若直接按 i64 取值会当场失败并让
        // 整份统计消失——正是 Swift 侧「经 Double 中转」防的那件事。
        let path = temp_db("real.db");
        seed(&path, &[
            "CREATE TABLE message (session_id TEXT, data TEXT, time_created INTEGER)",
            r#"INSERT INTO message VALUES ('s', '{"role":"assistant","modelID":"oc1","tokens":{"input":10,"output":0.0,"reasoning":0},"cost":0.5}', 1000)"#,
            r#"INSERT INTO message VALUES ('s', '{"role":"assistant","modelID":"oc1","tokens":{"input":20,"output":0.0,"reasoning":0},"cost":0.5}', 1000)"#,
        ]);
        let part = query_open_code(&path, 0).expect("REAL 列也要读得到，而不是整份消失");
        assert_eq!(part.tokens_total, 30);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_and_drifted_databases_read_as_nothing_without_panicking() {
        let missing = temp_db("gone.db");
        assert!(query_open_code(&missing, 0).is_none());
        assert!(query_dim_tasks(&missing, 0).is_none());

        // 表形漂移（第三方升级换表）：同样算「读不到」，不许当成空库、也不许 panic
        let drift = temp_db("drift.db");
        seed(&drift, &["CREATE TABLE unrelated (a TEXT)"]);
        assert!(query_open_code(&drift, 0).is_none());
        assert!(query_dim_tasks(&drift, 0).is_none());
        let _ = std::fs::remove_file(&drift);
    }

    #[test]
    fn monitor_merges_the_sqlite_source_and_keeps_recorded_cost_unmarked() {
        // 端到端：档案声明了 OpenCode 库 → monitor() 的 TokenReport 必须带上它，
        // 且成本保持「记录值」（不置 cost_estimated），模型拆分也要出来。
        let path = temp_db("e2e.db");
        // time_created 用「现在」：24h 口径才有意义（用绝对小数字会落在 1970 年）
        seed(&path, &[&format!(
            "CREATE TABLE message (session_id TEXT, data TEXT, time_created INTEGER); \
             INSERT INTO message VALUES ('s', '{{\"role\":\"assistant\",\"modelID\":\"oc1\",\
             \"tokens\":{{\"input\":100,\"output\":200,\"reasoning\":300}},\"cost\":1.25}}', {});",
            now_ms()
        )]);
        let profile = crate::models::AgentProfile {
            id: "fixture".into(),
            name: "Fixture".into(),
            glyph: String::new(),
            emoji: String::new(),
            process_names: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: vec![],
            token_roots: vec![],
            session_database: Some(crate::models::SessionDatabase {
                path: path.clone(),
                schema: SessionSchema::OpenCode,
            }),
            category: "assistant".into(),
        };
        let mut monitor = TokenUsageMonitor::new();
        let report = monitor.monitor(&profile);
        assert_eq!(report.usage.tokens_total, 600);
        assert_eq!(report.usage.tokens24h, 600, "刚写入的行应在 24h 窗口内");
        assert!((report.usage.cost_total - 1.25).abs() < 1e-9);
        assert!(!report.usage.cost_estimated, "记录成本不许被标成估价");
        assert_eq!(report.models24h.len(), 1);
        assert_eq!(report.models24h[0].model, "oc1");
        assert!(!report.models24h[0].cost_estimated);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn jsonl_cost_stays_marked_as_an_estimate() {
        // 同一份 TokenReport 里两种成本来源必须分得开：JSONL 是估的、SQLite 是记录的。
        // 上一版靠「非零即估价」这条等价关系，SQLite 一进来它就不成立了。
        let dir = std::env::temp_dir().join(format!(
            "agentisland-tokens-jsonl-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("session.jsonl"), format!("{}\n", claude_line("claude-3-7-sonnet")))
            .unwrap();
        let profile = crate::models::AgentProfile {
            id: "jsonl-fixture".into(),
            name: "J".into(),
            glyph: String::new(),
            emoji: String::new(),
            process_names: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: vec![],
            token_roots: vec![dir.to_string_lossy().to_string()],
            session_database: None,
            category: "assistant".into(),
        };
        let mut monitor = TokenUsageMonitor::new();
        let report = monitor.monitor(&profile);
        assert_eq!(report.usage.tokens_total, 4000);
        assert!(report.usage.cost_estimated, "JSONL 的成本是估出来的，必须带标记");
        assert_eq!(report.models24h.len(), 1);
        assert!(report.models24h[0].cost_estimated, "按模型那一层也要带标记");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_index_schema_contributes_no_tokens() {
        // 状态索引只回答「最新一条状态」：它不是缺数据，是没这个概念——
        // 因此即便库存在、能打开，也不该贡献任何 token
        let path = temp_db("status.db");
        seed(&path, &["CREATE TABLE tasks (id TEXT, task_status TEXT, updated_at TEXT)"]);
        let profile = crate::models::AgentProfile {
            id: "zcode-fixture".into(),
            name: "Z".into(),
            glyph: String::new(),
            emoji: String::new(),
            process_names: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: vec![],
            token_roots: vec![],
            session_database: Some(crate::models::SessionDatabase {
                path: path.clone(),
                schema: SessionSchema::StatusIndex,
            }),
            category: "assistant".into(),
        };
        let mut monitor = TokenUsageMonitor::new();
        let report = monitor.monitor(&profile);
        assert_eq!(report.usage.tokens_total, 0);
        assert_eq!(report.usage.cost_total, 0.0);
        assert!(report.models24h.is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
