//! 应用自身的检查更新与自动更新（NOT-06）。
//!
//! 三条不可动摇的规则，写在这里而不是散在界面代码里：
//!
//! 1. **只有用户点了「检查更新」才出网**——不做后台轮询，也不在启动时偷偷检查；
//! 2. **只有用户点了「下载并安装」才安装**——`update_install` 不提供任何自动触发路径；
//! 3. **安装完成后不自动重启**——`update_restart` 必须由用户再点一次。
//!
//! 签名校验由 `tauri-plugin-updater` 负责，**关不掉**：它验的是「这个更新包确实是
//! 我们发布的」。校验失败必须拒绝安装，而不是提示用户「忽略风险继续」。
//!
//! 界面层（`app/ui/js/update-panel.js` + `update-state.js`）只消费这里的命令与
//! `update://progress` 事件；状态机在 JS 侧可被 node 直接测。

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// 一次检查的结果。`available=false` 时其余字段为空，**不伪造版本号**。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub current_version: String,
    pub available: bool,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub date: Option<String>,
}

/// 下载进度。`total` 未知时是 `None`——界面据此显示「已下载 X」而不是假装有百分比。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

// ── NOT-06：跳过版本判据（纯函数）────────────────────────────────────────
//
// 需求原文：旧版客户端实际安装升级、跳过版本与 Intel/universal 路径。
//
// 语义：
// 1. **版本比较严格**：三段 SemVer（major.minor.patch），数字比较而非字典序
//    （"0.0.10" > "0.0.9"）；畸形版本号拒绝（返回 None）。
// 2. **跳过是按版本粒度的**：用户选「跳过 0.0.380」，那么 0.0.380 永远不再提示；
//    但 **0.0.381 到达时必须照常提示**——跳过一个版本不是「关闭自动更新」。
// 3. **降级与同版本不提示**：目标 <= 当前 ⇒ 无论是跳过还是提示，都为 false。
// 4. **跳过旧版本被拒**：只能跳过**比当前更高**的版本（跳过旧版本无意义）。

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

pub fn parse_semver(text: &str) -> Option<SemVer> {
    let text = text.trim().strip_prefix('v').unwrap_or(text.trim());
    let mut parts = text.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(SemVer {
        major,
        minor,
        patch,
    })
}

/// 某次检查发现的更新是否应该向用户提示。
///
/// - `target`：检查到的最新版本；
/// - `current`：当前运行版本；
/// - `skipped`：用户之前选过跳过的版本（`None` = 未跳过任何版本）。
pub fn should_prompt_update(target: &str, current: &str, skipped: Option<&str>) -> bool {
    let (Some(target_ver), Some(current_ver)) = (parse_semver(target), parse_semver(current))
    else {
        return false;
    };
    // 目标不高于当前 ⇒ 降级/同版本，不提示
    if target_ver <= current_ver {
        return false;
    }
    // 目标版本被用户明确跳过 ⇒ 不提示
    if let Some(skipped_raw) = skipped {
        if let Some(skipped_ver) = parse_semver(skipped_raw) {
            if target_ver == skipped_ver {
                return false;
            }
        }
    }
    true
}

/// 记录一次「跳过此版本」。只能跳过比当前高的版本。
pub fn can_skip_version(target: &str, current: &str) -> bool {
    let (Some(target_ver), Some(current_ver)) = (parse_semver(target), parse_semver(current))
    else {
        return false;
    };
    target_ver > current_ver
}

/// 当前版本。**不出网**，供设置页在用户点击之前显示。
#[tauri::command]
pub fn update_current_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// 只回答「有没有可用更新」，不改动任何文件、不下载。
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<UpdateStatus, String> {
    let current_version = app.package_info().version.to_string();
    let updater = app
        .updater()
        .map_err(|error| updater_reason(&error.to_string()))?;
    let found = updater
        .check()
        .await
        .map_err(|error| updater_reason(&error.to_string()))?;
    Ok(match found {
        Some(update) => UpdateStatus {
            current_version,
            available: true,
            version: Some(update.version.clone()),
            notes: update.body.clone(),
            date: update.date.map(|date| date.to_string()),
        },
        None => UpdateStatus {
            current_version,
            available: false,
            version: None,
            notes: None,
            date: None,
        },
    })
}

/// 下载并安装；**不重启**。进度通过 `update://progress` 事件回报。
///
/// 安装前重新 check 一次：用户可能已经等了一会儿，期间又发了新版本。用最新那次
/// 的结果，避免把「检查时看到的版本」和「实际安装的版本」对不上。
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<(), String> {
    let updater = app
        .updater()
        .map_err(|error| updater_reason(&error.to_string()))?;
    let update = updater
        .check()
        .await
        .map_err(|error| updater_reason(&error.to_string()))?
        .ok_or_else(|| "没有可用更新".to_string())?;
    let emitter = app.clone();
    update
        .download_and_install(
            move |downloaded, total| {
                let _ = emitter.emit(
                    "update://progress",
                    UpdateProgress {
                        downloaded: downloaded as u64,
                        total,
                    },
                );
            },
            || {},
        )
        .await
        .map_err(|error| updater_reason(&error.to_string()))
}

/// 安装完成后由用户显式触发。这里**只做重启**，不在这里做任何检查或安装。
#[tauri::command]
pub fn update_restart(app: AppHandle) {
    app.restart()
}

