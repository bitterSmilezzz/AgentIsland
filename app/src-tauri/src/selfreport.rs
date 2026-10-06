//! 可信自报的状态机（Swift `SelfReport.swift` 的状态部分）。
//!
//! 这一层管的是「谁在什么时候说了什么、这句话还有没有保质期」，**不管 HTTP**。
//! 三条照搬的规则，每条都有它要防的症状：
//!
//! ① **TTL 钳在 [15, 600] 秒，缺省 90**；非正数也算缺省。放任调用方自报一个
//!    「一万秒内我都算在工作」等于让自报变成永久标签。
//! ② **过期只盖戳，不删记录**。「过期」的语义是「不再采信」，不是「没说过」——
//!    删掉之后就没法解释「刚才那句话为什么显示过」。
//! ③ **文本截断到 200 字**（`detail` / `ask` 各自），不静默整条丢弃。
//!
//! 与 `ActivityLevel` 的关系：自报只有四种状态（working / attention / completed / idle）——
//! `offline` **不在其中**，那是观测才有的结论。

use crate::models::ActivityLevel;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 自报允许的最短/最长保质期（毫秒）与缺省值
pub const TTL_MIN_MS: i64 = 15_000;
pub const TTL_MAX_MS: i64 = 600_000;
pub const DEFAULT_TTL_MS: i64 = 90_000;
/// `detail` / `ask` 的截断上限（字符数，不是字节——中文按字算）
pub const TEXT_LIMIT: usize = 200;

/// 自报的状态。四种，与 `ActivityLevel` 的四档一一对应（不含 `offline`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReportedState {
    Working,
    Attention,
    Completed,
    Idle,
}

impl ReportedState {
    pub fn as_str(self) -> &'static str {
        match self {
            ReportedState::Working => "working",
            ReportedState::Attention => "attention",
            ReportedState::Completed => "completed",
            ReportedState::Idle => "idle",
        }
    }

    /// 认不出就 `None`（调用方据此拒绝，而不是猜一个）
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_lowercase().as_str() {
            "working" => Some(ReportedState::Working),
            "attention" => Some(ReportedState::Attention),
            "completed" => Some(ReportedState::Completed),
            "idle" => Some(ReportedState::Idle),
            _ => None,
        }
    }

    /// 与卡片那个 `ActivityLevel` 的对应关系（Swift `SelfReportState.level` 同义）。
    /// 文案不在这里抄一遍：界面显示的是 `ActivityLevel.label`，两边叫法必须逐字相同。
    pub fn level(self) -> ActivityLevel {
        match self {
            ReportedState::Working => ActivityLevel::Working,
            ReportedState::Attention => ActivityLevel::Attention,
            ReportedState::Completed => ActivityLevel::Completed,
            ReportedState::Idle => ActivityLevel::Idle,
        }
    }
}

/// 一条待登记的申报（已归一化）
#[derive(Debug, Clone, PartialEq)]
pub struct Submission {
    pub agent_id: String,
    pub session_id: String,
    pub pid: Option<i32>,
    pub state: ReportedState,
    pub detail: Option<String>,
    pub ask: Option<String>,
    pub ttl_ms: i64,
}

/// 拒绝的理由。**分开而不是合成一句话**：调用方要按理由决定回什么状态码，
/// 而 `/session` 的响应本身也是一条对外契约。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// 缺 `agent`/`session`，或 JSON 解不开
    Malformed,
    /// `state` 不在四个值里
    UnknownState,
    /// 收不到（或不合格）的 pid：**不静默当作没提**
    BadPid,
}

impl Rejection {
    pub fn reason(self) -> &'static str {
        match self {
            Rejection::Malformed => "malformed",
            Rejection::UnknownState => "unknownState",
            Rejection::BadPid => "badPid",
        }
    }
}

impl Submission {
    /// 归一化：裁空白、截断过长文本、钳 TTL。
    pub fn new(
        agent_id: String,
        session_id: String,
        pid: Option<i32>,
        state: ReportedState,
        detail: Option<String>,
        ask: Option<String>,
        ttl_ms: Option<i64>,
    ) -> Self {
        Submission {
            agent_id: agent_id.trim().to_string(),
            session_id: session_id.trim().to_string(),
            pid: pid.filter(|p| *p > 0),
            state,
            detail: clamp_text(detail),
            ask: clamp_text(ask),
            ttl_ms: clamp_ttl(ttl_ms),
        }
    }

    pub fn validate(&self) -> Result<(), Rejection> {
        if self.agent_id.is_empty() || self.session_id.is_empty() {
            return Err(Rejection::Malformed);
        }
        Ok(())
    }
}

