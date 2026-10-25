//! 条件/意图/会话路由的**判据**（GW-05 首片）。
//!
//! 需求原文：首规则、AND、项目/时间/图片/上下文等、意图降级、黏性、轮内一致性。
//! 与 `gateway_routing`（GW-04）、`gateway_retry`（GW-06）同一条理由：
//! **策略语义先于实现定死**——转发层接入时再定，每个适配器都会各定一套。
//!
//! 四条口径（本模块只做这四件，全是纯函数）：
//!
//! 1. **首规则**：按声明顺序取**第一条**满足的规则，不做「更具体者优先」的重排——
//!    重排规则会随条件类型增减而漂移，声明顺序是作者唯一能掌控的那一处。
//! 2. **AND**：一条规则的条件**全部**满足才算命中；任一条件读不到（未知）即不满足。
//!    未知不折算成「满足」也不折算成「不满足」——它就是**这条规则不命中**，
//!    继续往下一条规则找（与 GW-04/GW-08 的未知不折叠同口径）。
//! 3. **意图降级**：意图（模型给出的路由提示）超时/出错/无匹配时，**降级**到条件规则，
//!    而不是报错或随机选一个。意图是加速器，不是必需品。
//! 4. **黏性与轮内一致性**：同一轮（round）内一旦选定目标就**保持**，
//!    除非该目标这一轮不再可用；跨轮才允许重新选择。轮内换目标会让同一次
//!    对话的前后半段走不同的门，账目与审计都对不上。
//!
//! **时间窗口用调用方给的「本地分钟数」**：本模块不读系统时钟、不自己算时区——
//! 时区与夏令时由调用方用 `localclock::offset_minutes` 算好后注入。
//! 理由：本地时间的边界（含 DST 切换日）必须由「那个时区的规则」决定，
//! 而本仓不内置时区数据库（`localclock` 走系统日历接口）。

/// 单个条件。`None` 类事实（读不到）一律判为**不满足**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Condition {
    /// 项目匹配（精确比较，缺大小写折叠——项目名是用户写的字面量）。
    Project(String),
    /// 本地时间窗，`start_minutes`/`end_minutes` 是从午夜起的分钟数；
    /// `start > end` 表示**跨午夜**（如 22:00–02:00）。
    LocalTime { start_minutes: u16, end_minutes: u16 },
    /// 上下文里有没有图片（本次请求带没带图）。
    HasImage(bool),
    /// 上下文里是否提到某个词（大小写不敏感）。
    ContextMentions(String),
    /// **目录/文件边界**：请求触及的路径必须落在声明的根目录内。
    ///
    /// 判据是「路径解析后仍在根内」，不是「字符串前缀相同」——
    /// `/work/app-evil` 以 `/work/app` 开头但**不在** `/work/app` 内，
    /// 前缀比较会把它放行。含 `..` 的相对路径一律不满足（归一规则跨实现不一致）。
    PathWithin { root: String, path: String },
    /// **上下文升级**：上下文轮次达到阈值才命中（用于「长上下文升级到更强模型」）。
    ContextDepth { min_turns: u32 },
}

/// 一次路由决策所依据的事实。**读不到的字段是 `None`**，不是默认值。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub project: Option<String>,
    /// 本地时间（午夜起的分钟数）。`None` = 读不到当地时间。
    pub local_minutes: Option<u16>,
    pub has_image: Option<bool>,
    pub context: Option<String>,
    /// 本次请求触及的路径（相对路径或绝对路径都行）。
    pub touched_path: Option<String>,
    /// 上下文轮次（对话已进行多少轮）。`None` = 读不到。
    pub context_turns: Option<u32>,
}

/// 一条规则：条件全部满足（AND）才命中，指向一个目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub name: String,
    pub conditions: Vec<Condition>,
    pub target: String,
}

