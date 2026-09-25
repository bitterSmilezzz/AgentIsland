use serde::{Deserialize, Serialize};
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
