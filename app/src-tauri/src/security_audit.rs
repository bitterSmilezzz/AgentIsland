//! 团队权限、双轨审计与最小凭据注入的**判据**（TASK-11 首片）。
//!
//! 需求原文：人/Agent/机器/API key 分层权限、密钥注入、变更/人类操作双轨日志、
//! 远程 shell 双授权。场景 token：人/Agent/机器/key 交叉权限；远程 shell 双开关；
//! 秘密最小注入/撤销；双轨审计不可冒充。
//!
//! 与 SEC-01、GW-01 同属凭据安全面：**权限与审计语义先于实现定死**。
//!
//! 本模块只做**判据**（纯函数，不碰钥匙串、不发网络、不起 shell）：
//!
//! 1. **双轨审计不可冒充**：记录携带 `Actor`（Human/Agent/Machine/ApiKey 四分层）；
//!    主体身份由**系统调用边界**绑定，不得由消息正文里的字段伪装（Agent 不能通过
//!    在 payload 里填 "human" 冒充人类操作）。
//! 2. **远程 shell 双授权（双开关）**：执行远程 shell 必须同时满足：
//!    - 全局主开关开启（`master_enabled`）；
//!    - 对应目标机器/会话的显式单独授权（`target_authorized`）；
//!    双开关缺任一即**拒绝**，拒绝码固定且不执行。
//! 3. **最小秘密注入**：向子进程/Agent 注入凭据时只注入**显式白名单**的环境变量，
//!    不拷贝父进程的完整环境（防止把未相关的 AWS/GitHub/SSH 凭据带进 Agent 进程）。
//! 4. **凭据撤销即失效**：被撤销的凭据引用立即返回 `None`，不退回旧值缓存。

use std::collections::HashMap;

/// 操作主体（四分层权限模型）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// 人类操作者（桌面 GUI 点击、确认横幅交互）。
    Human { user_id: String },
    /// 本地运行的 Agent（通过 hook / webhook 投递）。
    Agent { agent_id: String },
    /// 外部机器 / 自动化系统（CI、定时任务、Webhook 调用方）。
    Machine { machine_id: String },
    /// API Key 持有者（通过 HTTP Bearer 认证的调用方）。
    ApiKey { key_id: String },
}

impl Actor {
    pub fn role_str(&self) -> &'static str {
        match self {
            Self::Human { .. } => "human",
            Self::Agent { .. } => "agent",
            Self::Machine { .. } => "machine",
            Self::ApiKey { .. } => "api_key",
        }
    }

    pub fn identifier(&self) -> &str {
        match self {
            Self::Human { user_id } => user_id,
            Self::Agent { agent_id } => agent_id,
            Self::Machine { machine_id } => machine_id,
            Self::ApiKey { key_id } => key_id,
        }
    }
}

/// 一条不可冒充的审计记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub seq: u64,
    pub actor: Actor,
    pub action: String,
    pub target: String,
    pub at_ms: i64,
}

/// 双轨审计日志管理器（内存判据状态）。
#[derive(Debug, Default)]
pub struct DualTrackAudit {
    entries: Vec<AuditEntry>,
}

impl DualTrackAudit {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录一条操作。主体身份由调用上下文**强行绑定**，不是从 action/target 里解析。
    pub fn record(&mut self, actor: Actor, action: &str, target: &str, now_ms: i64) -> u64 {
        let seq = self.entries.len() as u64 + 1;
        self.entries.push(AuditEntry {
            seq,
            actor,
            action: action.trim().to_string(),
            target: target.trim().to_string(),
            at_ms: now_ms,
        });
        seq
    }

    /// 过滤出所有人类操作（人类轨）。
    pub fn human_track(&self) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .filter(|e| matches!(e.actor, Actor::Human { .. }))
            .collect()
    }

    /// 过滤出所有非人类操作（自动化/机器/Agent 轨）。
    pub fn automated_track(&self) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .filter(|e| !matches!(e.actor, Actor::Human { .. }))
            .collect()
    }
}

/// 远程 shell 执行授权判据（双开关）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteShellPolicy {
    /// 全局主开关。
    pub master_enabled: bool,
    /// 目标是否经过显式单独授权。
    pub target_authorized: bool,
}

/// 权限拒绝原因。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthRefusal {
    /// 全局主开关关闭。
    MasterDisabled,
    /// 目标未单独授权。
    TargetUnauthorized,
    /// 凭据已被撤销。
    SecretRevoked,
}

impl AuthRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MasterDisabled => "master_disabled",
            Self::TargetUnauthorized => "target_unauthorized",
            Self::SecretRevoked => "secret_revoked",
        }
    }
}

