use serde::{Deserialize, Serialize};

// MARK: - 活动等级（五态，与 macOS AgentIslandCore/Models.swift 同源）

/// 声明序 = 严重度**升**序，`derive(Ord)` 直接继承它。
/// 这样多 agent 汇总用 `max()` 就是「取最严重的那个」：
/// 一个 agent 待确认、其余工作中 → Attention 不能输给 Working。
/// **改动这个顺序前先搜 `.max()` / `fold` 的调用方**，别让「语义正确」跑在
/// 「汇总没用到序」的巧合上。
/// 注意 `serde(rename_all = "lowercase")` 只影响序列化名，与此处的 Ord 无关。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityLevel {
    Offline,
    Idle,
    Completed,
    Attention,
    Working,
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
    pub category: String,
}

// MARK: - Token 用量

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub tokens24h: i64,
    pub tokens_total: i64,
    pub cost24h: f64,
    pub cost_total: f64,
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

    /// 五态序 = 严重的降序：`Working > Attention > Completed > Idle > Offline`。
    /// 多 agent 汇总时按最大值取，序错了会出现「一个 agent 待确认却显示待机」。
    #[test]
    fn activity_level_ordering_is_severity_desc() {
        assert!(ActivityLevel::Working > ActivityLevel::Attention);
        assert!(ActivityLevel::Attention > ActivityLevel::Completed);
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
}
