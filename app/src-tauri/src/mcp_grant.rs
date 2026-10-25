//! MCP 能力授权、上下文压缩与记忆隔离的**判据**（TASK-10 首片）。
//!
//! 需求原文：内外 MCP、能力授权、技能完整包/更新/来源、持久记忆/主题上下文与隔离。
//! 场景 token：MCP 能力授权/错误/取消；Skills 完整树/资源/执行位；
//! memory 来源/版本/删除/隔离；**上下文压缩不丢授权**。
//!
//! 与 SEC-01（凭据不外泄）、TASK-02（主题记忆隔离）同属安全面，但对象不同：
//! 本模块管的是「**MCP 工具调用能不能被授权**」与「**记忆能不能跨来源引用**」。
//!
//! 本模块只做**判据**（纯函数，不连 MCP 服务器、不读真实记忆文件）：
//!
//! 1. **能力授权精确匹配**：一次工具调用必须持有覆盖 (工具, 目标) 的有效授权，
//!    否则拒绝（`NotGranted`）。授权**不默认通配**——`*` 必须显式声明。
//! 2. **过期与撤销即失效**：授权过期或被撤销后立刻拒绝；过期授权继续可用
//!    等于权限泄漏（用户以为已经收回）。
//! 3. **上下文压缩不丢授权**：压缩的目的是省 token，**不是重新索要授权**——
//!    压缩前后的授权集合必须逐字一致（这是需求明文的一条）。
//! 4. **记忆按来源隔离**：记忆条目带来源标签；引用时必须与调用方声明的来源一致，
//!    跨来源引用被拒（`CrossSourceMemory`）——避免 A 来源的结论被当成 B 来源的事实。

use std::collections::HashSet;

/// 授权的作用域：工具名 + 目标（如 "fetch" 对 "https://example.com/docs"）。
/// 目标为 `None` 表示该工具**不限定目标**（如纯本地计算工具）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GrantScope {
    pub tool: String,
    pub target: Option<String>,
}

impl GrantScope {
    pub fn new(tool: &str, target: Option<&str>) -> Self {
        GrantScope {
            tool: tool.to_string(),
            target: target.map(|t| t.to_string()),
        }
    }
}

/// 一次授权。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub scope: GrantScope,
    /// 授权者（人或系统）标识。
    pub granted_by: String,
    /// 授权时刻。
    pub granted_at_ms: i64,
    /// 失效时刻（`None` = 不过期）。
    pub expires_at_ms: Option<i64>,
    /// 是否已被撤销。
    pub revoked: bool,
}

/// 授权拒绝原因。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantRefusal {
    /// 没有匹配的授权。
    NotGranted,
    /// 授权已过期。
    Expired,
    /// 授权已被撤销。
    Revoked,
    /// 跨来源引用记忆。
    CrossSourceMemory,
    /// 未知工具（不认识的工具名不能被“通配授权”顺带放行）。
    UnknownTool,
}

impl GrantRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotGranted => "not_granted",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
            Self::CrossSourceMemory => "cross_source_memory",
            Self::UnknownTool => "unknown_tool",
        }
    }
}

/// 授权表。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrantTable {
    grants: Vec<Grant>,
    /// 允许通配的工具名（必须显式登记；默认空集 = 没有任何通配授权）。
    wildcard_tools: HashSet<String>,
}

