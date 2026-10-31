//! GW-06 流中失败状态判据（待未来转发层消费）。
//!
//! 对同一请求维护首字节提交边界、候选位置及每次上游尝试的终态。首字节之前
//! 复用 `gateway_retry` 的资格/预算与 `gateway_fallback` 的有序候选规则；首字节
//! 之后任何失败只会终止部分响应，避免把第二次回答拼进同一响应或再次交付工具事件。
//!
//! 本模块不发网络请求、不发送工具、不记账、不读取时钟。尝试记录是内存中的状态
//! 判据，不是计费账本，也不证明上游没有收费。

use crate::gateway_fallback::{self, FallbackRefusal, MAX_FALLBACK_CHAIN};
use crate::gateway_retry::{self, Eligibility, MAX_RETRIES_PER_CANDIDATE};

pub const MAX_ATTEMPTS_PER_CANDIDATE: usize = MAX_RETRIES_PER_CANDIDATE as usize + 1;
/// 最大候选链乘以每候选的总尝试数，作为任何请求都不能越过的硬上限。
pub const MAX_TOTAL_UPSTREAM_ATTEMPTS: usize = MAX_FALLBACK_CHAIN * MAX_ATTEMPTS_PER_CANDIDATE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    AwaitingFirstByte,
    Streaming,
    WaitingRetryAfter,
    Terminal,
}

/// 一次上游尝试的终态；`None` 只表示当前尝试还在进行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptOutcome {
    Completed,
    FailedBeforeFirstByte,
    AbortedAfterFirstByte,
    Cancelled,
}

/// 单次请求内存中的尝试记录，不包含响应正文、凭据或费用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptRecord {
    /// 在已校验候选链中的位置，从 0 开始。
    pub candidate_index: usize,
    /// 最后一项 `None` 表示目前进行中的尝试。
    pub outcome: Option<AttemptOutcome>,
    pub observed_first_byte: bool,
}

/// 一次失败后的唯一下一步决策。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFailureAction {
    /// 首字节前，在同一候选剩余预算内重试。
    RetrySameCandidate,
    /// 同一候选预算用尽后，移到已校验链中的下一候选。
    FallbackNextCandidate,
    /// 由上游 Retry-After 决定之后的尝试；本模块不自动等待或重试。
    WaitForRetryAfter,
    /// 首字节前终止，未产生可交付响应。
    StopBeforeOutput,
    /// 首字节后终止部分响应，不重试、不降级。
    AbortPartialResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamMachineRefusal {
    /// 当前请求已完成、取消或失败，不能继续写入或再次结算。
    Terminal,
    /// 已返回限流等待决策，必须由调用方确认 Retry-After 后恢复。
    AwaitingRetryAfter,
    /// 当前没有等待中的 Retry-After 可恢复。
    NotAwaitingRetryAfter,
}

impl StreamMachineRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::AwaitingRetryAfter => "awaiting_retry_after",
            Self::NotAwaitingRetryAfter => "not_awaiting_retry_after",
        }
    }
}

/// 一次网关请求的纯尝试状态机。
#[derive(Debug)]
pub struct StreamFailureMachine {
    candidate_count: usize,
    candidate_index: usize,
    retries_on_candidate: u32,
    phase: Phase,
    attempts: Vec<AttemptRecord>,
}

impl StreamFailureMachine {
    /// 创建请求状态并复用既有 fallback 链校验。
    pub fn new(candidate_chain: &[String]) -> Result<Self, FallbackRefusal> {
        gateway_fallback::validate_chain(candidate_chain)?;
        let mut machine = Self {
            candidate_count: candidate_chain.len(),
            candidate_index: 0,
            retries_on_candidate: 0,
            phase: Phase::AwaitingFirstByte,
            attempts: Vec::new(),
        };
        machine.start_attempt(0);
        Ok(machine)
    }

    /// 通知状态机上游将要交付的字节块。空块不越过首字节边界。
    pub fn observe_upstream_chunk(&mut self, chunk: &[u8]) -> Result<(), StreamMachineRefusal> {
        self.ensure_not_terminal()?;
        if self.phase == Phase::WaitingRetryAfter {
            return Err(StreamMachineRefusal::AwaitingRetryAfter);
        }
        if chunk.is_empty() {
            return Ok(());
        }
        self.phase = Phase::Streaming;
        if let Some(attempt) = self.attempts.last_mut() {
            attempt.observed_first_byte = true;
        }
        Ok(())
    }

