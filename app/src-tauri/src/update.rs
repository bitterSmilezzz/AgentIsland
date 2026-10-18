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
}
