use serde::{Deserialize, Serialize};

// MARK: - 活动等级（五态，与 macOS AgentIslandCore/Models.swift 同源）

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
