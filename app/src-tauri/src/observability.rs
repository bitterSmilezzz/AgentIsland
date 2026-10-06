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

#[derive(Clone)]
pub struct Evidence {
    pub level: ActivityLevel,
    pub process_running: bool,
    /// `None` means installation was not checked; it must not imply absence.
    pub installed: Option<bool>,
    /// 这一拍的状态是谁说的。与 [`crate::selfreport::Provenance`] 同枚举——
    /// 自报不是会话强语义，把它写成「本轮有活动信号」等于拿别人的证据给自己的结论背书。
    pub provenance: Option<crate::selfreport::Provenance>,
    /// 本轮的活跃会话数（真口径：窗口内有写入的会话文件数）。
    ///
    /// 此前 Rust 侧用 `recent_session_write: bool` 顶替它——**文件新鲜度与活跃会话
    /// 不是同一个量**：一次写入只证明「它动过」，不证明「有几场会话在跑」。
    /// 而 `noLocalData` 判定的输入正是这个数，所以它必须是真的。
    pub active_sessions: usize,
    /// 本轮「会话源读不到」的**原文**（`SessionProbeHealth.diagnostic_text`）。
    ///
    /// 此前这里是个 `bool`，只覆盖「元数据/列举失败」与「路径不是目录」，
    /// 深层文件读取与解析失败仍被吞掉——而 `None` 与 `Some(false)` 在界面上
    /// 长得一模一样，`doctor` 也会照着说「结论可信」。
    pub probe_health: Option<String>,
    /// 旧故障是否还有效。超过保质期就当它已经过去：挂着一条几小时前的
    /// 「读不到」而源早已修好，那是**另一种谎报**。
    pub probe_health_fresh: bool,
    pub has_local_detail_source: bool,
    pub has_token_usage: bool,
}

pub fn evaluate(e: &Evidence) -> Verdict {
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
    if e.probe_health.is_some() && e.probe_health_fresh {
        return Verdict::new(
            Code::BlindSessionSource,
            e.probe_health.clone().unwrap_or_default(),
        );
    }
    if e.active_sessions == 0 && !e.has_token_usage {
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
    Verdict::new(Code::Observed, format!("活跃会话 {} 个", e.active_sessions))
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
            probe_health: None,
            probe_health_fresh: false,
            has_local_detail_source: true,
            active_sessions: 0,
            has_token_usage: false,
        }
    }

    /// 探测层给的原文。抽成常量是因为下面两条用例都要用它，
    /// 而「界面逐字用它」正是要钉住的那件事。
    const HEALTHY_TEXT: &str =
        "会话源不可读：会话文件无法读取（/tmp/x.jsonl）——此后的「待机」只代表没有读到信号，不代表智能体真的空闲";

    #[test]
    fn five_verdicts_follow_the_swift_precedence_without_guessing_installation() {
        let mut e = input();
        assert_eq!(evaluate(&e).code, Code::NoLocalData);
        e.probe_health = Some(HEALTHY_TEXT.into());
        e.probe_health_fresh = true;
        let blind = evaluate(&e);
        assert_eq!(blind.code, Code::BlindSessionSource);
        assert_eq!(
            blind.evidence,
            vec![HEALTHY_TEXT.to_string()],
            "依据必须**逐字**用探测层给的那句，界面不自己再编一句"
        );
        // 陈旧的故障不参与判定：源早已修好却挂着一小时前的「读不到」，
        // 那是另一种谎报
        e.probe_health_fresh = false;
        assert_eq!(evaluate(&e).code, Code::NoLocalData, "过期的那条不算数");
        e.probe_health_fresh = true;
        e.probe_health = None;
        e.has_local_detail_source = false;
        assert_eq!(evaluate(&e).code, Code::SourceNotWired);
        e.active_sessions = 2;
        assert_eq!(evaluate(&e).code, Code::Observed);
        e.active_sessions = 0;
        e.has_token_usage = true;
        assert_eq!(evaluate(&e).code, Code::Observed);
        e.has_token_usage = false;
        e.level = ActivityLevel::Attention;
        assert_eq!(evaluate(&e).code, Code::Observed);
        e.level = ActivityLevel::Idle;
        e.process_running = false;
        assert_eq!(
            evaluate(&e).code,
            Code::Observed,
            "unknown install state is not absent"
        );
        e.installed = Some(false);
        assert_eq!(evaluate(&e).code, Code::NotInstalled);
    }

    /// `noLocalData` 的输入是**活跃会话数**，不是「文件动过没有」。
    ///
    /// 防的是一个具体的分叉：拿 `recent_write` 当代理时，
    /// 「这个 Agent 十分钟内写过会话文件、但此刻没有活跃会话」会被判成
    /// 「有活跃会话 ⇒ 结论可信」——而真实情况是它只是被动写过。
    #[test]
    fn the_no_local_data_verdict_keys_off_active_sessions_not_on_freshness() {
        let mut e = input();
        // 没有活跃会话 ⇒ 无本地明细
        assert_eq!(evaluate(&e).code, Code::NoLocalData);
        // 有活跃会话 ⇒ 结论可信，**依据里要写出那个数**
        e.active_sessions = 3;
        let verdict = evaluate(&e);
        assert_eq!(verdict.code, Code::Observed);
        assert_eq!(verdict.evidence, vec!["活跃会话 3 个"]);
        // 活跃会话 0 但历史有用量 ⇒ 仍算可信（有过使用是真的）
        e.active_sessions = 0;
        e.has_token_usage = true;
        assert_eq!(evaluate(&e).code, Code::Observed);
    }

    #[test]
    fn a_self_report_is_credited_to_the_self_report_not_to_observation() {
        // 防的症状：`selfreport` 落地之后，这一拍的状态其实来自带令牌的自报，
        // 而证据却写「本轮读到了会话强语义」——那是拿别人的证据给自己的结论背书。
        let mut e = input();
        e.level = ActivityLevel::Working;
        e.provenance = Some(crate::selfreport::Provenance::SelfReported);
        let verdict = evaluate(&e);
        assert_eq!(verdict.code, Code::Observed);
        assert_eq!(verdict.evidence, vec!["状态由带令牌、TTL 内的自报确认"]);

        e.provenance = Some(crate::selfreport::Provenance::Observed);
        assert_eq!(
            evaluate(&e).evidence,
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
            evaluate(&e).code,
            Code::Observed,
            "未核实安装状态时不能宣布未安装"
        );
        e.installed = Some(true);
        assert_eq!(evaluate(&e).code, Code::Observed);
        e.installed = Some(false);
        let verdict = evaluate(&e);
        assert_eq!(verdict.code, Code::NotInstalled);
        assert_eq!(verdict.summary, "未安装：不该期待状态");
    }

    #[test]
    fn outbound_codes_and_evidence_are_stable() {
        let verdict = evaluate(&input());
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
            token_alert_floor: None,
            session_dialect: crate::models::SessionDialect::GenericTail,
            session_database: None,
            category: "assistant".into(),
        };
        assert!(has_unreadable_source(&profile));
        fs::remove_file(&path).unwrap();
        assert!(!has_unreadable_source(&profile));
    }
}