impl Rule {
    /// 条件是否全部满足。**空条件 = 恒真**（兜底规则就该无条件命中）。
    pub fn matches(&self, facts: &Facts) -> bool {
        self.conditions.iter().all(|condition| match condition {
            Condition::Project(expected) => match &facts.project {
                Some(actual) => actual == expected,
                None => false,
            },
            Condition::LocalTime {
                start_minutes,
                end_minutes,
            } => match facts.local_minutes {
                Some(minutes) => {
                    if start_minutes <= end_minutes {
                        minutes >= *start_minutes && minutes < *end_minutes
                    } else {
                        // 跨午夜：22:00–02:00 覆盖 [start, 1440) ∪ [0, end)
                        minutes >= *start_minutes || minutes < *end_minutes
                    }
                }
                None => false,
            },
            Condition::HasImage(expected) => match facts.has_image {
                Some(actual) => actual == *expected,
                None => false,
            },
            Condition::ContextMentions(needle) => match &facts.context {
                Some(context) => context.to_lowercase().contains(&needle.to_lowercase()),
                None => false,
            },
            Condition::PathWithin { root, path } => match &facts.touched_path {
                Some(touched) => path_within(root, touched),
                None => false,
            },
            Condition::ContextDepth { min_turns } => match facts.context_turns {
                Some(turns) => turns >= *min_turns,
                None => false,
            },
        })
    }
}

/// 路径是否落在根目录内（**按路径段判断**，不是字符串前缀）。
///
/// `/work/app-evil` 以 `/work/app` 开头但不在其内——前缀比较会把它放行。
/// 含 `..` 的相对路径一律判为不在根内（归一规则跨实现不一致，宁可拒）。
pub fn path_within(root: &str, path: &str) -> bool {
    let root = root.trim_end_matches('/');
    let root_parts: Vec<&str> = root.split('/').filter(|s| !s.is_empty()).collect();
    let path_parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if path_parts.iter().any(|part| *part == "..") {
        return false;
    }
    if root_parts.is_empty() {
        return false;
    }
    if path_parts.len() < root_parts.len() {
        return false;
    }
    root_parts
        .iter()
        .zip(path_parts.iter())
        .all(|(root_part, path_part)| root_part == path_part)
}

/// 意图路由的结果。`Unavailable` 覆盖「超时/出错」——两者对路由的意义相同：
/// 拿不到意图就降级，不是报错。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// 意图可用且有对应目标。
    Resolved(String),
    /// 意图不可用（超时/出错）。
    Unavailable,
    /// 意图可用但没有匹配的目标。
    NoMatch,
}

/// 路由模式（GW-05 场景：Strict 不切）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RoutingMode {
    /// 默认模式：意图优先，不可用/无匹配时**降级**到条件规则。
    #[default]
    Normal,
    /// 严格模式（Strict 不切）：必须按意图走，意图失败/无匹配时**直接返回 Unrouted**，
    /// 不切回条件规则——用于「意图没把握就宁可不发」的场景。
    Strict,
}

/// 意图缓存条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedIntent {
    pub target: String,
    pub cached_at_ms: i64,
}

/// 意图缓存（避免同一 prompt 重复调模型分类；带 TTL 与容量上限）。
#[derive(Debug, Default)]
pub struct IntentCache {
    entries: std::collections::HashMap<String, CachedIntent>,
}

pub const INTENT_CACHE_TTL_MS: i64 = 10 * 60 * 1000; // 10 分钟
pub const INTENT_CACHE_CAP: usize = 1000;

impl IntentCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// 查缓存：存在且在 TTL 窗口内返回目标；超窗自动清除。
    pub fn get(&mut self, prompt: &str, now_ms: i64) -> Option<&str> {
        let key = prompt.trim();
        if let Some(entry) = self.entries.get(key) {
            if now_ms - entry.cached_at_ms <= INTENT_CACHE_TTL_MS {
                return Some(&self.entries.get(key).unwrap().target);
            }
        }
        self.entries.remove(key); // 超窗清除
        None
    }

    /// 写入缓存。容量达到 `INTENT_CACHE_CAP` 时淘汰最旧的条目。
    pub fn put(&mut self, prompt: &str, target: &str, now_ms: i64) {
        let key = prompt.trim();
        if self.entries.len() >= INTENT_CACHE_CAP {
            if let Some(oldest_key) = self
                .entries
                .iter()
                .min_by_key(|(_, v)| v.cached_at_ms)
                .map(|(k, _)| k.clone())
            {
                self.entries.remove(&oldest_key);
            }
        }
        self.entries.insert(
            key.to_string(),
            CachedIntent {
                target: target.to_string(),
                cached_at_ms: now_ms,
            },
        );
    }
}

/// 选择结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// 命中目标（附命中来源：意图还是规则名）。
    Target { target: String, source: String },
    /// 没有命中任何目标（规则全不满足且意图无匹配）——如实说「没路由到」。
    Unrouted,
}

/// 黏性状态：跨调用保存（同一轮内保持目标）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sticky {
    round: u64,
    target: Option<String>,
    source: Option<String>,
}