/// 远程 shell 执行前置判定（双开关缺一即拒）。
pub fn authorize_remote_shell(policy: RemoteShellPolicy) -> Result<(), AuthRefusal> {
    if !policy.master_enabled {
        return Err(AuthRefusal::MasterDisabled);
    }
    if !policy.target_authorized {
        return Err(AuthRefusal::TargetUnauthorized);
    }
    Ok(())
}

/// 最小秘密注入器：只允许将**白名单**变量注入执行环境。
///
/// 任何未在白名单里的环境变量（哪怕父进程里有）一律不带进子环境，
/// 防止 AWS/GitHub/SSH 等无关凭据泄漏给 Agent。
pub fn minimal_env_injection(
    parent_env: &HashMap<String, String>,
    whitelist_keys: &[&str],
) -> HashMap<String, String> {
    let mut injected = HashMap::new();
    for &key in whitelist_keys {
        if let Some(val) = parent_env.get(key) {
            injected.insert(key.to_string(), val.clone());
        }
    }
    injected
}

/// 凭据槽位：支持显式撤销（撤销后读不到）。
#[derive(Debug, Clone, Default)]
pub struct SecretSlot {
    value: Option<String>,
}

impl SecretSlot {
    pub fn new(val: &str) -> Self {
        SecretSlot {
            value: Some(val.to_string()),
        }
    }

    /// 读取凭据。已撤销时返回 `None`。
    pub fn read(&self) -> Option<&str> {
        self.value.as_deref()
    }

    /// 撤销凭据：内存立刻清空。
    pub fn revoke(&mut self) {
        self.value = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dual_track_audit_separates_human_from_automated_and_cannot_be_spoofed() {
        let mut audit = DualTrackAudit::new();

        // 人类操作
        audit.record(
            Actor::Human { user_id: "alice".into() },
            "approve_task",
            "task-1",
            1_000,
        );

        // Agent 试图在 action 文本里写 "human_approved" 冒充人类
        audit.record(
            Actor::Agent { agent_id: "codex".into() },
            "human_approved_spoofed",
            "task-2",
            2_000,
        );

        // API Key 调用
        audit.record(
            Actor::ApiKey { key_id: "key-ops".into() },
            "trigger_check",
            "gw-1",
            3_000,
        );

        // 人类轨只有 alice 那一条——不可被文本冒充
        let human = audit.human_track();
        assert_eq!(human.len(), 1);
        assert_eq!(human[0].actor.identifier(), "alice");

        // 自动化轨有两条
        let automated = audit.automated_track();
        assert_eq!(automated.len(), 2);
        assert_eq!(automated[0].actor.role_str(), "agent");
        assert_eq!(automated[1].actor.role_str(), "api_key");
    }

    #[test]
    fn remote_shell_requires_both_master_and_target_switches() {
        // 双关 ⇒ 拒 MasterDisabled
        assert_eq!(
            authorize_remote_shell(RemoteShellPolicy {
                master_enabled: false,
                target_authorized: false,
            }),
            Err(AuthRefusal::MasterDisabled)
        );

        // 主开但目标未授权 ⇒ 拒 TargetUnauthorized
        assert_eq!(
            authorize_remote_shell(RemoteShellPolicy {
                master_enabled: true,
                target_authorized: false,
            }),
            Err(AuthRefusal::TargetUnauthorized)
        );

        // 主关但目标误授权 ⇒ 仍拒 MasterDisabled
        assert_eq!(
            authorize_remote_shell(RemoteShellPolicy {
                master_enabled: false,
                target_authorized: true,
            }),
            Err(AuthRefusal::MasterDisabled)
        );

        // 双开 ⇒ 放行
        assert_eq!(
            authorize_remote_shell(RemoteShellPolicy {
                master_enabled: true,
                target_authorized: true,
            }),
            Ok(())
        );
    }

    #[test]
    fn minimal_env_injection_only_passes_whitelisted_keys() {
        let mut parent = HashMap::new();
        parent.insert("OPENAI_API_KEY".into(), "sk-target-key".into());
        parent.insert("AWS_SECRET_ACCESS_KEY".into(), "unrelated-aws-secret".into());
        parent.insert("GITHUB_TOKEN".into(), "unrelated-gh-pat".into());

        // 白名单只给 OPENAI_API_KEY
        let injected = minimal_env_injection(&parent, &["OPENAI_API_KEY"]);
        assert_eq!(injected.len(), 1);
        assert!(injected.contains_key("OPENAI_API_KEY"));
        // 无关凭据绝不流进子环境
        assert!(!injected.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert!(!injected.contains_key("GITHUB_TOKEN"));
    }

    #[test]
    fn secret_slot_revocation_clears_the_value_immediately() {
        let mut slot = SecretSlot::new("sk-active-token");
        assert_eq!(slot.read(), Some("sk-active-token"));

        slot.revoke();
        assert_eq!(slot.read(), None, "撤销后必须立即返回 None");
    }
}
