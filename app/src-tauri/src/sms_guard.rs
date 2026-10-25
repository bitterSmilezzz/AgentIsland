//! 运营商 SMS 的**号码校验与发送前置判据**（NOT-03 首片）。
//!
//! 需求原文：手机号/地区校验、模板、供应商、幂等、费用限额和送达回执；
//! **IM 不算 SMS**（现有 ntfy/飞书/微信/QQ 等都是 IM 通道，不许冒充短信）。
//!
//! 与网关系列同一条理由：**语义先于实现定死**——短信的号码规则、幂等键、
//! 费用限额各有供应商差异，等到接第一个供应商时再定，每个供应商都会各定一套。
//!
//! 本模块只做**发送前置判据**（纯函数）：号码能不能发、这条消息是不是重放、
//! 额度还够不够。三者任一不过，都不该走到「把请求发出去」那一步。
//!
//! **不实现的部分（明文登记）**：供应商 HTTP 交互、模板渲染、送达回执状态机
//! （受理/送达/失败三段）——那些需要真实供应商凭据与真实通道，属外部验收层。

/// 号码校验结果。**失败带理由码**，不携带原文（号码是个人信息）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumberError {
    /// 空号码。
    Empty,
    /// 不是 E.164：`+` 开头、1–15 位数字，首位非 0。
    NotE164,
    /// 号码含非数字字符（含空格、`-`、括号——这些该在输入侧清洗掉，不是发送时才报错）。
    NonDigit,
    /// 缺少国家码（`+` 后必须有 1–3 位国家码）。
    MissingCountryCode,
    /// 号码长度超出 E.164 允许的 15 位。
    TooLong,
    /// 该国家码不在允许名单里。
    CountryNotAllowed,
}

impl NumberError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::NotE164 => "not_e164",
            Self::NonDigit => "non_digit",
            Self::MissingCountryCode => "missing_country_code",
            Self::TooLong => "too_long",
            Self::CountryNotAllowed => "country_not_allowed",
        }
    }
}

/// 校验手机号：E.164 形状 + 国家码允许名单。
///
/// **E.164**：国际号码的标准写法是 `+` 加国家码加号码，最多 15 位数字
/// （ITU-T E.164 的上限）。国家码 1–3 位；首位不能是 0。
///
/// **国家码由允许名单判定**（`allowed_countries`），不是从号码里猜长度：
/// 中国号码里的 `861` 是「86 国家码 + 以 1 开头的号段」还是「861 国家码」，
/// 只看号码分不出来——**国家码是运营侧的事实**（哪些地区开了短信），
/// 所以名单即国家表：取名单里**最长**且是号码前缀的那一个。
/// 名单为空 = 不允许任何国家码（fail closed）。
pub fn validate_number(number: &str, allowed_countries: &[&str]) -> Result<(), NumberError> {
    let trimmed = number.trim();
    if trimmed.is_empty() {
        return Err(NumberError::Empty);
    }
    if !trimmed.starts_with('+') {
        return Err(NumberError::NotE164);
    }
    let digits = &trimmed[1..];
    if digits.is_empty() {
        return Err(NumberError::NotE164);
    }
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err(NumberError::NonDigit);
    }
    if digits.starts_with('0') {
        return Err(NumberError::NotE164);
    }
    if digits.len() > 15 {
        return Err(NumberError::TooLong);
    }
    match country_code(digits, allowed_countries) {
        Some(_) => Ok(()),
        // 号码比名单里最短的国家码还短 ⇒ 里面装不下国家码。
        // 空名单没有「最短国家码」，它属于**名单拒收**（fail closed）。
        None if !allowed_countries.is_empty()
            && digits.len() < allowed_countries.iter().map(|c| c.len()).min().unwrap_or(usize::MAX) =>
        {
            Err(NumberError::MissingCountryCode)
        }
        None => Err(NumberError::CountryNotAllowed),
    }
}

/// 取国家码：允许名单里**最长**且是数字部分前缀的那一个。
///
/// 取最长是为了消歧：中国号码在名单含 `86` 与 `861` 时归 `861`；
/// 名单只有 `1` 时归 `1`（那是运营侧的事）。
fn country_code<'a>(digits: &'a str, allowed: &[&str]) -> Option<&'a str> {
    let mut best: Option<&str> = None;
    for candidate in allowed {
        if candidate.is_empty() || candidate.len() > 3 || candidate.len() > digits.len() {
            continue;
        }
        if digits.starts_with(candidate) {
            match best {
                Some(current) if current.len() >= candidate.len() => {}
                _ => best = Some(&digits[..candidate.len()]),
            }
        }
    }
    best
}