/// TTL 钳制：缺失或非正数取缺省，其余钳进区间。
/// 用毫秒做单位（本仓的时间口径统一是 unix 毫秒）。
pub fn clamp_ttl(raw: Option<i64>) -> i64 {
    match raw {
        None => DEFAULT_TTL_MS,
        Some(value) if value <= 0 => DEFAULT_TTL_MS,
        Some(value) => value.clamp(TTL_MIN_MS, TTL_MAX_MS),
    }
}

/// 文本截断（按**字符**数，不是字节——中文按字算）。空白串归一成 `None`。
pub fn clamp_text(raw: Option<String>) -> Option<String> {
    let text = raw?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(TEXT_LIMIT).collect())
}

/// pid 的解析：只收正数（数字或纯数字字符串）。
///
/// Swift 那边特别记了一条：`4294967298` 走 `NSNumber.int32Value` 会**截断成 2**，
/// 而「截断后的 pid 再去比对名字是靠运气」——所以这里也宁可拒绝，不静默截断。
pub fn parse_pid(raw: Option<&serde_json::Value>) -> Result<Option<i32>, Rejection> {
    let Some(value) = raw else { return Ok(None) };
    let number = match value {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => s.trim().parse::<i64>().ok(),
        serde_json::Value::Null => return Ok(None),
        _ => return Err(Rejection::BadPid),
    };
    let Some(number) = number else {
        return Err(Rejection::BadPid);
    };
    if number <= 0 || number > i32::MAX as i64 {
        return Err(Rejection::BadPid);
    }
    Ok(Some(number as i32))
}

/// 一条已登记的自报
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Record {
    pub agent_id: String,
    pub session_id: String,
    pub pid: Option<i32>,
    pub state: ReportedState,
    pub detail: Option<String>,
    pub ask: Option<String>,
    pub received_ms: i64,
    pub expires_ms: i64,
    /// 过期**只是不再采信**，记录本身留着（见文件头 ②）
    pub expired: bool,
}

impl Record {
    pub fn is_believable(&self, now_ms: i64) -> bool {
        now_ms < self.expires_ms
    }
}

/// 按 agent 存一条「当前那句话」。同一个 agent 再来一条就**覆盖**上一条——
/// 与 Swift 同义：卡片上只有一句话的位置，留两条只会让人猜哪条算数。
#[derive(Debug, Default)]
pub struct Registry {
    by_agent: HashMap<String, Record>,
}

impl Registry {
    pub fn new() -> Self {
        Registry::default()
    }

    pub fn submit(&mut self, submission: Submission, now_ms: i64) -> Record {
        let record = Record {
            agent_id: submission.agent_id.clone(),
            session_id: submission.session_id,
            pid: submission.pid,
            state: submission.state,
            detail: submission.detail,
            ask: submission.ask,
            received_ms: now_ms,
            expires_ms: now_ms + submission.ttl_ms,
            expired: false,
        };
        self.by_agent.insert(submission.agent_id, record.clone());
        record
    }

    /// 还能采信的那条（未过期）。过期的**不给**——这是「保质期」的全部意义。
    pub fn believable(&self, agent_id: &str, now_ms: i64) -> Option<&Record> {
        self.by_agent
            .get(agent_id)
            .filter(|record| record.is_believable(now_ms))
    }

    /// 任何状态下的记录（含过期）。给「刚才那句话是什么」这类诊断与报表用。
    pub fn record(&self, agent_id: &str) -> Option<&Record> {
        self.by_agent.get(agent_id)
    }

    /// 到期的盖戳，返回这一拍**刚**过期的 agent。不删记录。
    pub fn sweep_expired(&mut self, now_ms: i64) -> Vec<String> {
        let mut just_expired = Vec::new();
        for (agent_id, record) in self.by_agent.iter_mut() {
            if !record.expired && !record.is_believable(now_ms) {
                record.expired = true;
                just_expired.push(agent_id.clone());
            }
        }
        just_expired.sort();
        just_expired
    }

    pub fn remove(&mut self, agent_id: &str) -> bool {
        self.by_agent.remove(agent_id).is_some()
    }

    pub fn len(&self) -> usize {
        self.by_agent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_agent.is_empty()
    }
}

// MARK: - 出处（Swift `AgentProvenance`）

/// 一句话的出处。四种，`conflict` 是重点：自报与观测对不上时**两条都要显示**，
/// 不许悄悄挑一个——这一维存在的全部理由就是这个。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Provenance {
    /// 带令牌、TTL 内的自报（状态直接采信）
    SelfReported,
    /// 进程 + 会话日志的强语义
    Observed,
    /// CPU/写入双信号兜底
    Inferred,
    /// 自报与观测对不上
    Conflict,
}

