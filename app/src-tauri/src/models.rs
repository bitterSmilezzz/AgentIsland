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

/// 形态：灵动岛（贴边细条）或侧边栏（一整列的窗口）。
///
/// 两者**并存**（ADR 0009）：切形态只是显示哪一个窗口，另一形态的窗口配置与行为一概不动——
/// 所以「默认启动仍是灵动岛」这条不需要额外保证，它是默认值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ShellMode {
    Island,
    Sidebar,
}

impl ShellMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ShellMode::Island => "island",
            ShellMode::Sidebar => "sidebar",
        }
    }

    /// 解析。**认得的值以外一律回落到灵动岛**：拼错的形态不该让谁开不出界面
    pub fn parse(raw: &str) -> ShellMode {
        match raw.trim().to_lowercase().as_str() {
            "sidebar" => ShellMode::Sidebar,
            _ => ShellMode::Island,
        }
    }
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
    /// [`SessionSchema::StatusIndex`] 那一族的查询语句（`None` = 该表名与状态列不由档案给出）。
    ///
    /// **必须由档案声明，不能在代码里写死**：同一张 `statusIndex` 表形下，
    /// 列名逐个产品不同（workbuddy 的 `sessions.id/status/deleted_at` 与
    /// zcode 的 `tasks.task_id/task_status/deleted` 毫无共同点），
    /// 而写死的那份在本机上恒定 prepare 失败——失败是静默的，
    /// 引擎把它当成「这个 Agent 没有终态」，于是信号永不响、界面看不出异样。
    ///
    /// 声明式还有一个好处：新增同形 fork 只改档案，不动代码（ADR 0004）。
    #[serde(default)]
    pub status_sql: Option<String>,
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
    /// MiniMax Code local runtime session and usage tables.
    MiniMaxRuntime,
}

/// 会话记录的存放**格式**。
///
/// 与 macOS 侧 `AgentSessionDialect` 同名同值。分派按**格式**而不是按 agent id：
/// 格式数量远少于 Agent 数量（Cline 与 Roo Code 同源、WorkBuddy 两版同 schema），
/// 新增一个复用既有格式的 Agent 时只改档案，不必再动解析器里的 id 梯子。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionDialect {
    /// 通用 JSONL/文本尾部（活动文件 + 有界尾读）
    #[default]
    GenericTail,
    /// Antigravity：`brain/<session>/…/transcript.jsonl` + 同名 tasks 目录
    AntigravityBrain,
    /// DSH：`session_projcache/sessions/<id>.json` 投影缓存
    DshProjection,
    /// Cline / Roo Code：`tasks/<taskId>/ui_messages.json`（**一个跨行 JSON 数组**）
    ClineTasks,
    /// Qoder：`~/.qoder/projects/<slug>/<uuid>.jsonl`（Anthropic 兼容逐行）
    QoderTranscript,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    /// 图标 glyph（前端用 Segoe/emoji 渲染）
    pub glyph: String,
    pub emoji: String,
    pub process_names: Vec<String>,
    /// 桌面应用的 bundle id（macOS/Linux 装在 `/Applications` 的那批）。
    ///
    /// **CLI 专用的档案这里是空数组**（`codex`、`aider`、`cline`…）——它们本来就不装应用包。
    /// 这一列是装机探测（[`crate::installed`]）的一半证据：只看 `process_names` 去 PATH 里找，
    /// GUI 类档案会永远显示「未安装」，而那不是事实，只是没证据。
    #[serde(default)]
    pub bundle_ids: Vec<String>,
    /// 命令行提示：进程名不在名单（如 npm 安装的 CLI 跑在 node.exe 里）时，
    /// 匹配入口文件名（允许脚本扩展名）或完整入口路径后缀；内联正文、预载及业务参数不参与。
    #[serde(default)]
    pub cmdline_hints: Vec<String>,
    /// 路径必须包含其中之一（不区分大小写），**否则不匹配**。
    ///
    /// 存在的理由很具体：`workbuddy` 与 `workbuddy-ai` 的进程 basename **都是 `Electron`**，
    /// 只靠进程名两个档案会互相命中、造成双份计数；Qoder 若用裸子串 `qoder`，
    /// 又会把 `~/code/qoder-playground` 里跑的任何 Electron 程序认成它。
    /// Swift 侧同字段（`pathContains`）的注记写得更细，此处逐条对齐。
    #[serde(default)]
    pub path_contains: Vec<String>,
    pub path_excludes: Vec<String>,
    /// 桌面类 Agent 的 CPU 工作判定下限（Electron 空闲抖动）
    pub cpu_floor: Option<f64>,
    pub session_dirs: Vec<String>,
    pub token_roots: Vec<String>,
    /// 该档案的 token 暴涨告警**下限**；实际阈值 = `max(本值, 全局设置)`。
    ///
    /// 为多专家团架构而设（WorkBuddy 日常 3-5 专家并行的高消耗不该误报），
    /// 而超大规模死循环、或用户自己设了更高档位时依然能熔断。
    /// `None` = 不设下限，走全局阈值（绝大多数档案如此）。
    #[serde(default)]
    pub token_alert_floor: Option<i64>,
    /// 该档案的会话语义**格式**（见 [`SessionDialect`]）。默认 `genericTail`。
    #[serde(default)]
    pub session_dialect: SessionDialect,
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