#[cfg(test)]
mod local_detail_tests {
    use super::*;
    use crate::models::AgentProfile;

    fn profile(id: &str, dirs: Vec<&str>, roots: Vec<&str>) -> AgentProfile {
        AgentProfile {
            id: id.into(),
            name: id.into(),
            glyph: "x".into(),
            emoji: "x".into(),
            process_names: vec![],
            bundle_ids: vec![],
            cmdline_hints: vec![],
            path_contains: vec![],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: dirs.into_iter().map(String::from).collect(),
            token_roots: roots.into_iter().map(String::from).collect(),
            token_alert_floor: None,
            session_dialect: crate::models::SessionDialect::GenericTail,
            session_database: None,
            category: "assistant".into(),
        }
    }

    /// 「有没有本地明细源」是一条**关于证据的断言**——它会出现在
    /// 「结论可信吗」那句话里。判错的后果是把「读不到」说成「没有明细」。
    #[test]
    fn having_any_declared_source_means_the_claim_is_honest() {
        assert!(has_local_detail_source(&profile("a", vec!["/x"], vec![])));
        assert!(has_local_detail_source(&profile("b", vec![], vec!["/y"])));
        assert!(
            !has_local_detail_source(&profile("c", vec![], vec![])),
            "两处都没声明 ⇒ 不能声称有本地明细源"
        );
    }

    /// **「声明了」不等于「读得到」**：目录可能压根不存在。
    ///
    /// 这一条要分清是因为它与 `is_observable` 的其它判据方向相反——
    /// 那边是「读不到就不给结论」，而这条只回答「有没有声明过来源」。
    /// 把两者混起来会得到一句既不真也不假的「本地无明细」。
    #[test]
    fn a_declared_but_absent_directory_still_counts_as_a_source() {
        let absent = profile("d", vec!["/definitely/not/here/xyz"], vec![]);
        assert!(
            has_local_detail_source(&absent),
            "这条只回答「有没有声明过来源」，不回答「现在读不读得到」"
        );
        // `evaluate` 那条路才产出「可不可观测」的结论。
        // 声明了来源但一条都没读到时，结论必须落到 `NoLocalData`——
        // 「有来源」与「有数据」是两件事，把前者当成后者就会说出一句谎话。
        let verdict = evaluate(&Evidence {
            // **必须是非空闲态的对面**：level 若是 Working / Attention / Completed，
            // `evaluate` 会在更早一支直接判「已观测」而根本走不到这条规则。
            // 这也是为什么这条断言要放在最后 —— 它钉的是「空闲 + 声明了来源」这一格。
            level: crate::models::ActivityLevel::Idle,
            process_running: true,
            installed: Some(true),
            provenance: None,
            active_sessions: 0,
            probe_health: None,
            probe_health_fresh: true,
            has_local_detail_source: true,
            has_token_usage: false,
        });
        assert_eq!(
            verdict.code,
            Code::NoLocalData,
            "声明了来源却没有任何数据时，不该说成「已观测」"
        );
    }
}

