use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// 持久化设置（macOS 端 UserDefaults 的跨平台对应物：
/// Windows %APPDATA%\AgentIsland\settings.json，macOS ~/Library/Application Support/…）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub appearance: String,     // system | light | dark
    pub dock_edge: String,      // top | bottom | left | right
    pub dock_anchor: f64,       // 0..1
    pub collapse_delay: f64,    // 秒
    pub sample_interval: f64,   // 秒
    pub cpu_threshold: f64,     // %
    pub token_alert_enabled: bool,
    pub token_alert_threshold: i64,
    pub notification_policy: String, // standard | focus | silent
    pub play_completion_sound: bool,
    pub disabled_agents: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            appearance: "system".into(),
            dock_edge: "top".into(),
            dock_anchor: 0.5,
            collapse_delay: 0.5,
            sample_interval: 2.0,
            cpu_threshold: 6.0,
            token_alert_enabled: true,
            token_alert_threshold: 200_000,
            notification_policy: "standard".into(),
            play_completion_sound: true,
            disabled_agents: vec![],
        }
    }
}

pub fn config_dir() -> PathBuf {
    let mut dir = dirs::data_dir().unwrap_or_else(std::env::temp_dir);
    dir.push("AgentIsland");
    dir
}

impl Settings {
    pub fn load() -> Self {
        let path = config_dir().join("settings.json");
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(s) = serde_json::from_str::<Settings>(&text) {
                return s.normalized();
            }
        }
        Settings::default()
    }

    pub fn save(&self) {
        let dir = config_dir();
        let _ = fs::create_dir_all(&dir);
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(dir.join("settings.json"), json);
        }
    }

    /// 脏值钳制（与 macOS EngineConfig.normalized() 同规则）
    pub fn normalized(&self) -> Self {
        let mut s = self.clone();
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
        s.collapse_delay = clamp(s.collapse_delay, 0.2, 30.0);
        if s.sample_interval < 0.5 {
            s.sample_interval = 0.5;
        }
        s.token_alert_threshold = s.token_alert_threshold.clamp(1_000, 10_000_000);
        s.dock_anchor = s.dock_anchor.clamp(0.0, 1.0);
        s
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
        assert!(s.disabled_agents.is_empty(), "缺禁用列表应落默认值");
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
        hi.sample_interval = 1e9;
        hi.collapse_delay = 1e9;
        hi.dock_anchor = 1e9;
        hi.token_alert_threshold = i64::MAX;
        let n = hi.normalized();

        assert_eq!(n.cpu_threshold, 50.0, "cpu_threshold 上限");
        assert_eq!(n.sample_interval, 600.0, "sample_interval 上限");
        assert_eq!(n.collapse_delay, 30.0, "collapse_delay 上限");
        assert_eq!(n.dock_anchor, 1.0, "dock_anchor 上限");
        assert_eq!(n.token_alert_threshold, 10_000_000, "token_alert_threshold 上限");
    }

    /// `normalized()` 是幂等的：钳一次和钳两次必须一样。
    /// 非幂等意味着每次 `load()` 都会再夹一次，用户的合法值会被慢慢推离他设的那个数。
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
}