impl Provenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Provenance::SelfReported => "selfReported",
            Provenance::Observed => "observed",
            Provenance::Inferred => "inferred",
            Provenance::Conflict => "conflict",
        }
    }

    /// 副标题上那一格短标签。`None` = 不额外标注：观测与推断是常态，
    /// 给它们都挂标签等于把噪声当信息。
    pub fn badge_text(self) -> Option<&'static str> {
        match self {
            Provenance::SelfReported => Some("自报"),
            Provenance::Conflict => Some("自报冲突"),
            Provenance::Observed | Provenance::Inferred => None,
        }
    }

    /// 完整说法（CLI 与报表用）。脚本侧要的是「这句话凭什么」，
    /// 所以观测/推断在这里**有**名字，界面上刻意没有。
    pub fn explain_text(self) -> &'static str {
        match self {
            Provenance::SelfReported => "自报（带令牌、TTL 内）",
            Provenance::Conflict => "自报冲突",
            Provenance::Observed => "观测（会话强语义）",
            Provenance::Inferred => "推断（CPU/写入兜底）",
        }
    }

    /// 副标题后面那一小截。**由这里给**：卡片、悬停、详情、报表各自拼一遍
    /// 「 · 自报」，就会有一处改了别处没改的那天。
    pub fn badge_suffix(provenance: Option<Provenance>) -> String {
        match provenance.and_then(|p| p.badge_text()) {
            Some(badge) => format!(" · {badge}"),
            None => String::new(),
        }
    }
}