/// `has_unreadable_source` 与 `has_local_detail_source` **成对**：
/// 一条问「声明过本地明细源吗」，一条问「读得到吗」。
/// 只钉一条是半截——而这一对合起来才回答「本机这份数据能不能信」。
///
/// 上一版补了 `has_local_detail_source`（它回答「有没有声明」），
/// 这一条一直没动，于是这个判断链上留了个口子。
#[cfg(test)]
mod unreadable_source_tests {
    use super::*;
    use crate::testutil::Sandbox;

    fn profile_with(dirs: Vec<String>) -> AgentProfile {
        AgentProfile {
            id: "fixture".into(),
            name: "Fixture".into(),
            glyph: String::new(),
            emoji: String::new(),
            process_names: vec![],
            bundle_ids: vec![],
            cmdline_hints: vec![],
            path_contains: vec![],
            path_excludes: vec![],
            cpu_floor: None,
            session_dirs: dirs,
            token_roots: vec![],
            token_alert_floor: None,
            session_dialect: crate::models::SessionDialect::GenericTail,
            session_database: None,
            category: "assistant".into(),
        }
    }

    /// **没装过 ≠ 读不到。**
    ///
    /// 一个压根不存在的可选目录**不是**权限失败（那是 `NotFound`）。
    /// 把它算成「读不到」会让「这个 Agent 从没在这台机器上跑过」显示成一个故障——
    /// 而这两种事在界面上完全不像。
    #[test]
    fn a_missing_optional_directory_is_not_an_access_failure() {
        let sandbox = Sandbox::new("unreadable-missing");
        let absent = sandbox.path().join("never-created");
        let profile = profile_with(vec![absent.to_string_lossy().into_owned()]);
        assert!(
            !has_unreadable_source(&profile),
            "不存在的目录不是权限失败；判错会让「没装过」显示成「读不到」"
        );
    }

    /// 路径存在但**不是目录** ⇒ 读不到（解析器会拿到一个文件而不是目录）。
    #[test]
    fn a_path_that_is_a_file_counts_as_unreadable() {
        let sandbox = Sandbox::new("unreadable-file");
        let file = sandbox.path().join("a-file");
        std::fs::write(&file, b"not a directory").unwrap();
        let profile = profile_with(vec![file.to_string_lossy().into_owned()]);
        assert!(
            has_unreadable_source(&profile),
            "存在却不是目录 ⇒ 明细源读不到"
        );
    }

    /// 存在且能列 ⇒ 读得到。
    #[test]
    fn an_existing_readable_directory_is_not_unreadable() {
        let sandbox = Sandbox::new("unreadable-ok");
        let dir = sandbox.path().join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jsonl"), b"{}").unwrap();
        let profile = profile_with(vec![dir.to_string_lossy().into_owned()]);
        assert!(!has_unreadable_source(&profile));
    }

    /// **一个**读不到就够 ⇒ 任何一个目录坏了就报读不到。
    #[test]
    fn one_bad_directory_among_good_ones_is_enough() {
        let sandbox = Sandbox::new("unreadable-mixed");
        let good = sandbox.path().join("sessions");
        std::fs::create_dir_all(&good).unwrap();
        let bad = sandbox.path().join("a-file");
        std::fs::write(&bad, b"x").unwrap();
        let profile = profile_with(vec![
            good.to_string_lossy().into_owned(),
            bad.to_string_lossy().into_owned(),
        ]);
        assert!(has_unreadable_source(&profile), "混着一个坏的 ⇒ 整体读不到");

        // 反过来也成立：一个坏的都没有 ⇒ 读得到
        let ok = profile_with(vec![good.to_string_lossy().into_owned()]);
        assert!(!has_unreadable_source(&ok));
    }

    /// 什么都没声明 ⇒ 两条都别说话（不能把「没配置」说成「读不到」）。
    #[test]
    fn an_agent_with_no_declared_dirs_is_neither_unreadable_nor_a_source() {
        let profile = profile_with(vec![]);
        assert!(!has_unreadable_source(&profile));
        assert!(!has_local_detail_source(&profile));
    }
}
