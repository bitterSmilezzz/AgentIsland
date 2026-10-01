use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// 持久化设置（macOS 端 UserDefaults 的跨平台对应物：
/// Windows %APPDATA%\AgentIsland\settings.json，macOS ~/Library/Application Support/…）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: String,     // system | light | dark
    /// 形态：`island` | `sidebar`（ADR 0009：两者并存，默认仍是灵动岛）
    pub shell_mode: String,
    /// 侧边栏贴在左边还是右边（只有这两档）
    pub sidebar_edge: String,
    /// 侧边栏宽度（**记忆**：用户拉过一次，下次开还在那儿）
    pub sidebar_width: f64,
    pub dock_edge: String,      // top | bottom | left | right
    pub dock_anchor: f64,       // 0..1
    pub collapse_delay: f64,    // 秒
    pub sample_interval: f64,   // 秒
    /// 全闲置时的降频间隔（Swift `idleSampleInterval`，默认 5.0）。
    /// 此前 Rust 没有这个字段，用 `sample_interval × 2.5` 顶替——那是**另一个公式**，
    /// 于是两侧的耗电量与「岛多久变灰」对不上。区间与 Swift 同用 `sampleIntervalRange`。
    pub idle_sample_interval: f64,
    /// 「有文件写入即工作」的窗口（Swift `workingWindow`，默认 60s）。此前硬编码。
    pub working_window: f64,
    /// 滞回：working 信号消失后保持的最短时长（Swift `minWorkingHold`，默认 10s）。此前硬编码。
    pub min_working_hold: f64,
    /// 活跃会话计数窗口（Swift `activeSessionWindow`，默认 600s）。
    pub active_session_window: f64,
    /// 持续高负荷（死循环）告警开关（Swift `runawayCpuAlert`，默认开）。
    /// 此前 Rust **没有这个开关**——CPU 熔断无法关闭。
    pub runaway_cpu_alert: bool,
    /// 持续高负荷的 CPU 阈值（Swift `runawayCpuThreshold`，默认 70%）。此前硬编码。
    pub runaway_cpu_threshold: f64,
    /// 持续高负荷需持续多久（Swift `runawayDurationThreshold`，默认 300s）。此前硬编码。
    pub runaway_duration_threshold: f64,
    pub cpu_threshold: f64,     // %
    /// 电池供电时降频（Swift `batterySaverEnabled`，默认开）。此前 Rust 无此字段。
    pub battery_saver_enabled: bool,
    pub token_alert_enabled: bool,
    /// 异常驻留 / 死锁持续守护的开关（Swift 侧 `autoAnomaliesAlertEnabled`，默认开）。
    /// 关掉它只关告警，**不影响** `is_hung` 与健康度判定——采集与告警解耦。
    pub auto_anomalies_alert: bool,
    /// 外发（远程通知）配置。Swift 侧散在四个 UserDefaults 键里
    /// （`remote.notify.policy.v1` / `kind.v1` / `channel.<kind>.v1`）；
    /// Rust 按 ADR 0011 收在一个 settings.json 里，**非密钥字段**照旧。
    /// 密钥永远不在这里：只存钥匙串条目名的推导规则（`remote.<kind>`）。
    pub remote_kind: String,
    pub remote_policy: crate::remote::Policy,
    /// 每个通道各一份配置，切换通道时互不覆盖——否则用户试完邮箱再试 ntfy，
    /// 回来发现 SMTP 全空了。键是通道名（`ntfy` / `customHTTP` / `smtpEmail`）。
    pub remote_channels: HashMap<String, crate::remote::ChannelConfig>,
    pub token_alert_threshold: i64,
    /// 日预算（token）。**0 = 未设**（与 Swift `dailyTokenBudget` 同键同默认同区间）。
    /// 口径是滚动 24 小时，不是自然日——见 `budget.rs` 的模块头。
    pub daily_token_budget: i64,
    /// 预算告警开关（Swift `budgetAlertEnabled`，默认开）。
    /// 关掉它只关告警，不影响用量统计与卡片。
    pub budget_alert_enabled: bool,
    pub notification_policy: String, // standard | focus | silent
    pub play_completion_sound: bool,
    pub disabled_agents: Vec<String>,
    // MARK: 界面侧开关（Swift `SettingsStore` 的散点，Rust 侧此前整块缺失）

    /// 开机自启（Swift `launchAtLogin`，默认关）
    pub launch_at_login: bool,
    /// 收起时隐藏 6pt 微细条（Swift `hideDockedSliver`，默认关 = 显示）
    pub hide_docked_sliver: bool,
    /// 紧凑视图（Swift `compactView`，默认关）
    pub compact_view: bool,
    /// 全局热键（Swift `globalHotKeyEnabled`，默认开）
    pub global_hot_key_enabled: bool,
    /// 菜单栏徽标模式（Swift `menuBarBadgeMode`）：`iconOnly` | `activeCount` | `tokenUsage`
    pub menu_bar_badge_mode: String,
    /// 屏幕跟随模式（Swift `screenFollowMode`）：`followMouse` | `mainScreen` | `builtInScreen` | `externalScreen`
    pub screen_follow_mode: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            appearance: "system".into(),
            shell_mode: "island".into(),
            sidebar_edge: "right".into(),
            sidebar_width: crate::placement::DEFAULT_SIDEBAR_WIDTH,
            dock_edge: "top".into(),
            dock_anchor: 0.5,
            collapse_delay: 0.5,
            sample_interval: 2.0,
            idle_sample_interval: 5.0,
            working_window: 60.0,
            min_working_hold: 10.0,
            active_session_window: 600.0,
            runaway_cpu_alert: true,
            runaway_cpu_threshold: crate::health::RUNAWAY_CPU_THRESHOLD,
            runaway_duration_threshold: crate::health::RUNAWAY_DURATION_MS as f64 / 1000.0,
            cpu_threshold: 6.0,
            battery_saver_enabled: true,
            token_alert_enabled: true,
            auto_anomalies_alert: true,
            token_alert_threshold: 200_000,
            daily_token_budget: 0,
            budget_alert_enabled: true,
            remote_kind: "ntfy".into(),
            remote_policy: crate::remote::Policy::default(),
            remote_channels: HashMap::new(),
            notification_policy: "standard".into(),
            play_completion_sound: true,
            // 出厂就关掉的档案。与 macOS 侧 `defaultEnabled: false` 同值。
            //
            // **语义与 macOS 端相反**（那边存启用名单、空集=全关；这里存禁用名单、
            // 空集=全开），所以「默认关」在这边必须**显式列进黑名单**，
            // 否则它会默认开着——而用户在 macOS 上从没见它开着。
            // 迁移设置时别照抄：两边的空集意思相反。
            disabled_agents: vec!["continue".into()],
            launch_at_login: false,
            hide_docked_sliver: false,
            compact_view: false,
            global_hot_key_enabled: true,
            menu_bar_badge_mode: "iconOnly".into(),
            screen_follow_mode: "followMouse".into(),
        }
    }
}

