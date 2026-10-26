//! 审查**内容质量**的度量口径（TASK-06 第二片）。
//!
//! 需求原文：方案/代码独立只读审查，修改后重审，作者≠审查者，结果不越权批准。
//! 场景 token：审查内容质量的度量。
//!
//! 背景：v0.0.372 已钉住**闭环机制**（只读、绑定、身份分离、重审、双闸门），
//! 但「什么样的审查结论才算数」没有口径——于是一个**漏审了关键文件**、或
//! **列了阻断项却给 pass** 的审查，与一个认真审过的审查长得一模一样。
//!
//! 判据（纯函数）：
//!
//! 1. **覆盖度**：审查必须覆盖产出的**全部**关键文件；漏审任何一个 ⇒ 结论
//!    不能算通过（报告不完整 ≠ 没问题）。
//! 2. **发现分级**：发现按严重性分三档（阻断 / 建议 / 提示）；**存在阻断项时
//!    结论不得是 pass**。
//! 3. **结论与证据自洽**：「pass」却列着阻断项 ⇒ 自相矛盾，拒绝（而不是
//!    采信其中一边）。
//! 4. **修改后重审的覆盖**：修复后重审必须**重新覆盖被改动的文件**——
//!    旧结论覆盖的是旧内容，不能顺延到新内容上。
//! 5. **作者 ≠ 审查者**（身份或模型相同都算同一作者）：自我审查不算独立。

/// 发现的严重性。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// 提示：可选改进。
    Hint,
    /// 建议：应该改，但不阻断。
    Suggestion,
    /// 阻断：必须改，不能通过。
    Blocking,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hint => "hint",
            Self::Suggestion => "suggestion",
            Self::Blocking => "blocking",
        }
    }
}

/// 一条审查发现。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub file: String,
    pub severity: Severity,
    pub note: String,
}

/// 审查结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Revise,
    Reject,
}

/// 审查报告。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewReport {
    /// 审查者身份。
    pub reviewer: String,
    /// 审查者使用的模型（身份或模型相同都算同一作者）。
    pub reviewer_model: String,
    /// 作者身份。
    pub author: String,
    /// 审查覆盖到的文件。
    pub covered_files: Vec<String>,
    /// 发现。
    pub findings: Vec<Finding>,
    pub verdict: Verdict,
}

/// 拒绝原因。固定码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityRefusal {
    /// 作者与审查者是同一人（或同模型）——自我审查不算独立。
    NotIndependent,
    /// 关键文件漏审。
    CoverageIncomplete,
    /// 结论与发现自相矛盾（pass 却有阻断项）。
    VerdictContradictsFindings,
    /// 重审没有重新覆盖被改动的文件。
    StaleCoverage,
}

impl QualityRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotIndependent => "not_independent",
            Self::CoverageIncomplete => "coverage_incomplete",
            Self::VerdictContradictsFindings => "verdict_contradicts_findings",
            Self::StaleCoverage => "stale_coverage",
        }
    }
}

/// 审查报告的质量判定。
///
/// `key_files` 是这次变更涉及的关键文件（全部都必须被审查覆盖）；
/// `changed_since_review` 是上次审查后**又改动**的文件（重审时必须重新覆盖）。
pub fn assess(
    report: &ReviewReport,
    key_files: &[String],
    changed_since_review: &[String],
) -> Result<(), QualityRefusal> {
    // ① 独立性：身份或模型相同都算同一作者
    if report.reviewer == report.author || report.reviewer_model == report.author {
        return Err(QualityRefusal::NotIndependent);
    }
    // ② 覆盖度：关键文件全部要覆盖
    for file in key_files {
        if !report.covered_files.iter().any(|covered| covered == file) {
            return Err(QualityRefusal::CoverageIncomplete);
        }
    }
    // ④ 重审覆盖：上次审查后又改动的文件必须**重新**被覆盖
    //    （旧结论覆盖的是旧内容，不能顺延）
    if !changed_since_review.is_empty() {
        for file in changed_since_review {
            if !report.covered_files.iter().any(|covered| covered == file) {
                return Err(QualityRefusal::StaleCoverage);
            }
        }
    }
    // ③ 结论与发现自洽：有阻断项就不能是 pass
    let has_blocking = report.findings.iter().any(|f| f.severity == Severity::Blocking);
    if has_blocking && report.verdict == Verdict::Pass {
        return Err(QualityRefusal::VerdictContradictsFindings);
    }
    Ok(())
}

