//! 手机推送与移动设备的**注册、配对与前置判据**（NOT-04 首片）。
//!
//! 需求原文：设备注册/配对/撤销，后台/离线补偿，点击精确进入对应事项；
//! **渠道推送与自有 App 分开**（ntfy/PushPlus 等渠道推送不等于自建移动入口）。
//!
//! 与 SMS（NOT-03）同一条理由：**语义先于实现定死**——设备 token 的生命周期、
//! 配对码生成、撤销后的失效、离线补偿的窗口，等接第一个真机时再定会各写一套。
//!
//! 本模块只做**前置判据**（纯函数，不碰网络、不读真钥匙串）：
//!
//! 1. **两类入口明确分开**：`PushKind::Channel`（渠道推送，如 ntfy/第三方）
//!    与 `PushKind::NativeApp`（自有 App 注册的设备）在类型上分开——
//!    不能把第三方中转的通道当作自有设备管理。
//! 2. **配对与注册**：一次性配对码有时效；成功配对生成设备 token；
//!    同一设备重复注册更新时间戳（幂等），不产出两个设备实例。
//! 3. **撤销即失效**：被撤销的设备 token 立即不可发——**撤销后的投递尝试直接拒绝**，
//!    不进入网络重试（那不是链路抖动）。
//! 4. **离线补偿**：离线期间累积的提醒**有界保留**（最近 N 条）；
//!    超过窗口的陈旧通知不补发——避免手机连上网时遭遇「通知风暴」。
//!
//! **不实现的部分（如实）**：APNs/FCM 真实网络交互、自有移动端 App、
//! 真机点击通知后的深链跳转——那些需要移动端工程与真实苹果/谷歌开发者凭据。

use std::collections::HashMap;

/// 推送入口的种类（需求明文要求「分开」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushKind {
    /// 现有第三方渠道推送（ntfy、PushPlus 等）。
    Channel,
    /// 自有 App 绑定的真机设备（APNs / FCM）。
    NativeApp,
}

impl PushKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Channel => "channel",
            Self::NativeApp => "native_app",
        }
    }

    /// 该入口是否由「设备注册表」管理。渠道推送走通道配置，不受设备表管。
    pub fn is_device_managed(self) -> bool {
        matches!(self, Self::NativeApp)
    }
}

/// 移动设备记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub platform: String, // "ios" | "android"
    pub device_token: String,
    pub paired_at_ms: i64,
    pub last_seen_ms: i64,
    /// 是否已被人工撤销。
    pub revoked: bool,
}

/// 配对码（一次性，有时效）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingCode {
    pub code: String,
    pub expires_at_ms: i64,
}

impl PairingCode {
    pub fn is_valid(&self, now_ms: i64) -> bool {
        now_ms < self.expires_at_ms
    }
}

/// 离线待补偿的一条通知。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPush {
    pub event_id: String,
    pub created_at_ms: i64,
    pub title: String,
}

/// 补偿队列上限：离线期间最多保留 20 条。
pub const COMPENSATE_CAP: usize = 20;
/// 补偿窗口：超过 24 小时的离线通知不再补发（那是历史不是提醒）。
pub const COMPENSATE_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;

/// 推送前置拒绝原因。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushRefusal {
    /// 设备不存在。
    DeviceNotFound,
    /// 设备已被撤销。
    DeviceRevoked,
    /// 渠道推送走错了设备管理入口。
    ChannelMisrouted,
    /// 补偿通知已过期。
    CompensateExpired,
}

impl PushRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DeviceNotFound => "device_not_found",
            Self::DeviceRevoked => "device_revoked",
            Self::ChannelMisrouted => "channel_misrouted",
            Self::CompensateExpired => "compensate_expired",
        }
    }
}

/// 设备注册表（纯内存判据状态）。
#[derive(Debug, Default)]
pub struct DeviceRegistry {
    devices: HashMap<String, Device>,
    pairing: Option<PairingCode>,
    /// 设备 id → 离线期间待补偿的通知队列。
    pending: HashMap<String, Vec<PendingPush>>,
}