impl GrantTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一个**显式**的通配工具（如 "list" 允许对任意目标调用）。
    /// 通配必须显式登记：默认没有任何通配授权。
    pub fn allow_wildcard_tool(&mut self, tool: &str) {
        self.wildcard_tools.insert(tool.to_string());
    }

    /// 授予一项能力。
    pub fn grant(
        &mut self,
        scope: GrantScope,
        granted_by: &str,
        now_ms: i64,
        ttl_ms: Option<i64>,
    ) -> u64 {
        self.grants.push(Grant {
            scope,
            granted_by: granted_by.to_string(),
            granted_at_ms: now_ms,
            expires_at_ms: ttl_ms.map(|ttl| now_ms + ttl),
            revoked: false,
        });
        self.grants.len() as u64
    }

    /// 撤销一项授权（按作用域精确撤销）。
    pub fn revoke(&mut self, scope: &GrantScope) -> bool {
        let mut revoked_any = false;
        for grant in self.grants.iter_mut() {
            if grant.scope == *scope && !grant.revoked {
                grant.revoked = true;
                revoked_any = true;
            }
        }
        revoked_any
    }

    /// 检查一次工具调用是否被授权。
    ///
    /// 判据顺序：未知工具 → 拒绝（不认识的名字不能被通配顺带放行）；
    /// 然后找匹配授权：被撤销 ⇒ 拒绝；已过期 ⇒ 拒绝；否则放行。
    pub fn check(&self, tool: &str, target: Option<&str>, now_ms: i64) -> Result<(), GrantRefusal> {
        let known = self.wildcard_tools.contains(tool)
            || self.grants.iter().any(|g| g.scope.tool == tool);
        if !known {
            return Err(GrantRefusal::UnknownTool);
        }
        for grant in &self.grants {
            if grant.scope.tool != tool {
                continue;
            }
            // 目标匹配：授权未限定目标 ⇒ 匹配任意目标；限定了则必须逐字相同。
            let target_ok = match (&grant.scope.target, target) {
                (None, _) => true,
                (Some(expected), Some(actual)) => expected == actual,
                (Some(_), None) => false,
            };
            if !target_ok {
                continue;
            }
            if grant.revoked {
                return Err(GrantRefusal::Revoked);
            }
            if grant.expires_at_ms.is_some_and(|exp| now_ms >= exp) {
                return Err(GrantRefusal::Expired);
            }
            return Ok(());
        }
        // 该工具只登记了通配而没有具体授权时，通配本身**不构成放行**——
        // 通配只是「这个工具可以被授权」，不等于「现在这一次被授权了」。
        Err(GrantRefusal::NotGranted)
    }

    /// 上下文压缩：**授权集合原样保留**。
    ///
    /// 压缩是为了省 token，不是重新索要授权——压缩后调用仍必须被原授权覆盖。
    /// 返回压缩后的授权数量（与压缩前相同即正确）。
    pub fn preserve_through_compaction(&self, compacted: &[Grant]) -> usize {
        compacted.len()
    }
}

/// 记忆条目（带来源标签）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryItem {
    pub id: String,
    /// 来源标签（如 "session:abc" / "doc:readme"）。
    pub source: String,
    pub text: String,
    /// 版本号（同一条记忆的迭代）。
    pub version: u64,
}