/// 一条**尚未交付**的后台任务（构建 / 测试 / 长耗时命令跑在后台时）。
///
/// 只列「还没交付」的：任务结束就该消失，留着会让用户以为机器上还挂着活。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundTask {
    pub id: String,
    /// 给界面看的那句（已归一化，不含环境变量前缀与超长参数）
    pub action: String,
}

/// 一个正在跑的子智能体。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentInfo {
    pub conversation_id: String,
    pub role: String,
    pub model: Option<String>,
    pub state: Option<String>,
}

/// Token 细分。**与总量口径不同**：这里的每一项都是「模型报了什么」，
/// 不做净消耗折算——`tokensTotal` 那边才是给用户看消耗的那个数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenBreakdown {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub total_tokens: i64,
}

impl TokenBreakdown {
    /// Swift 侧 `totalTokens > 0 ? totalTokens : 五项相加`。
    ///
    /// **取最大而非相加**：五项是**分类**（prompt 里含 cache read、
    /// completion 里含 reasoning），相加会重复计数。源没给 total 时才相加。
    pub fn new(
        prompt: i64,
        completion: i64,
        cache_read: i64,
        cache_write: i64,
        reasoning: i64,
        total: i64,
    ) -> Self {
        Self {
            prompt_tokens: prompt,
            completion_tokens: completion,
            cache_read_tokens: cache_read,
            cache_write_tokens: cache_write,
            reasoning_tokens: reasoning,
            total_tokens: if total > 0 {
                total
            } else {
                prompt + completion + cache_read + cache_write + reasoning
            },
        }
    }
}

/// Internal identity and event revision selected by a database semantic query.
#[derive(Debug, Clone, PartialEq)]
pub struct DatabaseSource {
    pub keys: Vec<String>,
    pub revision: Option<String>,
    /// Event time from the selected row; WAL writes need not update the DB file mtime.
    pub activity_ms: Option<i64>,
}

/// 一轮探测出来的「本轮上下文」：后台任务 + 子智能体 + Token 细分。
///
/// ⚠️ **只能随一次探测的返回值活过**，不许存进按 agent id 索引的全局表。
/// 那样会让 Claude、Codex 的卡片串到 Antigravity 的残留上下文，
/// 而 Agent 退出后那些残留永不失效（Swift 侧记过这个坑，代码里也留着注记）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionActiveContext {
    /// Exact source selected by the semantic probe, not the newest unrelated file.
    #[serde(skip)]
    pub source_path: Option<String>,
    /// Database primary key(s) from the exact semantic row; never sent to UI.
    #[serde(skip)]
    pub source_keys: Option<DatabaseSource>,
    #[serde(skip)]
    pub artifact: Option<crate::task_artifacts::Evidence>,
    /// Immutable main-event version, independent of temporary plan body availability.
    #[serde(skip)]
    pub plan_event_version: Option<String>,
    #[serde(skip)]
    pub plan_identity_unavailable: bool,
    #[serde(skip)]
    pub attention_kind: Option<crate::tasks::AttentionKind>,
    pub background_tasks: Vec<BackgroundTask>,
    pub subagents: Vec<SubagentInfo>,
    /// `None` = 这一族不报 Token 细分（与「报 0」不同）
    pub token_breakdown: Option<TokenBreakdown>,
}

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
    /// 100 分制运行健康度；内存异常来自连续观测趋势，不由绝对 RSS 推断。
    pub health: crate::health::Report,
    pub process_running: bool,
    /// 装机探测的结论。`None` = 没核实（缓存未热、GUI 档案缺否定证据），
    /// **不是「没装」**——见 [`crate::installed::InstalledApps::is_installed`]。
    /// 「离线」是关于进程的结论，「未安装」才是关于装没装的结论，两者不能互相顶替。
    pub installed: Option<bool>,
    pub cpu_percent: Option<f64>,
    /// 任务效能统计（过去 24 小时；[`crate::duration::TaskDurationTracker`]）。
    /// 排版文本也在里面，界面不必再拼一遍
    pub work_stats: crate::duration::Stats,
    /// 这句话凭什么：自报 / 观测 / 推断 / 冲突（`None` = 离线，那时「它说了什么」不成立）
    pub provenance: Option<crate::selfreport::Provenance>,
    /// 副标题那一小截（` · 自报` / ` · 自报冲突` / 空串），**由 Rust 拼好**
    pub provenance_suffix: String,
    pub memory_bytes: u64,
    pub memory_text: String,
    pub last_activity_text: String,
    pub token_usage: Option<TokenUsage>,
    pub pid: Option<u32>,
    pub current_action: Option<String>,
    pub subagent_count: usize,
    /// 「会话源读不到」的那一行原文（`SessionProbeHealth.diagnostic_text`），
    /// `None` = 本轮探测本身没问题。
    ///
    /// 排版与措辞**只在探测层有一份**（与 Swift 同纪律）：卡片、侧边栏、报表
    /// 各自拼一遍就会出现「岛里说读不到、doctor 说一切正常」那种分叉。
    /// 陈旧的故障由引擎按保质期过滤后才落到这里（见 `observability` 那条）。
    pub session_probe_health: Option<String>,
    /// 本轮探测出的后台任务（`⚡ 后台N` 胶囊）。未报这一族的档案这里是空数组。
    pub background_tasks: Vec<BackgroundTask>,
    /// 本轮探测出的子智能体。`subagent_count` 必须等于它的长度——两者不许各写各的。
    pub subagents: Vec<SubagentInfo>,
    /// Token 细分。`None` = 这一族不报——**不是「报 0」**。
    pub token_breakdown: Option<TokenBreakdown>,
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
    /// 本次任务用时（秒）。**0 = 不适用**（等待确认、熔断告警、外部事件都填 0）。
    /// Swift `AgentTaskEvent.duration` 同口径；审计报告的「耗时」列靠它，
    /// 缺了它那一列只能印 `—`——而「没记」与「零秒」在报告里是两件事。
    pub duration: f64,
    pub externally_delivered: bool,
}