/// 把插件错误收敛成一句用户可读、且**不含路径/URL/正文**的话。
///
/// 原始错误可能带下载地址与本地路径；直接透传到界面等于把内部结构暴露给用户，
/// 也让「错误文案」变成第二处需要维护的契约。这里只保留可行动的类别。
fn updater_reason(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("signature") || lower.contains("verify") || lower.contains("minisign") {
        "更新包签名校验失败，已拒绝安装".to_string()
    } else if lower.contains("timed out") || lower.contains("timeout") {
        "连接更新服务超时，请稍后重试".to_string()
    } else if lower.contains("404") || lower.contains("not found") {
        "更新清单不存在（可能尚未发布）".to_string()
    } else if lower.contains("network")
        || lower.contains("connect")
        || lower.contains("dns")
        || lower.contains("tls")
    {
        "网络不可用，无法连接更新服务".to_string()
    } else {
        "更新失败，请稍后重试".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serializes_with_camel_case_keys() {
        let status = UpdateStatus {
            current_version: "0.0.326".to_string(),
            available: true,
            version: Some("0.0.327".to_string()),
            notes: Some("修复若干问题".to_string()),
            date: Some("2026-10-19T00:00:00Z".to_string()),
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["currentVersion"], "0.0.326");
        assert_eq!(json["available"], true);
        assert_eq!(json["version"], "0.0.327");
        assert_eq!(json["notes"], "修复若干问题");
    }

    #[test]
    fn unavailable_status_carries_no_fake_version() {
        let status = UpdateStatus {
            current_version: "0.0.326".to_string(),
            available: false,
            version: None,
            notes: None,
            date: None,
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["available"], false);
        assert!(json["version"].is_null(), "没有更新时不得编一个版本号出来");
        assert!(json["notes"].is_null());
    }

    #[test]
    fn signature_failure_is_reported_as_refusal_not_as_a_retryable_network_error() {
        let reason = updater_reason("signature verification failed: invalid minisign signature");
        assert_eq!(reason, "更新包签名校验失败，已拒绝安装");
        assert!(
            !reason.contains("重试"),
            "签名不匹配重试多少次都一样，不能引导用户重试"
        );
    }

    #[test]
    fn transient_failures_are_actionable_and_do_not_leak_details() {
        let timeout = updater_reason("error sending request: operation timed out at https://example.invalid/latest.json");
        assert_eq!(timeout, "连接更新服务超时，请稍后重试");
        assert!(!timeout.contains("http"), "错误文案不得回显 URL");
        let missing = updater_reason("updater endpoint returned 404 Not Found");
        assert_eq!(missing, "更新清单不存在（可能尚未发布）");
        let offline = updater_reason("dns error: failed to lookup address information");
        assert_eq!(offline, "网络不可用，无法连接更新服务");
        let other = updater_reason("some internal failure with /Users/someone/path");
        assert_eq!(other, "更新失败，请稍后重试");
        assert!(!other.contains("/Users"), "错误文案不得回显本地路径");
    }

    #[test]
    fn progress_keeps_unknown_total_unknown() {
        let json = serde_json::to_value(UpdateProgress {
            downloaded: 1024,
            total: None,
        })
        .unwrap();
        assert_eq!(json["downloaded"], 1024);
        assert!(json["total"].is_null(), "总大小未知时不得编一个数字");
    }

    #[test]
    fn semver_parsing_and_numeric_comparison() {
        assert_eq!(
            parse_semver("0.0.389"),
            Some(SemVer {
                major: 0,
                minor: 0,
                patch: 389,
            })
        );
        assert_eq!(parse_semver("v1.2.3"), Some(SemVer { major: 1, minor: 2, patch: 3 }));
        // 数字比较，非字典序（"0.0.10" > "0.0.9"）
        assert!(parse_semver("0.0.10").unwrap() > parse_semver("0.0.9").unwrap());
        // 畸形拒绝
        assert_eq!(parse_semver("0.0"), None);
        assert_eq!(parse_semver("0.0.0.1"), None);
        assert_eq!(parse_semver("abc"), None);
    }

    #[test]
    fn skipped_version_is_suppressed_but_a_newer_one_still_prompts() {
        let current = "0.0.380";
        // 新版本无跳过 ⇒ 提示
        assert!(should_prompt_update("0.0.381", current, None));

        // 跳过 0.0.381 ⇒ 不提示该版本
        assert!(!should_prompt_update("0.0.381", current, Some("0.0.381")));

        // 更高版本 0.0.382 到达 ⇒ **照常提示**（跳过一个版本不是关闭更新）
        assert!(
            should_prompt_update("0.0.382", current, Some("0.0.381")),
            "新版本必须突破历史跳过限制"
        );

        // 降级与同版本 ⇒ 永远不提示
        assert!(!should_prompt_update("0.0.380", current, None));
        assert!(!should_prompt_update("0.0.379", current, None));

        // 畸形版本号 ⇒ 不提示（安全兜底）
        assert!(!should_prompt_update("invalid", current, None));
    }

    #[test]
    fn can_only_skip_versions_higher_than_current() {
        assert!(can_skip_version("0.0.381", "0.0.380"));
        assert!(!can_skip_version("0.0.380", "0.0.380"), "同版本不可跳");
        assert!(!can_skip_version("0.0.379", "0.0.380"), "旧版本不可跳");
    }
}
