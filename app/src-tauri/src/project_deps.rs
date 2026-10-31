//! 项目依赖关系的**判据**（TASK-01 进展：项目依赖/循环）。
//!
//! 需求原文：CRUD、归属、筛选、拖拽、归档、**依赖**、跨项目搜索与状态版本冲突。
//! 场景 token：项目依赖/循环。
//!
//! `tasks::Project` 将依赖 id 持久化在 `tasks.v1.json`；本模块只提供纯函数判据，
//! 由 `tasks::Store` 在读写时校验完整依赖图：
//!
//! 1. **依赖存在性**：依赖的项目必须存在（指向不存在项目的依赖是配置错）。
//! 2. **循环拒绝**：A 依赖 B、B 依赖 A（或更长的环）必须**拒绝**——
//!    带环的依赖图会让「先做谁」永远算不出来。
//! 3. **深度上限**：依赖链超过上限即拒绝（避免过深的依赖链让计划无法展开）。
//! 4. **删除守卫**：被其他项目依赖的项目**不许删**（要么先解除依赖，
//!    要么连带删除）——删掉被依赖方会让留下的依赖指向空气。
//! 5. **自依赖拒绝**：项目不能依赖自己。

/// 依赖图：项目 id → 它依赖的项目 id 列表。
pub type DepGraph = Vec<(String, Vec<String>)>;

/// 依赖判据的拒绝原因。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepRefusal {
    /// 依赖的项目不存在。
    UnknownDependency,
    /// 项目依赖列表中出现重复条目。
    DuplicateDependency,
    /// 图中出现重复项目。
    DuplicateProject,
    /// 形成循环依赖。
    Cycle,
    /// 依赖链超过深度上限。
    DepthExceeded,
    /// 项目依赖自己。
    SelfDependency,
    /// 项目仍被其他项目依赖，不许删除。
    StillDependedOn,
}

impl DepRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownDependency => "unknown_dependency",
            Self::DuplicateDependency => "duplicate_dependency",
            Self::DuplicateProject => "duplicate_project",
            Self::Cycle => "cycle",
            Self::DepthExceeded => "depth_exceeded",
            Self::SelfDependency => "self_dependency",
            Self::StillDependedOn => "still_depended_on",
        }
    }
}

/// 依赖链深度上限（默认 8，与路由组深度同量级）。
pub const MAX_DEPENDENCY_DEPTH: usize = 8;

/// 取某项目的依赖列表。
fn deps_of<'a>(graph: &'a DepGraph, project: &str) -> &'a [String] {
    graph
        .iter()
        .find(|(id, _)| id == project)
        .map(|(_, deps)| deps.as_slice())
        .unwrap_or(&[])
}

/// 新增一条依赖 `from → to` 前的校验。
///
/// 判据顺序：自依赖 → 目标存在 → 环 → 深度。
pub fn check_dependency(
    graph: &DepGraph,
    from: &str,
    to: &str,
    max_depth: usize,
) -> Result<(), DepRefusal> {
    if from == to {
        return Err(DepRefusal::SelfDependency);
    }
    if !exists(graph, to) {
        return Err(DepRefusal::UnknownDependency);
    }
    // 环检测：从 `to` 出发能否走回 `from`。
    if reaches(graph, to, from, &mut Vec::new()) {
        return Err(DepRefusal::Cycle);
    }
    // 深度：加上这条边之后，从 from 出发的最长链。
    let existing_child_depth = deps_of(graph, from)
        .iter()
        .map(|child| dependency_depth(graph, child, &mut Vec::new()))
        .max()
        .unwrap_or(0);
    let child_depth = existing_child_depth.max(dependency_depth(graph, to, &mut Vec::new()));
    if child_depth.saturating_add(1) > max_depth {
        return Err(DepRefusal::DepthExceeded);
    }
    Ok(())
}

/// 校验完整依赖图：引用存在、无重复、无环，且所有根的最长链不超过上限。
pub fn validate_graph(graph: &DepGraph) -> Result<(), DepRefusal> {
    validate_graph_at_depth(graph, MAX_DEPENDENCY_DEPTH)
}