    /// 结算当前尝试的失败并给出唯一后续动作。
    pub fn fail(
        &mut self,
        eligibility: Eligibility,
    ) -> Result<StreamFailureAction, StreamMachineRefusal> {
        self.ensure_not_terminal()?;
        if self.phase == Phase::WaitingRetryAfter {
            return Err(StreamMachineRefusal::AwaitingRetryAfter);
        }

        if self.phase == Phase::Streaming {
            self.finish_current(AttemptOutcome::AbortedAfterFirstByte);
            self.phase = Phase::Terminal;
            return Ok(StreamFailureAction::AbortPartialResponse);
        }

        self.finish_current(AttemptOutcome::FailedBeforeFirstByte);
        if eligibility == Eligibility::Throttled {
            if self.can_start_attempt(self.candidate_index) {
                self.phase = Phase::WaitingRetryAfter;
                return Ok(StreamFailureAction::WaitForRetryAfter);
            }
            self.phase = Phase::Terminal;
            return Ok(StreamFailureAction::StopBeforeOutput);
        }

        if gateway_retry::budget(eligibility, self.retries_on_candidate)
            && self.can_start_attempt(self.candidate_index)
        {
            self.retries_on_candidate += 1;
            if self.start_attempt(self.candidate_index) {
                return Ok(StreamFailureAction::RetrySameCandidate);
            }
        }

        if gateway_fallback::should_fallback(eligibility)
            && self.candidate_index + 1 < self.candidate_count
        {
            self.candidate_index += 1;
            self.retries_on_candidate = 0;
            if self.start_attempt(self.candidate_index) {
                return Ok(StreamFailureAction::FallbackNextCandidate);
            }
        }

        self.phase = Phase::Terminal;
        Ok(StreamFailureAction::StopBeforeOutput)
    }

    /// 调用方确认 Retry-After 条件满足后，沿同一候选和同一请求预算恢复。
    /// 此方法不读取时钟或自行等待。
    pub fn resume_after_retry_after(
        &mut self,
    ) -> Result<StreamFailureAction, StreamMachineRefusal> {
        self.ensure_not_terminal()?;
        if self.phase != Phase::WaitingRetryAfter {
            return Err(StreamMachineRefusal::NotAwaitingRetryAfter);
        }
        if self.start_attempt(self.candidate_index) {
            Ok(StreamFailureAction::RetrySameCandidate)
        } else {
            self.phase = Phase::Terminal;
            Ok(StreamFailureAction::StopBeforeOutput)
        }
    }

    /// 正常结束当前请求并留下完成终态。
    pub fn complete(&mut self) -> Result<(), StreamMachineRefusal> {
        self.ensure_not_terminal()?;
        if self.phase == Phase::WaitingRetryAfter {
            return Err(StreamMachineRefusal::AwaitingRetryAfter);
        }
        self.finish_current(AttemptOutcome::Completed);
        self.phase = Phase::Terminal;
        Ok(())
    }

    /// 取消当前请求；后续字节和失败都必须被拒绝。
    pub fn cancel(&mut self) -> Result<(), StreamMachineRefusal> {
        self.ensure_not_terminal()?;
        if self.phase != Phase::WaitingRetryAfter {
            self.finish_current(AttemptOutcome::Cancelled);
        }
        self.phase = Phase::Terminal;
        Ok(())
    }

    pub fn attempts(&self) -> &[AttemptRecord] {
        &self.attempts
    }

    /// 已启动的每次尝试是否都已写入终态（等待 Retry-After 时也成立）。
    pub fn attempts_are_settled(&self) -> bool {
        self.attempts
            .iter()
            .all(|attempt| attempt.outcome.is_some())
    }

    fn ensure_not_terminal(&self) -> Result<(), StreamMachineRefusal> {
        if self.phase == Phase::Terminal {
            Err(StreamMachineRefusal::Terminal)
        } else {
            Ok(())
        }
    }

    fn can_start_attempt(&self, candidate_index: usize) -> bool {
        let request_limit = self.request_attempt_limit();
        self.attempts.len() < request_limit
            && self
                .attempts
                .iter()
                .filter(|attempt| attempt.candidate_index == candidate_index)
                .count()
                < MAX_ATTEMPTS_PER_CANDIDATE
    }

    fn request_attempt_limit(&self) -> usize {
        self.candidate_count
            .saturating_mul(MAX_ATTEMPTS_PER_CANDIDATE)
            .min(MAX_TOTAL_UPSTREAM_ATTEMPTS)
    }

    fn start_attempt(&mut self, candidate_index: usize) -> bool {
        if !self.can_start_attempt(candidate_index) {
            return false;
        }
        self.attempts.push(AttemptRecord {
            candidate_index,
            outcome: None,
            observed_first_byte: false,
        });
        self.phase = Phase::AwaitingFirstByte;
        true
    }