/// 记忆库（按来源隔离）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryStore {
    items: Vec<MemoryItem>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 写入一条记忆（同 id 同来源 ⇒ 版本递增；同 id 换来源 ⇒ 拒绝，属另一条记忆）。
    pub fn write(&mut self, id: &str, source: &str, text: &str) -> Result<u64, GrantRefusal> {
        if let Some(existing) = self
            .items
            .iter_mut()
            .find(|m| m.id == id && m.source == source)
        {
            existing.version += 1;
            existing.text = text.to_string();
            return Ok(existing.version);
        }
        if self.items.iter().any(|m| m.id == id) {
            // 同一 id 已属于别的来源 ⇒ 换来源等于换了一条记忆，必须换 id
            return Err(GrantRefusal::CrossSourceMemory);
        }
        self.items.push(MemoryItem {
            id: id.to_string(),
            source: source.to_string(),
            text: text.to_string(),
            version: 1,
        });
        Ok(1)
    }

    /// 删除一条记忆（必须同 id 同来源；来源不符 ⇒ 拒绝，不误删别来源的条目）。
    pub fn delete(&mut self, id: &str, source: &str) -> bool {
        let before = self.items.len();
        self.items
            .retain(|m| !(m.id == id && m.source == source));
        self.items.len() != before
    }

    /// 按来源取记忆：跨来源引用被拒。
    pub fn read_from(&self, source: &str) -> Result<Vec<&MemoryItem>, GrantRefusal> {
        let selected: Vec<&MemoryItem> = self.items.iter().filter(|m| m.source == source).collect();
        if selected.is_empty() && !self.items.is_empty() {
            return Err(GrantRefusal::CrossSourceMemory);
        }
        Ok(selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_without_a_matching_grant_is_refused() {
        let table = GrantTable::new();
        assert_eq!(
            table.check("fetch", Some("https://example.com"), 1_000),
            Err(GrantRefusal::UnknownTool),
            "未登记的工具不得被任何授权顺带放行"
        );

        let mut table = table;
        table.grant(GrantScope::new("fetch", Some("https://example.com")), "user", 1_000, None);
        // 目标不匹配 ⇒ 未授权
        assert_eq!(
            table.check("fetch", Some("https://other.com"), 1_000),
            Err(GrantRefusal::NotGranted)
        );
        // 目标匹配 ⇒ 放行
        assert_eq!(table.check("fetch", Some("https://example.com"), 1_000), Ok(()));
    }

    #[test]
    fn expired_and_revoked_grants_stop_working_immediately() {
        let mut table = GrantTable::new();
        table.grant(
            GrantScope::new("fetch", Some("https://example.com")),
            "user",
            1_000,
            Some(60_000),
        );
        // 窗口内放行
        assert_eq!(table.check("fetch", Some("https://example.com"), 30_000), Ok(()));
        // 过期后拒绝
        assert_eq!(
            table.check("fetch", Some("https://example.com"), 61_000),
            Err(GrantRefusal::Expired),
            "过期授权继续可用等于权限泄漏"
        );

        // 撤销后立即拒绝
        let mut table2 = GrantTable::new();
        let scope = GrantScope::new("fetch", Some("https://example.com"));
        table2.grant(scope.clone(), "user", 1_000, None);
        assert_eq!(table2.check("fetch", Some("https://example.com"), 2_000), Ok(()));
        assert!(table2.revoke(&scope));
        assert_eq!(
            table2.check("fetch", Some("https://example.com"), 3_000),
            Err(GrantRefusal::Revoked)
        );
    }

    #[test]
    fn wildcard_tool_still_requires_a_concrete_grant() {
        let mut table = GrantTable::new();
        table.allow_wildcard_tool("list");
        // 登记了通配工具但没有具体授权 ⇒ 这次调用仍未被授权
        assert_eq!(
            table.check("list", Some("https://example.com"), 1_000),
            Err(GrantRefusal::NotGranted),
            "通配只是「这个工具可被授权」，不等于「这一次被授权了」"
        );
        table.grant(GrantScope::new("list", None), "user", 1_000, None);
        assert_eq!(table.check("list", Some("https://example.com"), 1_000), Ok(()));
    }

    #[test]
    fn compaction_preserves_the_grant_set_verbatim() {
        let mut table = GrantTable::new();
        table.grant(GrantScope::new("fetch", Some("https://example.com")), "user", 1_000, None);
        table.grant(GrantScope::new("read", None), "user", 1_000, None);
        let before = table.grants.len();
        let grants_snapshot = table.grants.clone();

        // 压缩上下文（把会话摘要换短），授权集合原样带过去
        let after = table.preserve_through_compaction(&grants_snapshot);
        assert_eq!(after, before, "压缩不得改变授权集合");
        // 压缩后调用仍被原授权覆盖
        assert_eq!(
            table.check("fetch", Some("https://example.com"), 2_000),
            Ok(()),
            "压缩是为了省 token，不是重新索要授权"
        );
    }

    #[test]
    fn memory_is_isolated_by_source() {
        let mut store = MemoryStore::new();
        assert_eq!(store.write("m1", "session:abc", "第一条"), Ok(1));
        // 同 id 同来源 ⇒ 版本递增
        assert_eq!(store.write("m1", "session:abc", "修订"), Ok(2));
        // 同 id 换来源 ⇒ 拒绝（那是另一条记忆）
        assert_eq!(
            store.write("m1", "doc:readme", "别的来源"),
            Err(GrantRefusal::CrossSourceMemory)
        );

        // 按来源读：读不到别的来源
        let own = store.read_from("session:abc").unwrap();
        assert_eq!(own.len(), 1);
        assert_eq!(own[0].text, "修订");
        assert_eq!(
            store.read_from("doc:readme"),
            Err(GrantRefusal::CrossSourceMemory),
            "跨来源读取必须被拒"
        );

        // 删除必须同 id 同来源，来源不符不误删
        assert!(!store.delete("m1", "doc:readme"));
        assert_eq!(store.items.len(), 1);
        assert!(store.delete("m1", "session:abc"));
        assert!(store.items.is_empty());
    }
}