pub(crate) fn config_dir() -> PathBuf {
    let mut dir = dirs::data_dir().unwrap_or_else(std::env::temp_dir);
    dir.push("AgentIsland");
    dir
}

/// 把一份**解析不回来**的 `settings.json` 改名留档，返回留档文件名。
///
/// 与 `todos.rs::stash_broken` 同一处置。两个细节是那边没有的：
///
/// - **不覆盖已有留档。** `fs::rename` 在 POSIX 上会**静默覆盖**目标文件，
///   所以「先改名再判存在」会把上一次留档冲掉——那等于用丢一份换丢一份。
///   这里先查存在、撞名就加序号。
/// - **时间戳取文件自己的 mtime**，不取墙上时钟。它恰好是「这份设置最后被写下的
///   时刻」，比「发现它坏掉的时刻」更有信息量；更重要的是它是**数据**而不是
///   **采样**，不走 `sampling_clock_sentinel` 那条「采样时钟由引擎盖章」的纪律
///   （那条只管 `session.rs` 的探测层），测试也完全确定。
fn stash_broken(path: &Path) -> Option<String> {
    let stamp = fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    let parent = path.parent()?;
    let base = path.file_name()?.to_string_lossy().to_string();
    // 撞名要能一直往下找：同一毫秒内坏两次、或 mtime 取不到（都退化成 0）都会撞。
    for n in 0..64u32 {
        let name = if n == 0 {
            format!("{base}.broken-{stamp}")
        } else {
            format!("{base}.broken-{stamp}-{n}")
        };
        let target = parent.join(&name);
        if target.exists() {
            continue; // 已有同名留档：换序号，绝不覆盖
        }
        return fs::rename(path, &target).ok().map(|()| name);
    }
    None
}

/// 现存文件读得到、却解析不回来 ⇒ 它是用户的东西，不是垃圾：改名留档。
///
/// 读不到（不存在 / 无权限）**不算坏**——那没有东西可丢。
fn stash_broken_if_unparseable(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    if serde_json::from_str::<Settings>(&text).is_ok() {
        return None;
    }
    stash_broken(path)
}

impl Settings {
    pub fn load() -> Self {
        Self::load_from(&config_dir())
    }

    /// 读设置（`load()` 传真实配置目录，测试传临时目录）。
    ///
    /// **解析失败先留档，再回落出厂值。** 此前这里只 `if let Ok(..)` 一笔带过，
    /// 坏文件的命运交给下一次 `save()`——而 `save_to` 写得挺稳的，于是那份坏文件
    /// 被一份「出厂值」干净利落地覆盖掉：用户改过的设置整份消失，**没有留档、没有日志、
    /// 没有提示**。本文件开头把「写一半崩掉 → 静默回落出厂值」写成落盘必须原子的理由，
    /// 却只挡住了成因（写一半），没挡住后果（已经坏掉的那份被覆盖）。
    ///
    /// 处置与 `todos.rs::stash_broken` 一致——**同一件事只能有一种待遇**。
    pub(crate) fn load_from(dir: &Path) -> Self {
        let path = dir.join("settings.json");
        let Ok(text) = fs::read_to_string(&path) else {
            return Settings::default(); // 不存在或读不动：不是「坏」，没什么可留档
        };
        match serde_json::from_str::<Settings>(&text) {
            Ok(s) => s.normalized(),
            Err(error) => {
                match stash_broken(&path) {
                    Some(name) => crate::log_line(&format!(
                        "[settings] settings.json 解析失败（{error}），已留档为 {name}，本次回落出厂值"
                    )),
                    None => crate::log_line(&format!(
                        "[settings] settings.json 解析失败（{error}）且留档失败——下一次落盘会覆盖它"
                    )),
                }
                Settings::default()
            }
        }
    }

    pub fn save(&self) {
        self.save_to(&config_dir());
    }

    /// 落盘到指定目录（`save()` 传真实配置目录，测试传临时目录）。
    ///
    /// **必须原子替换**：此前这里是裸 `fs::write`，写一半崩掉就留下截断的 JSON，
    /// 下次启动 `load()` 解析失败、静默回落出厂值——用户的设置整份消失且没有任何提示，
    /// 比解析失败更难发现。校验用「能被自己解析回来」，与 `load()` 同一套形状；
    /// 校验没过则原文件逐字节不动（由 `atomicfile` 保证）。
    ///
    /// 原子性只挡住「**写坏**」，挡不住「**覆盖已经坏掉的**」：文件可能是老版本写的、
    /// 手改写坏的、磁盘写满截断的、或备份工具动过的。所以落盘前先看一眼现存文件——
    /// 读得到却解析不回来就改名留档，再写新的（`load_from` 也做同一件事，
    /// 覆盖「跑起来之后才坏掉」的那一半）。
    ///
    /// 内部兼容调用仍只记录错误；UI 经 try_save_to 返回错误，不得把未落盘报成已保存。
    pub(crate) fn save_to(&self, dir: &Path) {
        if let Err(error) = self.try_save_to(dir) {
            crate::log_line(&format!("[settings] 落盘失败：{error}"));
        }
    }

