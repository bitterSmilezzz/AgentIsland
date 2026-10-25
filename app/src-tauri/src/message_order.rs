//! 对话过程消息的**顺序重排与断线续接判据**（TASK-04 首片）。
//!
//! 需求原文：过程消息、结构化提问、反馈修订；场景 token：**消息顺序/断线续接**。
//!
//! 背景：在网络抖动、流式分片或跨进程异步推送下，消息可能乱序到达；
//! 机器休眠唤醒或连接断开后，重连推过来的消息流可能出现**间隙（gap）**或
//! **重叠（overlap）**。没有集中判据时，每个入口（Claude plan receiver、
//! webhook、session probe）各做一套去重，容易出现「先到的被丢掉、后到的
//! 插在前面」的消息倒序事故。
//!
//! 本模块做四条纯函数判据：
//!
//! 1. **单调序号重排**：消息携带序号 `seq`（单调递增）；乱序到达的消息
//!    按 `seq` **缓冲并排序**，只有连续的前缀才交付给下游（上屏/落盘）。
//! 2. **重复丢弃（幂等）**：已交付过的序号再次到达，**静默丢弃**——
//!    重连推过来的重叠部分不产生第二条消息。
//! 3. **断线间隙检测（gap）**：如果收到的序号与上一次交付的序号之间有空洞
//!    （如已交付到 3，下一条直接是 5，4 迟迟未到），等待有界窗口；
//!    超时后**声明间隙**并推进，不永久挂起。
//! 4. **缓冲区有界**：乱序缓冲区最多容纳 `MAX_BUFFER` 条消息，超限时
//!    淘汰最远未来的序号（防止恶意发送方塞大序号耗尽内存）。

use std::collections::BTreeMap;

/// 一条过程消息的元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderedMessage {
    pub seq: u64,
    pub content: String,
    pub role: String, // "user" | "assistant" | "system"
}

/// 缓冲重排器的处理结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intake {
    /// 这批消息可以按序交付了（可能包含多条排好序的连续消息）。
    Deliver(Vec<OrderedMessage>),
    /// 暂时没有可以交付的消息（正在等待空洞补齐，或收到的是重复消息）。
    Buffered,
    /// 出现空洞且等待超时：声明间隙跳过，交付后续消息。
    GapDetected { missing_from: u64, missing_to: u64 },
}

/// 乱序缓冲区的最大容量。
pub const MAX_BUFFER: usize = 200;

/// 消息顺序与续接管理器。
#[derive(Debug, Default)]
pub struct MessageSequencer {
    /// 上一次交付给下游的最高序号。0 = 尚未交付过任何消息。
    last_delivered_seq: u64,
    /// 暂存的乱序消息（按序号自然排序）。
    buffer: BTreeMap<u64, OrderedMessage>,
    /// 是否已初始化起点（第一条消息建立起点）。
    initialized: bool,
}

impl MessageSequencer {
    pub fn new() -> Self {
        Self::default()
    }

    /// 上一次成功交付的序号。
    pub fn last_delivered(&self) -> u64 {
        self.last_delivered_seq
    }

    /// 接收一条消息，返回当前可交付的有序列表。
    ///
    /// - 第一条到达的消息建立起点序号；
    /// - 序号 <= 已交付序号 ⇒ 视为重放/重叠，**静默丢弃**；
    /// - 恰好等于 `last + 1` ⇒ 立即交付，并顺带交付缓冲区里连续的后续消息；
    /// - 大于 `last + 1` ⇒ 进缓冲区等待空洞补齐。
    pub fn push(&mut self, message: OrderedMessage) -> Intake {
        let seq = message.seq;

        // 首条消息初始化起点
        if !self.initialized {
            self.initialized = true;
            self.last_delivered_seq = seq;
            return Intake::Deliver(vec![message]);
        }

        // 已交付过的重复消息直接丢弃
        if seq <= self.last_delivered_seq {
            return Intake::Buffered;
        }

        // 重复到达但尚未交付的，不覆盖已缓冲的
        self.buffer.entry(seq).or_insert(message);

        // 防内存膨胀：超限时淘汰缓冲区里序号最大的
        while self.buffer.len() > MAX_BUFFER {
            if let Some(highest) = self.buffer.keys().next_back().copied() {
                self.buffer.remove(&highest);
            }
        }

        // 尝试从 last + 1 开始交付连续段
        let mut deliverable = Vec::new();
        while let Some(msg) = self.buffer.remove(&(self.last_delivered_seq + 1)) {
            self.last_delivered_seq = msg.seq;
            deliverable.push(msg);
        }

        if deliverable.is_empty() {
            Intake::Buffered
        } else {
            Intake::Deliver(deliverable)
        }
    }