/// 判定「这句话凭什么」，并给出**该显示哪一档状态**。照搬 Swift `ActivityEngine`：
///
/// · 没有可信自报 ⇒ 观测（读到会话强语义）或推断（兜底）；离线时**留 `None`**——
///   「它说了什么」在进程都不在时不成立，硬盖一个 `.observed` 等于给没发生过的对撞盖章。
/// · 自报与**强语义观测**对不上 ⇒ `Conflict`，**状态仍按观测走**（界面不该为一个自称在干活的
///   会话挂一张不存在的脸），但两句话都要显示。
/// · 自报与弱信号（兜底推断）对不上 ⇒ **采信自报**：带令牌的那句话比「CPU 有点高」
///   更有资格决定卡片写什么，而 TTL 就是它的保质期。
pub fn resolve(
    level: ActivityLevel,
    has_session_signal: bool,
    report: Option<&Record>,
) -> (ActivityLevel, Option<Provenance>) {
    match report {
        Some(record) => {
            if level == ActivityLevel::Offline
                || (record.state.level() != level && has_session_signal)
            {
                (level, Some(Provenance::Conflict))
            } else {
                (record.state.level(), Some(Provenance::SelfReported))
            }
        }
        None => {
            let provenance = match level {
                ActivityLevel::Offline => None,
                _ if has_session_signal => Some(Provenance::Observed),
                _ => Some(Provenance::Inferred),
            };
            (level, provenance)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn submission(state: ReportedState, ttl_ms: Option<i64>) -> Submission {
        Submission::new(
            "claude".into(),
            "s-1".into(),
            Some(4242),
            state,
            Some("正在改 view".into()),
            None,
            ttl_ms,
        )
    }

    #[test]
    fn ttl_is_clamped_and_defaults_instead_of_being_trusted() {
        assert_eq!(clamp_ttl(None), DEFAULT_TTL_MS);
        assert_eq!(clamp_ttl(Some(0)), DEFAULT_TTL_MS, "非正数算缺省");
        assert_eq!(clamp_ttl(Some(-5)), DEFAULT_TTL_MS);
        assert_eq!(clamp_ttl(Some(1)), TTL_MIN_MS, "太短：钳到 15 秒");
        assert_eq!(clamp_ttl(Some(10_000_000)), TTL_MAX_MS, "太长：钳到 600 秒");
        assert_eq!(clamp_ttl(Some(120_000)), 120_000, "区间内原样");
    }

    #[test]
    fn text_is_truncated_by_characters_not_dropped() {
        assert_eq!(clamp_text(None), None);
        assert_eq!(clamp_text(Some("   ".into())), None, "全空白等于没提");
        assert_eq!(
            clamp_text(Some("  有内容  ".into())).as_deref(),
            Some("有内容")
        );
        let long = "字".repeat(TEXT_LIMIT + 50);
        let clamped = clamp_text(Some(long)).unwrap();
        assert_eq!(
            clamped.chars().count(),
            TEXT_LIMIT,
            "按字截断（不是按字节）"
        );
    }

    #[test]
    fn a_report_is_believable_only_inside_its_window() {
        let mut registry = Registry::new();
        let record = registry.submit(submission(ReportedState::Working, Some(30_000)), 1_000);
        assert_eq!(record.expires_ms, 31_000);
        assert!(registry.believable("claude", 30_999).is_some(), "窗口内");
        assert!(
            registry.believable("claude", 31_000).is_none(),
            "到期那一毫秒就不该再采信"
        );
        assert!(
            registry.believable("gemini", 1_000).is_none(),
            "没自报过的没有"
        );
    }

    #[test]
    fn expiry_stamps_the_record_instead_of_deleting_it() {
        let mut registry = Registry::new();
        registry.submit(submission(ReportedState::Working, Some(15_000)), 0);
        assert!(
            registry.sweep_expired(10_000).is_empty(),
            "还没到期，不该盖戳"
        );
        assert_eq!(registry.sweep_expired(15_000), vec!["claude".to_string()]);
        // 盖过戳就不再重复报（采样每拍都会调它）
        assert!(registry.sweep_expired(20_000).is_empty());
        // **记录还在**：过期是「不再采信」，不是「没说过」
        let record = registry.record("claude").expect("过期后记录必须留着");
        assert!(record.expired);
        assert_eq!(record.state, ReportedState::Working);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn a_newer_report_replaces_the_previous_one_and_unstamps_it() {
        let mut registry = Registry::new();
        registry.submit(submission(ReportedState::Working, Some(15_000)), 0);
        registry.sweep_expired(20_000);
        assert!(registry.record("claude").unwrap().expired);
        // 同一个 agent 再报一条：覆盖，且不再是过期态
        let second = registry.submit(submission(ReportedState::Idle, Some(60_000)), 30_000);
        assert!(!second.expired);
        assert_eq!(registry.len(), 1, "同一个 agent 只留一条");
        assert_eq!(
            registry.record("claude").unwrap().state,
            ReportedState::Idle
        );
        assert!(registry.believable("claude", 31_000).is_some());
    }

    #[test]
    fn malformed_submissions_are_rejected_with_a_reason() {
        let empty_agent = Submission::new(
            "  ".into(),
            "s-1".into(),
            None,
            ReportedState::Idle,
            None,
            None,
            None,
        );
        assert_eq!(empty_agent.validate(), Err(Rejection::Malformed));
        let empty_session = Submission::new(
            "claude".into(),
            "".into(),
            None,
            ReportedState::Idle,
            None,
            None,
            None,
        );
        assert_eq!(empty_session.validate(), Err(Rejection::Malformed));
        assert_eq!(Rejection::Malformed.reason(), "malformed");
    }

    #[test]
    fn states_round_trip_and_never_include_offline() {
        for state in [
            ReportedState::Working,
            ReportedState::Attention,
            ReportedState::Completed,
            ReportedState::Idle,
        ] {
            assert_eq!(ReportedState::parse(state.as_str()), Some(state));
            assert_ne!(
                state.level(),
                ActivityLevel::Offline,
                "自报没有「离线」这一档"
            );
        }
        assert_eq!(
            ReportedState::parse("offline"),
            None,
            "离线是观测才有的结论"
        );
        assert_eq!(
            ReportedState::parse("  WORKING "),
            Some(ReportedState::Working)
        );
        assert_eq!(ReportedState::parse("干活中"), None, "认不出就拒绝，不猜");
    }

    #[test]
    fn pids_are_rejected_rather_than_silently_truncated() {
        use serde_json::json;
        assert_eq!(parse_pid(None), Ok(None));
        assert_eq!(parse_pid(Some(&json!(null))), Ok(None));
        assert_eq!(parse_pid(Some(&json!(4242))), Ok(Some(4242)));
        assert_eq!(parse_pid(Some(&json!("4242"))), Ok(Some(4242)));
        assert_eq!(parse_pid(Some(&json!(0))), Err(Rejection::BadPid));
        assert_eq!(parse_pid(Some(&json!(-1))), Err(Rejection::BadPid));
        // 这个数走 32 位截断会变成 2：截断后的 pid 去比对名字是靠运气
        assert_eq!(
            parse_pid(Some(&json!(4_294_967_298u64))),
            Err(Rejection::BadPid)
        );
        assert_eq!(parse_pid(Some(&json!("abc"))), Err(Rejection::BadPid));
        assert_eq!(parse_pid(Some(&json!(true))), Err(Rejection::BadPid));
        // 归一化时顺手挡掉非正 pid
        assert_eq!(submission(ReportedState::Idle, None).pid, Some(4242));
        let negative = Submission::new(
            "claude".into(),
            "s".into(),
            Some(-3),
            ReportedState::Idle,
            None,
            None,
            None,
        );
        assert_eq!(negative.pid, None);
    }
}

#[cfg(test)]
mod provenance_tests {
    use super::*;

    fn report(state: ReportedState) -> Record {
        Record {
            agent_id: "claude".into(),
            session_id: "s-1".into(),
            pid: Some(1),
            state,
            detail: None,
            ask: None,
            received_ms: 0,
            expires_ms: 1_000_000,
            expired: false,
        }
    }

    #[test]
    fn without_a_report_observed_and_inferred_are_told_apart_and_offline_gets_nothing() {
        // 读到会话强语义 ⇒ 观测
        let (level, p) = resolve(ActivityLevel::Working, true, None);
        assert_eq!(
            (level, p),
            (ActivityLevel::Working, Some(Provenance::Observed))
        );
        // 只有 CPU/写入兜底 ⇒ 推断
        let (level, p) = resolve(ActivityLevel::Idle, false, None);
        assert_eq!(
            (level, p),
            (ActivityLevel::Idle, Some(Provenance::Inferred))
        );
        // 离线：不给出处——「它说了什么」在进程都不在时不成立
        let (level, p) = resolve(ActivityLevel::Offline, false, None);
        assert_eq!((level, p), (ActivityLevel::Offline, None));
    }

    #[test]
    fn a_believable_report_is_adopted_and_its_level_decides_the_card() {
        let record = report(ReportedState::Working);
        // 观测只是「空闲」（弱信号），自报说在工作 ⇒ 采信自报
        let (level, p) = resolve(ActivityLevel::Idle, false, Some(&record));
        assert_eq!(
            (level, p),
            (ActivityLevel::Working, Some(Provenance::SelfReported))
        );
        // 观测与会话信号一致时也采信（不冲突）
        let (level, p) = resolve(ActivityLevel::Working, true, Some(&record));
        assert_eq!(
            (level, p),
            (ActivityLevel::Working, Some(Provenance::SelfReported))
        );
    }

    #[test]
    fn a_report_that_disagrees_with_a_strong_signal_is_a_conflict_and_the_observation_wins() {
        let record = report(ReportedState::Working);
        // 会话强语义说「空闲」，自报说「在工作」 ⇒ 冲突，且**显示按观测走**
        let (level, p) = resolve(ActivityLevel::Idle, true, Some(&record));
        assert_eq!(
            (level, p),
            (ActivityLevel::Idle, Some(Provenance::Conflict)),
            "冲突时状态必须按观测走，否则界面会挂一张不存在的脸"
        );
    }

    #[test]
    fn a_report_for_a_process_that_is_gone_is_a_conflict() {
        let record = report(ReportedState::Working);
        let (level, p) = resolve(ActivityLevel::Offline, false, Some(&record));
        assert_eq!(
            (level, p),
            (ActivityLevel::Offline, Some(Provenance::Conflict))
        );
    }

    #[test]
    fn only_the_two_remarkable_provenances_get_a_badge() {
        assert_eq!(Provenance::SelfReported.badge_text(), Some("自报"));
        assert_eq!(Provenance::Conflict.badge_text(), Some("自报冲突"));
        assert_eq!(
            Provenance::Observed.badge_text(),
            None,
            "观测是常态，不挂标签"
        );
        assert_eq!(Provenance::Inferred.badge_text(), None, "推断也是常态");
        // 后缀由 Rust 拼好：界面不自己拼「 · 自报」
        assert_eq!(
            Provenance::badge_suffix(Some(Provenance::SelfReported)),
            " · 自报"
        );
        assert_eq!(
            Provenance::badge_suffix(Some(Provenance::Conflict)),
            " · 自报冲突"
        );
        assert_eq!(Provenance::badge_suffix(Some(Provenance::Observed)), "");
        assert_eq!(Provenance::badge_suffix(None), "");
        // 完整说法（报表/CLI 用）
        assert_eq!(Provenance::Observed.explain_text(), "观测（会话强语义）");
        assert_eq!(
            Provenance::SelfReported.explain_text(),
            "自报（带令牌、TTL 内）"
        );
        for p in [
            Provenance::SelfReported,
            Provenance::Observed,
            Provenance::Inferred,
            Provenance::Conflict,
        ] {
            assert!(!p.explain_text().is_empty());
            assert_eq!(serde_json::to_value(p).unwrap().as_str(), Some(p.as_str()));
        }
    }
}
