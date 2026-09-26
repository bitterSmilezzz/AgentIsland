use serde::{Deserialize, Serialize};

// MARK: - 活动等级（五态，与 macOS AgentIslandCore/Models.swift 同源）

/// 声明序 = 严重度**升**序，`derive(Ord)` 直接继承它，`max()` 即「取最需要用户理的那个」。
///
/// **`attention` 必须排在 `working` 之上**，不是反过来：`attention` 是「agent 停下来等用户
/// 拍板」，`working` 是「agent 自己还在跑」。前者需要人立刻回去，后者不需要。
/// 这个序与 Swift 侧 `ActivityLevel.order()` 逐值一致
/// （[Models.swift:64-72](../../../Sources/AgentIslandCore/Models.swift)：offline 0 / idle 1 /
/// completed 2 / working 3 / attention 4）。
/// `418805c` 曾把它改反过（让 working 最大），被
/// [23 号口径对照表](23-swift-rust-parity-matrix.md)逐行比出来——**两边序不同，
/// 多 agent 汇总 `max()` 时会选出不同的 agent**，正是「两边都有但算法不同」那条红线。
/// `serde(rename_all = "lowercase")` 只影响序列化名，与此处的 Ord 无关。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityLevel {
    Offline,
    Idle,
    Completed,
    Working,
    Attention,
}

impl ActivityLevel {
    pub fn label(&self) -> &'static str {
        match self {
            ActivityLevel::Offline => "离线",
            ActivityLevel::Idle => "待机",
            ActivityLevel::Completed => "已完成",
            ActivityLevel::Working => "工作中",
            ActivityLevel::Attention => "待确认",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ActivityLevel::Offline => "offline",
            ActivityLevel::Idle => "idle",
            ActivityLevel::Completed => "completed",
            ActivityLevel::Working => "working",
            ActivityLevel::Attention => "attention",
        }
    }
}

// MARK: - 贴边停靠

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DockEdge {
    Top,
    Bottom,
    Left,
    Right,
}

impl DockEdge {
    pub fn as_str(&self) -> &'static str {
        match self {
            DockEdge::Top => "top",
            DockEdge::Bottom => "bottom",
            DockEdge::Left => "left",
            DockEdge::Right => "right",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "bottom" => DockEdge::Bottom,
            "left" => DockEdge::Left,
            "right" => DockEdge::Right,
            _ => DockEdge::Top,
        }
    }

    pub fn is_horizontal(&self) -> bool {
        matches!(self, DockEdge::Top | DockEdge::Bottom)
    }
}

// MARK: - Agent 档案

/// 第三方 Agent 的会话 / 明细库（位置与方言都由档案声明）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionDatabase {
    pub path: String,
    pub schema: SessionSchema,
}

/// 表形与方言：查询按**形状**路由，不按产品名——新增同形 fork 只改档案（ADR 0004）
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SessionSchema {
    /// DimAgent：`usage_ledger`，usage 是 JSON 列，cost 是记录值
    DimTasks,
    /// 通用状态索引：只回答「最新一条状态」，**不含 token**
    StatusIndex,
    /// OpenCode 及其同表 fork（如小米 MiMo Code）：`message.data` 是 JSON
    OpenCode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    /// 图标 glyph（前端用 Segoe/emoji 渲染）
    pub glyph: String,
    pub emoji: String,
    pub process_names: Vec<String>,
    /// 命令行提示：进程名不在名单（如 npm 安装的 CLI 跑在 node.exe 里）时，
    /// 命令行包含提示词即算命中
    #[serde(default)]
    pub cmdline_hints: Vec<String>,
    pub path_excludes: Vec<String>,
    /// 桌面类 Agent 的 CPU 工作判定下限（Electron 空闲抖动）
    pub cpu_floor: Option<f64>,
    pub session_dirs: Vec<String>,
    pub token_roots: Vec<String>,
    /// 会话 / 明细库。**库路径只在这一处声明**——别处再写一遍字面量，
    /// 档案换目录或改名以后只有一半会生效（`dimcode.sqlite` 与 WorkBuddy 的
    /// `projects/` 上各踩过一次，见 ADR 0004）。
    #[serde(default)]
    pub session_database: Option<SessionDatabase>,
    pub category: String,
}

// MARK: - Token 用量

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub tokens24h: i64,
    pub tokens_total: i64,
    pub cost24h: f64,
    pub cost_total: f64,
    /// 成本是**估**出来的还是日志里**记录**的。Rust 侧今天只有 JSONL 源、日志不带成本，
    /// 所以非零成本一律是估价，界面必须带 `~` 显示——把估的当记录值印出来，
    /// 就是「没读到 ≠ 编一个数」那条口径的反面。
    #[serde(default)]
    pub cost_estimated: bool,
}