    /// 声明超时放弃等待空洞，强行跳过间隙并交付后续已缓冲的消息。
    ///
    /// 用于断线后丢失某些消息的场景：不能因一条丢失让整条对话永远卡住。
    pub fn force_gap_skip(&mut self) -> Intake {
        let Some(&lowest_buffered) = self.buffer.keys().next() else {
            return Intake::Buffered;
        };
        let missing_from = self.last_delivered_seq + 1;
        let missing_to = lowest_buffered - 1;

        // 推进起点到缓冲区的最小序号
        let mut deliverable = Vec::new();
        if let Some(msg) = self.buffer.remove(&lowest_buffered) {
            self.last_delivered_seq = msg.seq;
            deliverable.push(msg);
        }
        while let Some(msg) = self.buffer.remove(&(self.last_delivered_seq + 1)) {
            self.last_delivered_seq = msg.seq;
            deliverable.push(msg);
        }

        Intake::GapDetected {
            missing_from,
            missing_to,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(seq: u64, text: &str) -> OrderedMessage {
        OrderedMessage {
            seq,
            content: text.into(),
            role: "assistant".into(),
        }
    }

    #[test]
    fn in_order_messages_deliver_immediately() {
        let mut seq = MessageSequencer::new();
        assert_eq!(seq.push(msg(1, "一")), Intake::Deliver(vec![msg(1, "一")]));
        assert_eq!(seq.push(msg(2, "二")), Intake::Deliver(vec![msg(2, "二")]));
        assert_eq!(seq.push(msg(3, "三")), Intake::Deliver(vec![msg(3, "三")]));
        assert_eq!(seq.last_delivered(), 3);
    }

    #[test]
    fn out_of_order_messages_buffer_and_deliver_when_hole_is_filled() {
        let mut seq = MessageSequencer::new();
        assert_eq!(seq.push(msg(1, "一")), Intake::Deliver(vec![msg(1, "一")]));
        // 先到 3、再到 4：都不该交付（等 2）
        assert_eq!(seq.push(msg(3, "三")), Intake::Buffered);
        assert_eq!(seq.push(msg(4, "四")), Intake::Buffered);
        assert_eq!(seq.last_delivered(), 1, "未交付，起点不动");

        // 2 到达：空洞补齐，2/3/4 一起按序交付
        let result = seq.push(msg(2, "二"));
        assert_eq!(
            result,
            Intake::Deliver(vec![msg(2, "二"), msg(3, "三"), msg(4, "四")])
        );
        assert_eq!(seq.last_delivered(), 4);
    }

    #[test]
    fn duplicate_and_overlapping_messages_are_silently_dropped() {
        let mut seq = MessageSequencer::new();
        seq.push(msg(1, "一"));
        seq.push(msg(2, "二"));

        // 重复发送 1 和 2（重连推过来的重叠片段）：不得交付、也不得进缓冲区
        assert_eq!(seq.push(msg(1, "一-重放")), Intake::Buffered);
        assert_eq!(seq.push(msg(2, "二-重放")), Intake::Buffered);
        assert_eq!(seq.last_delivered(), 2, "重放不得影响已交付进度");
        assert!(
            seq.buffer.is_empty(),
            "已交付的重复消息不得堆积在缓冲区：{:?}",
            seq.buffer
        );

        // 接着发 3 照常交付
        assert_eq!(seq.push(msg(3, "三")), Intake::Deliver(vec![msg(3, "三")]));
    }

    #[test]
    fn gap_skip_advances_over_lost_messages_instead_of_hanging() {
        let mut seq = MessageSequencer::new();
        seq.push(msg(1, "一"));
        // 消息 2 永远丢失了；3、4 已到达
        seq.push(msg(3, "三"));
        seq.push(msg(4, "四"));

        // 超时触发跳空：跳过丢失的 2，把已就绪的 3 和 4 连续交付
        let gap = seq.force_gap_skip();
        assert_eq!(
            gap,
            Intake::GapDetected {
                missing_from: 2,
                missing_to: 2,
            }
        );
        assert_eq!(seq.last_delivered(), 4, "跳空后交付了 3 和紧随其后的连续 4");

        // 紧接着发 5：按新起点连续交付
        assert_eq!(seq.push(msg(5, "五")), Intake::Deliver(vec![msg(5, "五")]));
    }

    #[test]
    fn buffer_capacity_is_bounded() {
        let mut seq = MessageSequencer::new();
        seq.push(msg(1, "首条"));
        // 塞入超过 MAX_BUFFER 条乱序消息
        for i in 3..=(MAX_BUFFER as u64 + 10) {
            seq.push(msg(i, "乱序"));
        }
        assert!(seq.buffer.len() <= MAX_BUFFER, "缓冲区不得无界膨胀");
    }
}