    pub(crate) fn try_save_to(&self, dir: &Path) -> io::Result<()> {
        fs::create_dir_all(dir)?;
        let path = dir.join("settings.json");
        if let Some(name) = stash_broken_if_unparseable(&path) {
            crate::log_line(&format!("[settings] 落盘前发现坏掉的 settings.json，已留档为 {name}"));
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let result = crate::atomicfile::atomic_replace_validated(&path, json.as_bytes(), |staged| {
            let text = fs::read_to_string(staged)?;
            serde_json::from_str::<Settings>(&text)
                .map(|_| ())
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        });
        result
    }

    pub(crate) fn patched(&self, patch: serde_json::Value) -> Result<Self, String> {
        let mut value = serde_json::to_value(self).map_err(|error| error.to_string())?;
        let current = value.as_object_mut().ok_or("设置不是对象")?;
        let fields = patch.as_object().ok_or("设置变更必须是对象")?;
        for (key, field) in fields {
            if !current.contains_key(key) { return Err(format!("未知设置项：{key}")); }
            current.insert(key.clone(), field.clone());
        }
        serde_json::from_value::<Self>(value).map(|settings| settings.normalized()).map_err(|error| error.to_string())
    }

    /// 脏值钳制（与 macOS `EngineConfig.normalized()` 同规则）
    ///
    /// 顺序与 Swift 一致：**先归位 NaN，再钳区间，最后拉平相互依赖的两个字段**。
    /// 少了 NaN 那一步，`NaN.clamp()` 会原样返回 NaN，而 NaN 进了比较就是 false——
    /// 于是所有 `if cpu > threshold` 判定会静默走「没超」那一支。
    pub fn normalized(&self) -> Self {
        let mut s = self.clone();
        // NaN 先归位到出厂值：手改 settings.json 写进 NaN 是现实（`1e999` 之类），
        // 而 `f64::clamp` 对 NaN 不生效。逐个字段与 Swift 的 `if … isNaN` 一一对应。
        let base = Settings::default();
        for (value, fallback) in [
            (&mut s.sample_interval, base.sample_interval),
            (&mut s.idle_sample_interval, base.idle_sample_interval),
            (&mut s.working_window, base.working_window),
            (&mut s.cpu_threshold, base.cpu_threshold),
            (&mut s.active_session_window, base.active_session_window),
            (&mut s.min_working_hold, base.min_working_hold),
            (&mut s.runaway_cpu_threshold, base.runaway_cpu_threshold),
            (&mut s.runaway_duration_threshold, base.runaway_duration_threshold),
        ] {
            if value.is_nan() {
                *value = fallback;
            }
        }
        // Registry ID was `roo` before Swift/Rust parity. Preserve an existing
        // disabled choice when loading older settings instead of enabling it anew.
        for id in &mut s.disabled_agents {
            if id == "roo" {
                *id = "roo-code".into();
            }
        }
        let mut seen = HashSet::new();
        s.disabled_agents.retain(|id| seen.insert(id.clone()));
        let clamp = |v: f64, lo: f64, hi: f64| v.clamp(lo, hi);
        s.cpu_threshold = clamp(s.cpu_threshold, 1.0, 50.0);
        s.sample_interval = clamp(s.sample_interval, 0.5, 600.0);
        s.idle_sample_interval = clamp(s.idle_sample_interval, 0.5, 600.0);
        s.working_window = clamp(s.working_window, 10.0, 300.0);
        s.active_session_window = clamp(s.active_session_window, 60.0, 3600.0);
        s.min_working_hold = clamp(s.min_working_hold, 1.0, 300.0);
        s.runaway_cpu_threshold = clamp(s.runaway_cpu_threshold, 10.0, 100.0);
        s.runaway_duration_threshold = clamp(s.runaway_duration_threshold, 30.0, 3600.0);
        // 收起延迟上限 5s（Swift `SettingLimits.collapseDelayRange`）。此前 Rust 放到 30s，
        // 脏值能让面板久驻十几秒——那不是「宽容」，是用户找不到它关哪儿了。
        s.collapse_delay = clamp(s.collapse_delay, 0.2, 5.0);
        // 有活动间隔不得大于闲置间隔（Swift `normalized()` 末尾同一条）：
        // 否则「有活动」反而比「全闲置」更慢，滞回与降频都失去意义。
        if s.sample_interval > s.idle_sample_interval {
            s.sample_interval = s.idle_sample_interval;
        }
        s.token_alert_threshold = s.token_alert_threshold.clamp(1_000, 10_000_000);
        // 与 Swift `SettingLimits.dailyTokenBudgetRange` 同区间（0…1e9）。
        // 手改 settings.json 写进一个负预算会让「超额」永远成立
        s.daily_token_budget = s.daily_token_budget.clamp(0, 1_000_000_000);
        s.dock_anchor = s.dock_anchor.clamp(0.0, 1.0);
        // 两个枚举字段：认得的值以外一律回落出厂值。
        // 回落到「默认值」而不是「第一个变体」——默认值才是这个键出厂时的样子。
        s.menu_bar_badge_mode = match s.menu_bar_badge_mode.as_str() {
            "iconOnly" | "activeCount" | "tokenUsage" => s.menu_bar_badge_mode.clone(),
            _ => "iconOnly".to_string(),
        };
        s.screen_follow_mode = match s.screen_follow_mode.as_str() {
            "followMouse" | "mainScreen" | "builtInScreen" | "externalScreen" => {
                s.screen_follow_mode.clone()
            }
            _ => "followMouse".to_string(),
        };
        // 形态与侧边栏：认得的值以外一律回落，宽度钳进可用区间
        // （手改 settings.json 写进 0 或 5000 会让侧边栏变成一条缝或盖住整块屏）
        s.shell_mode = crate::models::ShellMode::parse(&s.shell_mode)
            .as_str()
            .to_string();
        s.sidebar_edge = if s.sidebar_edge.trim().eq_ignore_ascii_case("left") {
            "left".to_string()
        } else {
            "right".to_string()
        };
        s.sidebar_width = crate::placement::clamp_sidebar_width(s.sidebar_width);

        // 外发配置：读出即归一化。损坏/越界的值在落盘时就可能已经写进去了，
        // 读这条路是唯一防线（Swift `loadPolicy` / `loadConfig` 同一个位置做同一件事）
        s.remote_policy = s.remote_policy.normalized();
        for config in s.remote_channels.values_mut() {
            // 手改 JSON 留下 smtpPort = 0 会每次都连向一个注定不存在的端口，比退回默认更难查
            if config.smtp_port <= 0 || config.smtp_port > 65_535 {
                config.smtp_port = 465;
            }
        }
        s
    }
}

#[cfg(test)]
mod shell_tests {
    use super::*;