// MARK: - 实时快照

#[derive(Debug, Clone, Serialize)]
pub struct AgentSnapshot {
    pub id: String,
    pub name: String,
    pub glyph: String,
    pub emoji: String,
    pub level: ActivityLevel,
    pub level_label: String,
    pub observability: crate::observability::Verdict,
    /// 卡死 / 僵卡三态：`None` 是「本轮判不出」（连续观测不足阈值），**不是「没有」**。
    /// 判定规则见 [`crate::health::is_hung`]。
    pub is_hung: Option<bool>,
    /// 100 分制健康度报告（[`crate::health::evaluate`]，对齐 Swift `AgentHealthEvaluator`）
    pub health: crate::health::Report,
    pub process_running: bool,
    pub cpu_percent: Option<f64>,
    pub memory_bytes: u64,
    pub memory_text: String,
    pub last_activity_text: String,
    pub token_usage: Option<TokenUsage>,
    pub pid: Option<u32>,
    pub current_action: Option<String>,
    pub subagent_count: usize,
}

// MARK: - 任务事件

#[derive(Debug, Clone, Serialize)]
pub struct AgentTaskEvent {
    pub id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub event_type: String, // completed | attention | costSpike
    pub timestamp: i64,     // unix ms
    pub message: Option<String>,
    pub detail: Option<String>,
    pub externally_delivered: bool,
}

impl AgentTaskEvent {
    pub fn summary(&self) -> String {
        if let Some(m) = &self.message {
            if !m.is_empty() {
                return m.clone();
            }
        }
        match self.event_type.as_str() {
            "attention" => format!("{} 等待确认操作", self.agent_name),
            "costSpike" => format!("⚠️ {} 资源/Token 消耗突增", self.agent_name),
            _ => format!("{} 任务完成", self.agent_name),
        }
    }
}

// MARK: - Token 明细（分析页/详情页）

#[derive(Debug, Clone, Serialize)]
pub struct ModelUsage {
    pub model: String,
    pub tokens: i64,
    pub cost: f64,
    /// 同 `TokenUsage::cost_estimated`：这个模型的花费是不是估的。
    pub cost_estimated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenReport {
    pub usage: TokenUsage,
    pub models24h: Vec<ModelUsage>,
    pub models_total: Vec<ModelUsage>,
    /// (unix ms 桶起点, tokens)
    pub hourly30d: Vec<(i64, i64)>,
}

// MARK: - 引擎推送给 UI 的完整状态

#[derive(Debug, Clone, Serialize)]
pub struct EngineState {
    pub snapshots: Vec<AgentSnapshot>,
    pub latest_event: Option<AgentTaskEvent>,
    pub grand_total: TokenUsage,
    pub dock_edge: DockEdge,
    pub appearance: String, // system | light | dark
    pub any_working: bool,
    pub has_attention: bool,
    pub demo: bool,
}

// MARK: - 测试（M1：Rust 测试基建，见 docs/adr/0010-swift-freeze-and-rust-prerequisites.md）

#[cfg(test)]
mod tests {
    use super::*;

    /// 五态与 Swift 侧 `Models.swift:44-72` 同源。**序列化名是 API 契约**：
    /// 前端 `ui/` 与 island/sidebar 两形态都吃这几个字符串，改名即静默错配。
    #[test]
    fn activity_level_serde_names_are_contract() {
        assert_eq!(ActivityLevel::Offline.as_str(), "offline");
        assert_eq!(ActivityLevel::Idle.as_str(), "idle");
        assert_eq!(ActivityLevel::Completed.as_str(), "completed");
        assert_eq!(ActivityLevel::Working.as_str(), "working");
        assert_eq!(ActivityLevel::Attention.as_str(), "attention");
    }

    /// 五态序 = 需要用户插手的急迫度升序：`Attention > Working > Completed > Idle > Offline`。
    /// **attention 高于 working**：前者是「agent 停下来等用户拍板」，后者是「agent 自己还在跑」。
    /// 多 agent 汇总用 `max()` 取「最需要人理的那个」，序错了会出现「有 agent 在等确认，
    /// 汇总却显示工作中」——它把人按在电脑前，而真正该催的那件事没人看见。
    /// 与 Swift 侧 `Models.swift:64-72` 的 `order()` 逐值一致（红线：两边算法必须相同）。
    #[test]
    fn activity_level_ordering_matches_swift() {
        assert!(ActivityLevel::Attention > ActivityLevel::Working);
        assert!(ActivityLevel::Working > ActivityLevel::Completed);
        assert!(ActivityLevel::Completed > ActivityLevel::Idle);
        assert!(ActivityLevel::Idle > ActivityLevel::Offline);
    }