/// 路由决策入口（默认 Normal 模式；与既有签名兼容）。
pub fn route(
    rules: &[Rule],
    facts: &Facts,
    intent: &Intent,
    round: u64,
    sticky: &mut Sticky,
) -> Selection {
    route_with_mode(rules, facts, intent, round, sticky, RoutingMode::Normal)
}

/// 带模式的路由决策入口。
pub fn route_with_mode(
    rules: &[Rule],
    facts: &Facts,
    intent: &Intent,
    round: u64,
    sticky: &mut Sticky,
    mode: RoutingMode,
) -> Selection {
    // 轮内一致性：同一轮且上一轮选的目标这一轮仍可用 → 保持。
    if sticky.round == round {
        if let Some(target) = &sticky.target {
            return Selection::Target {
                target: target.clone(),
                source: sticky.source.clone().unwrap_or_else(|| "sticky".into()),
            };
        }
    }
    // 意图处理
    let (target, source) = match intent {
        Intent::Resolved(target) => (target.clone(), "intent".to_string()),
        Intent::Unavailable | Intent::NoMatch => {
            // Strict 模式：意图失败不切条件规则，直接 Unrouted
            if mode == RoutingMode::Strict {
                sticky.round = round;
                sticky.target = None;
                sticky.source = None;
                return Selection::Unrouted;
            }
            // Normal 模式：降级到条件规则（首条满足的）
            match rules.iter().find(|rule| rule.matches(facts)) {
                Some(rule) => (rule.target.clone(), format!("rule:{}", rule.name)),
                None => {
                    sticky.round = round;
                    sticky.target = None;
                    sticky.source = None;
                    return Selection::Unrouted;
                }
            }
        }
    };
    sticky.round = round;
    sticky.target = Some(target.clone());
    sticky.source = Some(source.clone());
    Selection::Target { target, source }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(name: &str, conditions: Vec<Condition>, target: &str) -> Rule {
        Rule {
            name: name.into(),
            conditions,
            target: target.into(),
        }
    }

    fn base_facts() -> Facts {
        Facts {
            project: Some("web".into()),
            local_minutes: Some(9 * 60), // 09:00
            has_image: Some(false),
            context: Some("请修一下登录页的样式".into()),
            touched_path: None,
            context_turns: None,
        }
    }

    #[test]
    fn the_first_matching_rule_wins_in_declaration_order() {
        let rules = vec![
            rule("图片", vec![Condition::HasImage(true)], "vision"),
            rule("项目", vec![Condition::Project("web".into())], "web-target"),
            rule("兜底", vec![], "fallback"),
        ];
        let mut sticky = Sticky::default();
        let selection = route(&rules, &base_facts(), &Intent::Unavailable, 1, &mut sticky);
        assert_eq!(
            selection,
            Selection::Target {
                target: "web-target".into(),
                source: "rule:项目".into()
            },
            "首规则命中：图片规则不满足，项目规则先于兜底"
        );
    }

    #[test]
    fn conditions_are_anded_and_unknown_facts_do_not_match() {
        let rules = vec![
            rule(
                "项目且图片",
                vec![
                    Condition::Project("web".into()),
                    Condition::HasImage(true),
                ],
                "both",
            ),
            rule("兜底", vec![], "fallback"),
        ];
        // 图片为 false ⇒ AND 不满足 ⇒ 落到兜底
        let mut sticky = Sticky::default();
        assert_eq!(
            route(&rules, &base_facts(), &Intent::Unavailable, 1, &mut sticky),
            Selection::Target {
                target: "fallback".into(),
                source: "rule:兜底".into()
            }
        );
        // 事实读不到（None）⇒ 同样不满足（不折算成 true 也不折算成 false）
        let mut unknown = base_facts();
        unknown.has_image = None;
        let mut sticky = Sticky::default();
        assert_eq!(
            route(&rules, &unknown, &Intent::Unavailable, 1, &mut sticky),
            Selection::Target {
                target: "fallback".into(),
                source: "rule:兜底".into()
            },
            "未知事实不得让规则命中"
        );
    }

    #[test]
    fn local_time_window_handles_the_day_boundary() {
        let rules = vec![rule(
            "夜间",
            vec![Condition::LocalTime {
                start_minutes: 22 * 60,
                end_minutes: 2 * 60,
            }],
            "night",
        )];
        // 每个时刻用**独立**的黏性状态与轮次：否则上一例的黏性目标会被
        // 下一例继承（轮内一致性按设计如此——用例不能共享状态）。
        let at = |minutes: u16| {
            let mut facts = base_facts();
            facts.local_minutes = Some(minutes);
            let mut sticky = Sticky::default();
            route(&rules, &facts, &Intent::Unavailable, minutes as u64, &mut sticky)
        };
        // 23:30 在夜间窗内（跨午夜的后半段）
        assert!(
            matches!(at(23 * 60 + 30), Selection::Target { ref target, .. } if target == "night"),
            "23:30 应命中夜间窗"
        );
        // 01:00 也在窗内（跨午夜的前半段）
        assert!(
            matches!(at(60), Selection::Target { ref target, .. } if target == "night"),
            "01:00 应命中跨午夜窗的前半段"
        );
        // 12:00 不在窗内
        assert_eq!(at(12 * 60), Selection::Unrouted, "12:00 不在夜间窗内");
        // 22:00 是窗的起点（含），02:00 是终点（不含）
        assert!(matches!(
            at(22 * 60),
            Selection::Target { ref target, .. } if target == "night"
        ));
        assert_eq!(at(2 * 60), Selection::Unrouted, "终点不含");
    }

    #[test]
    fn an_unavailable_intent_degrades_to_the_rules_instead_of_failing() {
        let rules = vec![rule("项目", vec![Condition::Project("web".into())], "web-target")];
        let mut sticky = Sticky::default();
        // 意图不可用 ⇒ 降级到条件规则
        assert_eq!(
            route(&rules, &base_facts(), &Intent::Unavailable, 1, &mut sticky),
            Selection::Target {
                target: "web-target".into(),
                source: "rule:项目".into()
            }
        );
        // 意图可用 ⇒ 用意图目标（优先于规则）
        let mut sticky = Sticky::default();
        assert_eq!(
            route(&rules, &base_facts(), &Intent::Resolved("intent-target".into()), 1, &mut sticky),
            Selection::Target {
                target: "intent-target".into(),
                source: "intent".into()
            }
        );
        // 意图可用但无匹配 ⇒ 也降级到规则（不是 Unrouted）
        let mut sticky = Sticky::default();
        assert_eq!(
            route(&rules, &base_facts(), &Intent::NoMatch, 1, &mut sticky),
            Selection::Target {
                target: "web-target".into(),
                source: "rule:项目".into()
            }
        );
    }

    #[test]
    fn a_round_keeps_its_target_and_a_new_round_may_switch() {
        let rules = vec![
            rule("图片", vec![Condition::HasImage(true)], "vision"),
            rule("项目", vec![Condition::Project("web".into())], "web-target"),
        ];
        let mut sticky = Sticky::default();
        // 第一轮：命中项目规则
        let first = route(&rules, &base_facts(), &Intent::Unavailable, 1, &mut sticky);
        assert_eq!(
            first,
            Selection::Target {
                target: "web-target".into(),
                source: "rule:项目".into()
            }
        );
        // 同一轮内：事实变了（图片出现）也**保持**原目标（轮内一致性）
        let mut with_image = base_facts();
        with_image.has_image = Some(true);
        assert_eq!(
            route(&rules, &with_image, &Intent::Unavailable, 1, &mut sticky),
            Selection::Target {
                target: "web-target".into(),
                source: "rule:项目".into()
            },
            "轮内不得换目标——同一对话的前后半段必须走同一道门"
        );
        // 跨轮：允许重新选择
        assert_eq!(
            route(&rules, &with_image, &Intent::Unavailable, 2, &mut sticky),
            Selection::Target {
                target: "vision".into(),
                source: "rule:图片".into()
            },
            "新轮次才允许重新选择"
        );
    }

    #[test]
    fn strict_mode_does_not_fall_back_to_rules() {
        let rules = vec![rule("项目", vec![Condition::Project("web".into())], "web-target")];
        let mut sticky = Sticky::default();

        // Normal 模式：意图不可用 ⇒ 降级到条件规则
        assert_eq!(
            route_with_mode(&rules, &base_facts(), &Intent::Unavailable, 1, &mut sticky, RoutingMode::Normal),
            Selection::Target { target: "web-target".into(), source: "rule:项目".into() }
        );

        // Strict 模式：意图不可用 ⇒ **直接 Unrouted**（Strict 不切，不猜）
        let mut sticky = Sticky::default();
        assert_eq!(
            route_with_mode(&rules, &base_facts(), &Intent::Unavailable, 1, &mut sticky, RoutingMode::Strict),
            Selection::Unrouted,
            "Strict 模式下意图失败不许降级到条件规则"
        );
        // Strict 模式：意图命中正常走
        let mut sticky = Sticky::default();
        assert_eq!(
            route_with_mode(&rules, &base_facts(), &Intent::Resolved("t1".into()), 1, &mut sticky, RoutingMode::Strict),
            Selection::Target { target: "t1".into(), source: "intent".into() }
        );
    }

    #[test]
    fn intent_cache_hits_within_ttl_and_evicts_oldest_on_cap() {
        let mut cache = IntentCache::new();
        cache.put("帮我写个登录页", "model-coding", 1_000);

        // 窗口内命中
        assert_eq!(cache.get("帮我写个登录页", 1_000 + 60_000), Some("model-coding"));
        // 超窗未命中（自动清除）
        assert_eq!(
            cache.get("帮我写个登录页", 1_000 + INTENT_CACHE_TTL_MS + 1),
            None,
            "超窗的意图必须重新识别"
        );

        // 塞满容量淘汰最旧
        let mut small_cache = IntentCache::new();
        for i in 0..INTENT_CACHE_CAP {
            small_cache.put(&format!("prompt-{i}"), &format!("target-{i}"), 1_000 + i as i64);
        }
        assert_eq!(small_cache.entries.len(), INTENT_CACHE_CAP);
        // 塞第 1001 条：最旧的 prompt-0 被淘汰
        small_cache.put("prompt-new", "target-new", 2_000);
        assert_eq!(small_cache.get("prompt-0", 2_000), None);
        assert_eq!(small_cache.get("prompt-new", 2_000), Some("target-new"));
    }

    #[test]
    fn path_conditions_are_judged_by_path_segments_not_string_prefix() {
        let rules = vec![rule(
            "工作区内",
            vec![Condition::PathWithin {
                root: "/work/app".into(),
                path: "/work/app/src/main.rs".into(),
            }],
            "in-workspace",
        )];

        // 根内的路径命中
        let mut facts = base_facts();
        facts.touched_path = Some("/work/app/src/main.rs".into());
        let mut sticky = Sticky::default();
        assert_eq!(
            route(&rules, &facts, &Intent::Unavailable, 1, &mut sticky),
            Selection::Target {
                target: "in-workspace".into(),
                source: "rule:工作区内".into()
            }
        );

        // **同前缀但不在根内** ⇒ 不命中（前缀比较会放行，这是最容易写错的一处）
        let mut outside = base_facts();
        outside.touched_path = Some("/work/app-evil/steal.sh".into());
        let mut sticky = Sticky::default();
        assert_eq!(
            route(&rules, &outside, &Intent::Unavailable, 1, &mut sticky),
            Selection::Unrouted,
            "/work/app-evil 不在 /work/app 内"
        );

        // 含 `..` 的相对路径一律不满足
        let mut traversal = base_facts();
        traversal.touched_path = Some("/work/app/../../etc/passwd".into());
        let mut sticky = Sticky::default();
        assert_eq!(
            route(&rules, &traversal, &Intent::Unavailable, 1, &mut sticky),
            Selection::Unrouted,
            "含 .. 的路径不得被放行"
        );

        // 读不到路径 ⇒ 不满足
        let mut sticky = Sticky::default();
        assert_eq!(
            route(&rules, &base_facts(), &Intent::Unavailable, 1, &mut sticky),
            Selection::Unrouted
        );
    }

    #[test]
    fn context_depth_drives_the_upgrade_rule() {
        let rules = vec![rule(
            "长上下文升级",
            vec![Condition::ContextDepth { min_turns: 20 }],
            "strong-model",
        )];

        // 轮次不足 ⇒ 不命中升级规则
        let mut short = base_facts();
        short.context_turns = Some(5);
        let mut sticky = Sticky::default();
        assert_eq!(
            route(&rules, &short, &Intent::Unavailable, 1, &mut sticky),
            Selection::Unrouted
        );

        // 轮次达到阈值 ⇒ 命中
        let mut deep = base_facts();
        deep.context_turns = Some(20);
        let mut sticky = Sticky::default();
        assert_eq!(
            route(&rules, &deep, &Intent::Unavailable, 1, &mut sticky),
            Selection::Target {
                target: "strong-model".into(),
                source: "rule:长上下文升级".into()
            }
        );

        // 读不到轮次 ⇒ 不满足（不猜）
        let mut sticky = Sticky::default();
        assert_eq!(
            route(&rules, &base_facts(), &Intent::Unavailable, 1, &mut sticky),
            Selection::Unrouted
        );
    }
}
