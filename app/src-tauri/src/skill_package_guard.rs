//! 技能包的**完整性与复制安全判据**（TASK-10 第二片）。
//!
//! 需求原文：技能完整包/更新/来源；场景 token：**Skills 完整树/资源/执行位**。
//!
//! `skill_files.rs` 已经把技能包表示为「有界的、描述符相对的树」并记录了每个
//! 节点的执行位；本模块补上**跨设备复制的语义**——技能包从一个机器复制到
//! 另一个机器时，哪些属性必须保持、哪些必须被重新审视。
//!
//! 判据（纯函数，不碰文件系统）：
//!
//! 1. **完整树判据**：技能包必须含 `SKILL.md`（入口正文）；缺它就不是一个
//!    技能包，而是散落的资源目录。
//! 2. **执行位必须保持**：复制到另一台机器时，脚本的**可执行位不能丢**——
//!    丢了脚本就跑不起来（用户看到「技能装了但说不会用」）。
//! 3. **执行位不得凭空获得**：正文与资源文件**不得带执行位**——
//!    一个 `SKILL.md` 带 x 位意味着它可能被当成脚本执行；这是资源型攻击面。
//! 4. **来源可核对**：每个节点记录来源（`Origin`），复制后来源信息不得
//!    被改写成别的来源（否则「这个技能从哪来」无法追溯）。
//! 5. **越界路径拒绝**：节点相对路径不得逃出技能根（`..`、绝对路径、
//!    符号链接式的 `a/../../b` 一律拒绝）。

use std::collections::BTreeMap;

/// 技能包内的相对路径（统一用 `/` 分隔）。
pub type RelPath = String;

/// 节点来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// 随包自带的资源（脚本、模板、参考文档）。
    Bundled,
    /// 用户后来添加的（不属于原始包，但保留在树里）。
    UserAdded,
}

/// 技能包中的一个节点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillNode {
    pub path: RelPath,
    /// 是否可执行（脚本）。
    pub executable: bool,
    pub origin: Origin,
    /// 字节数。
    pub bytes: usize,
}

/// 技能包完整性拒绝原因。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageRefusal {
    /// 缺少 SKILL.md 入口。
    MissingEntry,
    /// 路径越界。
    PathEscapes,
    /// 正文/资源带执行位。
    UnexpectedExecutable,
    /// 复制后执行位丢失。
    ExecutableLost,
    /// 来源信息与原包不一致。
    OriginMismatch,
    /// 空包。
    EmptyPackage,
}

impl PackageRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingEntry => "missing_entry",
            Self::PathEscapes => "path_escapes",
            Self::UnexpectedExecutable => "unexpected_executable",
            Self::ExecutableLost => "executable_lost",
            Self::OriginMismatch => "origin_mismatch",
            Self::EmptyPackage => "empty_package",
        }
    }
}

/// 技能包节点集合。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillPackage {
    pub nodes: BTreeMap<RelPath, SkillNode>,
}

/// 一份「源包」与一份「复制到目标机的包」的对照。
pub struct PackageCopy<'a> {
    pub source: &'a SkillPackage,
    pub target: &'a SkillPackage,
}

/// 校验一个技能包的**基本形态**：非空、含 SKILL.md、路径不越界、正文与资源不带执行位。
pub fn validate_package(package: &SkillPackage) -> Result<(), PackageRefusal> {
    if package.nodes.is_empty() {
        return Err(PackageRefusal::EmptyPackage);
    }
    if !package.nodes.contains_key("SKILL.md") {
        return Err(PackageRefusal::MissingEntry);
    }
    for node in package.nodes.values() {
        if escapes_root(&node.path) {
            return Err(PackageRefusal::PathEscapes);
        }
        // 脚本才该有执行位；正文与普通资源带 x 位是攻击面
        if node.executable && is_entry_or_resource(&node.path) {
            return Err(PackageRefusal::UnexpectedExecutable);
        }
    }
    Ok(())
}

/// 校验「复制到目标机」的结果：脚本的执行位必须保持、来源信息必须一致。
///
/// 目标机重新落盘时可能丢掉执行位（打包/传输/解压都常见）——那会让
/// 「技能装了但脚本跑不起来」变成一句查不到原因的话。
pub fn validate_copy(copy: PackageCopy<'_>) -> Result<(), PackageRefusal> {
    for (path, source_node) in &copy.source.nodes {
        let Some(target_node) = copy.target.nodes.get(path) else {
            continue; // 目标机缺节点：由复制流程自身负责，不在此判据范围
        };
        if source_node.executable && !target_node.executable {
            return Err(PackageRefusal::ExecutableLost);
        }
        if source_node.origin != target_node.origin {
            return Err(PackageRefusal::OriginMismatch);
        }
    }
    Ok(())
}