    #[test]
    fn a_hand_edited_shell_mode_or_edge_falls_back_instead_of_breaking_the_window() {
        let mut s = Settings::default();
        assert_eq!(s.shell_mode, "island", "默认仍是灵动岛（ADR 0009 的并存前提）");
        assert_eq!(s.sidebar_edge, "right");
        assert_eq!(s.sidebar_width, crate::placement::DEFAULT_SIDEBAR_WIDTH);

        s.shell_mode = "SIDEBAR".into();
        s.sidebar_edge = "LEFT".into();
        let n = s.normalized();
        assert_eq!(n.shell_mode, "sidebar", "大小写不该影响识别");
        assert_eq!(n.sidebar_edge, "left");

        // 认不出的一律回落：形态回落灵动岛、边沿回落右
        s.shell_mode = "sider".into();
        s.sidebar_edge = "top".into();
        let n = s.normalized();
        assert_eq!(n.shell_mode, "island");
        assert_eq!(n.sidebar_edge, "right", "侧边栏没有上下两档");
    }

    #[test]
    fn an_absurd_sidebar_width_is_clamped_so_it_cannot_become_a_slit_or_cover_the_screen() {
        let mut s = Settings::default();
        for (raw, want) in [
            (0.0, crate::placement::MIN_SIDEBAR_WIDTH),
            (-100.0, crate::placement::MIN_SIDEBAR_WIDTH),
            (5_000.0, crate::placement::MAX_SIDEBAR_WIDTH),
            (420.0, 420.0),
            (f64::NAN, crate::placement::DEFAULT_SIDEBAR_WIDTH),
            (f64::INFINITY, crate::placement::MAX_SIDEBAR_WIDTH),
        ] {
            s.sidebar_width = raw;
            assert_eq!(s.normalized().sidebar_width, want, "raw={raw}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `#[serde(default)]` 是**跨版本迁移的唯一通道**：老设置文件缺新字段时补默认值，
    /// 而不是整份解析失败退化成 `Settings::default()`——
    /// 后者会让用户「所有设置悄悄回到出厂值」，比解析失败更难发现。
    #[test]
    fn settings_deserialize_tolerates_missing_fields() {
        let partial = r#"{"appearance":"dark"}"#;
        let s: Settings = serde_json::from_str(partial).expect("缺字段应被默认值补齐，而非解析失败");
        assert_eq!(s.appearance, "dark");
        assert_eq!(s.dock_edge, "top", "缺 dock_edge 应落默认值");
        assert_eq!(
            s.disabled_agents,
            vec!["continue".to_string()],
            "缺禁用列表应落**出厂**值——而出厂不是空的：`continue` 在 macOS 侧默认关闭"
        );
    }

    /// `normalized()` 钳的是**用户可写的脏值**（设置界面是文本/滑块，写坏是常态）。
    /// 上下界每个都要覆盖：越界必须被夹回——否则 `sample_interval = 0` 会让引擎空转占核。
    #[test]
    fn normalized_clamps_every_dirty_field() {
        let mut lo = Settings::default();
        lo.cpu_threshold = 0.0;
        lo.sample_interval = 0.0;
        lo.collapse_delay = 0.0;
        lo.dock_anchor = -5.0;
        lo.token_alert_threshold = 0;
        let n = lo.normalized();

        assert_eq!(n.cpu_threshold, 1.0, "cpu_threshold 下限");
        assert_eq!(n.sample_interval, 0.5, "sample_interval 下限（0 会让引擎空转占核）");
        assert_eq!(n.collapse_delay, 0.2, "collapse_delay 下限");
        assert_eq!(n.dock_anchor, 0.0, "dock_anchor 下限");
        assert_eq!(n.token_alert_threshold, 1_000, "token_alert_threshold 下限");

        let mut hi = Settings::default();
        hi.cpu_threshold = 1e9;
        hi.working_window = 1e9;
        hi.active_session_window = 1e9;
        hi.min_working_hold = 1e9;
        hi.runaway_cpu_threshold = 1e9;
        hi.runaway_duration_threshold = 1e9;
        hi.collapse_delay = 1e9;
        hi.dock_anchor = 1e9;
        hi.token_alert_threshold = i64::MAX;
        let n = hi.normalized();

        assert_eq!(n.cpu_threshold, 50.0, "cpu_threshold 上限");
        assert_eq!(n.working_window, 300.0, "working_window 上限");
        assert_eq!(n.active_session_window, 3600.0, "active_session_window 上限");
        assert_eq!(n.min_working_hold, 300.0, "min_working_hold 上限");
        assert_eq!(n.runaway_cpu_threshold, 100.0, "runaway_cpu_threshold 上限");
        assert_eq!(n.runaway_duration_threshold, 3600.0, "runaway_duration_threshold 上限");
        assert_eq!(n.collapse_delay, 5.0, "collapse_delay 上限（Swift SettingLimits 是 5s，不是 30s）");
        assert_eq!(n.dock_anchor, 1.0, "dock_anchor 上限");
        assert_eq!(n.token_alert_threshold, 10_000_000, "token_alert_threshold 上限");
    }

    /// 区间与默认值**逐个**对着 Swift `EngineConfig` 核。
    ///
    /// 这条是给「补字段」这件事上的机械守卫：上一轮补了 8 个字段，
    /// 手抄区间最容易抄错上下界，而错一个界不会有任何症状——
    /// 只会让用户在设置页拖到一个本该被夹回去的值。
    #[test]
    fn every_engine_field_matches_the_swift_default_and_range() {
        let d = Settings::default();
        assert_eq!(d.sample_interval, 2.0, "sampleInterval");
        assert_eq!(d.idle_sample_interval, 5.0, "idleSampleInterval");
        assert_eq!(d.working_window, 60.0, "workingWindow");
        assert_eq!(d.cpu_threshold, 6.0, "cpuThreshold");
        assert_eq!(d.active_session_window, 600.0, "activeSessionWindow");
        assert_eq!(d.min_working_hold, 10.0, "minWorkingHold");
        assert!(d.runaway_cpu_alert, "runawayCpuAlert 默认开");
        assert_eq!(d.runaway_cpu_threshold, 70.0, "runawayCpuThreshold");
        assert_eq!(d.runaway_duration_threshold, 300.0, "runawayDurationThreshold");
        assert!(d.battery_saver_enabled, "batterySaverEnabled 默认开");
        assert!(!d.launch_at_login, "launchAtLogin 默认关");
        assert!(!d.hide_docked_sliver, "hideDockedSliver 默认关");
        assert!(!d.compact_view, "compactView 默认关");
        assert!(d.global_hot_key_enabled, "globalHotKeyEnabled 默认开");
        assert_eq!(d.menu_bar_badge_mode, "iconOnly", "menuBarBadgeMode");
        assert_eq!(d.screen_follow_mode, "followMouse", "screenFollowMode");

        // 下界逐个核（Swift 的 range lowerBound）
        let mut lo = Settings::default();
        lo.sample_interval = -1.0;
        lo.idle_sample_interval = -1.0;
        lo.working_window = 0.0;
        lo.active_session_window = 0.0;
        lo.min_working_hold = 0.0;
        lo.runaway_cpu_threshold = 0.0;
        lo.runaway_duration_threshold = 0.0;
        let n = lo.normalized();
        assert_eq!(n.sample_interval, 0.5, "sampleIntervalRange 下界");
        assert_eq!(n.idle_sample_interval, 0.5, "idleSampleIntervalRange 下界");
        assert_eq!(n.working_window, 10.0, "workingWindowRange 下界");
        assert_eq!(n.active_session_window, 60.0, "activeSessionWindowRange 下界");
        assert_eq!(n.min_working_hold, 1.0, "minWorkingHoldRange 下界");
        assert_eq!(n.runaway_cpu_threshold, 10.0, "runawayCpuThresholdRange 下界");
        assert_eq!(n.runaway_duration_threshold, 30.0, "runawayDurationThresholdRange 下界");
    }

    /// NaN 必须先归位再钳。
    ///
    /// `f64::clamp` 对 NaN **不生效**（返回 NaN），而 NaN 进了任何比较都是 false——
    /// 于是 `if cpu > threshold` 会静默走「没超」那一支，引擎把高负载当没发生。
    /// 手改 settings.json 写进 `1e999` 就是 NaN，这是现实输入。
    #[test]
    fn a_nan_interval_falls_back_to_the_factory_value_instead_of_silently_disabling() {
        let mut s = Settings::default();
        s.cpu_threshold = f64::NAN;
        s.working_window = f64::NAN;
        s.runaway_cpu_threshold = f64::NAN;
        s.sample_interval = f64::NAN;
        let n = s.normalized();
        assert_eq!(n.cpu_threshold, 6.0, "NaN 归位到出厂值");
        assert_eq!(n.working_window, 60.0);
        assert_eq!(n.runaway_cpu_threshold, 70.0);
        assert_eq!(n.sample_interval, 2.0);
        assert!(n.normalized().cpu_threshold.is_nan() == false, "归一化后不得仍是 NaN");
    }

    /// 有活动间隔不得大于闲置间隔（Swift `normalized()` 末尾同一条）。
    /// 否则「有活动」反而比「全闲置」更慢，滞回与降频都失去意义。
    #[test]
    fn the_active_interval_is_pulled_down_to_the_idle_one_when_it_exceeds_it() {
        let mut s = Settings::default();
        s.sample_interval = 120.0;
        s.idle_sample_interval = 5.0;
        let n = s.normalized();
        assert_eq!(n.sample_interval, 5.0, "有活动间隔不得慢于全闲置");
        assert_eq!(n.idle_sample_interval, 5.0, "闲置间隔不动");

        // 反向合法值不受影响
        let mut ok = Settings::default();
        ok.sample_interval = 2.0;
        ok.idle_sample_interval = 5.0;
        assert_eq!(ok.normalized().sample_interval, 2.0);
    }

    /// 两个枚举字段认不出的值一律回出厂值——回落到**默认值**而不是第一个变体。
    #[test]
    fn unknown_enum_settings_fall_back_to_their_factory_value() {
        let mut s = Settings::default();
        s.menu_bar_badge_mode = "rainbow".into();
        s.screen_follow_mode = "thirdMonitor".into();
        let n = s.normalized();
        assert_eq!(n.menu_bar_badge_mode, "iconOnly");
        assert_eq!(n.screen_follow_mode, "followMouse");

        for good in ["iconOnly", "activeCount", "tokenUsage"] {
            let mut keep = Settings::default();
            keep.menu_bar_badge_mode = good.into();
            assert_eq!(keep.normalized().menu_bar_badge_mode, good);
        }
        for good in ["followMouse", "mainScreen", "builtInScreen", "externalScreen"] {
            let mut keep = Settings::default();
            keep.screen_follow_mode = good.into();
            assert_eq!(keep.normalized().screen_follow_mode, good);
        }
    }

    /// `normalized()` 是幂等的：钳一次和钳两次必须一样。
    /// 非幂等意味着每次 `load()` 都会再夹一次，用户的合法值会被慢慢推离他设的那个数。
    /// 外发配置也是「读出即归一化」：坏值在落盘时就可能已经写进去了，读这条路是唯一防线
    /// （Swift `loadPolicy` / `loadConfig` 在同一个位置做同一件事）。
    #[test]
    fn remote_config_is_normalized_on_load_and_bad_ports_fall_back() {
        let mut dirty = Settings::default();
        dirty.remote_policy.throttle_seconds = 5;
        dirty.remote_policy.quiet_start = "25:00".into();
        dirty.remote_policy.quiet_end = "07:00".into();
        dirty.remote_channels.insert(
            "smtpEmail".into(),
            crate::remote::ChannelConfig {
                smtp_port: 0,
                ..Default::default()
            },
        );

        let clean = dirty.normalized();
        assert_eq!(clean.remote_policy.throttle_seconds, crate::remote::THROTTLE_RANGE.0);
        assert!(
            clean.remote_policy.quiet_start.is_empty() && clean.remote_policy.quiet_end.is_empty(),
            "写坏的时刻必须整对作废"
        );
        assert_eq!(clean.remote_channels["smtpEmail"].smtp_port, 465);

        // 归一化幂等：再读一遍不会再变
        assert_eq!(clean.normalized().remote_policy, clean.remote_policy);
    }

    #[test]
    fn normalized_is_idempotent() {
        let mut s = Settings::default();
        s.cpu_threshold = 999.0;
        s.dock_anchor = 3.0;
        let once = s.normalized();
        let twice = once.normalized();
        assert_eq!(once.cpu_threshold, twice.cpu_threshold);
        assert_eq!(once.dock_anchor, twice.dock_anchor);
        assert_eq!(once.sample_interval, twice.sample_interval);
    }

    /// serde 往返无损：设置项一旦丢字段，界面显示的值与实际生效的值会不一致。
    #[test]
    fn settings_serde_roundtrip_is_lossless() {
        let mut original = Settings::default();
        original.disabled_agents = vec!["codex".into(), "opencode".into()];
        original.dock_edge = "left".into();
        original.appearance = "dark".into();

        let json = serde_json::to_string(&original).expect("应可序列化");
        let back: Settings = serde_json::from_str(&json).expect("应可反序列化");

        assert_eq!(original.appearance, back.appearance);
        assert_eq!(original.dock_edge, back.dock_edge, "dock_edge 往返后变了");
        assert_eq!(original.disabled_agents, back.disabled_agents, "禁用列表往返后变了");
        assert_eq!(original.cpu_threshold, back.cpu_threshold);
        assert_eq!(original.token_alert_threshold, back.token_alert_threshold);
    }

    #[test]
    fn legacy_roo_disabled_choice_survives_id_alignment() {
        let old: Settings = serde_json::from_str(
            r#"{"disabled_agents":["roo","codex","roo-code"]}"#,
        )
        .unwrap();
        let normalized = old.normalized();
        assert_eq!(normalized.disabled_agents, vec!["roo-code", "codex"]);
        assert_eq!(normalized.normalized().disabled_agents, normalized.disabled_agents);
    }

    /// 落盘必须原子：写出来的东西要能被 `load()` 的形状解析回来，且不留暂存文件。
    /// 用临时目录而不是真实配置目录——测试不许碰用户的真设置。
    /// 反过来那条（写坏时原文件不动）由 `atomicfile` 的用例直接覆盖。
    #[test]
    fn save_to_replaces_settings_atomically_and_leaves_no_staging_file() {
        let sandbox = crate::testutil::Sandbox::new("settings");
        let dir = sandbox.path().to_path_buf();
        let mut s = Settings::default();
        s.appearance = "dark".into();
        s.disabled_agents = vec!["codex".into()];
        s.save_to(&dir);

        let entries: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            entries,
            vec![std::ffi::OsString::from("settings.json")],
            "暂存文件泄漏: {entries:?}"
        );
        let text = fs::read_to_string(dir.join("settings.json")).unwrap();
        let back: Settings = serde_json::from_str(&text).expect("写出来的必须能解析回来");
        assert_eq!(back.appearance, "dark");
        assert_eq!(back.disabled_agents, vec!["codex"]);

        // 覆盖写：第二次落盘同样不留暂存文件，且内容是最新那份
        s.appearance = "light".into();
        s.save_to(&dir);
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            1,
            "覆盖写之后目录里只该有一个文件"
        );
        let again: Settings =
            serde_json::from_str(&fs::read_to_string(dir.join("settings.json")).unwrap()).unwrap();
        assert_eq!(again.appearance, "light");

        fs::remove_dir_all(&dir).unwrap();
    }

    // MARK: 坏掉的 settings.json 不许被静默覆盖

    fn list_dir(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// 目录里的留档文件名（`settings.json.broken-*`）。
    fn broken_archives(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = list_dir(dir)
            .into_iter()
            .filter(|n| n.starts_with("settings.json.broken"))
            .collect();
        names.sort();
        names
    }

    /// 坏掉的 `settings.json` 不能被静默覆盖——那份坏文件是用户的东西。
    ///
    /// 修复前这条**不可能通过**：旧 `save_to` 只管写、不看现存文件是什么，
    /// 于是坏文件被新写的一份顶掉，目录里只剩 `settings.json`。
    /// 断言的是**留档逐字节等于原来那份坏文件**，不是「有个备份」——
    /// 否则一个留档写成空文件也能蒙混过关。
    #[test]
    fn a_corrupt_settings_file_is_archived_instead_of_silently_overwritten() {
        let sandbox = crate::testutil::Sandbox::new("settings-broken");
        let dir = sandbox.path().to_path_buf();
        // 截断的 JSON：正是本文档开头那场「写一半崩掉」的真实产物形状
        let corrupt = r#"{"appearance": "dar"#;
        fs::write(dir.join("settings.json"), corrupt).unwrap();

        let mut s = Settings::default();
        s.appearance = "dark".into();
        s.save_to(&dir);

        let archives = broken_archives(&dir);
        assert_eq!(archives.len(), 1, "坏文件应留档一份，目录：{:?}", list_dir(&dir));
        assert_eq!(
            fs::read_to_string(dir.join(&archives[0])).unwrap(),
            corrupt,
            "留档必须逐字节等于原来那份坏文件"
        );
        let text = fs::read_to_string(dir.join("settings.json")).unwrap();
        let back: Settings = serde_json::from_str(&text).expect("落盘后必须能解析");
        assert_eq!(back.appearance, "dark", "新值要真的落下去");
    }

    /// 读的那条路也必须留档：**发现**坏文件是在读的时候，回落出厂值之后
    /// 下一次 `save()` 就会覆盖它——所以留档得发生在回落之前。
    #[test]
    fn load_archives_a_corrupt_file_before_falling_back_to_defaults() {
        let sandbox = crate::testutil::Sandbox::new("settings-load-broken");
        let dir = sandbox.path().to_path_buf();
        let corrupt = "[] 这不是对象";
        fs::write(dir.join("settings.json"), corrupt).unwrap();

        let s = Settings::load_from(&dir);
        assert_eq!(s.appearance, "system", "解析失败应回落出厂值");
        let archives = broken_archives(&dir);
        assert_eq!(archives.len(), 1, "回落前必须先留档，目录：{:?}", list_dir(&dir));
        assert_eq!(fs::read_to_string(dir.join(&archives[0])).unwrap(), corrupt);
    }

    /// 留档不能互相覆盖：`fs::rename` 在 POSIX 上会**静默顶掉**同名目标。
    /// 只查「有没有留档」会被这条钻空子——两份坏文件只剩一份，仍然「有留档」。
    ///
    /// **两次坏掉的 mtime 必须钉成同一个值**：留档名是 `…broken-<mtime 毫秒>`，
    /// 两次若落不同时刻，名字本来就不撞，这条会「靠运气」全绿——
    /// 事实上第一版正是如此，变异去掉存在性检查后它**照样通过**，等于没测。
    /// 所以这里显式把 mtime 设成固定常量，逼出真正要防的那件事。
    #[test]
    fn archiving_never_clobbers_an_existing_archive() {
        use std::fs::FileTimes;
        use std::time::{Duration, SystemTime};

        let sandbox = crate::testutil::Sandbox::new("settings-broken-twice");
        let dir = sandbox.path().to_path_buf();
        let pinned = SystemTime::UNIX_EPOCH + Duration::from_millis(1_700_000_000_000);
        let write_corrupt = |text: &str| {
            let path = dir.join("settings.json");
            fs::write(&path, text).unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(FileTimes::new().set_modified(pinned))
                .unwrap();
        };

        write_corrupt("第一次坏掉");
        Settings::default().save_to(&dir);
        write_corrupt("第二次坏掉");
        Settings::default().save_to(&dir);

        let archives = broken_archives(&dir);
        assert_eq!(archives.len(), 2, "两次坏掉要留两份档，目录：{:?}", list_dir(&dir));
        let bodies: Vec<String> = archives
            .iter()
            .map(|n| fs::read_to_string(dir.join(n)).unwrap())
            .collect();
        // 按「两份都在」断言，不按顺序：这里要证明的是谁都没被冲掉，
        // 顺序只取决于码点与 mtime，与要证明的事无关。
        assert!(
            bodies.contains(&"第一次坏掉".to_string()),
            "第一份坏文件应还在留档里：{bodies:?}"
        );
        assert!(
            bodies.contains(&"第二次坏掉".to_string()),
            "第二份坏文件应还在留档里：{bodies:?}"
        );
    }

    /// 反向：不许退化成「什么都留档」。好文件一次都不该被改名。
    /// 这条同时守住健康路径没被我改坏（读回来是最新那份）。
    #[test]
    fn a_healthy_settings_file_is_never_archived() {
        let sandbox = crate::testutil::Sandbox::new("settings-healthy");
        let dir = sandbox.path().to_path_buf();
        let mut s = Settings::default();
        s.appearance = "light".into();
        s.save_to(&dir);
        s.appearance = "dark".into();
        s.save_to(&dir);
        assert!(
            broken_archives(&dir).is_empty(),
            "好文件不该留档，目录：{:?}",
            list_dir(&dir)
        );
        assert_eq!(Settings::load_from(&dir).appearance, "dark", "读回来应是最新那份");
    }
}

/// 侧边栏「高级设置」页的字段表（`app/ui/js/views.js` 的 `SETTING_FIELDS`）
/// 必须与本结构体**双向**对齐。
///
/// 为什么要这条哨兵：设置页的字段表是手写的清单，而 `Settings` 是另一份。
/// 任何一边加了字段而另一边没跟上，都不会报错——只会有一个**永远显示不出来**
/// 或**界面上有、点下去没反应**的设置项。设置项多起来之后，这类漂移迟早发生，
/// 而它对用户的症状是「这个开关不管用」，极难自查。
#[cfg(test)]
mod ui_parity {
    use super::*;

    /// 从 `views.js` 里粗解析出 `SETTING_FIELDS` 用到的键。
    /// 只认 `{ key: '...' }` 这一种形状——解析失败宁可返回空集让断言报出来，
    /// 也不要猜一个键名继续往下走。
    fn ui_field_keys() -> Vec<String> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../app/ui/js/views.js");
        let Ok(text) = std::fs::read_to_string(&path) else {
            panic!("读不到 views.js：{}", path.display());
        };
        let start = text.find("const SETTING_FIELDS = [")
            .expect("views.js 里应当有 SETTING_FIELDS");
        let end = text[start..].find("\n];").expect("SETTING_FIELDS 应当有结束标记") + start;
        let block = &text[start..end];

        let mut keys: Vec<String> = Vec::new();
        let mut rest = block;
        while let Some(at) = rest.find("{ key: '") {
            rest = &rest[at + 8..];
            let Some(endq) = rest.find('\'') else { break };
            keys.push(rest[..endq].to_string());
            rest = &rest[endq..];
        }
        keys.sort();
        keys.dedup();
        keys
    }

    /// `Settings` 的字段名（从源码文本取，避免为测试造一套反射）。
    fn rust_field_names() -> Vec<String> {
        let text = std::fs::read_to_string(file!()).expect("读不到 settings.rs");
        let start = text.find("pub struct Settings {").expect("应能定位结构体");
        let end = text[start..].find("\n}").expect("应能定位结构体结尾") + start;
        text[start..end]
            .lines()
            .skip(1) // 首行是 `pub struct Settings {` 本身，不是字段
            .filter_map(|l| {
                let t = l.trim();
                if !t.starts_with("pub ") {
                    return None;
                }
                t.strip_prefix("pub ")?
                    .split(':')
                    .next()
                    .map(|s| s.trim().to_string())
            })
            .collect()
    }

    /// **不由设置页管的字段**，逐条写明「谁在管」。
    ///
    /// 这份名单是**缺口清单，不是免责清单**：每一条要么是直接操作（拖拽/拉伸），
    /// 要么是「这一页还没有、已在待办里」。后者不允许长期留在这里——
    /// 每补一页就从这里移走一条，名单空了这条断言就自动收紧成「零例外」。
    const MANAGED_ELSEWHERE: &[(&str, &str)] = &[
        ("sidebar_width", "侧边栏直接拖拽调整（记宽度），不是滑块"),
        ("dock_anchor", "灵动岛顶栏拖拽 + 松手吸附直接操作，滑块给不出同样的手感"),
        ("remote_kind", "远程通知页有自己的通道选择器（三个通道字段不一样，挤进通用行会很难用）"),
        ("remote_policy", "远程通知页有「发送策略」分组，含静默时段这类成对字段"),
        ("remote_channels", "同上：每个通道一份独立配置，不是单个标量"),
        ("disabled_agents", "Agent 启停页是逐项列表（`List<String>`），不是设置页里的一行控件"),
    ];

    #[test]
    fn every_settings_field_is_reachable_from_the_sidebar_page() {
        let ui = ui_field_keys();
        let rust = rust_field_names();
        let exempt: Vec<&str> = MANAGED_ELSEWHERE.iter().map(|(k, _)| *k).collect();
        let missing: Vec<&String> = rust
            .iter()
            .filter(|f| !ui.contains(f) && !exempt.contains(&f.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "Settings 里有这些字段但设置页没有对应控件，用户永远改不了：{missing:?}"
        );
    }

    /// 名单里的字段**必须真的存在**，否则就是一条写脏了的豁免在替真缺口挡枪。
    #[test]
    fn every_exemption_names_a_field_that_actually_exists() {
        let rust = rust_field_names();
        for (key, why) in MANAGED_ELSEWHERE {
            assert!(
                rust.iter().any(|f| f == key),
                "豁免 {key}（{why}）指向一个已经不存在的字段——多半是它被改名或删掉了"
            );
        }
    }

    /// 名单里**不许再有「还没做」的条目**。
    ///
    /// v0.0.200 时这里有四条标着 ❌（远程通知三件套 + 启停列表），
    /// 那一版把它们做掉了，于是本条从「盯四条具体字段」变成「❌ 一条都不许有」。
    /// 形状变了——**这正是它该有的形状**：做掉一批就少一批，
    /// 而豁免只剩「这里为什么不能是一行控件」这类结构性理由。
    #[test]
    fn no_exemption_is_still_an_unfinished_gap() {
        let unfinished: Vec<(&str, &str)> = MANAGED_ELSEWHERE
            .iter()
            .filter(|(_, why)| why.starts_with('❌'))
            .copied()
            .collect();
        assert!(
            unfinished.is_empty(),
            "这些字段对应的页面还没做，标了 ❌：{unfinished:?}"
        );
    }

    #[test]
    fn the_settings_page_has_no_control_for_a_field_that_does_not_exist() {
        let ui = ui_field_keys();
        let rust = rust_field_names();
        let ghost: Vec<&String> = ui.iter().filter(|f| !rust.contains(f)).collect();
        assert!(
            ghost.is_empty(),
            "设置页引用了不存在的字段，界面上有、点下去静默失效：{ghost:?}"
        );
    }
}

/// **`continue` 出厂即关闭**（与 macOS 侧 `defaultEnabled: false` 同值）。
///
/// 这条钉的是一个**跨端不可见的不一致**：Rust 侧的 `disabled_agents` 是黑名单、
/// 空集=全开，所以「默认关」必须显式列出来。
/// 不列的话，Continue 会在新装的 Rust 端**默认开着**，
/// 而用户在 macOS 上从没见过它开着——一个只在一边出现的幽灵条目。
#[cfg(test)]
mod default_off {
    use super::*;

    #[test]
    fn continue_is_off_out_of_the_box_and_nothing_else_is() {
        let d = Settings::default();
        assert_eq!(
            d.disabled_agents,
            vec!["continue".to_string()],
            "只有 Continue 该默认关闭"
        );
        // 且这个档案**必须存在**——把一个不存在的 id 写进黑名单是静默无效的
        assert!(
            crate::registry::builtin().iter().any(|p| p.id == "continue"),
            "黑名单里的 `continue` 必须在注册表里"
        );
    }

    /// 归一化**不会**把默认关闭的那些清掉：那是用户的显式选择。
    #[test]
    fn normalization_keeps_the_default_off_choice() {
        let d = Settings::default();
        assert_eq!(d.normalized().disabled_agents, d.disabled_agents);
    }
}

#[cfg(test)]
mod patch_regressions {
    use super::*;
    #[test]
    fn independent_window_patches_preserve_threshold_and_remote_policy() {
        let settings = Settings::default().patched(serde_json::json!({"cpu_threshold": 23})).unwrap();
        let settings = settings.patched(serde_json::json!({"remote_policy": {"master_enabled": true, "throttle_seconds": 140}})).unwrap();
        assert_eq!(settings.cpu_threshold, 23.0);
        assert!(settings.remote_policy.master_enabled);
        assert_eq!(settings.remote_policy.throttle_seconds, 140);
        assert!(settings.patched(serde_json::json!({"cpu_threshold": "bad"})).is_err());
        assert!(settings.patched(serde_json::json!({"unknown_setting": true})).is_err());
    }
    #[test]
    fn disk_failure_is_returned_to_the_ui_caller() {
        let sandbox = crate::testutil::Sandbox::new("settings-write-error");
        let file = sandbox.path().join("blocked");
        std::fs::write(&file, b"fixture").unwrap();
        assert!(Settings::default().try_save_to(&file).is_err());
    }
}