    /// `label` 是用户可见文案，且每个态都必须有中文——空串会让灵动岛渲染成空白卡片
    /// （codenotch 的空态教训：无会话时 cell 直接消失，而不是留一句「什么都没发生」）。
    #[test]
    fn activity_level_labels_are_non_empty_chinese() {
        for lv in [
            ActivityLevel::Offline,
            ActivityLevel::Idle,
            ActivityLevel::Completed,
            ActivityLevel::Working,
            ActivityLevel::Attention,
        ] {
            assert!(!lv.label().trim().is_empty(), "{lv:?} 的 label 为空");
            assert!(
                lv.label().chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
                "{lv:?} 的 label 不是中文"
            );
        }
    }

    /// `DockEdge::parse` 对未知值兜底为 `Top`。这是**有意为之的兼容路径**
    /// （旧设置里可能存着已被删除的枚举值），所以默认值必须被钉住——
    /// 谁把兜底改成别的边，老用户的 dock 位置会静默漂移。
    #[test]
    fn dock_edge_parse_defaults_to_top() {
        assert_eq!(DockEdge::parse("bottom"), DockEdge::Bottom);
        assert_eq!(DockEdge::parse("left"), DockEdge::Left);
        assert_eq!(DockEdge::parse("right"), DockEdge::Right);
        assert_eq!(DockEdge::parse("top"), DockEdge::Top);
        assert_eq!(DockEdge::parse(""), DockEdge::Top);
        assert_eq!(DockEdge::parse("diagonal"), DockEdge::Top);
    }

    /// `is_horizontal` 决定 placement 走哪条轴。Top|Bottom 与 Left|Right 必须是**全集划分**，
    /// 漏一个就有一侧的窗口永远定位到错误的坐标空间。
    #[test]
    fn dock_edge_is_horizontal_is_total_partition() {
        for e in [DockEdge::Top, DockEdge::Bottom, DockEdge::Left, DockEdge::Right] {
            assert!(e.is_horizontal() || !e.is_horizontal(), "{e:?} 的值不是布尔");
        }
        assert!(DockEdge::Top.is_horizontal());
        assert!(DockEdge::Bottom.is_horizontal());
        assert!(!DockEdge::Left.is_horizontal());
        assert!(!DockEdge::Right.is_horizontal());
    }

    /// **视图读的字段必须在快照 JSON 里真实存在。**
    ///
    /// 视图读一个后端没输出的键时，不会报错、不会变空白——那一档功能静默失效。
    /// 本仓真发生过：`views.js` 读 `snap.isHung`，而 Rust 输出的是 `is_hung`（快照没有
    /// `rename_all`），于是「卡死时圆环变红」这条路径从来没有亮过。
    /// 这条哨兵**直接解析 `views.js` 源码**，与费率表的跨语言哨兵同一套思路：
    /// 改名字而忘了改另一侧，测试就红。
    #[test]
    fn every_snapshot_field_the_ui_reads_exists_in_the_json() {
        let snapshot = AgentSnapshot {
            id: "fixture".into(),
            name: "Fixture".into(),
            glyph: String::new(),
            emoji: String::new(),
            level: ActivityLevel::Idle,
            level_label: "空闲".into(),
            observability: crate::observability::Verdict {
                code: crate::observability::Code::Observed,
                summary: "",
                evidence: Vec::new(),
            },
            is_hung: Some(true),
            health: crate::health::Report::not_running(),
            process_running: true,
            cpu_percent: Some(1.0),
            memory_bytes: 0,
            memory_text: "—".into(),
            last_activity_text: "—".into(),
            token_usage: Some(TokenUsage::default()),
            pid: None,
            current_action: None,
            subagent_count: 0,
        };
        let value = serde_json::to_value(&snapshot).expect("快照应能序列化");
        let object = value.as_object().expect("快照应序列化成对象");

        let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/js/views.js"))
            .expect("读不到 views.js——这条哨兵的存在意义就是跨文件对名字");
        let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for (index, _) in source.match_indices("snap.") {
            let rest = &source[index + "snap.".len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                names.insert(name);
            }
        }
        assert!(names.len() >= 8, "只抓到 {} 个 snap. 字段，解析八成坏了", names.len());
        let missing: Vec<&String> = names.iter().filter(|n| !object.contains_key(*n)).collect();
        assert!(
            missing.is_empty(),
            "views.js 读了快照里没有的字段：{missing:?}（静默失效，不会报错）"
        );
    }
}