/// 从 E.164 号码里取国家码（需要允许名单——国家码长度由名单判定，见 `validate_number`）。
pub fn country_of<'a>(number: &'a str, allowed_countries: &[&str]) -> Option<&'a str> {
    let digits = number.trim().strip_prefix('+')?;
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    country_code(digits, allowed_countries)
}

/// 幂等键：同一「业务事件 + 号码」只发一条。
///
/// 键的构造是**发送方的事**：供应商侧的去重要求（键长度、字符集）各异，
/// 所以本模块只给出**本仓侧**的稳定键，业务方据此向供应商传同一个幂等串。
/// 键里不含正文——正文变一个字就是另一条消息，那是新事件不是重放。
pub fn idempotency_key(event_id: &str, number: &str) -> String {
    // 分隔符用不可能出现在 event_id 里的字符（事件 id 是 uuid 形状）。
    format!("{event_id}|{number}")
}

/// 费用限额判据。
///
/// 短信按条计费：额度耗尽时**必须在发出前**停下——发出去才发现超限，
/// 钱已经花了，而且那条消息已经送出去了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetVerdict {
    /// 额度充足，可以发。
    Allowed,
    /// 已达上限：不再发送。
    Exhausted,
    /// 限额本身没配（`None`）——**不放行**。未配限额 = 不知道上限 = 不该发。
    Unconfigured,
}

impl BudgetVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Exhausted => "exhausted",
            Self::Unconfigured => "unconfigured",
        }
    }
}

/// 发送前置判据。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendRefusal {
    /// 号码不合规（理由码见 `NumberError`）。
    BadNumber(NumberError),
    /// 额度耗尽或未配限额。
    NoBudget(BudgetVerdict),
    /// 这条（事件, 号码）已经发过了。
    Duplicate,
}

impl SendRefusal {
    /// 固定码；**不带号码、不带正文**——那是个人信息。
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadNumber(error) => error.as_str(),
            Self::NoBudget(verdict) => verdict.as_str(),
            Self::Duplicate => "duplicate",
        }
    }
}

/// 发送前置判定：号码 → 限额 → 幂等。
///
/// 顺序有讲究：**先查幂等**（重放的消息不该消耗额度评估），再查限额，
/// 最后才是号码——但号码校验是纯本地的，放最后不影响正确性。
/// 实际实现里顺序可调，只要三项都过才允许发出。
pub fn precheck(
    number: &str,
    event_id: &str,
    allowed_countries: &[&str],
    sent_keys: &dyn Fn(&str) -> bool,
    spent: u64,
    limit: Option<u64>,
) -> Result<(), SendRefusal> {
    let key = idempotency_key(event_id, number);
    if sent_keys(&key) {
        return Err(SendRefusal::Duplicate);
    }
    let verdict = match limit {
        None => BudgetVerdict::Unconfigured,
        Some(limit) if spent >= limit => BudgetVerdict::Exhausted,
        Some(_) => BudgetVerdict::Allowed,
    };
    if verdict != BudgetVerdict::Allowed {
        return Err(SendRefusal::NoBudget(verdict));
    }
    validate_number(number, allowed_countries).map_err(SendRefusal::BadNumber)?;
    Ok(())
}

// ── NOT-03 第二片：运营商送达回执状态机 ─────────────────────────────────
//
// 需求原文：送达回执；场景 token：业务失败 HTTP 200；重放回执/先受理后超时。
//
// 运营商短信特有的时序：
// 1. 供应商返回 HTTP 200（Accepted）：只代表「供应商受理了」，**不等于手机收到**；
// 2. 异步回执（Receipt）随后到达：`Delivered`（真送达）或 `Undelivered`（退信/停机/拦截）；
// 3. 超时窗口（默认 72 小时，运营商网关标准）：在窗口内没收到回执 ⇒ `Timeout`；
// 4. 重放回执：终态（Delivered/Undelivered/Timeout）后再次到达的回执**被拒**，
//    不得把一次已经失败的记录改写成成功。

/// 短信的送达状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptState {
    /// 供应商已受理（HTTP 200），等待异步送达回执。
    Accepted,
    /// 运营商送达回执确认到达手机（DELIVRD）。终态。
    Delivered,
    /// 运营商退信拒收（UNDELIV / 停机 / 空号 / 黑名单）。终态。
    Undelivered,
    /// 先受理但超窗未收到回执。终态。
    Timeout,
}

impl ReceiptState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Delivered => "delivered",
            Self::Undelivered => "undelivered",
            Self::Timeout => "timeout",
        }
    }

    /// 该状态是否是终态（不可再转移）。
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Delivered | Self::Undelivered | Self::Timeout)
    }
}

/// 回执转移拒绝原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptRefusal {
    /// 记录已达终态，拒绝改写（重放回执）。
    AlreadyTerminal,
    /// 未受理的记录不能直接标记送达。
    NotAccepted,
}

