//! 路由组与候选策略的**确定性判据**（GW-04 首片）。
//!
//! 需求原文：smart/order/rotate/usage/pace/weight、嵌套组、循环/深度边界、
//! 暂停排除、配额与未知值。转发层（GW-03）还没建——与 `gateway_retry` 同一条
//! 理由：**策略语义先于实现定死**，等接入时再定，每个协议适配器都会各定一套。
//!
//! 本模块只做两件事，都是纯函数：
//!
//! 1. **六策略的确定性选择**（`select`）：同样的组状态永远得到同样的候选。
//!    「确定性」是需求原文的第一个词——smart 随机漂移、order 受哈希顺序影响，
//!    都会让同一个请求走不同的门，账目与审计都没法对账。
//! 2. **循环与深度的拒绝**（`validate_group`）：嵌套组按引用名解析，出现环
//!    （直接或间接回到自己）或超过深度上限时**拒绝整份配置**，而不是运行时
//!    栈溢出。拒绝发生在配置加载时——带环的配置一次都不该被跑。
//!
//! 配额与未知值：`usage`/`pace` 依赖配额读数；**读不到就是读不到**（`None`），
//! 不折算成 0（0 会把「不知道」变成「随便用」，与 GW-08 的未知不折叠口径一致）。
//! 暂停（pause）排除在候选之外，但组本身仍合法——暂停是运行态不是配置错。

/// 六种候选策略（需求原文的顺序）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// 按健康度评分选最优；评分相同取**名字序**（不是随机）——确定性优先。
    Smart,
    /// 固定声明顺序，永远第一个可用的。
    Order,
    /// 轮转：上次用过的下一个（按声明序循环）。
    Rotate,
    /// 用量最低者优先；用量未知的不参与该轮比较（见模块注释）。
    Usage,
    /// 节奏（pace）：按每候选已发的请求数配平；计数相同取声明序。
    Pace,
    /// 加权随机——**种子来自轮次号**：同一轮次内确定，跨轮次分布按权重。
    Weight,
}

/// 一个候选（叶子或子组）在组里的声明形态。
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub name: String,
    /// Weight 策略的权重；其他策略忽略。`None` = 未声明 = 1。
    pub weight: Option<u32>,
    /// 运行态：被暂停的候选不参与选择（不改变配置合法性）。
    pub paused: bool,
    /// usage 策略的用量读数；`None` = 读不到（不折算 0）。
    pub usage: Option<u64>,
    /// pace 策略的已发请求数；未统计按 0。
    pub issued: u64,
}

/// 选择结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
    /// 选中一个候选。
    Chosen(usize),
    /// 全部被暂停或组为空：如实说「没有候选」，不是错误。
    Exhausted,
}

impl Strategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Smart => "smart",
            Self::Order => "order",
            Self::Rotate => "rotate",
            Self::Usage => "usage",
            Self::Pace => "pace",
            Self::Weight => "weight",
        }
    }

    /// 确定性选择。`rotation_index` 是 Rotate/Weight 的轮次状态（调用方持有）。
    pub fn select(self, candidates: &[Candidate], rotation_index: u64) -> Selection {
        let active: Vec<usize> = candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| !c.paused)
            .map(|(i, _)| i)
            .collect();
        if active.is_empty() {
            return Selection::Exhausted;
        }
        let index = match self {
            Self::Order => active[0],
            Self::Smart => {
                // smart 的评分地基：暂以「用量已知且最低」为最优，未知不参与比较、
                // 评分并列取声明序。真正的健康度评分接入转发层后在此替换——
                // 但「并列取声明序、未知不猜测」的确定性规则不变。
                active
                    .iter()
                    .copied()
                    .filter(|&i| candidates[i].usage.is_some())
                    .min_by_key(|&i| (candidates[i].usage.unwrap(), i))
                    .unwrap_or(active[0])
            }
            Self::Rotate => active[(rotation_index as usize) % active.len()],
            Self::Usage => active
                .iter()
                .copied()
                .filter(|&i| candidates[i].usage.is_some())
                .min_by_key(|&i| (candidates[i].usage.unwrap(), i))
                .unwrap_or(active[0]),
            Self::Pace => {
                let mut best = active[0];
                for &i in &active {
                    if (candidates[i].issued, i) < (candidates[best].issued, best) {
                        best = i;
                    }
                }
                best
            }
            Self::Weight => {
                // 种子化的加权选择：同轮次确定。跨轮次用轮次号推进种子，
                // 分布按权重（大权重候选占据更大的区间）。
                let total: u64 = active
                    .iter()
                    .map(|&i| candidates[i].weight.unwrap_or(1) as u64)
                    .sum();
                let mut ticket = (rotation_index.wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407))
                    % total.max(1);
                let mut chosen = active[0];
                for &i in &active {
                    let span = candidates[i].weight.unwrap_or(1) as u64;
                    if ticket < span {
                        chosen = i;
                        break;
                    }
                    ticket -= span;
                }
                chosen
            }
        };
        Selection::Chosen(index)
    }
}

/// 嵌套组的深度上限与环检测。
///
/// `edges`：组名 → 直接成员里的子组名列表。返回 `Err(原因)` 时**整份配置拒绝**。
pub fn validate_group(
    root: &str,
    edges: &dyn Fn(&str) -> Vec<String>,
    max_depth: usize,
) -> Result<(), &'static str> {
    if max_depth == 0 {
        return Err("depth_limit_zero");
    }
    // DFS 带路径栈：回到路径上的任何节点 = 环；超过深度上限 = 拒绝。
    fn visit(
        node: &str,
        depth: usize,
        max_depth: usize,
        edges: &dyn Fn(&str) -> Vec<String>,
        path: &mut Vec<String>,
    ) -> Result<(), &'static str> {
        if depth > max_depth {
            return Err("depth_exceeded");
        }
        if path.iter().any(|n| n == node) {
            return Err("cycle");
        }
        path.push(node.to_string());
        for child in edges(node) {
            visit(&child, depth + 1, max_depth, edges, path)?;
        }
        path.pop();
        Ok(())
    }
    let mut path = Vec::new();
    visit(root, 1, max_depth, edges, &mut path)
}