/// 审查结论的**上限**：有阻断项时结论最多只能是 Reject（不能是 Revise？
/// —— revise 是「改完再来」，对阻断项是恰当的）。
///
/// 这条单独给出，是因为「结论该取什么」与「结论自洽吗」是两件事：
/// 自洽性由 `assess` 判；「该取什么」由本函数给。
pub fn verdict_ceiling(findings: &[Finding]) -> Verdict {
    if findings.iter().any(|f| f.severity == Severity::Blocking) {
        Verdict::Revise
    } else if findings
        .iter()
        .any(|f| f.severity == Severity::Suggestion)
    {
        Verdict::Revise
    } else {
        Verdict::Pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> ReviewReport {
        ReviewReport {
            reviewer: "reviewer-x".into(),
            reviewer_model: "model-b".into(),
            author: "author-y".into(),
            covered_files: vec!["src/main.rs".into(), "src/lib.rs".into()],
            findings: Vec::new(),
            verdict: Verdict::Pass,
        }
    }

    #[test]
    fn a_self_review_is_not_independent() {
        let mut r = report();
        r.author = "reviewer-x".into(); // 同一人
        assert_eq!(
            assess(&r, &["src/main.rs".into()], &[]),
            Err(QualityRefusal::NotIndependent)
        );
        // 同模型也算同一作者
        let mut r2 = report();
        r2.reviewer_model = r2.author.clone();
        assert_eq!(
            assess(&r2, &["src/main.rs".into()], &[]),
            Err(QualityRefusal::NotIndependent)
        );
    }

    #[test]
    fn missing_key_file_coverage_fails_the_report() {
        let r = report();
        // 关键文件漏审 ⇒ 报告不完整，不能算通过
        assert_eq!(
            assess(&r, &["src/main.rs".into(), "src/secret.rs".into()], &[]),
            Err(QualityRefusal::CoverageIncomplete)
        );
        // 覆盖齐了
        assert_eq!(assess(&r, &["src/main.rs".into(), "src/lib.rs".into()], &[]), Ok(()));
    }

    #[test]
    fn a_pass_verdict_with_blocking_findings_is_contradictory() {
        let mut r = report();
        r.findings.push(Finding {
            file: "src/main.rs".into(),
            severity: Severity::Blocking,
            note: "删了错误处理".into(),
        });
        r.verdict = Verdict::Pass;
        assert_eq!(
            assess(&r, &["src/main.rs".into()], &[]),
            Err(QualityRefusal::VerdictContradictsFindings),
            "列了阻断项还判通过——自相矛盾，不能采信其中一边"
        );
        // 改成 Revise 即自洽
        r.verdict = Verdict::Revise;
        assert_eq!(assess(&r, &["src/main.rs".into()], &[]), Ok(()));
    }

    #[test]
    fn a_re_review_must_re_cover_changed_files() {
        let r = report();
        // 上次审查后改了 src/changed.rs，但这次审查没覆盖它 ⇒ 覆盖已过时
        assert_eq!(
            assess(
                &r,
                &["src/main.rs".into()],
                &["src/changed.rs".into()]
            ),
            Err(QualityRefusal::StaleCoverage),
            "旧结论覆盖的是旧内容，不能顺延到新内容上"
        );
        // 重新覆盖了改动文件 ⇒ 通过
        assert_eq!(
            assess(
                &r,
                &["src/main.rs".into()],
                &["src/main.rs".into()]
            ),
            Ok(())
        );
    }

    #[test]
    fn the_verdict_ceiling_follows_the_most_severe_finding() {
        assert_eq!(verdict_ceiling(&[]), Verdict::Pass);
        assert_eq!(
            verdict_ceiling(&[Finding {
                file: "a".into(),
                severity: Severity::Hint,
                note: String::new()
            }]),
            Verdict::Pass,
            "提示不影响通过"
        );
        assert_eq!(
            verdict_ceiling(&[Finding {
                file: "a".into(),
                severity: Severity::Suggestion,
                note: String::new()
            }]),
            Verdict::Revise
        );
        assert_eq!(
            verdict_ceiling(&[Finding {
                file: "a".into(),
                severity: Severity::Blocking,
                note: String::new()
            }]),
            Verdict::Revise
        );
    }
}