/// 路径是否逃出技能根。
pub fn escapes_root(path: &str) -> bool {
    if path.is_empty() {
        return true;
    }
    // 绝对路径
    if path.starts_with('/') || path.contains(':') {
        return true;
    }
    // 逐段检查：任何 `..` 段都算越界（`a/../b` 归一后仍在根内，但**归一规则
    // 跨实现不一致**——按描述符相对路径处理时一律拒绝更安全）
    path.split('/').any(|segment| segment == "..")
}

/// 入口正文或普通资源（不可执行的那类）。
fn is_entry_or_resource(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".md") || lower.ends_with(".txt") || lower.ends_with(".json") || lower.ends_with(".yaml")
        || lower.ends_with(".yml")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(path: &str, executable: bool, origin: Origin) -> SkillNode {
        SkillNode {
            path: path.into(),
            executable,
            origin,
            bytes: 128,
        }
    }

    fn package_with(nodes: Vec<SkillNode>) -> SkillPackage {
        let mut map = BTreeMap::new();
        for n in nodes {
            map.insert(n.path.clone(), n);
        }
        SkillPackage { nodes: map }
    }

    #[test]
    fn a_complete_package_needs_its_entry_document() {
        let good = package_with(vec![
            node("SKILL.md", false, Origin::Bundled),
            node("scripts/run.sh", true, Origin::Bundled),
        ]);
        assert_eq!(validate_package(&good), Ok(()), "完整包应通过");

        // 缺 SKILL.md ⇒ 只是散落的资源目录
        let no_entry = package_with(vec![node("scripts/run.sh", true, Origin::Bundled)]);
        assert_eq!(validate_package(&no_entry), Err(PackageRefusal::MissingEntry));

        // 空包
        assert_eq!(
            validate_package(&package_with(vec![])),
            Err(PackageRefusal::EmptyPackage)
        );
    }

    #[test]
    fn paths_that_escape_the_root_are_refused() {
        for path in ["../outside.md", "a/../../b.md", "/etc/passwd", ""] {
            assert!(escapes_root(path), "{path:?} 必须判为越界");
        }
        for path in ["SKILL.md", "scripts/run.sh", "docs/guide.md"] {
            assert!(!escapes_root(path), "{path:?} 不该判为越界");
        }
        let escaping = package_with(vec![
            node("SKILL.md", false, Origin::Bundled),
            node("../evil.sh", true, Origin::Bundled),
        ]);
        assert_eq!(
            validate_package(&escaping),
            Err(PackageRefusal::PathEscapes)
        );
    }

    #[test]
    fn documents_and_resources_must_not_carry_the_executable_bit() {
        let bad = package_with(vec![
            node("SKILL.md", false, Origin::Bundled),
            node("scripts/run.sh", true, Origin::Bundled),
            node("docs/guide.md", true, Origin::UserAdded), // 正文带 x 位 = 攻击面
        ]);
        assert_eq!(
            validate_package(&bad),
            Err(PackageRefusal::UnexpectedExecutable)
        );
    }

    #[test]
    fn copying_to_another_machine_must_keep_the_executable_bit_and_origin() {
        let source = package_with(vec![
            node("SKILL.md", false, Origin::Bundled),
            node("scripts/run.sh", true, Origin::Bundled),
        ]);
        // 目标机正确复制：执行位与来源都保持
        let good_target = package_with(vec![
            node("SKILL.md", false, Origin::Bundled),
            node("scripts/run.sh", true, Origin::Bundled),
        ]);
        assert_eq!(
            validate_copy(PackageCopy {
                source: &source,
                target: &good_target
            }),
            Ok(())
        );

        // 目标机丢了执行位 ⇒ 拒绝（技能装了但脚本跑不起来）
        let lost_bit = package_with(vec![
            node("SKILL.md", false, Origin::Bundled),
            node("scripts/run.sh", false, Origin::Bundled),
        ]);
        assert_eq!(
            validate_copy(PackageCopy {
                source: &source,
                target: &lost_bit
            }),
            Err(PackageRefusal::ExecutableLost)
        );

        // 来源被改写 ⇒ 拒绝（「这个技能从哪来」必须可追溯）
        let rewritten_origin = package_with(vec![
            node("SKILL.md", false, Origin::Bundled),
            node("scripts/run.sh", true, Origin::UserAdded),
        ]);
        assert_eq!(
            validate_copy(PackageCopy {
                source: &source,
                target: &rewritten_origin
            }),
            Err(PackageRefusal::OriginMismatch)
        );
    }
}
