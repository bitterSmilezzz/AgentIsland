//! Chief 四类资源（角色/运行时/模型/记忆）的**配置形态判据**（TASK-02 第二片）。
//!
//! 需求原文：目标拆解、分派、主题/watch/check-in、**角色/运行时/模型/记忆**，
//! 主控与工人分工。
//!
//! 背景：首片（v0.0.381）定了编排语义（拆解去重、分派去重、watch 幂等、
//! 跨主题记忆隔离），但**四类资源怎么配**一直没有口径——各调用点各自拼一套，
//! 于是「分派前才发现这个角色没有可用的模型」这类问题总在运行中才暴露。
//!
//! 判据（纯函数）：
//!
//! 1. **绑定完整性**：一个角色要能开工，四类资源**各自必须已绑定**或显式
//!    声明默认——缺任何一类就是「跑到一半发现没东西可用」。
//! 2. **能力相容**：角色声明的能力必须被运行时与模型支持；不相容的组合
//!    在**绑定时**拒绝，而不是运行时才发现。
//! 3. **记忆作用域相容**：角色的记忆作用域必须**不超过**它被授予的授权作用域
//!    （跨主题读记忆被拒——与首片的主题隔离同口径）。
//! 4. **运行时并发上限**：同一运行时的并发槽位有上限（与 TASK-09 的机器槽位
//!    同一口径），超限的分派在**分派时**被拒。

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// 角色。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleSpec {
    pub name: String,
    /// 角色要求的能力（运行时/模型必须支持）。
    pub required_capabilities: Vec<String>,
    /// 角色被授予的记忆作用域（主题列表）。
    pub granted_scopes: Vec<String>,
}

/// 运行时（执行环境）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeSpec {
    pub name: String,
    pub capabilities: Vec<String>,
    /// 并发槽位上限。
    pub max_concurrency: usize,
}

/// 模型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSpec {
    pub name: String,
    pub capabilities: Vec<String>,
}

/// 资源绑定（四类资源合在一起）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResourceBinding {
    pub role: Option<String>,
    pub runtime: Option<String>,
    pub model: Option<String>,
    /// 记忆作用域（绑定的记忆范围）。
    pub memory_scope: Option<String>,
}

/// 拒绝原因。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceRefusal {
    /// 缺少某一类资源绑定。
    MissingBinding,
    /// 运行时或模型不支持角色要求的能力。
    CapabilityIncompatible,
    /// 记忆作用域超出角色授权。
    ScopeEscalation,
    /// 运行时并发已满。
    RuntimeBusy,
}

impl ResourceRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingBinding => "missing_binding",
            Self::CapabilityIncompatible => "capability_incompatible",
            Self::ScopeEscalation => "scope_escalation",
            Self::RuntimeBusy => "runtime_busy",
        }
    }
}

/// 资源配置表。
pub struct ResourceRegistry {
    roles: HashMap<String, RoleSpec>,
    runtimes: HashMap<String, RuntimeSpec>,
    models: HashMap<String, ModelSpec>,
    /// 运行时 → 当前占用的槽位数。
    runtime_usage: HashMap<String, usize>,
}

impl ResourceRegistry {
    pub fn new() -> Self {
        ResourceRegistry {
            roles: HashMap::new(),
            runtimes: HashMap::new(),
            models: HashMap::new(),
            runtime_usage: HashMap::new(),
        }
    }

    pub fn add_role(&mut self, role: RoleSpec) {
        self.roles.insert(role.name.clone(), role);
    }

    pub fn add_runtime(&mut self, runtime: RuntimeSpec) {
        self.runtimes.insert(runtime.name.clone(), runtime);
    }

    pub fn add_model(&mut self, model: ModelSpec) {
        self.models.insert(model.name.clone(), model);
    }

    /// 校验一个绑定是否可以开工。
    ///
    /// `binding.memory_scope` 是这次要用的记忆作用域；它必须落在角色被授予的
    /// 作用域内（完全相等或为 granted 的子集）。
    pub fn check_binding(&self, binding: &ResourceBinding) -> Result<(), ResourceRefusal> {
        // ① 绑定完整性：四类资源都要有
        let (Some(role_name), Some(runtime_name), Some(model_name), Some(scope)) = (
            binding.role.as_ref(),
            binding.runtime.as_ref(),
            binding.model.as_ref(),
            binding.memory_scope.as_ref(),
        ) else {
            return Err(ResourceRefusal::MissingBinding);
        };
        let role = self
            .roles
            .get(role_name)
            .ok_or(ResourceRefusal::MissingBinding)?;
        let runtime = self
            .runtimes
            .get(runtime_name)
            .ok_or(ResourceRefusal::MissingBinding)?;
        let model = self
            .models
            .get(model_name)
            .ok_or(ResourceRefusal::MissingBinding)?;

        // ② 能力相容：角色要求的能力，运行时与模型都要支持
        for capability in &role.required_capabilities {
            if !runtime.capabilities.contains(capability) {
                return Err(ResourceRefusal::CapabilityIncompatible);
            }
            if !model.capabilities.contains(capability) {
                return Err(ResourceRefusal::CapabilityIncompatible);
            }
        }

        // ③ 记忆作用域不得超出角色授权
        if !role.granted_scopes.iter().any(|granted| granted == scope) {
            return Err(ResourceRefusal::ScopeEscalation);
        }
        Ok(())
    }