    fn finish_current(&mut self, outcome: AttemptOutcome) {
        if let Some(attempt) = self.attempts.last_mut() {
            attempt.outcome = Some(outcome);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(count: usize) -> Vec<String> {
        (0..count)
            .map(|index| format!("candidate-{index}"))
            .collect()
    }

    #[test]
    fn empty_chunks_do_not_commit_and_transient_failure_retries_same_candidate() {
        let mut machine = StreamFailureMachine::new(&chain(2)).unwrap();
        machine.observe_upstream_chunk(&[]).unwrap();
        assert_eq!(
            machine.resume_after_retry_after(),
            Err(StreamMachineRefusal::NotAwaitingRetryAfter)
        );

        assert_eq!(
            machine.fail(Eligibility::Transient),
            Ok(StreamFailureAction::RetrySameCandidate)
        );
        assert_eq!(machine.attempts().len(), 2);
        assert_eq!(machine.attempts()[0].candidate_index, 0);
        assert_eq!(
            machine.attempts()[0].outcome,
            Some(AttemptOutcome::FailedBeforeFirstByte)
        );
        assert_eq!(machine.attempts()[1].outcome, None);
        assert!(!machine.attempts_are_settled());
    }

    #[test]
    fn partial_response_failure_is_terminal_and_never_retries_or_falls_back() {
        let mut machine = StreamFailureMachine::new(&chain(2)).unwrap();
        machine.observe_upstream_chunk(b"data: partial").unwrap();

        assert_eq!(
            machine.fail(Eligibility::Transient),
            Ok(StreamFailureAction::AbortPartialResponse)
        );
        assert_eq!(machine.attempts().len(), 1, "不得启动第二次上游尝试");
        assert_eq!(machine.attempts()[0].candidate_index, 0);
        assert_eq!(
            machine.attempts()[0].outcome,
            Some(AttemptOutcome::AbortedAfterFirstByte)
        );
        assert!(machine.attempts()[0].observed_first_byte);
        assert!(machine.attempts_are_settled());
        assert_eq!(
            machine.observe_upstream_chunk(b"data: second answer"),
            Err(StreamMachineRefusal::Terminal),
            "终止后不得拼接第二份响应或重复交付工具事件"
        );
        assert_eq!(
            machine.fail(Eligibility::Transient),
            Err(StreamMachineRefusal::Terminal)
        );
    }

    #[test]
    fn exhausted_candidate_retry_budget_falls_back_in_order_then_stops() {
        let mut machine = StreamFailureMachine::new(&chain(2)).unwrap();
        assert_eq!(
            machine.fail(Eligibility::Transient),
            Ok(StreamFailureAction::RetrySameCandidate)
        );
        assert_eq!(
            machine.fail(Eligibility::Transient),
            Ok(StreamFailureAction::RetrySameCandidate)
        );
        assert_eq!(
            machine.fail(Eligibility::Transient),
            Ok(StreamFailureAction::FallbackNextCandidate)
        );
        assert_eq!(machine.attempts().last().unwrap().candidate_index, 1);
        assert_eq!(
            machine.fail(Eligibility::Transient),
            Ok(StreamFailureAction::RetrySameCandidate)
        );
        assert_eq!(
            machine.fail(Eligibility::Transient),
            Ok(StreamFailureAction::RetrySameCandidate)
        );
        assert_eq!(
            machine.fail(Eligibility::Transient),
            Ok(StreamFailureAction::StopBeforeOutput)
        );
        assert!(machine.attempts_are_settled());
        assert_eq!(machine.attempts().len(), 6);
        assert!(machine
            .attempts()
            .iter()
            .all(|attempt| { attempt.outcome == Some(AttemptOutcome::FailedBeforeFirstByte) }));
    }

    #[test]
    fn maximum_candidate_chain_cannot_exceed_total_attempt_budget() {
        let mut machine = StreamFailureMachine::new(&chain(MAX_FALLBACK_CHAIN)).unwrap();
        loop {
            let action = machine.fail(Eligibility::Transient).unwrap();
            if action == StreamFailureAction::StopBeforeOutput {
                break;
            }
            assert!(matches!(
                action,
                StreamFailureAction::RetrySameCandidate
                    | StreamFailureAction::FallbackNextCandidate
            ));
            assert!(machine.attempts().len() <= MAX_TOTAL_UPSTREAM_ATTEMPTS);
        }
        assert_eq!(machine.attempts().len(), MAX_TOTAL_UPSTREAM_ATTEMPTS);
        assert!(machine.attempts_are_settled());
    }

    #[test]
    fn non_transient_failures_do_not_auto_retry_or_fallback() {
        for eligibility in [
            Eligibility::Credential,
            Eligibility::Request,
            Eligibility::Configuration,
        ] {
            let mut machine = StreamFailureMachine::new(&chain(2)).unwrap();
            assert_eq!(
                machine.fail(eligibility),
                Ok(StreamFailureAction::StopBeforeOutput),
                "{eligibility:?} must stop before output"
            );
            assert_eq!(machine.attempts().len(), 1);
            assert!(machine.attempts_are_settled());
        }

        let mut throttled = StreamFailureMachine::new(&chain(2)).unwrap();
        assert_eq!(
            throttled.fail(Eligibility::Throttled),
            Ok(StreamFailureAction::WaitForRetryAfter)
        );
        assert_eq!(throttled.attempts().len(), 1);
        assert!(throttled.attempts_are_settled());
        assert_eq!(
            throttled.observe_upstream_chunk(b"too early"),
            Err(StreamMachineRefusal::AwaitingRetryAfter)
        );
        assert_eq!(
            throttled.complete(),
            Err(StreamMachineRefusal::AwaitingRetryAfter)
        );
        assert_eq!(
            throttled.fail(Eligibility::Transient),
            Err(StreamMachineRefusal::AwaitingRetryAfter)
        );
        assert_eq!(
            throttled.resume_after_retry_after(),
            Ok(StreamFailureAction::RetrySameCandidate)
        );
        assert_eq!(throttled.attempts().len(), 2);
        assert_eq!(throttled.attempts()[1].candidate_index, 0);
        assert_eq!(
            throttled.attempts()[0].outcome,
            Some(AttemptOutcome::FailedBeforeFirstByte)
        );
        assert_eq!(throttled.attempts()[1].outcome, None);
        assert!(!throttled.attempts_are_settled());
        assert_eq!(
            throttled.fail(Eligibility::Throttled),
            Ok(StreamFailureAction::WaitForRetryAfter)
        );
        assert_eq!(
            throttled.resume_after_retry_after(),
            Ok(StreamFailureAction::RetrySameCandidate)
        );
        assert_eq!(throttled.attempts().len(), 3);
        assert_eq!(
            throttled.fail(Eligibility::Throttled),
            Ok(StreamFailureAction::StopBeforeOutput),
            "Retry-After 恢复也消耗每候选预算，且不能降级"
        );
        assert!(throttled.attempts_are_settled());
    }

    #[test]
    fn completion_and_cancellation_close_the_attempt_once() {
        let mut completed = StreamFailureMachine::new(&chain(1)).unwrap();
        completed.observe_upstream_chunk(b"ok").unwrap();
        completed.complete().unwrap();
        assert_eq!(
            completed.attempts()[0].outcome,
            Some(AttemptOutcome::Completed)
        );
        assert!(completed.attempts_are_settled());
        assert_eq!(completed.complete(), Err(StreamMachineRefusal::Terminal));

        let mut cancelled = StreamFailureMachine::new(&chain(1)).unwrap();
        cancelled.cancel().unwrap();
        assert_eq!(
            cancelled.attempts()[0].outcome,
            Some(AttemptOutcome::Cancelled)
        );
        assert!(cancelled.attempts_are_settled());
        assert_eq!(
            cancelled.observe_upstream_chunk(b"late"),
            Err(StreamMachineRefusal::Terminal)
        );

        let mut cancelled_while_waiting = StreamFailureMachine::new(&chain(2)).unwrap();
        assert_eq!(
            cancelled_while_waiting.fail(Eligibility::Throttled),
            Ok(StreamFailureAction::WaitForRetryAfter)
        );
        cancelled_while_waiting.cancel().unwrap();
        assert!(cancelled_while_waiting.attempts_are_settled());
        assert_eq!(
            cancelled_while_waiting.resume_after_retry_after(),
            Err(StreamMachineRefusal::Terminal)
        );
    }

    #[test]
    fn candidate_validation_is_reused_and_total_budget_is_bounded() {
        assert_eq!(
            StreamFailureMachine::new(&[]).err(),
            Some(FallbackRefusal::EmptyChain)
        );
        let duplicate = vec!["same".to_string(), "same".to_string()];
        assert_eq!(
            StreamFailureMachine::new(&duplicate).err(),
            Some(FallbackRefusal::DuplicateCandidate)
        );
        assert_eq!(MAX_TOTAL_UPSTREAM_ATTEMPTS, 24);
    }
}
