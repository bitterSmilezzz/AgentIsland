//! 网关 fallback（候选耗尽后的有序降级）判据（GW-06 第二片）。
//!
//! 需求原文：账号/模型/请求错误分开；首字节前重试，流中失败不拼接回答或重复工具/计费；
//! **fallback、退避与错误分类**。
//!
//! 与 `gateway_retry`（重试资格）、`gateway_routing`（候选选择）同属一个未来消费方：
//! **候选选不出来之后怎么办**。
//!
//! 核心语义：**fallback 不是「失败就换下一个」**。账号/权限类错误换节点也一样
//! （凭据不会因为换台机器变有效）——那类错误该停下报人；只有「这个候选不行、
//! 但别的候选可能行」才值得降级。重试与降级的区别也在这里：重试是**同一个**
//! 目标再试一次，fallback 是**换目标**。
//!
//! 判据（纯函数）：
//!
//! 1. **降级资格**：只有「目标侧/链路侧」的失败才降级（Transient）；
//!    账号权限类（Credential）、请求类（Request）、配置类（Configuration）
//!    **不降级**——换机器换凭据都换不来结果，只会把失败扩散到整条链。
//! 2. **有序链**：fallback 链是**声明顺序**的候选列表，逐个尝试；
//!    不做智能挑选（挑选是 `gateway_routing` 的职责，两者不混）。
//! 3. **链上环检测**：链里出现重复候选即拒绝（否则会绕圈重试同一个）。
//! 4. **链长上限**：避免一条几十长的链把一次请求拖成几十次尝试。

use crate::gateway_retry::Eligibility;

/// 降级链的拒绝原因。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackRefusal {
    /// 链里有重复候选（会绕圈）。
    DuplicateCandidate,
    /// 链长超过上限。
    ChainTooLong,
    /// 链里没有候选。
    EmptyChain,
}

impl FallbackRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DuplicateCandidate => "duplicate_candidate",
            Self::ChainTooLong => "chain_too_long",
            Self::EmptyChain => "empty_chain",
        }
    }
}

/// fallback 链的最大候选数。
pub const MAX_FALLBACK_CHAIN: usize = 8;

/// 校验一条 fallback 链。
pub fn validate_chain(chain: &[String]) -> Result<(), FallbackRefusal> {
    if chain.is_empty() {
        return Err(FallbackRefusal::EmptyChain);
    }
    if chain.len() > MAX_FALLBACK_CHAIN {
        return Err(FallbackRefusal::ChainTooLong);
    }
    for (index, candidate) in chain.iter().enumerate() {
        if chain[..index].contains(candidate) {
            return Err(FallbackRefusal::DuplicateCandidate);
        }
    }
    Ok(())
}

/// 这次失败**是否值得**降级到下一个候选。
///
/// 判据只看错误的**类别**，不看具体候选：账号权限类与请求类不降级
/// （换机器换凭据都换不来结果）；限流类也不自动降级——对端说了「什么时候再来」，
/// 换节点去撞同一堵墙只是浪费配额；只有 Transient（链路/服务抖动）才降级。
pub fn should_fallback(eligibility: Eligibility) -> bool {
    matches!(eligibility, Eligibility::Transient)
}

/// 在链上推进到下一个候选。
///
/// 返回 `None` 表示链已走完（没有更多候选）——**不是错误**：调用方据此如实
/// 报「所有候选都试过了」。
pub fn next_candidate<'a>(chain: &'a [String], current: &str) -> Option<&'a str> {
    let index = chain.iter().position(|candidate| candidate == current)?;
    chain.get(index + 1).map(String::as_str)
}

/// 降级链的第一个候选（链的起点）。
pub fn first_candidate(chain: &[String]) -> Option<&str> {
    chain.first().map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain() -> Vec<String> {
        vec![
            "primary".into(),
            "secondary".into(),
            "tertiary".into(),
        ]
    }

    #[test]
    fn only_transient_failures_are_worth_falling_back() {
        // 链路抖动 ⇒ 降级（换节点可能就好）
        assert!(should_fallback(Eligibility::Transient));
        // 账号/权限类 ⇒ **不降级**（换机器换凭据都换不来结果）
        assert!(!should_fallback(Eligibility::Credential));
        // 请求/模型侧 ⇒ 不降级（原样重发必然同样失败）
        assert!(!should_fallback(Eligibility::Request));
        // 配置错 ⇒ 不降级
        assert!(!should_fallback(Eligibility::Configuration));
        // 限流 ⇒ 不自动降级（对端说了什么时候再来）
        assert!(!should_fallback(Eligibility::Throttled));
    }

    #[test]
    fn the_chain_walks_in_declaration_order_and_ends_honestly() {
        let chain = chain();
        assert_eq!(first_candidate(&chain), Some("primary"));
        assert_eq!(next_candidate(&chain, "primary"), Some("secondary"));
        assert_eq!(next_candidate(&chain, "secondary"), Some("tertiary"));
        // 链走完 ⇒ None（不是错误：调用方如实报「所有候选都试过了」）
        assert_eq!(next_candidate(&chain, "tertiary"), None);
        // 不在链里的候选 ⇒ None
        assert_eq!(next_candidate(&chain, "unknown"), None);
    }

    #[test]
    fn chains_with_duplicates_or_excessive_length_are_refused() {
        assert_eq!(
            validate_chain(&["a".into(), "b".into(), "a".into()]),
            Err(FallbackRefusal::DuplicateCandidate),
            "重复候选会绕圈"
        );
        assert_eq!(validate_chain(&[]), Err(FallbackRefusal::EmptyChain));

        let too_long: Vec<String> = (0..=MAX_FALLBACK_CHAIN).map(|i| format!("c{i}")).collect();
        assert_eq!(
            validate_chain(&too_long),
            Err(FallbackRefusal::ChainTooLong),
            "过长的链会把一次请求拖成几十次尝试"
        );

        assert_eq!(validate_chain(&chain()), Ok(()));
    }

    #[test]
    fn a_fallback_walk_visits_every_candidate_exactly_once() {
        let chain = chain();
        let mut visited = Vec::new();
        let mut current = first_candidate(&chain).unwrap().to_string();
        visited.push(current.clone());
        while let Some(next) = next_candidate(&chain, &current) {
            current = next.to_string();
            visited.push(current.clone());
        }
        assert_eq!(visited, vec!["primary", "secondary", "tertiary"]);
        assert_eq!(visited.len(), 3, "每个候选恰好一次");
    }
}