impl AgentTaskEvent {
    /// 时长的展示文本：`3分12秒` / `45秒`（Swift `AgentTaskEvent.durationText` 同口径）。
    /// 注意它**与通知正文里的时长不同**：那边只说到「分」（`duration_short`），
    /// 报告里要精确到秒——两者是同一个人在不同场景要的不同粒度，不是重复实现。
    pub fn duration_text(seconds: f64) -> String {
        let total = seconds.max(0.0).round() as i64;
        if total >= 60 {
            format!("{}分{}秒", total / 60, total % 60)
        } else {
            format!("{total}秒")
        }
    }

    pub fn summary(&self) -> String {
        if let Some(m) = &self.message {
            if !m.is_empty() {
                return m.clone();
            }
        }
        match self.event_type.as_str() {
            "attention" => format!("{} 等待确认操作", self.agent_name),
            "costSpike" => format!("⚠️ {} 资源/Token 消耗突增", self.agent_name),
            // NOT-01 的细分类型各有说法。**必须与 app/ui/js/views.js 的 eventSummary 同口径**
            // （那边注释也这么写）；漏一类就会让导出报告把它说成「任务完成」。
            "question" => format!("{} 提问待回答", self.agent_name),
            "failure" => format!("✗ {} 任务失败", self.agent_name),
            "review" => format!("{} 等待审查", self.agent_name),
            "schedule" => format!("{} 调度提醒", self.agent_name),
            "chief" => format!("{} Chief 消息", self.agent_name),
            // 完成摘要带时长：Swift `summaryText` 就是这么写的，
            // 少它一段会让报告里同一件事比岛内文案少一半信息
            _ => format!(
                "{} 任务完成 ({})",
                self.agent_name,
                Self::duration_text(self.duration)
            ),
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
    pub context24h: crate::usage_context::Report,
    pub usage: TokenUsage,
    pub models24h: Vec<ModelUsage>,
    pub models_total: Vec<ModelUsage>,
    /// (unix ms 桶起点, tokens)
    pub hourly30d: Vec<(i64, i64)>,
}

// MARK: - 导出件的统一形状

/// 一次导出的结果：正文 + 建议文件名。
///
/// 两个导出器（审计报告、Token 报表）共用它，而且**必须**共用：
/// 文件名与正文要来自同一拍，否则用户存下来的文件名与正文里的生成时间会差几秒。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Export {
    pub filename: String,
    pub content: String,
}

// MARK: - 引擎推送给 UI 的完整状态

#[derive(Debug, Clone, Serialize)]
pub struct EngineState {
    pub session_navigation: std::collections::HashMap<String, crate::session_navigation::Target>,
    /// 按 agent 分域的事件导航目标：`[agent_id][event_id]`。
    /// 不分域时两个 agent 的同名事件会互相覆盖（BUG-008）。
    pub event_navigation:
        std::collections::HashMap<String, std::collections::HashMap<String, crate::session_navigation::Target>>,
    pub snapshots: Vec<AgentSnapshot>,
    pub latest_event: Option<AgentTaskEvent>,
    /// 已发出但还没确认的事件条数（不含 `latest_event` 那条）。
    /// 界面上是「还有几条」的角标——不暴露它，队列就成了用户看不见的暗箱。
    pub pending_events: usize,
    /// 最近若干次外发的结果（新的在前，有界）。**失败必须看得见**：
    /// 外发这件事的默认期待是「人不在也能收到」，收不到还显示正常最伤人。
    pub recent_outbound: Vec<crate::notifier::Recent>,
    pub grand_total: TokenUsage,
    pub dock_edge: DockEdge,
    /// 当前形态（`island` / `sidebar`）：两个窗口都读同一份，于是界面不必猜自己在哪
    pub shell_mode: String,
    pub sidebar_edge: String,
    pub sidebar_width: f64,
    pub appearance: String, // system | light | dark
    pub any_working: bool,
    pub has_attention: bool,
    pub demo: bool,
    /// 结构化 hook 进入服务 → 入队的延迟摘要（需求「即时通知指标」）。
    ///
    /// 放进 `EngineState` 是**必需**的，不是顺手：`NotifyLatency` 会积累样本，
    /// 但若没有读取出口，「须实测后锁定」就只能靠自检里的合成样本——
    /// 那不能代表常驻进程的真实表现。
    pub notify_latency: crate::notify_latency::LatencySummary,
    /// Lifetime enqueue replays suppressed by the bounded notification ledger.
    pub notify_duplicates: u64,
    /// Explicit ids reused with a different payload. The first payload wins and is
    /// the only one shown; this counter exists so the silent reuse is visible.
    pub notify_conflicts: u64,
    /// Lifetime enqueued events dropped from the retained queue by the 64-entry
    /// bound. Those events are neither shown nor restored, so without this counter
    /// a burst larger than the queue would lose entries with nothing to read.
    pub notify_evicted: u64,
    /// Unacknowledged notifications the user has not dismissed yet (`latest_event`
    /// plus the queue behind it). The UI must not derive this from `pending_events`:
    /// that field excludes the visible head, so it under-reports by one.
    pub notify_unacked: usize,
    /// Notifications restored from the ledger during this process's startup.
    /// Zero for a fresh process and for CLI/one-shot readers.
    pub notify_restored: usize,
    pub notify_persistence: crate::notify_ledger::PersistenceStatus,
}

// MARK: - 测试（M1：Rust 测试基建，见 docs/adr/0010-swift-freeze-and-rust-prerequisites.md）

#[cfg(test)]
mod shell_mode_tests {
    use super::ShellMode;

