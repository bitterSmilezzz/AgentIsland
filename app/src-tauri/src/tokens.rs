use crate::cost;
use crate::models::{ModelUsage, SessionSchema, TokenReport, TokenUsage};
use crate::session;
use crate::sqlite;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Token 用量监控：只读解析会话 JSONL 的结构化 usage 字段
/// （净消耗口径，不含缓存读取），文件指纹增量缓存。
pub struct TokenUsageMonitor {
    states: HashMap<String, FileState>,
}

/// 明细保留窗口。**70 天不是拍的**：分析页最宽 30 天，那一档还要往前读同样长的
/// 「上一周期」做对比，所以取 2×最宽档 + 10 天余量（余量免得刚好掉出边界的明细
/// 被反复折出/折回，折回要重读整份文件）。与 Swift `detailRetention` 同一个数。
const RETENTION_MS: i64 = 70 * 86_400_000;

/// 单文件明细条数硬上限（与 Swift `maxDetailEventsPerFile` 同值）。触顶时折掉**最早**的一段：
/// 明细有界，累计不受影响。
const MAX_DETAIL_PER_FILE: usize = 20_000;

/// 单文件的（类型, 时间, 大小）。**三者任一变化都要重判怎么读**：
/// 只看「变小了」会漏掉等长的原地重写（日志轮转、备份恢复都是这个形状），
/// 只看大小会漏掉「同名同长但换了内容」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    inode: u64,
    mtime_ns: i64,
    size: u64,
}

fn stamp_of(path: &Path) -> Option<Stamp> {
    let meta = fs::metadata(path).ok()?;
    #[cfg(unix)]
    let inode = {
        use std::os::unix::fs::MetadataExt;
        meta.ino()
    };
    #[cfg(not(unix))]
    let inode = 0u64;
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0);
    Some(Stamp { inode, mtime_ns, size: meta.len() })
}

/// 一条待统计的明细：(unix ms, 模型, tokens, cost)
type Entry = (i64, String, i64, f64);

#[derive(Default)]
struct FileState {
    /// 上一次读完之后文件的标识
    stamp: Option<Stamp>,
    /// 窗口内的明细（≤ RETENTION_MS，且 ≤ MAX_DETAIL_PER_FILE 条）
    entries: Vec<Entry>,
    /// 折入合计：掉出窗口或被上限裁掉的那部分。
    /// **累计 = rolled + Σentries** —— 「折入保和」因此是结构上成立的，
    /// 而不是靠两处加减互相对齐（旧实现用累加器，文件被整体重写时会把同一批再加一遍）。
    rolled_tokens: i64,
    rolled_cost: f64,
    /// 折入条数。**非零 ⇒ 不许增量续读**：折入的去重看不见已经折掉的那段键。
    rolled_count: i64,
    /// 增量续读用的去重键。resume/fork 会把同一响应在同一份文件里抄第二遍，
    /// 不挡就会计两次。只在 `rolled_count == 0` 期间保留——一旦有折入就每轮整份重读、
    /// 当轮内去重，这个集合随之清空（内存也就有界了）。
    seen_ids: HashSet<String>,
    /// 上次是否停在整行边界上。false 时不许增量续读（否则半个 JSON 行会被跳过去）。
    ended_with_newline: bool,
}

/// 折入：先按窗口分，再按条数上限裁。**保和**——被折掉的每一条都进合计。
fn fold(events: Vec<Entry>, carry: Vec<Entry>, cutoff_ms: i64) -> (Vec<Entry>, i64, f64, i64) {
    let mut kept: Vec<Entry> = Vec::new();
    let mut tokens = 0i64;
    let mut cost = 0.0f64;
    let mut count = 0i64;
    for event in carry.into_iter().chain(events) {
        if event.0 < cutoff_ms {
            tokens += event.2;
            cost += event.3;
            count += 1;
        } else {
            kept.push(event);
        }
    }
    // 上限只裁明细、不裁累计：折掉**最早**的那一段，留住最近（图表用得上的部分）。
    // 用下标集合而不是先排序，是为了别打乱存留明细的解析顺序。
    if kept.len() > MAX_DETAIL_PER_FILE {
        let mut order: Vec<usize> = (0..kept.len()).collect();
        order.sort_by_key(|&i| kept[i].0);
        let doomed: HashSet<usize> = order.into_iter().take(kept.len() - MAX_DETAIL_PER_FILE).collect();
        let mut survivors: Vec<Entry> = Vec::with_capacity(MAX_DETAIL_PER_FILE);
        for (index, event) in kept.into_iter().enumerate() {
            if doomed.contains(&index) {
                tokens += event.2;
                cost += event.3;
                count += 1;
            } else {
                survivors.push(event);
            }
        }
        kept = survivors;
    }
    (kept, tokens, cost, count)
}

struct UsageLine {
    /// 记录自带的 id（没有则调用方用「文件 + 段起点 + 行号」兜底）
    id: Option<String>,
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

