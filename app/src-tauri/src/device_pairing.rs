//! 扫码登录/配对与**能力差异说明**的判据（TASK-12 首片）。
//!
//! 需求原文：手机/桌面/Web 的任务、对话、方案/diff、审查、机器管理；
//! 扫码登录/配对与能力差异说明。场景 token：Android→iOS 配对/后台/弱网；
//! 手机读 plan/diff/答题；**二维码过期/重放**；跨端并发；Web 部署与认证。
//!
//! 与 NOT-04（推送设备注册）同源但对象不同：NOT-04 管「推送送到哪台设备」，
//! 本模块管「**谁通过扫码成为可信端**」与「**各端能力差在哪**」。
//!
//! 本模块只做**判据**（纯函数，不画二维码、不连服务端）：
//!
//! 1. **配对码一次性且有时效**：扫码码有短 TTL（默认 120 秒）；过期作废。
//! 2. **重放即拒**：用过的码立即作废——截图转发给别人等于把账号交出去，
//!    这是扫码登录与密码登录最大的差别。
//! 3. **扫码绑定**：绑定后得到设备标识与凭据引用；原始配对码**不落任何存储**。
//! 4. **能力差异如实说明**：不同端的能力集不同，界面必须明确告知「这个端能做 A、
//!    不能做 B」，而不是静默地把不可用的按钮藏起来。
//! 5. **跨端并发唯一归属**：同一份待办（回答/审批）的写操作**只归属一台设备**，
//!    两端同时操作时后到者被拒（`ClaimHeldByOtherDevice`）——两台设备各答一半
//!    是最坏的结果。

use std::collections::HashMap;

/// 终端类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    Desktop,
    Mobile,
    Web,
}

impl DeviceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Mobile => "mobile",
            Self::Web => "web",
        }
    }
}

/// 终端能力（可执行的动作集合）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    /// 查看任务与对话。
    ReadTasks,
    /// 阅读方案与 diff。
    ReadPlan,
    /// 回答问题 / 审批关口。
    Answer,
    /// 触发执行。
    TriggerRun,
    /// 管理机器与连接。
    ManageMachines,
    /// 接收推送。
    ReceivePush,
}

/// 各端的固定能力集（需求原文的能力差异）。
pub fn capabilities_for(kind: DeviceKind) -> Vec<Capability> {
    match kind {
        // 桌面：全能力
        DeviceKind::Desktop => vec![
            Capability::ReadTasks,
            Capability::ReadPlan,
            Capability::Answer,
            Capability::TriggerRun,
            Capability::ManageMachines,
            Capability::ReceivePush,
        ],
        // 手机：读 + 回答 + 推送；不能触发执行、不能管机器
        DeviceKind::Mobile => vec![
            Capability::ReadTasks,
            Capability::ReadPlan,
            Capability::Answer,
            Capability::ReceivePush,
        ],
        // Web：只读 + 回答；无推送（浏览器无 APNs 通道）、不能触发执行
        DeviceKind::Web => vec![
            Capability::ReadTasks,
            Capability::ReadPlan,
            Capability::Answer,
        ],
    }
}

/// 能力差异说明（界面据此如实告知用户）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityDiff {
    pub kind: DeviceKind,
    pub available: Vec<Capability>,
    pub unavailable: Vec<Capability>,
}

impl CapabilityDiff {
    /// 该端是否具备某能力。
    pub fn supports(&self, capability: Capability) -> bool {
        self.available.contains(&capability)
    }
}

/// 计算某端相对**全能力集**（桌面）的差异。
pub fn capability_diff(kind: DeviceKind) -> CapabilityDiff {
    let available = capabilities_for(kind);
    let unavailable = capabilities_for(DeviceKind::Desktop)
        .into_iter()
        .filter(|c| !available.contains(c))
        .collect();
    CapabilityDiff {
        kind,
        available,
        unavailable,
    }
}

/// 配对拒绝原因。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingRefusal {
    /// 配对码不存在或已被使用（重放）。
    CodeInvalid,
    /// 配对码已过期。
    CodeExpired,
    /// 该待办的写归属已被另一台设备持有。
    ClaimHeldByOtherDevice,
}

impl PairingRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CodeInvalid => "code_invalid",
            Self::CodeExpired => "code_expired",
            Self::ClaimHeldByOtherDevice => "claim_held_by_other_device",
        }
    }
}

/// 配对码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingCode {
    pub code: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    /// 是否已被使用（一次性）。
    pub used: bool,
}

/// 默认配对码有效期：120 秒（扫码场景要求短，截图转发风险窗口小）。
pub const PAIRING_CODE_TTL_MS: i64 = 120 * 1000;

/// 扫码配对管理器（纯判据状态）。
#[derive(Debug, Clone, Default)]
pub struct PairingManager {
    codes: Vec<PairingCode>,
    /// 已绑定的设备：device_id -> DeviceKind。
    devices: HashMap<String, DeviceKind>,
    /// 待办写归属：claim_id -> device_id。
    claims: HashMap<String, String>,
}