    /// 占用一个运行时槽位（分派时调用）。超限拒绝。
    pub fn acquire_runtime_slot(&mut self, runtime_name: &str) -> Result<(), ResourceRefusal> {
        let runtime = self
            .runtimes
            .get(runtime_name)
            .ok_or(ResourceRefusal::MissingBinding)?;
        let used = self.runtime_usage.get(runtime_name).copied().unwrap_or(0);
        if used >= runtime.max_concurrency {
            return Err(ResourceRefusal::RuntimeBusy);
        }
        self.runtime_usage.insert(runtime_name.to_string(), used + 1);
        Ok(())
    }

    /// 释放一个运行时槽位。
    pub fn release_runtime_slot(&mut self, runtime_name: &str) {
        if let Some(used) = self.runtime_usage.get_mut(runtime_name) {
            *used = used.saturating_sub(1);
        }
    }

    /// 当前某个运行时的占用槽位数。
    pub fn runtime_usage(&self, runtime_name: &str) -> usize {
        self.runtime_usage.get(runtime_name).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> ResourceRegistry {
        let mut reg = ResourceRegistry::new();
        reg.add_role(RoleSpec {
            name: "reviewer".into(),
            required_capabilities: vec!["read-repo".into()],
            granted_scopes: vec!["repo:web".into()],
        });
        reg.add_runtime(RuntimeSpec {
            name: "mac-mini".into(),
            capabilities: vec!["read-repo".into()],
            max_concurrency: 1,
        });
        reg.add_model(ModelSpec {
            name: "model-a".into(),
            capabilities: vec!["read-repo".into()],
        });
        reg
    }

    fn binding() -> ResourceBinding {
        ResourceBinding {
            role: Some("reviewer".into()),
            runtime: Some("mac-mini".into()),
            model: Some("model-a".into()),
            memory_scope: Some("repo:web".into()),
        }
    }

    #[test]
    fn a_complete_compatible_binding_passes() {
        let reg = registry();
        assert_eq!(reg.check_binding(&binding()), Ok(()));
    }

    #[test]
    fn a_missing_resource_binding_is_refused() {
        let reg = registry();
        for missing in ["role", "runtime", "model", "memory_scope"] {
            let mut b = binding();
            match missing {
                "role" => b.role = None,
                "runtime" => b.runtime = None,
                "model" => b.model = None,
                _ => b.memory_scope = None,
            }
            assert_eq!(
                reg.check_binding(&b),
                Err(ResourceRefusal::MissingBinding),
                "{missing} 缺失必须被拒"
            );
        }
    }

    #[test]
    fn an_incapable_runtime_or_model_is_refused_at_binding_time() {
        let mut reg = registry();
        // 换一个不支持 read-repo 的运行时
        reg.add_runtime(RuntimeSpec {
            name: "mac-mini".into(),
            capabilities: vec!["write-repo".into()],
            max_concurrency: 1,
        });
        assert_eq!(
            reg.check_binding(&binding()),
            Err(ResourceRefusal::CapabilityIncompatible),
            "能力不相容必须在绑定时拒绝，不是运行时才发现"
        );

        // 换一个不支持 read-repo 的模型
        let mut reg = registry();
        reg.add_model(ModelSpec {
            name: "model-a".into(),
            capabilities: vec!["write-repo".into()],
        });
        assert_eq!(
            reg.check_binding(&binding()),
            Err(ResourceRefusal::CapabilityIncompatible)
        );
    }

    #[test]
    fn a_memory_scope_beyond_the_granted_one_is_refused() {
        let reg = registry();
        let mut b = binding();
        b.memory_scope = Some("repo:other".into());
        assert_eq!(
            reg.check_binding(&b),
            Err(ResourceRefusal::ScopeEscalation),
            "记忆作用域不得超出角色授权"
        );
    }

    #[test]
    fn runtime_concurrency_is_capped() {
        let mut reg = registry();
        assert_eq!(reg.acquire_runtime_slot("mac-mini"), Ok(()));
        assert_eq!(reg.runtime_usage("mac-mini"), 1);
        // 槽位已满 ⇒ 拒绝
        assert_eq!(
            reg.acquire_runtime_slot("mac-mini"),
            Err(ResourceRefusal::RuntimeBusy)
        );
        // 释放后可再取
        reg.release_runtime_slot("mac-mini");
        assert_eq!(reg.acquire_runtime_slot("mac-mini"), Ok(()));
    }
}