fn validate_graph_at_depth(graph: &DepGraph, max_depth: usize) -> Result<(), DepRefusal> {
    use std::collections::{HashMap, HashSet, VecDeque};

    let mut indices = HashMap::with_capacity(graph.len());
    for (index, (project, _)) in graph.iter().enumerate() {
        if indices.insert(project.as_str(), index).is_some() {
            return Err(DepRefusal::DuplicateProject);
        }
    }

    let mut dependents = vec![Vec::new(); graph.len()];
    let mut remaining_dependencies = vec![0usize; graph.len()];
    for (project_index, (_, dependencies)) in graph.iter().enumerate() {
        let mut seen = HashSet::with_capacity(dependencies.len());
        for dependency in dependencies {
            if !seen.insert(dependency.as_str()) {
                return Err(DepRefusal::DuplicateDependency);
            }
            let Some(&dependency_index) = indices.get(dependency.as_str()) else {
                return Err(DepRefusal::UnknownDependency);
            };
            if dependency_index == project_index {
                return Err(DepRefusal::SelfDependency);
            }
            dependents[dependency_index].push(project_index);
            remaining_dependencies[project_index] += 1;
        }
    }

    // Process dependency leaves first. This detects cycles and computes each
    // project's longest dependency chain without recursive traversal or
    // repeatedly scanning the full project list.
    let mut ready = VecDeque::new();
    let mut depth = vec![1usize; graph.len()];
    for (index, count) in remaining_dependencies.iter().enumerate() {
        if *count == 0 {
            ready.push_back(index);
        }
    }
    let mut processed = 0usize;
    while let Some(dependency_index) = ready.pop_front() {
        processed += 1;
        for &project_index in &dependents[dependency_index] {
            depth[project_index] =
                depth[project_index].max(depth[dependency_index].saturating_add(1));
            remaining_dependencies[project_index] -= 1;
            if remaining_dependencies[project_index] == 0 {
                ready.push_back(project_index);
            }
        }
    }
    if processed != graph.len() {
        return Err(DepRefusal::Cycle);
    }
    if depth.iter().any(|project_depth| *project_depth > max_depth) {
        return Err(DepRefusal::DepthExceeded);
    }
    Ok(())
}

/// 项目是否存在于图中。
fn exists(graph: &DepGraph, project: &str) -> bool {
    graph.iter().any(|(id, _)| id == project)
}

/// 从 `start` 出发能否到达 `target`（用于增边判据的环检测）。
fn reaches(graph: &DepGraph, start: &str, target: &str, path: &mut Vec<String>) -> bool {
    if path.iter().any(|node| node == start) {
        // 回到已访问的节点 ⇒ 图里本来就有环
        return true;
    }
    path.push(start.to_string());
    for next in deps_of(graph, start) {
        if next == target || reaches(graph, next, target, path) {
            return true;
        }
    }
    path.pop();
    false
}

/// 从 `project` 出发的最长依赖链长度（自身计 1）。
fn dependency_depth(graph: &DepGraph, project: &str, path: &mut Vec<String>) -> usize {
    if path.iter().any(|node| node == project) {
        return path.len(); // 已成环：按当前深度封顶，不无限递归
    }
    path.push(project.to_string());
    let mut longest_child = 0;
    for next in deps_of(graph, project) {
        longest_child = longest_child.max(dependency_depth(graph, next, path));
    }
    path.pop();
    longest_child + 1
}

/// 删除项目的守卫：仍被别的项目依赖时拒绝。
pub fn check_removable(graph: &DepGraph, project: &str) -> Result<(), DepRefusal> {
    let depended_on_by: Vec<&str> = graph
        .iter()
        .filter(|(id, deps)| id != project && deps.iter().any(|dep| dep == project))
        .map(|(id, _)| id.as_str())
        .collect();
    if depended_on_by.is_empty() {
        Ok(())
    } else {
        Err(DepRefusal::StillDependedOn)
    }
}

/// 依赖闭包：某项目的全部传递依赖（按声明顺序，去重）。
pub fn transitive_dependencies(graph: &DepGraph, project: &str) -> Vec<String> {
    let mut collected = Vec::new();
    let mut path = Vec::new();
    collect(graph, project, &mut collected, &mut path);
    collected
}