impl PairingManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// 签发一个配对码（二维码里放的就是它）。
    pub fn issue_code(&mut self, code: &str, now_ms: i64, ttl_ms: i64) {
        self.codes.push(PairingCode {
            code: code.to_string(),
            issued_at_ms: now_ms,
            expires_at_ms: now_ms + ttl_ms.max(1_000),
            used: false,
        });
    }

    /// 用码扫码绑定设备。
    ///
    /// 判据：码必须存在且未用过（重放即拒）、且未过期；成功后码**立即作废**。
    pub fn pair(
        &mut self,
        code: &str,
        device_id: &str,
        kind: DeviceKind,
        now_ms: i64,
    ) -> Result<DeviceKind, PairingRefusal> {
        let Some(entry) = self.codes.iter_mut().find(|c| c.code == code) else {
            return Err(PairingRefusal::CodeInvalid);
        };
        if entry.used {
            // 重放：截图转发出去后第二次扫码
            return Err(PairingRefusal::CodeInvalid);
        }
        if now_ms >= entry.expires_at_ms {
            return Err(PairingRefusal::CodeExpired);
        }
        entry.used = true;
        self.devices.insert(device_id.to_string(), kind);
        Ok(kind)
    }

    /// 该设备是否具备某能力（未配对的设备没有任何能力）。
    pub fn device_supports(&self, device_id: &str, capability: Capability) -> bool {
        match self.devices.get(device_id) {
            Some(kind) => capabilities_for(*kind).contains(&capability),
            None => false,
        }
    }

    /// 取得某待办的写归属。
    ///
    /// 第一次调用者获得归属；之后同一台设备可继续操作，**别的设备被拒**。
    pub fn claim(&mut self, claim_id: &str, device_id: &str) -> Result<(), PairingRefusal> {
        match self.claims.get(claim_id) {
            Some(holder) if holder != device_id => {
                Err(PairingRefusal::ClaimHeldByOtherDevice)
            }
            Some(_) => Ok(()),
            None => {
                self.claims.insert(claim_id.to_string(), device_id.to_string());
                Ok(())
            }
        }
    }

    /// 该设备当前持有的归属数量（用于诊断与界面展示）。
    pub fn claims_held(&self, device_id: &str) -> usize {
        self.claims
            .values()
            .filter(|holder| *holder == device_id)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_code_is_single_use_and_expires() {
        let mut mgr = PairingManager::new();
        mgr.issue_code("QR-123", 1_000, PAIRING_CODE_TTL_MS);

        // 有效期内扫码成功
        assert_eq!(
            mgr.pair("QR-123", "phone-1", DeviceKind::Mobile, 2_000),
            Ok(DeviceKind::Mobile)
        );

        // 重放：同一个码再扫一次 ⇒ 拒绝
        assert_eq!(
            mgr.pair("QR-123", "phone-2", DeviceKind::Mobile, 3_000),
            Err(PairingRefusal::CodeInvalid),
            "截图转发等于把账号交出去——码必须一次性"
        );

        // 过期码拒绝
        let mut mgr2 = PairingManager::new();
        mgr2.issue_code("QR-OLD", 1_000, PAIRING_CODE_TTL_MS);
        assert_eq!(
            mgr2.pair("QR-OLD", "phone-3", DeviceKind::Mobile, 1_000 + PAIRING_CODE_TTL_MS + 1),
            Err(PairingRefusal::CodeExpired)
        );
    }

    #[test]
    fn capability_differences_are_explicit_per_device_kind() {
        // 手机：能答不能触发执行/管机器
        let mobile = capability_diff(DeviceKind::Mobile);
        assert!(mobile.supports(Capability::Answer));
        assert!(!mobile.supports(Capability::TriggerRun));
        assert!(!mobile.supports(Capability::ManageMachines));
        assert_eq!(mobile.unavailable.len(), 2, "不可用能力必须被明确列出而不是静默隐藏");

        // Web：无推送能力
        let web = capability_diff(DeviceKind::Web);
        assert!(web.supports(Capability::ReadPlan));
        assert!(!web.supports(Capability::ReceivePush));
        assert!(web.unavailable.contains(&Capability::ReceivePush));

        // 桌面：全能力
        let desktop = capability_diff(DeviceKind::Desktop);
        assert!(desktop.unavailable.is_empty());
        assert_eq!(desktop.available.len(), 6);
    }

    #[test]
    fn a_paired_device_gets_only_its_own_capabilities() {
        let mut mgr = PairingManager::new();
        mgr.issue_code("QR-1", 1_000, PAIRING_CODE_TTL_MS);
        mgr.pair("QR-1", "phone-1", DeviceKind::Mobile, 2_000).unwrap();

        assert!(mgr.device_supports("phone-1", Capability::ReadPlan));
        assert!(!mgr.device_supports("phone-1", Capability::TriggerRun));
        // 未配对的设备没有任何能力
        assert!(!mgr.device_supports("stranger", Capability::ReadTasks));
    }

    #[test]
    fn a_write_claim_belongs_to_one_device_only() {
        let mut mgr = PairingManager::new();
        // phone-1 先取得归属
        assert_eq!(mgr.claim("claim-1", "phone-1"), Ok(()));
        // 同一台设备可继续
        assert_eq!(mgr.claim("claim-1", "phone-1"), Ok(()));
        // 另一台设备被拒
        assert_eq!(
            mgr.claim("claim-1", "phone-2"),
            Err(PairingRefusal::ClaimHeldByOtherDevice),
            "两台设备各答一半是最坏的结果——归属必须唯一"
        );
        assert_eq!(mgr.claims_held("phone-1"), 1);
    }
}
