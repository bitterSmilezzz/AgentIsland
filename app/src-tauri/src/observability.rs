//! One verdict for whether an Agent's displayed state has local evidence.
//! Keep unavailable data distinct from an observed zero or an observed idle.

use crate::models::{ActivityLevel, AgentProfile};
use serde::Serialize;
use std::fs;
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Code {
    Observed,
    BlindSessionSource,
    NoLocalData,
    SourceNotWired,
    NotInstalled,
}

#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub code: Code,
    pub summary: &'static str,
    pub evidence: Vec<&'static str>,
}

impl Verdict {
    fn new(code: Code, evidence: &'static str) -> Self {
        let summary = match code {
            Code::Observed => "结论可信",
            Code::BlindSessionSource => "待机不可信：会话源读不到",
            Code::NoLocalData => "无本地明细：读不到会话与用量",
            Code::SourceNotWired => "未接入明细源",
            Code::NotInstalled => "未安装：不该期待状态",
        };
        Self {
            code,
            summary,
            evidence: vec![evidence],
        }
    }
}

#[derive(Clone, Copy)]
pub struct Evidence {
    pub level: ActivityLevel,
    pub process_running: bool,
    /// `None` means installation was not checked; it must not imply absence.
    pub installed: Option<bool>,
    pub source_unreadable: bool,
    pub has_local_detail_source: bool,
    pub recent_session_write: bool,
    pub has_token_usage: bool,
}

pub fn evaluate(e: Evidence) -> Verdict {
    if !e.process_running {
        return if e.installed == Some(false) {
            Verdict::new(Code::NotInstalled, "安装探测与进程表均未发现该智能体")
        } else {
            Verdict::new(
                Code::Observed,
                "进程不在，离线结论来自进程表；安装状态尚未核实",
            )
        };
    }
    // Same precedence as Swift: non-idle activity has direct work evidence.
    if matches!(
        e.level,
        ActivityLevel::Attention | ActivityLevel::Completed | ActivityLevel::Working
    ) {
        return Verdict::new(Code::Observed, "本轮有活动或任务状态信号");
    }
    if e.source_unreadable {
        return Verdict::new(
            Code::BlindSessionSource,
            "已登记的本地会话源无法读取；待机不代表真的空闲",
        );
    }
    if !e.recent_session_write && !e.has_token_usage {
        return if e.has_local_detail_source {
            Verdict::new(
                Code::NoLocalData,
                "已登记本地明细源，但没有近期会话写入或用量记录",
            )
        } else {
            Verdict::new(
                Code::SourceNotWired,
                "该档案未登记本地明细源，不代表它没在工作",
            )
        };
    }
    Verdict::new(Code::Observed, "有近期会话写入或历史用量记录")
}

/// Detect only proven access failures. A missing optional directory is not a
/// permission failure; the verdict may still become `NoLocalData`.
pub fn has_unreadable_source(profile: &AgentProfile) -> bool {
    profile
        .session_dirs
        .iter()
        .any(|root| match fs::metadata(root) {
            Ok(meta) if meta.is_dir() => fs::read_dir(root).is_err(),
            Ok(_) => true,
            Err(error) => error.kind() != io::ErrorKind::NotFound,
        })
}

pub fn has_local_detail_source(profile: &AgentProfile) -> bool {
    !profile.session_dirs.is_empty() || !profile.token_roots.is_empty()
}

pub fn recent_write(path: Option<&std::time::SystemTime>, now: std::time::SystemTime) -> bool {
    path.is_some_and(|mtime| {
        now.duration_since(*mtime)
            .map_or(true, |age| age.as_secs() < 600)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> Evidence {
        Evidence {
            level: ActivityLevel::Idle,
            process_running: true,
            installed: None,
            source_unreadable: false,
            has_local_detail_source: true,
            recent_session_write: false,
            has_token_usage: false,
        }
    }

    #[test]
    fn five_verdicts_follow_the_swift_precedence_without_guessing_installation() {
        let mut e = input();
        assert_eq!(evaluate(e).code, Code::NoLocalData);
        e.source_unreadable = true;
        assert_eq!(evaluate(e).code, Code::BlindSessionSource);
        e.source_unreadable = false;
        e.has_local_detail_source = false;
        assert_eq!(evaluate(e).code, Code::SourceNotWired);
        e.recent_session_write = true;
        assert_eq!(evaluate(e).code, Code::Observed);
        e.recent_session_write = false;
        e.has_token_usage = true;
        assert_eq!(evaluate(e).code, Code::Observed);
        e.has_token_usage = false;
        e.level = ActivityLevel::Attention;
        assert_eq!(evaluate(e).code, Code::Observed);
        e.level = ActivityLevel::Idle;
        e.process_running = false;
        assert_eq!(
            evaluate(e).code,
            Code::Observed,
            "unknown install state is not absent"
        );
        e.installed = Some(false);
        assert_eq!(evaluate(e).code, Code::NotInstalled);
    }

    #[test]
    fn outbound_codes_and_evidence_are_stable() {
        let verdict = evaluate(input());
        let json = serde_json::to_value(verdict).unwrap();
        assert_eq!(json["code"], "noLocalData");
        assert_eq!(json["summary"], "无本地明细：读不到会话与用量");
        assert!(json["evidence"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty()));
    }

    #[test]
    fn existing_file_instead_of_session_directory_is_unreadable() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "agentisland-observability-{}-{stamp}",
            std::process::id()
        ));
        fs::write(&path, b"synthetic").unwrap();
        let profile = AgentProfile {
            id: "fixture".into(),
            name: "Fixture".into(),
            glyph: String::new(),
            emoji: String::new(),
            process_names: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: vec![path.to_string_lossy().into_owned()],
            token_roots: vec![],
            category: "assistant".into(),
        };
        assert!(has_unreadable_source(&profile));
        fs::remove_file(&path).unwrap();
        assert!(!has_unreadable_source(&profile));
    }

    #[test]
    fn recent_write_uses_the_sampling_clock_and_accepts_future_mtime() {
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(10_000);
        assert!(!recent_write(None, now));
        assert!(recent_write(
            Some(&(now - std::time::Duration::from_secs(599))),
            now
        ));
        assert!(!recent_write(
            Some(&(now - std::time::Duration::from_secs(600))),
            now
        ));
        assert!(recent_write(
            Some(&(now + std::time::Duration::from_secs(1))),
            now
        ));
    }
}
