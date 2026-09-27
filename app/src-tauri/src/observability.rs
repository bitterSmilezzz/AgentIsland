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
    pub evidence: Vec<String>,
}

impl Verdict {
    fn new(code: Code, evidence: impl Into<String>) -> Self {
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
            evidence: vec![evidence.into()],
        }
    }
}

#[derive(Clone, Copy)]
pub struct Evidence {
    pub level: ActivityLevel,
    pub process_running: bool,
    /// `None` means installation was not checked; it must not imply absence.
    pub installed: Option<bool>,
    /// 这一拍的状态是谁说的。与 [`crate::selfreport::Provenance`] 同枚举——
    /// 自报不是会话强语义，把它写成「本轮有活动信号」等于拿别人的证据给自己的结论背书。
    pub provenance: Option<crate::selfreport::Provenance>,
    pub source_unreadable: bool,
    pub has_local_detail_source: bool,
    pub recent_session_write: bool,
    pub has_token_usage: bool,
}

pub fn evaluate(e: Evidence) -> Verdict {
    if !e.process_running {
        return if e.installed == Some(false) {
            Verdict::new(Code::NotInstalled, "PATH 与 /Applications 均未发现该智能体")
        } else {
            Verdict::new(
                Code::Observed,
                "进程不在，离线结论来自进程表；安装状态尚未核实",
            )
        };
    }
    // Same precedence as Swift: non-idle activity has direct work evidence —
    // **but say who said it**. 自报与观测是两种来路，把前者写成后者就是伪造出处。
    if matches!(
        e.level,
        ActivityLevel::Attention | ActivityLevel::Completed | ActivityLevel::Working
    ) {
        return if e.provenance == Some(crate::selfreport::Provenance::SelfReported) {
            Verdict::new(Code::Observed, "状态由带令牌、TTL 内的自报确认")
        } else {
            Verdict::new(
                Code::Observed,
                format!("本轮读到了会话强语义（{}）", e.level.label()),
            )
        };
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

impl Code {
    /// 文本出口（CSV/报告）用的写法。**必须与 serde 的 camelCase 表示同值**——
    /// 否则「用脚本核对报告」的人会发现自己拿到的字段名与 JSON 对不上。
    /// 有一条用例专门把两者逐个比过。
    pub fn as_str(self) -> &'static str {
        match self {
            Code::Observed => "observed",
            Code::BlindSessionSource => "blindSessionSource",
            Code::NoLocalData => "noLocalData",
            Code::SourceNotWired => "sourceNotWired",
            Code::NotInstalled => "notInstalled",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> Evidence {
        Evidence {
            level: ActivityLevel::Idle,
            process_running: true,
            installed: None,
            provenance: None,
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
    fn a_self_report_is_credited_to_the_self_report_not_to_observation() {
        // 防的症状：`selfreport` 落地之后，这一拍的状态其实来自带令牌的自报，
        // 而证据却写「本轮读到了会话强语义」——那是拿别人的证据给自己的结论背书。
        let mut e = input();
        e.level = ActivityLevel::Working;
        e.provenance = Some(crate::selfreport::Provenance::SelfReported);
        let verdict = evaluate(e);
        assert_eq!(verdict.code, Code::Observed);
        assert_eq!(verdict.evidence, vec!["状态由带令牌、TTL 内的自报确认"]);

        e.provenance = Some(crate::selfreport::Provenance::Observed);
        assert_eq!(
            evaluate(e).evidence,
            vec!["本轮读到了会话强语义（工作中）"],
            "有会话信号时依据要写明是哪一态"
        );
    }

    #[test]
    fn not_installed_needs_a_proven_absence_not_merely_an_unrunning_process() {
        // 进程不在只说明「离线」；要宣布「未安装」必须有装机的否定证据。
        let mut e = input();
        e.level = ActivityLevel::Offline;
        e.process_running = false;
        e.installed = None;
        assert_eq!(
            evaluate(e).code,
            Code::Observed,
            "未核实安装状态时不能宣布未安装"
        );
        e.installed = Some(true);
        assert_eq!(evaluate(e).code, Code::Observed);
        e.installed = Some(false);
        let verdict = evaluate(e);
        assert_eq!(verdict.code, Code::NotInstalled);
        assert_eq!(verdict.summary, "未安装：不该期待状态");
    }

    #[test]
    fn outbound_codes_and_evidence_are_stable() {        let verdict = evaluate(input());
        let json = serde_json::to_value(verdict).unwrap();
        assert_eq!(json["code"], "noLocalData");
        assert_eq!(json["summary"], "无本地明细：读不到会话与用量");
        assert!(json["evidence"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty()));
    }

    #[test]
    fn existing_file_instead_of_session_directory_is_unreadable() {
        // 沙箱取名统一走 `testutil`（它保证并行时也不撞名，并负责清理）
        let sandbox = crate::testutil::Sandbox::new("observability");
        let path = sandbox.path().join("not-a-directory");
        fs::write(&path, b"synthetic").unwrap();
        let profile = AgentProfile {
            id: "fixture".into(),
            name: "Fixture".into(),
            glyph: String::new(),
            emoji: String::new(),
            process_names: vec![],
            bundle_ids: vec![],
            cmdline_hints: vec![],
            path_excludes: vec![],
            path_contains: vec![],
            cpu_floor: None,
            session_dirs: vec![path.to_string_lossy().into_owned()],
            token_roots: vec![],
            session_database: None,
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