impl DeviceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 发行一次性配对码。
    pub fn issue_pairing_code(&mut self, code: &str, now_ms: i64, ttl_ms: i64) {
        self.pairing = Some(PairingCode {
            code: code.trim().to_string(),
            expires_at_ms: now_ms + ttl_ms.max(1000),
        });
    }

    /// 用配对码注册设备。
    ///
    /// - 配对码错/过期 ⇒ 拒绝；
    /// - 同一设备 id 重复注册 ⇒ 更新 token 与活跃时刻（幂等），不产出两个实例；
    /// - 成功后配对码**立刻作废**（一次性）。
    pub fn register_with_code(
        &mut self,
        code: &str,
        device_id: &str,
        platform: &str,
        device_token: &str,
        now_ms: i64,
    ) -> Result<&Device, &'static str> {
        let valid = self
            .pairing
            .as_ref()
            .is_some_and(|p| p.code == code && p.is_valid(now_ms));
        if !valid {
            return Err("pairing_code_invalid");
        }
        self.pairing = None; // 一次性
        let entry = self
            .devices
            .entry(device_id.to_string())
            .or_insert_with(|| Device {
                id: device_id.to_string(),
                platform: platform.to_string(),
                device_token: device_token.to_string(),
                paired_at_ms: now_ms,
                last_seen_ms: now_ms,
                revoked: false,
            });
        entry.device_token = device_token.to_string();
        entry.last_seen_ms = now_ms;
        entry.revoked = false; // 重新配对解除撤销
        Ok(entry)
    }

    /// 撤销一个设备。撤销后它的 token 立即不可发。
    pub fn revoke(&mut self, device_id: &str) -> bool {
        if let Some(device) = self.devices.get_mut(device_id) {
            device.revoked = true;
            self.pending.remove(device_id); // 撤销同时清空离线队列
            true
        } else {
            false
        }
    }

    /// 推送前置检查。
    pub fn precheck(&self, kind: PushKind, device_id: &str) -> Result<&Device, PushRefusal> {
        if !kind.is_device_managed() {
            return Err(PushRefusal::ChannelMisrouted);
        }
        let device = self.devices.get(device_id).ok_or(PushRefusal::DeviceNotFound)?;
        if device.revoked {
            return Err(PushRefusal::DeviceRevoked);
        }
        Ok(device)
    }

    /// 为设备累积一条离线待补偿通知。
    ///
    /// 超过 `COMPENSATE_CAP`（20 条）时淘汰最旧的；被撤销的设备不累积。
    pub fn enqueue_offline(&mut self, device_id: &str, event_id: &str, title: &str, now_ms: i64) {
        if self.devices.get(device_id).is_some_and(|d| d.revoked) {
            return;
        }
        let queue = self.pending.entry(device_id.to_string()).or_default();
        if queue.iter().any(|p| p.event_id == event_id) {
            return; // 幂等：同一事件不重复补偿
        }
        if queue.len() >= COMPENSATE_CAP {
            queue.remove(0); // 淘汰最旧
        }
        queue.push(PendingPush {
            event_id: event_id.to_string(),
            created_at_ms: now_ms,
            title: title.to_string(),
        });
    }

    /// 取出设备所有**仍在窗口内**的待补偿通知，并清空队列。
    pub fn drain_offline(&mut self, device_id: &str, now_ms: i64) -> Vec<PendingPush> {
        let queue = self.pending.remove(device_id).unwrap_or_default();
        queue
            .into_iter()
            .filter(|p| now_ms - p.created_at_ms <= COMPENSATE_WINDOW_MS)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_push_is_separated_from_device_management() {
        let reg = DeviceRegistry::new();
        // 渠道推送走设备入口直接拒绝——类型层就分清
        assert_eq!(
            reg.precheck(PushKind::Channel, "any-id"),
            Err(PushRefusal::ChannelMisrouted)
        );
        assert!(!PushKind::Channel.is_device_managed());
        assert!(PushKind::NativeApp.is_device_managed());
    }

    #[test]
    fn pairing_code_is_one_time_and_expires() {
        let mut reg = DeviceRegistry::new();
        reg.issue_pairing_code("123456", 1_000, 60_000); // 60s 有效
        // 过期前配对成功
        let registered = reg.register_with_code("123456", "dev-1", "ios", "token-a", 2_000);
        assert!(registered.is_ok());
        // 配对码是一次性的：同一码再用一次被拒
        assert_eq!(
            reg.register_with_code("123456", "dev-2", "ios", "token-b", 2_000),
            Err("pairing_code_invalid")
        );
        // 过期的码直接拒
        reg.issue_pairing_code("999999", 1_000, 10_000);
        assert_eq!(
            reg.register_with_code("999999", "dev-3", "android", "token-c", 20_000),
            Err("pairing_code_invalid")
        );
    }

    #[test]
    fn revoked_device_refuses_sends_immediately() {
        let mut reg = DeviceRegistry::new();
        reg.issue_pairing_code("code-1", 1_000, 60_000);
        reg.register_with_code("code-1", "dev-1", "ios", "token-a", 1_000).unwrap();
        // 撤销前允许
        assert!(reg.precheck(PushKind::NativeApp, "dev-1").is_ok());
        // 撤销
        assert!(reg.revoke("dev-1"));
        // 撤销后立即拒绝——不进网络重试
        assert_eq!(
            reg.precheck(PushKind::NativeApp, "dev-1"),
            Err(PushRefusal::DeviceRevoked)
        );
        assert_eq!(
            PushRefusal::DeviceRevoked.as_str(),
            "device_revoked"
        );
    }

    #[test]
    fn offline_compensation_is_bounded_and_drops_expired_entries() {
        let mut reg = DeviceRegistry::new();
        reg.issue_pairing_code("code-1", 1_000, 60_000);
        reg.register_with_code("code-1", "dev-1", "ios", "token-a", 1_000).unwrap();

        // 塞 25 条：上限 20 条，最旧 5 条被挤掉
        for i in 0..25 {
            reg.enqueue_offline("dev-1", &format!("evt-{i}"), &format!("通知 {i}"), 2_000 + i);
        }
        // 幂等：同一事件不重进
        reg.enqueue_offline("dev-1", "evt-24", "重放", 3_000);

        // 连网取出：恰好 20 条、最旧的是 evt-5（0..5 被挤掉）
        let drained = reg.drain_offline("dev-1", 3_000);
        assert_eq!(drained.len(), COMPENSATE_CAP);
        assert_eq!(drained[0].event_id, "evt-5");
        assert_eq!(drained.last().unwrap().event_id, "evt-24");

        // 超出 24 小时窗口的不补发
        reg.enqueue_offline("dev-1", "old-1", "昨天的", 10_000);
        let future = 10_000 + COMPENSATE_WINDOW_MS + 1000;
        let expired = reg.drain_offline("dev-1", future);
        assert!(expired.is_empty(), "超窗的离线通知不得补发");
    }
}