    /// 某个周期内的（tokens, cost）合计。
    ///
    /// 复用同一套解析：`parse_file` 本来就按传入的 cutoff 聚合（`summarize` 里逐条
    /// 比 `ts >= cutoff`），所以周期口径与 24h 那一档不会因为两处各写一份而对不上，
    /// 文件指纹缓存也照旧命中——重复换档查询不会重读文件。
    pub fn range_totals(
        &mut self,
        profile: &crate::models::AgentProfile,
        range_ms: i64,
        now: i64,
    ) -> (i64, f64) {
        let cutoff = now.saturating_sub(range_ms);
        let mut tokens = 0i64;
        let mut cost = 0f64;
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
                let (t, c, _, _, _) = self.parse_file(&entry.path().to_path_buf(), cutoff);
                tokens += t;
                cost += c;
            }
        }
        (tokens, cost)
    }

    fn parse_file(
        &mut self,
        path: &PathBuf,
        cutoff24: i64,
    ) -> (i64, f64, i64, f64, HashMap<String, (i64, f64)>) {
        let key = path.to_string_lossy().to_string();
        let now = now_ms();
        // stat 失败（文件消失 / 读不到）：保留上一轮的值——「没看到」不等于「没有」
        let Some(stamp) = stamp_of(path) else {
            return self.summarize(&key, cutoff24);
        };
        let (start, can_append) = match self.states.get(&key).and_then(|st| st.stamp) {
            // 戳逐字相同 ⇒ 事件集合与顺序都不变，摊平与折入整套跳过
            Some(prev) if prev == stamp => return self.summarize(&key, cutoff24),
            Some(prev) => {
                let st = self.states.get(&key).unwrap();
                // 增量续读四个条件缺一不可：没折过东西（折入的去重看不见已折掉的键）、
                // 同一个 inode、上次停在整行边界、文件确实变长了
                let can_append = st.rolled_count == 0
                    && st.ended_with_newline
                    && prev.inode == stamp.inode
                    && stamp.size > prev.size;
                (if can_append { prev.size } else { 0 }, can_append)
            }
            None => (0, false),
        };

        let mut collected: Vec<(u64, UsageLine)> = Vec::new();
        let mut index: u64 = 0;
        let consumed = session::for_each_complete_line(path, start, |line| {
            let line_index = index;
            index += 1;
            if line.len() < 8 {
                return;
            }
            if let Some(u) = parse_usage_line(line) {
                collected.push((line_index, u));
            }
        });

        {
            let st = self.states.entry(key.clone()).or_default();
            if !can_append {
                // 整份重读：明细与折入合计都按这一遍重建，去重也在这遍内完成
                st.entries.clear();
                st.rolled_tokens = 0;
                st.rolled_cost = 0.0;
                st.rolled_count = 0;
                st.seen_ids.clear();
            }
            let mut fresh: Vec<Entry> = Vec::new();
            for (line_index, u) in collected {
                // 去重键：优先用记录自带 id（resume/fork 会把同一响应抄第二遍）；
                // 没有 id 的行用「文件 + 段起点 + 行号」兜底——与 Swift 的 fallbackId 同构
                let dedup = match &u.id {
                    Some(id) => id.clone(),
                    None => format!("{key}#{start}-{line_index}"),
                };
                if !st.seen_ids.insert(dedup) {
                    continue; // 同一响应已经计过
                }
                fresh.push((u.ts_ms, u.model, u.tokens, u.cost));
            }
            let carry = if can_append {
                std::mem::take(&mut st.entries)
            } else {
                Vec::new()
            };
            let (kept, folded_tokens, folded_cost, folded_count) =
                fold(fresh, carry, now - RETENTION_MS);
            st.entries = kept;
            st.rolled_tokens += folded_tokens;
            st.rolled_cost += folded_cost;
            st.rolled_count += folded_count;
            if st.rolled_count > 0 {
                // 有折入 ⇒ 之后每轮走整份重读、当轮内去重，跨轮去重集不再需要（内存随之有界）
                st.seen_ids.clear();
            }
            if let Some((bytes, ended)) = consumed {
                // size 记**真正承认到的字节数**而不是 stat 到的大小：stat 与读之间文件
                // 还可能被追加，记大了下一轮的 offset 就落在从未解析过的字节之后
                st.stamp = Some(Stamp {
                    inode: stamp.inode,
                    mtime_ns: stamp.mtime_ns,
                    size: bytes,
                });
                st.ended_with_newline = ended;
            }
        }
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
        self.summarize(&key, cutoff24)
    }

    /// 由「折入合计 + 窗口内明细」汇总出这一轮的口径。
    /// 累计**每次重算**而不是往上累加：这既是「折入保和」的保证，也是「文件被整体重写
    /// 不重复计数」的保证（旧实现用累加器，同一份文件重读一遍就把同一批用量又加了一次）。
    fn summarize(
        &self,
        key: &str,
        cutoff24: i64,
    ) -> (i64, f64, i64, f64, HashMap<String, (i64, f64)>) {
        let Some(st) = self.states.get(key) else {
            return (0, 0.0, 0, 0.0, HashMap::new());
        };
        let mut models: HashMap<String, (i64, f64)> = HashMap::new();
        let mut tokens24 = 0i64;
        let mut cost24 = 0f64;
        let mut detail_tokens = 0i64;
        let mut detail_cost = 0f64;
        for (ts, model, tokens, cost) in &st.entries {
            detail_tokens += tokens;
            detail_cost += cost;
            if *ts >= cutoff24 {
                tokens24 += tokens;
                cost24 += cost;
                let e = models.entry(model.clone()).or_insert((0, 0.0));
                e.0 += tokens;
                e.1 += cost;
            }
        }
        (
            tokens24,
            cost24,
            st.rolled_tokens + detail_tokens,
            st.rolled_cost + detail_cost,
            models,
        )
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 各方言的**记录形状**（对齐 Swift `StructuredTokenUsageIndex.parse`）：
/// · Anthropic/Claude：`{"timestamp":…,"id"|"uuid":…,"message":{"model":…,"usage":{…}}}`
/// · Codex：`{"type":"token_usage_record","timestamp":…,"payload":{"response_id":…,"usage":{…}}}`
///   ——同一份日志里**还有另一族** `event_msg.payload.type == "token_count"`；本机 26 份实测
///   两族数值相差 0.3%、条数几乎一一对应，所以**只认一族**，免得同一笔用量被计两遍。
/// · ZCode rollout：`{"requestId":…,"model":{"modelId":…},"response":{"usage":{…}}}`
fn parse_usage_line(line: &str) -> Option<UsageLine> {
    // 日志里 99% 以上的行是对话正文：先在字符串上找标记，命中才解析 JSON
    if !line.contains("\"usage\"") {
        return None;
    }
    let doc: Value = serde_json::from_str(line).ok()?;
    let obj = doc.as_object()?;

    let (usage, model, cached_key, id): (Value, Option<String>, &str, Option<String>) =
        if let Some(msg) = obj.get("message") {
            let usage = msg.get("usage")?.clone();
            let model = msg.get("model").and_then(|v| v.as_str()).map(|s| s.to_string());
            let id = obj
                .get("id")
                .or_else(|| obj.get("uuid"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            (usage, model, "cache_read_input_tokens", id)
        } else if obj.get("type").and_then(|v| v.as_str()) == Some("token_usage_record") {
            let payload = obj.get("payload")?;
            let usage = payload.get("usage")?.clone();
            let id = payload
                .get("response_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            // 这一族记录里**没有模型名**（payload 只有 response_id / turn_id / usage 一族），
            // 所以落 unknown——旧实现写死 "gpt-5"，那是编的
            (usage, None, "cached_input_tokens", id)
        } else if let Some(resp) = obj.get("response") {
            // ZCode rollout：response.usage {inputTokens, outputTokens, cacheRead/WriteTokens}
            let usage = resp.get("usage")?.clone();
            let model = obj
                .get("model")
                .and_then(|m| m.get("modelId"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let id = obj
                .get("requestId")
                .or_else(|| resp.get("responseId"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            (usage, model, "cacheReadTokens", id)
        } else {
            return None;
        };

    let get = |key: &str| usage.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
    // snake_case 与 camelCase 两种拼法都认（实测各方言各用一种：Anthropic/Codex 用
    // `input_tokens`，ZCode 用 `inputTokens`）
    let input = get("input_tokens").max(get("inputTokens"));
    let output = get("output_tokens").max(get("outputTokens"));
    let cached = get(cached_key);
    // 净消耗 = **未命中缓存的输入** + 输出。与 Swift `netTokens` 逐字同口径：
    // `max(input - min(cached, input), 0) + max(output, 0)`。
    // 旧实现写的是 `input + output + cache_write`——把缓存命中的上下文当新输入全额计。
    // 本机 26 份真实 codex 日志实测因此虚高 **29 倍**（3.9M → 114M），
    // 见 docs/research/2026-09-27-jsonl-net-token-formula.md。
    let net = (input - cached.min(input)).max(0) + output.max(0);
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

    Some(UsageLine { id, tokens: net, cost, model: name, ts_ms })
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
///
/// **表名由 [`crate::sqlite::OpenCodeTables`] 现查**（老库 `message`，
/// 当前版本 `session_message`）。写死表名的后果不是报错，是**读到零**——
/// 那在界面上与「这个 Agent 真的没用过」完全一样。
fn query_open_code(path: &str, cutoff24: i64) -> Option<UsagePart> {
    let connection = sqlite::open_readonly(path).ok()?;
    let tables = sqlite::OpenCodeTables::resolve(&connection)?;
    let (tokens_total, cost_total, tokens24, cost24) = connection
        .query_row(
            &format!(
                "SELECT COALESCE(SUM(t),0), COALESCE(SUM(c),0), \
                        COALESCE(SUM(CASE WHEN time_created >= ?1 THEN t ELSE 0 END),0), \
                        COALESCE(SUM(CASE WHEN time_created >= ?1 THEN c ELSE 0 END),0) \
                 FROM ( \
                     SELECT time_created, \
                            COALESCE(json_extract(data,'$.cost'),0) AS c, \
                            COALESCE(json_extract(data,'$.tokens.input'),0) \
                              + COALESCE(json_extract(data,'$.tokens.output'),0) \
                              + COALESCE(json_extract(data,'$.tokens.reasoning'),0) AS t \
                     FROM {} \
                     WHERE json_extract(data,'$.role')='assistant' \
                 )",
                tables.message
            ),
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
        &format!(
            "SELECT json_extract(data,'$.modelID') AS model, \
                    COALESCE(SUM(COALESCE(json_extract(data,'$.tokens.input'),0) \
                      + COALESCE(json_extract(data,'$.tokens.output'),0) \
                      + COALESCE(json_extract(data,'$.tokens.reasoning'),0)),0), \
                    COALESCE(SUM(json_extract(data,'$.cost')),0) \
             FROM {} WHERE json_extract(data,'$.role')='assistant' \
             GROUP BY 1 ORDER BY 2 DESC",
            tables.message
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

/// 紧凑显示（Swift `TokenUsage.compact` 同口径、同阈值）：
/// 10 亿以上 `1.23B`、百万以上 `1.23M`、**一万以上** `1.2k`、其余原样整数。
///
/// 阈值与卡片、汇总栏共用一处——同一份用量在卡片上写 `1.2M`、在告警里写 `1200000`
/// 会让人以为是两件事。
pub fn compact(n: i64) -> String {
    // 唯一的**有意差异**：负数钳成 0。Swift 原样打印（`-1`），但负用量只可能来自
    // 统计回绕或数据源写入异常，把它显示出来等于说谎。其余（阈值、后缀、小数位、
    // `12.00M` 这种保留尾零）一律照搬。
    let n = n.max(0);
    let value = n as f64;
    if n >= 1_000_000_000 {
        return format!("{:.2}B", value / 1_000_000_000.0);
    }
    if n >= 1_000_000 {
        return format!("{:.2}M", value / 1_000_000.0);
    }
    if n >= 10_000 {
        return format!("{:.1}k", value / 1_000.0);
    }
    format!("{n}")
}


/// 本地时间的分量 `(年, 月, 日, 时, 分, 秒)`。审计报告与预估都要按**用户的钟表**说话，
/// 所以只在这一处读本地时区，别处不许再各写一遍（时区一分散就会有人忘掉偏移）。
pub fn local_time_parts(now_ms: i64) -> Option<(i32, i32, i32, i32, i32, i32)> {
    let seconds = now_ms.div_euclid(1000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&seconds, &mut tm) }.is_null() {
        return None;
    }
    Some((
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
    ))
}


#[cfg(test)]
mod compact_tests {
    use super::compact;

    /// 阈值、后缀与小数位照搬 Swift `TokenUsage.compact`。
    ///
    /// **这一轮修的是一个真实的分歧**：Rust 侧曾另有一份 `engine::compact`
    /// （1000 就打 `k`、十亿用 `G`、还剥尾零），而 Swift 全仓只有这一个函数，
    /// 界面上所有 token 文本（卡片、悬停、热力图、详情页、CLI）都走它。
    /// 两份实现同时存在时，「同一份用量在两处显示不同」只是时间问题——
    /// 而它当时已经用在一句 Swift 也用参考函数构造的告警里。
    #[test]
    fn thresholds_and_suffixes_match_the_swift_reference() {
        assert_eq!(compact(0), "0");
        assert_eq!(compact(999), "999");
        assert_eq!(compact(9_999), "9999", "一万以下不打 k（旧实现从 1000 就打）");
        assert_eq!(compact(10_000), "10.0k");
        assert_eq!(compact(820_000), "820.0k");
        assert_eq!(compact(1_000_000), "1.00M");
        assert_eq!(
            compact(12_000_000),
            "12.00M",
            "M 固定两位小数（Swift 保留 .00；旧实现会剥成 12M）"
        );
        assert_eq!(compact(1_000_000_000), "1.00B", "十亿是 B 不是 G");
        assert_eq!(compact(2_500_000_000), "2.50B");
    }

    #[test]
    fn the_unit_only_ever_goes_up_as_the_number_grows() {
        let seq = [0i64, 999, 9_999, 10_000, 999_999, 1_000_000, 12_345_678, 1_000_000_000];
        let rank = |unit: char| match unit {
            'B' => 3,
            'M' => 2,
            'k' => 1,
            _ => 0,
        };
        let mut previous = 0;
        for value in seq {
            let text = compact(value);
            let unit = text.chars().rev().find(|c| c.is_ascii_alphabetic()).unwrap_or(' ');
            assert!(
                rank(unit) >= previous,
                "量级倒退了：{value} → {text}（上一个量级 {previous}）"
            );
            previous = rank(unit);
            assert!(!text.trim().is_empty());
            assert!(text.len() <= 8, "compact({value}) = {text:?} 过长");
        }
    }

    /// 唯一的**有意差异**：负数显示 0。Swift 原样打 `-1`，但负用量只可能来自统计回绕
    /// 或数据源写入异常，显示出来等于说谎。
    #[test]
    fn a_negative_reading_shows_zero_instead_of_a_negative_number() {
        assert_eq!(compact(-1), "0");
        assert_eq!(compact(-999_999), "0");
    }
}

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

    // ── JSONL 净口径与去重（对齐 StructuredTokenUsageIndex）────────────────────

    fn codex_record(input: i64, cached: i64, output: i64, response_id: &str) -> String {
        codex_record_at(input, cached, output, response_id, "2020-01-01T00:00:00.000Z")
    }

    fn codex_record_at(
        input: i64,
        cached: i64,
        output: i64,
        response_id: &str,
        iso: &str,
    ) -> String {
        format!(
            r#"{{"type":"token_usage_record","timestamp":"{iso}","payload":{{"response_id":"{response_id}","usage":{{"input_tokens":{input},"cached_input_tokens":{cached},"output_tokens":{output},"cache_write_input_tokens":0}}}}}}"#
        )
    }

    /// 沙箱取名带**序号**：`as_nanos()` 在 macOS 上分辨率很粗，并行用例会撞名，
    /// 于是两个用例共用一个目录、先结束的把另一个的文件删掉（表现为「偶发挂几条」）。
    static NEXT_SANDBOX: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    pub(super) fn temp_dir(tag: &str) -> std::path::PathBuf {
        let serial = NEXT_SANDBOX.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "agentisland-tokens-{tag}-{}-{}-{serial}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sum_hourly(report: &TokenReport) -> i64 {
        report.hourly30d.iter().map(|(_, v)| *v).sum()
    }

    fn one_root_profile(id: &str, root: &str) -> crate::models::AgentProfile {
        crate::models::AgentProfile {
            bundle_ids: vec![],
            id: id.into(),
            name: "Fixture".into(),
            glyph: String::new(),
            emoji: String::new(),
            process_names: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: None,
            session_dirs: vec![],
            token_roots: vec![root.to_string()],
            token_alert_floor: None,
            session_database: None,
            category: "assistant".into(),
        }
    }

    #[test]
    fn net_tokens_exclude_cache_hits_like_swift() {
        // 本机真实一行的数值（codex）：input 43449 / cached 41728 / output 200。
        // Swift 口径 = max(input-cached,0)+output = 1921；
        // 旧 Rust 口径 = input+output+cache_write = 43649（虚高 23 倍，整批 29 倍）。
        let parsed = parse_usage_line(&codex_record(43_449, 41_728, 200, "resp-1")).expect("应解析");
        assert_eq!(parsed.tokens, 1_921, "缓存命中的上下文不许当新输入全额计");
        assert_ne!(parsed.tokens, 43_649, "43649 是被修掉的那个数");
        // 缓存命中比输入还多时下限为 0，不许出现负数
        let clamped = parse_usage_line(&codex_record(100, 9_999, 40, "resp-2")).expect("应解析");
        assert_eq!(clamped.tokens, 40);
    }

    #[test]
    fn codex_reads_only_the_token_usage_record_family() {
        // 同一份日志里两族并存（本机 26 份里 21 份两者都有）：只认一族，否则同一笔用量计两遍
        let other_family = r#"{"type":"event_msg","timestamp":"2020-01-01T00:00:00.000Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":1000,"cached_input_tokens":900,"output_tokens":50}}}}"#;
        assert!(parse_usage_line(other_family).is_none(), "另一族不得再被计入");

        let parsed = parse_usage_line(&codex_record(1_000, 900, 50, "resp-7")).expect("应解析");
        assert_eq!(parsed.tokens, 150);
        assert_eq!(parsed.id.as_deref(), Some("resp-7"), "去重键取自 payload.response_id");
        // 这一族记录里没有模型名：落 unknown，不许编一个
        assert_eq!(parsed.model, "unknown");
    }

    #[test]
    fn anthropic_uses_its_own_cache_key_and_falls_back_to_uuid() {
        let line = r#"{"id":"msg_1","timestamp":"2020-01-01T00:00:00.000Z","message":{"model":"claude-3-7-sonnet","usage":{"input_tokens":5000,"cache_read_input_tokens":4800,"cache_creation_input_tokens":700,"output_tokens":120}}}"#;
        let parsed = parse_usage_line(line).expect("应解析");
        // (5000-4800) + 120 = 320；cache_creation **不**进净消耗
        assert_eq!(parsed.tokens, 320);
        assert_eq!(parsed.id.as_deref(), Some("msg_1"));

        let by_uuid = r#"{"uuid":"u-9","timestamp":"2020-01-01T00:00:00.000Z","message":{"usage":{"input_tokens":10,"output_tokens":5}}}"#;
        assert_eq!(
            parse_usage_line(by_uuid).unwrap().id.as_deref(),
            Some("u-9"),
            "没有 id 时用 uuid 兜底"
        );
    }

    #[test]
    fn zcode_rollout_keeps_its_own_shape_and_cache_key() {
        let line = r#"{"requestId":"req-7","completedAt":"2020-01-01T00:00:00.000Z","model":{"modelId":"glm-4"},"response":{"responseId":"r1","usage":{"inputTokens":900,"cacheReadTokens":850,"outputTokens":30,"cacheWriteTokens":10}}}"#;
        let parsed = parse_usage_line(line).expect("应解析");
        // (900-850) + 30 = 80；cacheWrite 不进净消耗
        assert_eq!(parsed.tokens, 80);
        assert_eq!(parsed.id.as_deref(), Some("req-7"));
        assert_eq!(parsed.model, "glm-4");
    }

    #[test]
    fn lines_without_a_usage_marker_are_skipped_without_panicking() {
        // 日志里 99% 以上是对话正文：不含 "usage" 的行直接跳过
        assert!(parse_usage_line(r#"{"type":"message","text":"hello world"}"#).is_none());
        assert!(parse_usage_line("not json at all").is_none());
        assert!(parse_usage_line("").is_none());
    }

    #[test]
    fn duplicate_responses_in_one_file_are_counted_once() {
        // resume/fork 会把同一响应在同一份文件里抄第二遍：靠记录自带的 id 去重
        let dir = temp_dir("dedup");
        let a = codex_record(1_000, 900, 50, "resp-A"); // 净 150
        let b = codex_record(2_000, 1_900, 100, "resp-B"); // 净 200
        std::fs::write(dir.join("session.jsonl"), format!("{a}\n{a}\n{b}\n")).unwrap();

        let mut monitor = TokenUsageMonitor::new();
        let report = monitor.monitor(&one_root_profile("dedup", &dir.to_string_lossy()));
        assert_eq!(report.usage.tokens_total, 350, "同一 response_id 抄两遍只算一次");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ── 保留窗口 / 折入 / 上限（对齐 StructuredTokenUsageIndex 的保留口径）──────

    #[test]
    fn rewriting_a_file_does_not_double_count_its_usage() {
        // 旧实现用「累加器 + 只在 size 变小时重置游标」：文件被整体重写后会把同一批用量
        // 再加一遍（这里是 150+110=260），而正确答案是「按现在这一份算，110」。
        let dir = temp_dir("rewrite");
        let file = dir.join("session.jsonl");
        std::fs::write(&file, format!("{}\n", codex_record(10_000, 9_000, 1_000, "old-long-id"))).unwrap();
        let mut monitor = TokenUsageMonitor::new();
        let profile = one_root_profile("rewrite", &dir.to_string_lossy());
        assert_eq!(monitor.monitor(&profile).usage.tokens_total, 2_000);

        // 截短并换内容（size 变小）
        std::fs::write(&file, format!("{}\n", codex_record(200, 100, 10, "b"))).unwrap();
        assert_eq!(
            monitor.monitor(&profile).usage.tokens_total,
            110,
            "整体重写后应按现在这一份算，而不是把旧的也留着"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_same_size_in_place_rewrite_is_still_detected() {
        // 日志轮转/备份恢复会出现「等长但内容换过」：只看 size 的判定会一直用旧值
        let dir = temp_dir("same-size");
        let file = dir.join("session.jsonl");
        let profile = one_root_profile("same-size", &dir.to_string_lossy());
        let first = format!("{}\n", codex_record(1_000, 900, 50, "aaaa")); // 净 150
        std::fs::write(&file, &first).unwrap();
        let mut monitor = TokenUsageMonitor::new();
        assert_eq!(monitor.monitor(&profile).usage.tokens_total, 150);

        std::thread::sleep(std::time::Duration::from_millis(10));
        let second = format!("{}\n", codex_record(1_000, 900, 60, "cccc")); // 净 160，**等长**
        assert_eq!(second.len(), first.len(), "前置不成立：两份内容必须等长");
        std::fs::write(&file, &second).unwrap();
        assert_eq!(
            monitor.monitor(&profile).usage.tokens_total,
            160,
            "等长原地重写必须靠 mtime/inode 发现，不能只看大小"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detail_window_covers_the_whole_thirty_day_chart() {
        // 旧实现只留 7 天明细，于是 30 天的逐小时桶实际只画得出一周。
        // 这里放 29 天前 / 8 天前 / 刚刚三条：三条都必须在 30 天桶里。
        let dir = temp_dir("window");
        let now = now_ms();
        let day = 86_400_000i64;
        let lines = [
            codex_record_at(1_000, 900, 50, "d29", &iso_utc_from_ms(now - 29 * day)),
            codex_record_at(1_000, 900, 50, "d8", &iso_utc_from_ms(now - 8 * day)),
            codex_record_at(1_000, 900, 50, "d0", &iso_utc_from_ms(now - 60_000)),
        ]
        .join("\n");
        std::fs::write(dir.join("session.jsonl"), format!("{lines}\n")).unwrap();

        let mut monitor = TokenUsageMonitor::new();
        let report = monitor.monitor(&one_root_profile("window", &dir.to_string_lossy()));
        assert_eq!(report.usage.tokens_total, 450, "三条都要进累计");
        assert_eq!(sum_hourly(&report), 450, "29 天前与 8 天前那两条也得画得出来");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folding_out_of_window_detail_is_conservative() {
        // ADR 0007 的核心不变式：折的是明细，不是数字。掉出窗口的那条只进累计、不进图表。
        let dir = temp_dir("fold");
        let now = now_ms();
        let lines = [
            codex_record(1_000, 900, 50, "ancient"), // 2020-01-01，必然掉出 70 天窗口 → 净 150
            codex_record_at(1_000, 900, 50, "recent", &iso_utc_from_ms(now - 60_000)), // 净 150
        ]
        .join("\n");
        std::fs::write(dir.join("session.jsonl"), format!("{lines}\n")).unwrap();

        let mut monitor = TokenUsageMonitor::new();
        let report = monitor.monitor(&one_root_profile("fold", &dir.to_string_lossy()));
        assert_eq!(report.usage.tokens_total, 300, "折入保和：掉出窗口的那条仍计累计");
        assert_eq!(sum_hourly(&report), 150, "图表只画窗口内的那条");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn per_file_detail_cap_trims_the_oldest_without_touching_totals() {
        // 单文件明细硬上限：触顶折掉最早的，累计一条不少。
        // 期望值**写字面量**而不是引用 `MAX_DETAIL_PER_FILE`：写成常量的话，改常量时期望跟着变，
        // 这条用例就永远绿——反向验证时它确实没红，才改成现在这样（20_000 是定案口径）。
        const CAP: i64 = 20_000;
        const LINES: i64 = CAP + 1;
        let dir = temp_dir("cap");
        let now = now_ms();
        let mut body = String::new();
        for i in 0..LINES {
            body.push_str(&codex_record_at(
                1_000,
                900,
                50,
                &format!("r{i}"),
                &iso_utc_from_ms(now - 60_000 + i),
            ));
            body.push('\n');
        }
        std::fs::write(dir.join("session.jsonl"), body).unwrap();

        let mut monitor = TokenUsageMonitor::new();
        let report = monitor.monitor(&one_root_profile("cap", &dir.to_string_lossy()));
        let per = 150i64;
        assert_eq!(report.usage.tokens_total, per * LINES, "累计不受上限影响");
        assert_eq!(
            sum_hourly(&report),
            per * CAP,
            "明细被裁到上限，图表少画最早那一条（改成 20_000 以外就红了）"
        );
        let _ = std::fs::remove_dir_all(&dir);
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
    fn the_current_open_code_schema_reads_the_same_numbers_as_the_legacy_one() {
        // 同一份数据、两种表名，读出来的数必须**逐项相同**。
        // 写死 `FROM message` 时当前版本会安静地返回零——那在界面上与
        // 「这个 Agent 真的没用过」长得一模一样，所以这条要钉住。
        let rows = [
            r#"INSERT INTO {T} VALUES ('s1', '{"role":"assistant","modelID":"oc1","tokens":{"input":10,"output":20,"reasoning":5},"cost":0.5}', 1000)"#,
            r#"INSERT INTO {T} VALUES ('s2', '{"role":"user","modelID":"oc1","tokens":{"input":50000},"cost":9.9}', 1000)"#,
        ];

        let legacy = temp_db("legacy-oc.db");
        seed(&legacy, &[
            "CREATE TABLE message (session_id TEXT, data TEXT, time_created INTEGER)",
            &rows[0].replace("{T}", "message"),
            &rows[1].replace("{T}", "message"),
        ]);
        let current = temp_db("current-oc.db");
        seed(&current, &[
            "CREATE TABLE session_message (session_id TEXT, data TEXT, time_created INTEGER)",
            &rows[0].replace("{T}", "session_message"),
            &rows[1].replace("{T}", "session_message"),
        ]);

        let legacy_part = query_open_code(&legacy, 500).expect("老 schema 应读到");
        let current_part = query_open_code(&current, 500).expect("当前 schema 也应读到");
        assert_eq!(current_part.tokens_total, legacy_part.tokens_total);
        assert_eq!(current_part.tokens24, legacy_part.tokens24);
        assert_eq!(current_part.cost_total, legacy_part.cost_total);
        assert_eq!(current_part.models.len(), legacy_part.models.len());
        assert_eq!(
            (current_part.tokens_total, current_part.tokens24),
            (35, 35),
            "assistant 35 计入、user 行不计"
        );
        let _ = std::fs::remove_file(&legacy);
        let _ = std::fs::remove_file(&current);
    }

    #[test]
    fn a_database_without_the_message_table_is_reported_as_unreadable() {
        // 认不出来要明说「读不到」，而不是给一个全是零的统计——那会被当成「真的零用量」。
        let path = temp_db("not-oc.db");
        seed(&path, &["CREATE TABLE kv (k TEXT)"]);
        assert!(query_open_code(&path, 500).is_none());
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
            bundle_ids: vec![],
            id: "fixture".into(),
            name: "Fixture".into(),
            glyph: String::new(),
            emoji: String::new(),
            process_names: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: None,
            session_dirs: vec![],
            token_roots: vec![],
            token_alert_floor: None,
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
            bundle_ids: vec![],
            id: "jsonl-fixture".into(),
            name: "J".into(),
            glyph: String::new(),
            emoji: String::new(),
            process_names: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: None,
            session_dirs: vec![],
            token_roots: vec![dir.to_string_lossy().to_string()],
            token_alert_floor: None,
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
            bundle_ids: vec![],
            id: "zcode-fixture".into(),
            name: "Z".into(),
            glyph: String::new(),
            emoji: String::new(),
            process_names: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: None,
            session_dirs: vec![],
            token_roots: vec![],
            token_alert_floor: None,
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


#[cfg(test)]
mod range_tests {
    use super::*;
    use crate::models::AgentProfile;

    /// 带时间戳的 Claude 方言一行（4000 净 tokens ⇒ $0.024）
    fn line_at(iso: &str) -> String {
        format!(
            r#"{{"timestamp":"{iso}","id":"msg-{iso}","message":{{"model":"claude-3-7-sonnet","usage":{{"input_tokens":3000,"output_tokens":1000}}}}}}"#
        )
    }

    /// 复用 `tests::temp_dir`（**一处实现**：沙箱命名要带序号这件事只能有一个地方知道）
    fn temp_dir(tag: &str) -> std::path::PathBuf {
        super::tests::temp_dir(tag)
    }

    /// 把本地时间的分量写成 ISO 串。**它按 UTC 解析，所以与实际时刻最多差一个时区**
    /// （≤14h）——本用例的间距是 1 小时与 14 天，这点偏差不影响任何一条断言。
    /// 之所以不写固定字面量：`parse_file` 用**真实时钟**算 70 天明细保留窗口，
    /// 夹具时间戳离真实 now 太远（我第一版放在 8 个月前）会被折进累计、不再逐条保留，
    /// 于是区间查询全返回 0——那不是被测代码的问题，是夹具没贴近现实。
    fn iso_like(ms: i64) -> String {
        let (y, mo, d, h, mi, s) = local_time_parts(ms).expect("本地时间应可用");
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
    }

    /// 区间合计必须**按传入的 cutoff 分档**：同一份明细里，24h / 7d / 30d 要给出不同的数。
    #[test]
    fn the_range_cutoff_actually_separates_recent_from_old_detail() {
        let dir = temp_dir("cutoff");
        let real_now = now_ms();
        // 相隔 14 天的两条明细，都落在 70 天保留窗口内
        let recent_iso = iso_like(real_now - 3_600_000);
        let old_iso = iso_like(real_now - 14 * 86_400_000);
        std::fs::write(
            dir.join("session.jsonl"),
            format!("{}\n{}\n", line_at(&recent_iso), line_at(&old_iso)),
        )
        .unwrap();

        // 「现在」由用例给（`range_totals` 收 `now` 参数就是为了这个）
        let now = real_now;

        let profile = AgentProfile {
            bundle_ids: vec![],
            id: "fixture".into(),
            name: "Fixture".into(),
            glyph: String::new(),
            emoji: String::new(),
            process_names: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: None,
            session_dirs: vec![],
            token_roots: vec![dir.to_string_lossy().to_string()],
            token_alert_floor: None,
            session_database: None,
            category: "assistant".into(),
        };

        let mut monitor = TokenUsageMonitor::new();
        let (day_tokens, day_cost) = monitor.range_totals(&profile, 24 * 3_600_000, now);
        assert_eq!(day_tokens, 4_000, "24h 档只该含 1 小时前那条");
        assert!(day_cost > 0.0, "成本也要随区间缩放：{day_cost}");

        let (week_tokens, _) = monitor.range_totals(&profile, 7 * 86_400_000, now);
        assert_eq!(week_tokens, 4_000, "7 天档仍只含最近那条（另一条是 14 天前）");

        let (month_tokens, month_cost) = monitor.range_totals(&profile, 30 * 86_400_000, now);
        assert_eq!(month_tokens, 8_000, "30 天档两条都在");
        assert!(
            (month_cost - day_cost * 2.0).abs() < 1e-9,
            "两条同价 ⇒ 30 天应是 24h 的两倍：{month_cost} vs {day_cost}"
        );

        // 换档不重读文件：同一台 monitor 再查一次 24h，结果必须一致
        let (again, _) = monitor.range_totals(&profile, 24 * 3_600_000, now);
        assert_eq!(again, day_tokens, "重复查询（命中缓存）结果必须一致");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_profile_without_readable_roots_reports_zero_instead_of_panicking() {
        let profile = AgentProfile {
            bundle_ids: vec![],
            id: "empty".into(),
            name: "Empty".into(),
            glyph: String::new(),
            emoji: String::new(),
            process_names: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: None,
            session_dirs: vec![],
            token_roots: vec!["/nonexistent/agentisland-range".into()],
            token_alert_floor: None,
            session_database: None,
            category: "assistant".into(),
        };
        let mut monitor = TokenUsageMonitor::new();
        assert_eq!(monitor.range_totals(&profile, 24 * 3_600_000, now_ms()), (0, 0.0));
    }
}