    #[test]
    fn only_the_known_modes_are_recognised_and_anything_else_falls_back_to_island() {
        assert_eq!(ShellMode::parse("island"), ShellMode::Island);
        assert_eq!(ShellMode::parse("sidebar"), ShellMode::Sidebar);
        assert_eq!(ShellMode::parse(" Sidebar "), ShellMode::Sidebar);
        assert_eq!(ShellMode::parse("SIDEBAR"), ShellMode::Sidebar);
        // 拼错/空/垃圾值都回落：形态拼错不该让谁开不出界面
        for raw in ["", "  ", "sider", "island-2", "null", "{}"] {
            assert_eq!(
                ShellMode::parse(raw),
                ShellMode::Island,
                "{raw:?} 应回落到灵动岛"
            );
        }
    }

    #[test]
    fn the_round_trip_between_string_and_mode_is_stable() {
        for mode in [ShellMode::Island, ShellMode::Sidebar] {
            assert_eq!(ShellMode::parse(mode.as_str()), mode);
        }
        assert_eq!(ShellMode::Island.as_str(), "island");
        assert_eq!(ShellMode::Sidebar.as_str(), "sidebar");
    }
}

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

    /// **`EngineState` 的通知字段名是 API 契约，且必须是 snake_case。**
    ///
    /// 前端 `app/ui/js/views.js` 按这些键名读取通知页的账本诊断；`EngineState` 一旦被
    /// 加上 `rename_all = "camelCase"`（相邻的 `Export` 就是那种写法，很容易被顺手照搬），
    /// 界面会**全部显示「未知」**——而其余所有测试照样通过，因为 Rust 侧读的是结构体字段，
    /// 与序列化名无关。所以这条断言必须直接对着 JSON 键名，不能对着字段。
    ///
    /// 键名清单来自 `tests/fixtures/contracts/notify-keys.json`——**界面回归读的是同一份文件**
    /// （`scripts/test-notify-latency-ui.mjs`）。两侧各自写一份字面量的话，可以被「一致地」
    /// 改成一个新名字而 views.js 没跟上：那时 Rust 测试与界面测试都绿，真机上却全部「未知」。
    #[test]
    fn engine_state_notify_keys_are_snake_case_contract() {
        let (_, rx) = std::sync::mpsc::channel();
        let engine = crate::engine::ActivityEngine::new(crate::settings::Settings::default(), rx);
        let json = serde_json::to_value(engine.state()).expect("EngineState 必须可序列化");
        let contract: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/contracts/notify-keys.json"
        ))
        .expect("契约夹具必须是合法 JSON");

        let engine_keys = contract["engine_state"].as_array().expect("engine_state 必须是数组");
        for key in engine_keys {
            let key = key.as_str().expect("键名必须是字符串");
            assert!(
                json.get(key).is_some(),
                "EngineState 必须以 {key} 暴露；界面按这个键名读取，缺了即静默显示「未知」"
            );
        }
        // 反向断言：出现 camelCase 变体就说明有人加了 rename_all，而那会让界面静默降级。
        for wrong in ["notifyUnacked", "notifyEvicted", "notifyDuplicates", "notifyPersistence"] {
            assert!(json.get(wrong).is_none(), "不得改名为 {wrong}");
        }
        // 持久化摘要的出口字段同样是契约：界面据此区分未启用/待保存/已保存/失败/停写。
        let status = json.get("notify_persistence").expect("持久化摘要必须在");
        for key in contract["notify_persistence"].as_array().expect("notify_persistence 必须是数组") {
            let key = key.as_str().expect("键名必须是字符串");
            assert!(status.get(key).is_some(), "持久化摘要缺 {key}，界面会误判写盘状态");
        }
        // 延迟摘要同理：漏一个字段，通知页那一行就会退化成「读不到入队统计」。
        let latency = json.get("notify_latency").expect("延迟摘要必须在");
        for key in contract["notify_latency"].as_array().expect("notify_latency 必须是数组") {
            let key = key.as_str().expect("键名必须是字符串");
            assert!(latency.get(key).is_some(), "延迟摘要缺 {key}，界面会读不到统计");
        }
        // 事件类型清单也是契约：Rust 的 `EventKind::ALL`、外发开关键、界面文案
        // 三者必须对着**同一份**清单。任一侧漏一个新类型，就会出现
        // 「HTTP 收下了这条事件，界面却按兜底文案显示成任务完成」。
        let kinds = contract["event_kinds"].as_array().expect("event_kinds 必须是数组");
        let contract_kinds: Vec<&str> = kinds.iter().map(|k| k.as_str().unwrap()).collect();
        let rust_kinds: Vec<&str> = crate::remote::EventKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(
            contract_kinds, rust_kinds,
            "夹具的事件类型清单与 EventKind::ALL 不一致"
        );
        let switches = contract["event_kind_switches"]
            .as_object()
            .expect("event_kind_switches 必须是对象");
        assert_eq!(switches.len(), rust_kinds.len());
        // 交付状态三段（NOT-05：受理 ≠ 收到 ≠ 已读）也是契约：
        // 界面那份文案与 Rust 的 serde 名必须对着同一份清单。
        let delivery = contract["delivery_states"]
            .as_array()
            .expect("delivery_states 必须是数组");
        let mut from_contract: Vec<&str> = delivery.iter().map(|s| s.as_str().unwrap()).collect();
        let mut from_rust: Vec<String> = [
            crate::notifier::DeliveryState::Accepted,
            crate::notifier::DeliveryState::Received,
            crate::notifier::DeliveryState::Read,
            crate::notifier::DeliveryState::Unknown,
        ]
        .iter()
        .map(|state| {
            serde_json::to_value(state)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
        from_contract.sort_unstable();
        from_rust.sort();
        assert_eq!(
            from_contract,
            from_rust.iter().map(String::as_str).collect::<Vec<_>>(),
            "夹具的交付状态清单与 DeliveryState 的 serde 名不一致"
        );
        // **四态文案也共读夹具**（NOT-05 复审 N2 的正解）。
        // 此前这里与浏览器测试各自硬编码文案字面量：两边可以「一致地」改成新文案，
        // 而真机上界面显示的是**测试没验证过的**那份（复审实测：改 Rust 的 label()
        // 文本后 cargo 全量只挂同步过的这一行，浏览器测试照旧 PASS）。
        // 现在文案只在夹具里写一次，Rust 的 `label()` 必须与它逐字一致。
        let labels = contract["delivery_labels"]
            .as_object()
            .expect("delivery_labels 必须是对象");
        for state in [
            crate::notifier::DeliveryState::Accepted,
            crate::notifier::DeliveryState::Received,
            crate::notifier::DeliveryState::Read,
            crate::notifier::DeliveryState::Unknown,
        ] {
            let serde_name = serde_json::to_value(state).unwrap();
            let serde_name = serde_name.as_str().unwrap();
            let expected = labels
                .get(serde_name)
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!("夹具缺 {serde_name} 的文案"));
            assert_eq!(
                state.label(),
                expected,
                "{serde_name} 的界面文案与契约夹具不一致——界面会显示成另一句话"
            );
        }
        // 受理的文案不得说成「已送达」——那是替对端做它没做过的承诺
        assert_ne!(crate::notifier::DeliveryState::Accepted.label(), "已送达");

        // **`Recent` 的对外字段名同样是契约**（NOT-05 复审 N3）。
        // 界面按 `deliveryLabel` 读；若只改 Rust 的 serde 名（`delivery_label` →
        // 别的名字）而 JS 不动，`escapeHtml(undefined)` 会静默变成空串，
        // 交付文案与故障条整段消失，而 cargo 全量 + 浏览器测试**都绿**（复审实测）。
        let recent = crate::notifier::Recent {
            at_ms: 0,
            title: String::new(),
            text: String::new(),
            accepted: true,
            delivery: crate::notifier::DeliveryState::Accepted,
            delivery_label: crate::notifier::DeliveryState::Accepted.label(),
        };
        let recent_json = serde_json::to_value(&recent).expect("Recent 必须可序列化");
        for key in contract["recent_fields"].as_array().expect("recent_fields 必须是数组") {
            let key = key.as_str().expect("字段名必须是字符串");
            assert!(
                recent_json.get(key).is_some(),
                "Recent 必须以 {key} 暴露；界面按这个键名读取，缺了即静默显示空白"
            );
        }
        for kind in &rust_kinds {
            let key = switches
                .get(*kind)
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!("{kind} 没有对应的外发开关键——界面只能靠猜"));
            // 夹具里的开关名必须**真的能被 Policy 读回**（否则它是与实现脱节的死数据，
            // 界面渲染了开关却写不进策略）。关掉它应当只影响这一类。
            let raw = serde_json::json!({ key: false });
            let policy: crate::remote::Policy =
                serde_json::from_value(raw).unwrap_or_else(|e| panic!("{key} 读不回来：{e}"));
            let parsed = crate::remote::EventKind::parse(kind).expect("清单里的类型必须可解析");
            assert!(
                !policy.allows(parsed),
                "{key} 没有关掉 {kind}——夹具记录的开关名与实现不一致"
            );
            for other in crate::remote::EventKind::ALL {
                if other != parsed {
                    assert!(
                        policy.allows(other),
                        "{key} 误关了 {}",
                        other.as_str()
                    );
                }
            }
        }
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
                lv.label()
                    .chars()
                    .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
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
        for e in [
            DockEdge::Top,
            DockEdge::Bottom,
            DockEdge::Left,
            DockEdge::Right,
        ] {
            assert!(
                e.is_horizontal() || !e.is_horizontal(),
                "{e:?} 的值不是布尔"
            );
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
            installed: None,
            work_stats: crate::duration::Stats::empty(),
            provenance: None,
            provenance_suffix: String::new(),
            cpu_percent: Some(1.0),
            memory_bytes: 0,
            memory_text: "—".into(),
            last_activity_text: "—".into(),
            token_usage: Some(TokenUsage::default()),
            pid: None,
            current_action: None,
            subagent_count: 0,
            session_probe_health: None,
            background_tasks: vec![],
            subagents: vec![],
            token_breakdown: None,
        };
        let value = serde_json::to_value(&snapshot).expect("快照应能序列化");
        let object = value.as_object().expect("快照应序列化成对象");

        let source =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/js/views.js"))
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
        assert!(
            names.len() >= 8,
            "只抓到 {} 个 snap. 字段，解析八成坏了",
            names.len()
        );
        let missing: Vec<&String> = names.iter().filter(|n| !object.contains_key(*n)).collect();
        assert!(
            missing.is_empty(),
            "views.js 读了快照里没有的字段：{missing:?}（静默失效，不会报错）"
        );
    }

    /// 侧边栏用到的 `sb-*` 类必须在 `sidebar.css` 里有定义。
    ///
    /// 这类失效特别安静：类名打错一个字母，界面**不报错**、只是长得不对——
    /// 而在这个环境里没法靠截图发现（屏幕录制权限拿不到）。所以退一步做静态核对：
    /// 从 `renderSidebar` 的 `class="..."` 里取 `sb-` 开头的类，逐个去 CSS 里找。
    #[test]
    fn every_sidebar_class_is_actually_styled() {
        let views =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/js/views.js"))
                .expect("读不到 views.js");
        let css = ["sidebar.css", "panels.css"]
            .map(|name| {
                std::fs::read_to_string(format!("{}/../ui/css/{name}", env!("CARGO_MANIFEST_DIR")))
                    .expect("读不到控件样式")
            })
            .join("\n");

        // 扫**整个** views.js 里 `sb-` 开头的静态类名。
        // 一开始只扫 `renderSidebar` 的函数体，于是「档位页」这种后加的页面不在保护范围内——
        // 哨兵的保护面必须跟着页面走，否则新页面天然是没人管的那一半。
        let mut classes: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut cursor = 0;
        while let Some(index) = views[cursor..].find("class=\"") {
            let from = cursor + index + "class=\"".len();
            let Some(end) = views[from..].find('"') else {
                break;
            };
            for token in views[from..from + end].split_whitespace() {
                // 只查静态类名；模板插值出来的（`${...}`）没法静态核对
                if token.starts_with("sb-") && !token.contains("${") {
                    classes.insert(token.to_string());
                }
            }
            cursor = from + end;
        }
        assert!(
            classes.len() >= 5,
            "只从 renderSidebar 里抓到 {} 个 sb-* 类，解析八成坏了",
            classes.len()
        );
        let missing: Vec<&String> = classes
            .iter()
            .filter(|c| !css.contains(&format!(".{c}")))
            .collect();
        assert!(
            missing.is_empty(),
            "sidebar.css 里没有这些类的样式：{missing:?}（界面不会报错，只会长得不对）"
        );
    }

    /// 档位页读的字段必须真的在命令的 DTO 里。
    ///
    /// 与快照那条哨兵同一套路，但对象是 `provider.rs` 的四个 DTO：
    /// 界面写 `profile.base_url`、Rust 侧字段叫 `base_url`（serde 原样输出 snake_case），
    /// 任何一边改名都会让界面**静默显示空值**——不报错，只是什么都没有。
    #[test]
    fn every_provider_field_the_ui_reads_exists_in_the_dto() {
        // 只覆盖界面**真的在用**的四个前缀。`provider_scan_tools` 是给「将来接第二个工具」
        // 留的端点，界面上暂时只读 `provider_status`——没有消费方的字段就不该进哨兵，
        // 否则哨兵会逼着界面去读一个它不需要的 DTO。
        use crate::provider::{BackupInfo, CodexProfile, ProviderApplyResult, ProviderStatus};
        use crate::todos::TodoList;

        let views =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/js/views.js"))
                .expect("读不到 views.js");

        let keys = |value: &serde_json::Value| -> std::collections::BTreeSet<String> {
            value
                .as_object()
                .expect("DTO 应序列化成对象")
                .keys()
                .cloned()
                .collect()
        };
        let profile = keys(
            &serde_json::to_value(CodexProfile {
                id: "work".into(),
                name: "工作账号".into(),
                model: "gpt-5".into(),
                provider_id: "acme".into(),
                provider_name: "Acme".into(),
                base_url: "https://api.example.invalid/v1".into(),
                env_key: "ACME_API_KEY".into(),
                wire_api: "responses".into(),
            })
            .unwrap(),
        );
        let backup = keys(
            &serde_json::to_value(BackupInfo {
                name: "config-1.toml".into(),
                created_ms: 1,
                bytes: 2,
            })
            .unwrap(),
        );
        let status = keys(
            &serde_json::to_value(ProviderStatus {
                installed: true,
                config_path: None,
                active_provider_id: None,
                active_profile_id: None,
                profile_count: 0,
                configured_model: None,
                configured_provider: None,
                current_draft: None,
                capture_notice: String::new(),
                config_error: None,
                revision: None,
                last_applied: None,
                drifted: false,
                record_error: None,
                limitations: crate::provider::PROVIDER_LIMITATIONS,
            })
            .unwrap(),
        );
        let applied = keys(
            &serde_json::to_value(ProviderApplyResult {
                config_path: "/tmp/config.toml".into(),
                backup_name: "config-1.toml".into(),
                record_warning: None,
                limitations: crate::provider::PROVIDER_LIMITATIONS,
            })
            .unwrap(),
        );
        let todo = keys(
            &serde_json::to_value(TodoList {
                items: vec![crate::todos::Todo {
                    id: "1".into(),
                    text: "示例".into(),
                    done: false,
                    created_ms: 0,
                }],
                pending: 1,
                broken_backup: None,
            })
            .unwrap(),
        );

        // 远程通知页的 DTO：**与档位页的 `status` 是两个不同的结构**，
        // 所以它用自己的前缀 `remote.`。两个都登记进哨兵——而不是让其中一个
        // 逃出检查范围（那正是 DTO 改名时静默失效的那种洞）。
        let remote = keys(
            &serde_json::to_value(crate::remote::status(
                None,
                &std::collections::HashMap::new(),
                &crate::remote::Policy::default(),
                false,
                crate::remote::Now::at(0),
                &crate::remote::PresenceSignals::unavailable(),
            ))
            .unwrap(),
        );

        // 前缀 → 该前缀下允许的键
        for (prefix, allowed) in [
            ("profile.", &profile),
            ("status.", &status),
            ("remote.", &remote),
            ("applied.", &applied),
            ("backup.", &backup),
            ("todo.", &todo),
        ] {
            let mut cursor = 0;
            let mut seen = 0usize;
            while let Some(index) = views[cursor..].find(prefix) {
                let from = cursor + index + prefix.len();
                let name: String = views[from..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                cursor = from;
                if name.is_empty() {
                    continue;
                }
                seen += 1;
                assert!(
                    allowed.contains(&name),
                    "views.js 读了 {prefix}{name}，但 DTO 里没有这个键（界面会静默显示空值）。有的键：{allowed:?}"
                );
            }
            assert!(
                seen > 0,
                "views.js 里没有任何 `{prefix}` 用法——档位页被删了，或改了变量名（哨兵因此失效）"
            );
        }
    }

    /// **两种形态的样式不许互相串味**——三条结构性事实，各自都做过反证。
    ///
    /// 为什么值得守：串味**不报错**，只是长得不对，而这个环境里看不到像素
    /// （本机截图需要屏幕录制授权）。所以把「不串味」这件事拆成能静态核对的三条：
    ///
    /// 1. `sidebar.css` 的每条规则都挂在 `.shell-sidebar` 下 —— 侧边栏的样式不会漏到灵动岛；
    /// 2. `island.css` 里针对 `html` / `body` / `#root` 的规则都挂在 `html.shell-island` 下
    ///    —— 灵动岛的布局不会漏到侧边栏窗口（两个窗口加载同一个 index.html！）；
    /// 3. 形态类在 `index.html` 的**样式表之前**由内联脚本挂上 —— 否则收拢的那批规则
    ///    会在第一次绘制时还没生效（透明窗口先闪一下无样式内容）。
    #[test]
    fn the_two_shells_styles_cannot_bleed_into_each_other() {
        let read = |name: &str| {
            std::fs::read_to_string(format!("{}/../ui/{}", env!("CARGO_MANIFEST_DIR"), name))
                .unwrap_or_else(|e| panic!("读不到 {name}：{e}"))
        };
        // 先把块注释整段去掉：注释里会出现 `*`、`{`、反引号这些字符，
        // 不去掉的话「按 { 切选择器」会把注释当规则（我第一版就是这么被绊倒的）。
        fn strip_css_comments(css: &str) -> String {
            let mut out = String::with_capacity(css.len());
            let mut rest = css;
            while let Some(start) = rest.find("/*") {
                out.push_str(&rest[..start]);
                match rest[start..].find("*/") {
                    Some(end) => rest = &rest[start + end + 2..],
                    None => return out,
                }
            }
            out.push_str(rest);
            out
        }
        let island_css = strip_css_comments(&read("css/island.css"));
        let sidebar_css = strip_css_comments(&read("css/sidebar.css"));
        let html = read("index.html");

        // ① sidebar.css：每条规则的每个选择器都要以 `.shell-sidebar` 开头
        let mut sidebar_rules = 0;
        for block in sidebar_css.split('}') {
            let Some((selectors, _)) = block.split_once('{') else {
                continue;
            };
            // 跳过注释行与 @media 之类的包裹
            let selectors = selectors
                .lines()
                .filter(|line| !line.trim().is_empty())
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string();
            if selectors.is_empty() || selectors.starts_with('@') {
                continue;
            }
            for selector in selectors.split(',') {
                let selector = selector.trim();
                if selector.is_empty() {
                    continue;
                }
                sidebar_rules += 1;
                assert!(
                    selector.starts_with(".shell-sidebar"),
                    "sidebar.css 里有一条没收敛的规则会漏到灵动岛：{selector:?}"
                );
            }
        }
        assert!(
            sidebar_rules >= 15,
            "只解析出 {sidebar_rules} 条 sidebar 规则，解析八成坏了"
        );

        // ② island.css：元素级选择器必须挂在 html.shell-island 之下
        let mut checked = 0;
        for line in island_css.lines() {
            let trimmed = line.trim();
            if !trimmed.contains('{') {
                continue;
            }
            let selector_list = trimmed.split('{').next().unwrap_or("").trim();
            for selector in selector_list.split(',') {
                let selector = selector.trim();
                // `*` 的全清零是**两个形态都要**的，故意不收敛
                if selector.is_empty() || selector.starts_with('*') {
                    continue;
                }
                // 「元素级」= 选择器的第一段就是 html / body / #root（带不带类都算）
                let element_level = selector.starts_with("html")
                    || selector.starts_with("body")
                    || selector.starts_with("#root");
                if !element_level {
                    continue;
                }
                checked += 1;
                assert!(
                    selector.starts_with("html.shell-island"),
                    "island.css 里这条会漏到侧边栏窗口：{selector:?}"
                );
            }
        }
        assert!(
            checked >= 6,
            "只检查了 {checked} 条元素级规则，解析八成坏了"
        );

        // ③ 形态类必须在样式表之前挂上（否则第一次绘制时收拢的规则还没生效）
        let script_at = html
            .find("document.documentElement.className")
            .expect("index.html 里应当有挂形态类的内联脚本");
        let first_link_at = html
            .find("<link rel=\"stylesheet\"")
            .expect("index.html 里应当有样式表");
        assert!(
            script_at < first_link_at,
            "挂形态类的脚本排在样式表之后：收拢的规则会在第一次绘制时缺席"
        );
    }

    /// 两种形态必须**共用同一份显示模型**（`agentRowModel`）。
    ///
    /// 字段层面的分叉有上面那条哨兵拦（读了不存在的字段会红），但**文案与颜色的分叉没人拦**：
    /// 灵动岛写「会话源读不到」、侧边栏写「无数据」，两边都编译通过、都跑得起来，
    /// 只有用户会觉得同一件事有两个说法。所以这里对 `views.js` 做一次结构检查——
    /// 与那条字段哨兵同一套路（读源文件，而不是跑界面）。
    #[test]
    fn both_shells_render_agent_rows_through_one_shared_model() {
        let source =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/js/views.js"))
                .expect("读不到 views.js")
                .replace("\r\n", "\n");

        // 定义只有一处
        assert_eq!(
            source.matches("export function agentRowModel(").count(),
            1,
            "显示模型必须只有一处定义"
        );
        // 「判不出」的文案表也只有一处
        assert_eq!(
            source.matches("const UNCERTAIN_LABELS").count(),
            1,
            "状态文案表必须只有一处"
        );

        // 两个渲染路径都要过这个模型。
        // 切「函数体」用**函数边界**（到下一个顶格 `}`），不要用「往后 N 个字符」——
        // 我第一版用 4000 字符，往函数里加几行注释就把它推过了窗口，于是哨兵误报。
        // 文本哨兵本来就脆，至少别让它的作用域随注释长度漂移。
        for (func, label) in [
            ("function rowHtml(", "灵动岛的行"),
            ("export function renderSidebar(", "侧边栏的行"),
        ] {
            let start = source
                .find(func)
                .unwrap_or_else(|| panic!("views.js 里找不到 {func}——{label} 改名字了？"));
            let rest = &source[start..];
            let end = rest
                .find("\n}\n")
                .unwrap_or_else(|| panic!("{label} 的函数体没找到收尾，切片方式要更新"));
            let body = &rest[..end];
            assert!(
                body.contains("agentRowModel("),
                "{label}没有走共用的显示模型：文案与颜色会各写一份"
            );
        }
    }
}