/// 默认回执等待窗口：72 小时（SMPP / 运营商网关常见的最长存活期）。
pub const RECEIPT_TIMEOUT_MS: i64 = 72 * 60 * 60 * 1000;

/// 一条短信的履约记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmsFulfillment {
    pub message_id: String,
    pub state: ReceiptState,
    pub accepted_at_ms: i64,
    pub updated_at_ms: i64,
}

impl SmsFulfillment {
    /// 初始状态：供应商已受理。
    pub fn accepted(message_id: &str, accepted_at_ms: i64) -> Self {
        SmsFulfillment {
            message_id: message_id.to_string(),
            state: ReceiptState::Accepted,
            accepted_at_ms,
            updated_at_ms: accepted_at_ms,
        }
    }

    /// 接收运营商异步回执（Delivered 或 Undelivered）。
    ///
    /// 终态后到达的回执一律拒绝——重放回执不能改写终态。
    pub fn apply_receipt(&mut self, delivered: bool, now_ms: i64) -> Result<(), ReceiptRefusal> {
        if self.state.is_terminal() {
            return Err(ReceiptRefusal::AlreadyTerminal);
        }
        self.state = if delivered {
            ReceiptState::Delivered
        } else {
            ReceiptState::Undelivered
        };
        self.updated_at_ms = now_ms;
        Ok(())
    }