fn collect(graph: &DepGraph, project: &str, collected: &mut Vec<String>, path: &mut Vec<String>) {
    if path.iter().any(|node| node == project) {
        return; // 已成环：停止展开
    }
    path.push(project.to_string());
    for next in deps_of(graph, project) {
        if !collected.iter().any(|node| node == next) {
            collected.push(next.clone());
            collect(graph, next, collected, path);
        }
    }
    path.pop();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> DepGraph {
        vec![
            ("web".into(), vec!["api".into()]),
            ("api".into(), vec!["db".into()]),
            ("db".into(), vec![]),
            ("docs".into(), vec!["web".into()]),
        ]
    }

    #[test]
    fn valid_dependencies_are_accepted() {
        // docs → web 已存在；给 db 加一条指向 db 的**兄弟**项目 docs 的依赖会成环
        // （web → api → db，而 docs → web），所以那一条必须被拒（见环用例）。
        // 这里验证真正合法的一条：给 db 加一条指向「不与 db 有传递关系」的依赖。
        let mut g = graph();
        g.push(("standalone".into(), vec![]));
        assert_eq!(
            check_dependency(&g, "db", "standalone", MAX_DEPENDENCY_DEPTH),
            Ok(())
        );
        // 依赖一个尚不存在但将要创建的项目 ⇒ 拒绝（不能指向空气）
        assert_eq!(
            check_dependency(&g, "db", "以后才建的", MAX_DEPENDENCY_DEPTH),
            Err(DepRefusal::UnknownDependency)
        );
    }

    #[test]
    fn self_dependency_and_unknown_dependency_are_refused() {
        assert_eq!(
            check_dependency(&graph(), "web", "web", MAX_DEPENDENCY_DEPTH),
            Err(DepRefusal::SelfDependency)
        );
        assert_eq!(
            check_dependency(&graph(), "web", "不存在的项目", MAX_DEPENDENCY_DEPTH),
            Err(DepRefusal::UnknownDependency)
        );
    }

    #[test]
    fn direct_and_indirect_cycles_are_refused() {
        // 直接环：web → api，而 api 已依赖 db… 构造 web → web 的间接环
        let mut g = graph();
        // 让 api 依赖 web，形成 api → web → api
        g[1].1 = vec!["web".into()];
        assert_eq!(
            check_dependency(&g, "web", "api", MAX_DEPENDENCY_DEPTH),
            Err(DepRefusal::Cycle),
            "web → api 会与既有的 api → web 成环"
        );

        // 自成环的图也必须被识别，而不是无限递归
        let self_cycle = vec![("a".into(), vec!["a".into()])];
        assert_eq!(
            check_dependency(&self_cycle, "b", "a", MAX_DEPENDENCY_DEPTH),
            Err(DepRefusal::Cycle)
        );
    }

    #[test]
    fn dependency_chains_deeper_than_the_limit_are_refused() {
        // 造一条 10 层的链
        let mut g: DepGraph = Vec::new();
        for i in 0..10 {
            g.push((
                format!("p{i}"),
                if i == 9 {
                    vec![]
                } else {
                    vec![format!("p{}", i + 1)]
                },
            ));
        }
        assert_eq!(
            check_dependency(&g, "p0", "p9", MAX_DEPENDENCY_DEPTH),
            Err(DepRefusal::DepthExceeded),
            "过深的依赖链会让计划无法展开，必须在配置期拒绝"
        );
        // 浅链照常通过
        assert_eq!(
            check_dependency(&g, "p8", "p9", MAX_DEPENDENCY_DEPTH),
            Ok(())
        );
    }

    #[test]
    fn adding_a_shallow_project_to_a_deep_chain_respects_the_new_depth() {
        let mut graph: DepGraph = Vec::new();
        graph.push(("root".into(), vec![]));
        for i in 1..=8 {
            graph.push((
                format!("p{i}"),
                if i == 8 {
                    vec![]
                } else {
                    vec![format!("p{}", i + 1)]
                },
            ));
        }
        assert_eq!(
            check_dependency(&graph, "root", "p1", MAX_DEPENDENCY_DEPTH),
            Err(DepRefusal::DepthExceeded),
            "新边必须把目标已有链长计入深度"
        );

        let shallow: DepGraph = (0..8)
            .map(|i| {
                (
                    format!("p{i}"),
                    if i == 7 {
                        vec![]
                    } else {
                        vec![format!("p{}", i + 1)]
                    },
                )
            })
            .collect();
        assert_eq!(validate_graph(&shallow), Ok(()), "恰好 8 层应被接受");
    }

    #[test]
    fn complete_graph_validation_catches_late_upstream_depth_and_duplicates() {
        let too_deep: DepGraph = (0..9)
            .map(|i| {
                (
                    format!("p{i}"),
                    if i == 8 {
                        vec![]
                    } else {
                        vec![format!("p{}", i + 1)]
                    },
                )
            })
            .collect();
        assert_eq!(validate_graph(&too_deep), Err(DepRefusal::DepthExceeded));

        let duplicate = vec![
            ("a".into(), vec!["b".into(), "b".into()]),
            ("b".into(), vec![]),
        ];
        assert_eq!(
            validate_graph(&duplicate),
            Err(DepRefusal::DuplicateDependency)
        );
    }

    #[test]
    fn a_still_depended_on_project_cannot_be_removed() {
        // api 被 web 依赖 ⇒ 不许删
        assert_eq!(
            check_removable(&graph(), "api"),
            Err(DepRefusal::StillDependedOn)
        );
        // db 被 api 依赖 ⇒ 同样不许删（依赖指向空气）
        assert_eq!(
            check_removable(&graph(), "db"),
            Err(DepRefusal::StillDependedOn)
        );
        // 没人依赖它 ⇒ 可删
        let mut g = graph();
        g[1].1 = vec![]; // api 不再依赖 db
        assert_eq!(check_removable(&g, "db"), Ok(()));
    }

    #[test]
    fn transitive_dependencies_are_collected_in_declaration_order_without_duplicates() {
        // docs → web → api → db
        assert_eq!(
            transitive_dependencies(&graph(), "docs"),
            vec!["web".to_string(), "api".to_string(), "db".to_string()]
        );
        // 菱形依赖：a → b, a → c, b → d, c → d ⇒ d 只出现一次
        let diamond = vec![
            ("a".into(), vec!["b".into(), "c".into()]),
            ("b".into(), vec!["d".into()]),
            ("c".into(), vec!["d".into()]),
            ("d".into(), vec![]),
        ];
        let deps = transitive_dependencies(&diamond, "a");
        assert_eq!(
            deps.iter().filter(|d| *d == "d").count(),
            1,
            "菱形依赖不得重复计入"
        );
        assert_eq!(deps.len(), 3);
    }
}