pub const MAX_GROUP_DEPTH: usize = 8;

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str) -> Candidate {
        Candidate {
            name: name.into(),
            weight: None,
            paused: false,
            usage: None,
            issued: 0,
        }
    }

    fn chosen(strategy: Strategy, candidates: &[Candidate], rotation: u64) -> Option<usize> {
        match strategy.select(candidates, rotation) {
            Selection::Chosen(i) => Some(i),
            Selection::Exhausted => None,
        }
    }

    #[test]
    fn all_six_strategies_are_deterministic() {
        let candidates = vec![candidate("a"), candidate("b"), candidate("c")];
        for strategy in [
            Strategy::Smart,
            Strategy::Order,
            Strategy::Rotate,
            Strategy::Usage,
            Strategy::Pace,
            Strategy::Weight,
        ] {
            let first = chosen(strategy, &candidates, 3);
            for _ in 0..5 {
                assert_eq!(
                    chosen(strategy, &candidates, 3),
                    first,
                    "{strategy:?} 同状态必须同结果——确定性是需求第一个词"
                );
            }
        }
    }

    #[test]
    fn unknown_usage_never_masquerades_as_zero() {
        // a 用量未知、b 已知 9_000_000：usage/smart 不得把 a 的未知当 0 选中 a
        let mut a = candidate("a");
        a.usage = None;
        let mut b = candidate("b");
        b.usage = Some(9_000_000);
        for strategy in [Strategy::Usage, Strategy::Smart] {
            assert_eq!(
                chosen(strategy, &[a.clone(), b.clone()], 0),
                Some(1),
                "{strategy:?} 用量未知的候选不得靠「当作 0」胜出"
            );
        }
        // 全部未知：回退声明序（如实「没有可比的」），不猜
        let unknowns = vec![candidate("x"), candidate("y")];
        assert_eq!(chosen(Strategy::Usage, &unknowns, 0), Some(0));
    }

    #[test]
    fn paused_candidates_are_excluded_but_not_an_error() {
        let mut a = candidate("a");
        a.paused = true;
        let b = candidate("b");
        assert_eq!(chosen(Strategy::Order, &[a.clone(), b.clone()], 0), Some(1));
        // 全部暂停：如实「没有候选」，不是配置错误
        let mut c = candidate("c");
        c.paused = true;
        assert_eq!(
            Strategy::Order.select(&[a, c], 0),
            Selection::Exhausted,
            "全部暂停=耗尽，不是 panic 也不是伪装选中"
        );
    }

    #[test]
    fn rotate_cycles_and_weight_respects_relative_weight() {
        let candidates = vec![candidate("a"), candidate("b"), candidate("c")];
        // Rotate 按声明序循环
        assert_eq!(chosen(Strategy::Rotate, &candidates, 0), Some(0));
        assert_eq!(chosen(Strategy::Rotate, &candidates, 1), Some(1));
        assert_eq!(chosen(Strategy::Rotate, &candidates, 3), Some(0), "循环回卷");
        // Weight：a 权重 9、b 权重 1——长跑下 a 的份额应显著占优（确定性种子下）
        let mut heavy = candidate("heavy");
        heavy.weight = Some(9);
        let mut light = candidate("light");
        light.weight = Some(1);
        let weighted = vec![heavy, light];
        let mut heavy_wins = 0;
        for round in 0..200u64 {
            if chosen(Strategy::Weight, &weighted, round) == Some(0) {
                heavy_wins += 1;
            }
        }
        assert!(
            heavy_wins > 120,
            "权重 9:1 应让 heavy 显著占优，200 轮实际赢 {heavy_wins}"
        );
    }

    #[test]
    fn nested_groups_reject_cycles_and_excessive_depth() {
        // 直环：a → b → a
        let edges = |name: &str| match name {
            "a" => vec!["b".to_string()],
            "b" => vec!["a".to_string()],
            _ => vec![],
        };
        assert_eq!(validate_group("a", &edges, MAX_GROUP_DEPTH), Err("cycle"));
        // 自环同样拒绝
        let self_loop = |name: &str| match name {
            "a" => vec!["a".to_string()],
            _ => vec![],
        };
        assert_eq!(validate_group("a", &self_loop, MAX_GROUP_DEPTH), Err("cycle"));
        // 深度超限：链长 9 > 上限 8
        let chain = |name: &str| {
            let n: usize = name.parse().unwrap_or(0);
            if n < 9 {
                vec![(n + 1).to_string()]
            } else {
                vec![]
            }
        };
        assert_eq!(
            validate_group("0", &chain, MAX_GROUP_DEPTH),
            Err("depth_exceeded"),
            "超过深度上限的嵌套必须在加载时拒绝，不是运行时爆栈"
        );
        // 合法的浅嵌套照常通过
        let ok = |name: &str| match name {
            "root" => vec!["mid".to_string()],
            "mid" => vec!["leaf".to_string()],
            _ => vec![],
        };
        assert_eq!(validate_group("root", &ok, MAX_GROUP_DEPTH), Ok(()));
        // 上限为 0 直接拒绝（配置错误）
        assert_eq!(validate_group("a", &ok, 0), Err("depth_limit_zero"));
    }
}