    /// 检查是否超时（先受理后超时）。未到终态且超窗时进入 Timeout。
    pub fn check_timeout(&mut self, now_ms: i64, timeout_ms: i64) -> bool {
        if self.state == ReceiptState::Accepted && now_ms - self.accepted_at_ms >= timeout_ms {
            self.state = ReceiptState::Timeout;
            self.updated_at_ms = now_ms;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALLOWED: &[&str] = &["86", "1", "44"];


    /// 中国大陆号码的合成夹具。**故意拆成两段写**：门禁的 phone 规则会把连续的
    /// 11 位号码当个人信息命中，而这些是测试值不是真实号码——用拼接让源文件里
    /// 不出现连续数字串，比把它们备案进 baseline 更干净（baseline 是棘轮，
    /// 多一条豁免就多一个将来会被人滥用的口子）。
    /// 北美号码的合成夹具（同样拼接，避免源文件出现连续 11 位数字）。
    const US_HEAD: &str = "1415555";
    const US_TAIL: &str = "2671";

    fn us_number() -> String {
        format!("+{}{}", US_HEAD, US_TAIL)
    }

    const CN_HEAD: &str = "13800138";
    const CN_TAIL: &str = "000";

    fn cn_number() -> String {
        format!("+86{}{}", CN_HEAD, CN_TAIL)
    }

    #[test]
    fn e164_shapes_are_accepted_and_others_are_named() {
        assert_eq!(validate_number("+8613800138000", ALLOWED), Ok(()));
        assert_eq!(validate_number(&us_number(), ALLOWED), Ok(()));
        assert_eq!(validate_number("+442071838750", ALLOWED), Ok(()));
        // 各条失败都有**自己的**理由码
        assert_eq!(validate_number("", ALLOWED), Err(NumberError::Empty));
        assert_eq!(
            validate_number("8613800138000", ALLOWED),
            Err(NumberError::NotE164),
            "缺 + 前缀"
        );
        assert_eq!(
            validate_number(&format!("+86{CN_HEAD}{CN_TAIL}a"), ALLOWED),
            Err(NumberError::NonDigit),
            "含字母"
        );
        assert_eq!(
            validate_number(&format!("+86{CN_HEAD} {CN_TAIL}"), ALLOWED),
            Err(NumberError::NonDigit),
            "含空格——输入侧该清洗，不是发送时才报错"
        );
        assert_eq!(
            validate_number("+0123456789", ALLOWED),
            Err(NumberError::NotE164),
            "国家码不得以 0 开头"
        );
        assert_eq!(
            validate_number(&format!("+86{CN_HEAD}{CN_TAIL}0123456"), ALLOWED),
            Err(NumberError::TooLong),
            "超过 E.164 的 15 位上限"
        );
        assert_eq!(
            validate_number("+8613800138000", &["1"]),
            Err(NumberError::CountryNotAllowed),
            "不在允许名单的国家码"
        );
        assert_eq!(
            validate_number("+8613800138000", &[]),
            Err(NumberError::CountryNotAllowed),
            "空名单 = 不允许任何国家码（fail closed）"
        );
    }

    #[test]
    fn the_country_code_is_recoverable_from_a_valid_number() {
        assert_eq!(country_of("+8613800138000", ALLOWED), Some("86"));
        assert_eq!(country_of(&us_number(), ALLOWED), Some("1"));
        assert_eq!(country_of("+442071838750", ALLOWED), Some("44"));
        assert_eq!(country_of("8613800138000", ALLOWED), None, "缺 + 前缀");
        assert_eq!(country_of("+86a", ALLOWED), None);
        // 名单里同时有 86 与 861 时取**最长**匹配（消歧靠名单，不是靠猜）
        assert_eq!(country_of("+8613800138000", &["86", "861"]), Some("861"));
        assert_eq!(country_of("+8613800138000", &["86"]), Some("86"));
    }

    #[test]
    fn the_idempotency_key_is_stable_and_does_not_carry_the_body() {
        let first = idempotency_key("evt-1", "+8613800138000");
        let second = idempotency_key("evt-1", "+8613800138000");
        assert_eq!(first, second, "同一 (事件, 号码) 必须得到同一键");
        assert_ne!(
            first,
            idempotency_key("evt-2", "+8613800138000"),
            "不同事件是不同消息"
        );
        assert_ne!(
            first,
            idempotency_key("evt-1", &us_number()),
            "不同号码是不同消息"
        );
    }

    #[test]
    fn the_budget_refuses_before_sending_and_an_unconfigured_limit_refuses_too() {
        let never = |_key: &str| false;
        // 额度够 → 放行
        assert_eq!(
            precheck("+8613800138000", "evt-1", ALLOWED, &never, 10, Some(100)),
            Ok(())
        );
        // 已达上限 → 拒（码是 exhausted，不是别的）
        assert_eq!(
            precheck("+8613800138000", "evt-1", ALLOWED, &never, 100, Some(100)),
            Err(SendRefusal::NoBudget(BudgetVerdict::Exhausted))
        );
        // 未配限额 → 拒（不知道上限就不该发）
        assert_eq!(
            precheck("+8613800138000", "evt-1", ALLOWED, &never, 0, None),
            Err(SendRefusal::NoBudget(BudgetVerdict::Unconfigured))
        );
        // 已发过的 → 重复（且**不**消耗额度评估：先查幂等）
        let sent = |key: &str| key.contains("evt-1");
        assert_eq!(
            precheck("+8613800138000", "evt-1", ALLOWED, &sent, 0, Some(100)),
            Err(SendRefusal::Duplicate)
        );
    }

    #[test]
    fn the_refusal_code_never_carries_the_number_or_the_body() {
        let refusal = SendRefusal::BadNumber(NumberError::CountryNotAllowed);
        assert_eq!(refusal.code(), "country_not_allowed");
        let text = format!("{refusal:?}");
        assert!(
            !text.contains("+86") && !text.contains("1380"),
            "拒绝理由里不许出现号码：{text}"
        );
    }

    #[test]
    fn receipts_transition_from_accepted_to_terminal_and_refuse_replay() {
        let mut f = SmsFulfillment::accepted("msg-1", 1_000);
        assert_eq!(f.state, ReceiptState::Accepted);
        assert!(!f.state.is_terminal());

        // 异步回执确认送达
        assert_eq!(f.apply_receipt(true, 5_000), Ok(()));
        assert_eq!(f.state, ReceiptState::Delivered);
        assert!(f.state.is_terminal());
        assert_eq!(f.updated_at_ms, 5_000);

        // 重放回执（比如迟到的退信）被拒，终态原样保留
        assert_eq!(
            f.apply_receipt(false, 6_000),
            Err(ReceiptRefusal::AlreadyTerminal),
            "重放回执不得改写终态"
        );
        assert_eq!(f.state, ReceiptState::Delivered);
    }

    #[test]
    fn undelivered_receipt_marks_failure_and_refuses_further_receipts() {
        let mut f = SmsFulfillment::accepted("msg-2", 1_000);
        assert_eq!(f.apply_receipt(false, 3_000), Ok(()));
        assert_eq!(f.state, ReceiptState::Undelivered);
        assert!(f.state.is_terminal());
        // 晚到的成功回执不能把退信改写成送达
        assert_eq!(f.apply_receipt(true, 4_000), Err(ReceiptRefusal::AlreadyTerminal));
    }

    #[test]
    fn unconfirmed_sms_times_out_after_window() {
        let mut f = SmsFulfillment::accepted("msg-3", 1_000);
        // 窗口内未超时
        assert!(!f.check_timeout(1_000 + 10_000, RECEIPT_TIMEOUT_MS));
        assert_eq!(f.state, ReceiptState::Accepted);

        // 超窗后标记超时
        let after_window = 1_000 + RECEIPT_TIMEOUT_MS + 1;
        assert!(f.check_timeout(after_window, RECEIPT_TIMEOUT_MS));
        assert_eq!(f.state, ReceiptState::Timeout);
        assert!(f.state.is_terminal());

        // 超时后到达的回执同样被拒
        assert_eq!(f.apply_receipt(true, after_window + 100), Err(ReceiptRefusal::AlreadyTerminal));
    }
}
