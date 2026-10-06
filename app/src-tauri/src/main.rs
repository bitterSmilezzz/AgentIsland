// 始终隐藏控制台：日志统一走 %TEMP%gentisland-tauri.log（log_from_ui + panic hook）
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(target_os = "macos")]
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod atomicfile;
mod audit;
mod budget;
mod capabilities;
mod claude_hook_config;
mod claude_plan_capture;
mod claude_plan_receiver;
mod claude_plan_runtime;
mod cleaner;
mod cli;
mod connection_http;
mod connection_probe;
mod connections;
mod cost;
mod deeplink;
mod duration;
mod engine;
mod filemon;
mod forecast;
mod health;
mod im;
mod installed;
mod localclock;
mod mcp_config;
mod memory;
mod minimax;
mod models;
mod navigation;
mod notifier;
mod observability;
mod placement;
mod power;
mod private_text;
mod procmon;
mod prompts;
mod provider;
mod registry;
mod remote;
mod render;
mod report;
mod resilience;
mod resource_diagnostics;
mod secret;
mod selfreport;
mod selftest;
mod session;
mod session_catalog;
mod session_navigation;
mod settings;
mod skill_files;
mod skills_config;
#[cfg(unix)]
mod skills_package;
mod skills_picker;
mod smtp;
mod sqlite;
mod task_artifacts;
mod task_attention;
mod task_sources;
mod tasks;
#[cfg(test)]
mod testutil;
mod todos;
mod tokens;
mod transport;
mod trees;
mod webhook;
mod window_layout;
mod window_layout_execution;
mod window_layout_journal;
mod window_layout_native;
mod window_layout_readback;
mod window_layout_rules;
mod window_layout_service;
mod window_lifecycle;
mod window_smoke;
mod window_visibility;
mod workspace_journal;
mod workspaces;

use engine::ActivityEngine;
use models::DockEdge;
use settings::Settings;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State};

// Windows 端原先在这里 `use std::os::windows::process::CommandExt;`——
// **导入后全文件零引用**。macOS 上它被 `#[cfg(windows)]` 挡住，所以从不出现在
// 编译警告里；直到把 `placement.rs` 的 FFI 单独拿去做 Windows 目标编译才看见。
// 推断它对应一个被删掉的 `creation_flags` 调用（起子进程时不弹控制台窗口）。
// 这里**只删导入、不补行为**：Windows 上的进程拉起要不要设标志，属于待定需求，
// 不该由一条「顺手清警告」偷偷定下来。要加就连同调用点一起加。

type SharedEngine = Arc<Mutex<ActivityEngine>>;

type WindowLayoutStore = Mutex<window_layout_service::Store<window_layout_native::NativeWindow>>;
type WindowRuleStore = Mutex<window_layout_rules::Store>;
type ConnectionStore = Mutex<connections::Store>;
type WorkspaceStore = Mutex<workspaces::Store>;
type PromptStore = Mutex<prompts::Store>;
struct SkillPackageStore {
    #[cfg(unix)]
    inner: Mutex<skills_package::Store>,
}
impl SkillPackageStore {
    fn new() -> Self {
        Self {
            #[cfg(unix)]
            inner: Mutex::new(skills_package::Store::new(
                dirs::home_dir().unwrap_or_default(),
            )),
        }
    }
}

#[tauri::command]
async fn prompts_list(app: AppHandle) -> Result<prompts::List, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<PromptStore>()
            .lock()
            .map_err(|_| "提示词服务不可用")?
            .list()
    })
    .await
    .map_err(|_| "提示词读取未完成".to_string())?
}
#[tauri::command]
async fn prompt_save(
    app: AppHandle,
    id: Option<String>,
    draft: prompts::Draft,
    expected_revision: u64,
) -> Result<prompts::List, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<PromptStore>()
            .lock()
            .map_err(|_| "提示词服务不可用")?
            .save(id, draft, expected_revision)
    })
    .await
    .map_err(|_| "提示词保存未完成".to_string())?
}
#[tauri::command]
async fn prompt_remove(
    app: AppHandle,
    id: String,
    expected_revision: u64,
) -> Result<prompts::List, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<PromptStore>()
            .lock()
            .map_err(|_| "提示词服务不可用")?
            .remove(&id, expected_revision)
    })
    .await
    .map_err(|_| "提示词移除未完成".to_string())?
}
#[tauri::command]
async fn prompt_preview(
    app: AppHandle,
    target: Option<prompts::Client>,
    id: String,
    expected_revision: u64,
) -> Result<prompts::Preview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<PromptStore>()
            .lock()
            .map_err(|_| "提示词服务不可用")?
            .for_client(target.unwrap_or_default())?
            .preview(&id, expected_revision)
    })
    .await
    .map_err(|_| "指令预览未完成".to_string())?
}
#[tauri::command]
async fn prompt_apply(
    app: AppHandle,
    target: Option<prompts::Client>,
    id: String,
    expected_revision: u64,
    plan_id: String,
) -> Result<prompts::Applied, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<PromptStore>()
            .lock()
            .map_err(|_| "提示词服务不可用")?
            .for_client(target.unwrap_or_default())?
            .apply(&id, expected_revision, &plan_id)
    })
    .await
    .map_err(|_| "指令应用未完成".to_string())?
}
#[tauri::command]
async fn prompt_backups(
    app: AppHandle,
    target: Option<prompts::Client>,
) -> Result<Vec<prompts::Backup>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<PromptStore>()
            .lock()
            .map_err(|_| "提示词服务不可用")?
            .for_client(target.unwrap_or_default())?
            .backups()
    })
    .await
    .map_err(|_| "指令备份读取未完成".to_string())?
}
#[tauri::command]
async fn prompt_preview_restore(
    app: AppHandle,
    target: Option<prompts::Client>,
    name: String,
) -> Result<prompts::Preview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<PromptStore>()
            .lock()
            .map_err(|_| "提示词服务不可用")?
            .for_client(target.unwrap_or_default())?
            .preview_restore(&name)
    })
    .await
    .map_err(|_| "指令恢复预览未完成".to_string())?
}
#[tauri::command]
async fn prompt_restore(
    app: AppHandle,
    target: Option<prompts::Client>,
    name: String,
    plan_id: String,
) -> Result<prompts::Applied, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<PromptStore>()
            .lock()
            .map_err(|_| "提示词服务不可用")?
            .for_client(target.unwrap_or_default())?
            .restore(&name, &plan_id)
    })
    .await
    .map_err(|_| "指令恢复未完成".to_string())?
}

fn workspace_catalog_impl(app: &AppHandle) -> workspaces::Catalog {
    use workspaces::{Catalog, Choice};
    let mut catalog = Catalog {
        projects: vec![],
        profiles: vec![],
        layouts: vec![],
        tools: crate::registry::builtin()
            .into_iter()
            .map(|p| Choice {
                id: p.id,
                name: p.name,
                supported: true,
            })
            .collect(),
        errors: vec![],
    };
    match app
        .state::<TaskStore>()
        .lock()
        .ok()
        .and_then(|s| s.load().ok())
    {
        Some(d) if d.projects.len() <= 200 => {
            catalog.projects = d
                .projects
                .into_iter()
                .map(|p| Choice {
                    id: p.id,
                    name: p.name,
                    supported: true,
                })
                .collect()
        }
        _ => catalog
            .errors
            .push("项目来源无法读取或超过 200 项，请在任务页核对".into()),
    }
    match crate::provider::ProviderStore::at_default().workspace_choices() {
        Ok(p) => catalog.profiles = p,
        Err(_) => catalog
            .errors
            .push("档位来源暂不可用，请在模型页核对".into()),
    }
    match app
        .state::<WindowRuleStore>()
        .lock()
        .ok()
        .and_then(|s| s.list().ok())
    {
        Some(d) => {
            catalog.layouts = d
                .items
                .into_iter()
                .map(|r| Choice {
                    id: r.id,
                    name: r.name,
                    supported: true,
                })
                .collect()
        }
        _ => catalog
            .errors
            .push("布局来源暂不可用，请在窗口页核对".into()),
    }
    catalog
}
#[tauri::command]
async fn workspaces_list(app: AppHandle) -> Result<workspaces::List, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<WorkspaceStore>()
            .lock()
            .map_err(|_| "工作空间服务不可用")?
            .list()
    })
    .await
    .map_err(|_| "工作空间读取未完成".to_string())?
}
#[tauri::command]
async fn workspace_catalog(app: AppHandle) -> Result<workspaces::Catalog, String> {
    tauri::async_runtime::spawn_blocking(move || workspace_catalog_impl(&app))
        .await
        .map_err(|_| "工作空间来源读取未完成".to_string())
}
#[tauri::command]
async fn workspace_save(
    app: AppHandle,
    id: Option<String>,
    draft: workspaces::Draft,
    expected_revision: u64,
) -> Result<workspaces::List, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = workspace_catalog_impl(&app);
        app.state::<WorkspaceStore>()
            .lock()
            .map_err(|_| "工作空间服务不可用")?
            .save(id, draft, expected_revision, &catalog)
    })
    .await
    .map_err(|_| "工作空间保存未完成".to_string())?
}
#[tauri::command]
async fn workspace_remove(
    app: AppHandle,
    id: String,
    expected_revision: u64,
) -> Result<workspaces::List, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<WorkspaceStore>()
            .lock()
            .map_err(|_| "工作空间服务不可用")?
            .remove(&id, expected_revision)
    })
    .await
    .map_err(|_| "工作空间移除未完成".to_string())?
}
#[tauri::command]
async fn workspace_preview(
    app: AppHandle,
    id: String,
    expected_revision: u64,
) -> Result<workspaces::Preview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = workspace_catalog_impl(&app);
        app.state::<WorkspaceStore>()
            .lock()
            .map_err(|_| "工作空间服务不可用")?
            .preview(&id, expected_revision, &catalog)
    })
    .await
    .map_err(|_| "工作空间核对未完成".to_string())?
}

#[tauri::command]
async fn connection_test(
    id: String,
    expected_revision: u64,
    store: State<'_, ConnectionStore>,
) -> Result<connection_probe::Probe, String> {
    let connection = store
        .lock()
        .map_err(|_| "连接配置不可用")?
        .get(&id, expected_revision)?;
    let expected_connection = connection.clone();
    let lease = connection_probe::Lease::acquire()?;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _lease = lease;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "系统时间不可用")?
            .as_millis();
        let now: u64 = now.try_into().map_err(|_| "系统时间超出范围")?;
        Ok::<_, String>(connection_probe::inspect(&connection, now))
    })
    .await
    .map_err(|_| "服务检测未完成")??;
    if store
        .lock()
        .map_err(|_| "连接配置不可用")?
        .get(&id, expected_revision)?
        != expected_connection
    {
        return Err("连接配置已变化，请重新检测".into());
    }
    Ok(result)
}

#[tauri::command]
fn connections_list(store: State<'_, ConnectionStore>) -> Result<connections::Snapshot, String> {
    store.lock().map_err(|_| "连接配置不可用")?.list()
}

#[tauri::command]
fn connection_save(
    config: connections::Draft,
    expected_revision: u64,
    store: State<'_, ConnectionStore>,
) -> Result<connections::Snapshot, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "系统时间不可用")?
        .as_millis();
    let now: u64 = now.try_into().map_err(|_| "系统时间超出范围")?;
    store
        .lock()
        .map_err(|_| "连接配置不可用")?
        .save(config, expected_revision, now)
}

#[tauri::command]
fn connection_remove(
    id: String,
    expected_revision: u64,
    store: State<'_, ConnectionStore>,
) -> Result<connections::Snapshot, String> {
    store
        .lock()
        .map_err(|_| "连接配置不可用")?
        .remove(&id, expected_revision)
}

#[tauri::command]
fn window_layout_rules_list(
    store: State<'_, WindowRuleStore>,
) -> Result<window_layout_rules::RuleList, String> {
    store.lock().map_err(|_| "布局规则不可用")?.list()
}
#[tauri::command]
fn window_layout_save_rule(
    name: String,
    selection: Vec<String>,
    screen_id: String,
    template: window_layout::Template,
    gap: f64,
    expected_revision: u64,
    expected_rules_revision: u64,
    windows: State<'_, WindowLayoutStore>,
    rules: State<'_, WindowRuleStore>,
) -> Result<window_layout_rules::RuleList, String> {
    let (tools, screen) = windows
        .lock()
        .map_err(|_| "窗口列表不可用")?
        .rule_selection(&selection, &screen_id, expected_revision)?;
    rules.lock().map_err(|_| "布局规则不可用")?.save(
        name,
        tools,
        template,
        gap,
        screen,
        expected_rules_revision,
    )
}
#[tauri::command]
fn window_layout_remove_rule(
    id: String,
    expected_revision: u64,
    rules: State<'_, WindowRuleStore>,
) -> Result<window_layout_rules::RuleList, String> {
    rules
        .lock()
        .map_err(|_| "布局规则不可用")?
        .remove(&id, expected_revision)
}
#[tauri::command]
fn window_layout_resolve_rule(
    id: String,
    expected_rules_revision: u64,
    expected_revision: u64,
    rules: State<'_, WindowRuleStore>,
    windows: State<'_, WindowLayoutStore>,
) -> Result<window_layout_rules::Resolved, String> {
    let rule = rules
        .lock()
        .map_err(|_| "布局规则不可用")?
        .get(&id, expected_rules_revision)?;
    windows
        .lock()
        .map_err(|_| "窗口列表不可用")?
        .resolve_rule(rule, expected_revision)
}

#[tauri::command]
fn window_layout_capabilities() -> window_layout_native::Capabilities {
    window_layout_native::capabilities()
}

#[tauri::command]
async fn window_layout_candidates(
    app: AppHandle,
) -> Result<window_layout_service::Snapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            let _ = tx.send(window_layout_native::displays());
        })
        .map_err(|_| "无法读取屏幕")?;
        let displays = rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .map_err(|_| "屏幕读取超时")??;
        let probe = window_layout_native::enumerate()?;
        let windows = probe
            .windows
            .into_iter()
            .map(|w| (w.candidate.clone(), w))
            .collect();
        let state = app.state::<WindowLayoutStore>();
        let result =
            state
                .lock()
                .map_err(|_| "窗口列表不可用")?
                .replace(displays, windows, probe.warnings);
        result
    })
    .await
    .map_err(|_| "窗口读取任务失败")?
}

#[tauri::command]
fn window_layout_preview(
    selection: Vec<String>,
    screen_id: String,
    template: window_layout::Template,
    gap: f64,
    expected_revision: u64,
    store: State<'_, WindowLayoutStore>,
) -> Result<window_layout_service::Preview, String> {
    if !window_layout_native::capabilities().permission_granted {
        return Err("辅助功能权限不可用，请重新检查权限".into());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "系统时间不可用")?
        .as_millis();
    let now = u64::try_from(now).map_err(|_| "系统时间无效")?;
    store.lock().map_err(|_| "窗口列表不可用")?.preview(
        &selection,
        &screen_id,
        template,
        gap,
        expected_revision,
        now,
    )
}

fn layout_displays_on_main(app: &AppHandle) -> Result<Vec<window_layout::DisplayArea>, String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let _ = tx.send(window_layout_native::displays());
    })
    .map_err(|_| "无法读取屏幕")?;
    rx.recv_timeout(std::time::Duration::from_secs(3))
        .map_err(|_| "屏幕读取超时")?
}

#[tauri::command]
async fn window_layout_history() -> Result<window_layout_journal::List, String> {
    tauri::async_runtime::spawn_blocking(|| window_layout_journal::Store::at_default().list())
        .await
        .map_err(|_| "窗口历史读取未完成".to_string())?
}
#[tauri::command]
async fn window_layout_history_remove(
    app: AppHandle,
    id: String,
    expected_revision: String,
) -> Result<window_layout_journal::List, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // Serialize metadata deletion with apply, including native execution and settlement.
        let state = app.state::<WindowLayoutStore>();
        let _guard = state.lock().map_err(|_| "窗口历史不可用")?;
        window_layout_journal::Store::at_default().remove(&id, &expected_revision)
    })
    .await
    .map_err(|_| "历史删除未完成，请刷新核对".to_string())?
}
#[tauri::command]
async fn window_layout_recovery_preview(
    app: AppHandle,
    record_id: String,
    slot_id: String,
    window_id: String,
    history_revision: String,
    expected_revision: u64,
) -> Result<window_layout_service::Preview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if !window_layout_native::capabilities().permission_granted {
            return Err("辅助功能权限不可用".into());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "系统时间不可用")?
            .as_millis();
        let state = app.state::<WindowLayoutStore>();
        let result = state
            .lock()
            .map_err(|_| "窗口操作不可用")?
            .preview_recovery(
                &record_id,
                &slot_id,
                &window_id,
                &history_revision,
                expected_revision,
                u64::try_from(now).map_err(|_| "系统时间无效")?,
            );
        result
    })
    .await
    .map_err(|_| "恢复预览未完成".to_string())?
}
#[tauri::command]
async fn window_layout_apply(
    app: AppHandle,
    preview_id: String,
    expected_revision: u64,
    context: Option<window_layout_journal::WorkspaceContext>,
) -> Result<window_layout_execution::ResultDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if !window_layout_native::capabilities().permission_granted {
            return Err("辅助功能权限不可用".into());
        }
        let displays = layout_displays_on_main(&app)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "系统时间不可用")?
            .as_millis();
        let now = u64::try_from(now).map_err(|_| "系统时间无效")?;
        let state = app.state::<WindowLayoutStore>();
        // Workspace -> rule -> window order; hold authoritative references through mutation.
        let workspaces = app.state::<WorkspaceStore>();
        let workspaces = if context.is_some() {
            Some(workspaces.lock().map_err(|_| "工作空间服务不可用")?)
        } else {
            None
        };
        let rules = app.state::<WindowRuleStore>();
        let rules = if context.is_some() {
            Some(rules.lock().map_err(|_| "布局规则不可用")?)
        } else {
            None
        };
        let rule = if let Some(ctx) = &context {
            workspaces.as_ref().unwrap().check_layout_context(ctx)?;
            Some(
                rules
                    .as_ref()
                    .unwrap()
                    .get(&ctx.layout_id, ctx.expected_rules_revision)?,
            )
        } else {
            None
        };
        let result = state.lock().map_err(|_| "窗口操作不可用")?.apply_scoped(
            &preview_id,
            expected_revision,
            &displays,
            now,
            context.as_ref(),
            rule.as_ref(),
        );
        result
    })
    .await
    .map_err(|_| "窗口调整任务失败")?
}

#[tauri::command]
async fn window_layout_undo(
    app: AppHandle,
    operation_id: String,
    force_ids: Vec<String>,
) -> Result<window_layout_execution::ResultDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if !window_layout_native::capabilities().permission_granted {
            return Err("辅助功能权限不可用".into());
        }
        let displays = layout_displays_on_main(&app)?;
        let state = app.state::<WindowLayoutStore>();
        let result = state
            .lock()
            .map_err(|_| "窗口操作不可用")?
            .undo_on_displays(&operation_id, &force_ids, &displays);
        result
    })
    .await
    .map_err(|_| "窗口恢复任务失败")?
}

#[tauri::command]
fn window_layout_open_permissions() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .status()
            .map_err(|_| "无法打开系统设置")?;
        if status.success() {
            Ok(())
        } else {
            Err("系统设置打开失败".into())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("当前平台尚未接入窗口排列".into())
    }
}

#[derive(serde::Serialize, Clone)]
struct BootArgs {
    demo: bool,
    expand: bool,
    route: String,
}

#[tauri::command]
fn get_boot_args() -> BootArgs {
    let args: Vec<String> = std::env::args().collect();
    let route = args
        .iter()
        .find(|a| a.starts_with("--route="))
        .map(|a| a["--route=".len()..].to_string())
        .unwrap_or_default();
    BootArgs {
        demo: args.iter().any(|a| a == "--demo"),
        expand: args.iter().any(|a| a == "--expand"),
        route,
    }
}

// 每个窗口在订阅完成后领取自己的启动期意图。空队列才切到实时投递。
#[tauri::command]
fn drain_navigation(
    window: tauri::WebviewWindow,
    mailbox: State<Mutex<navigation::Mailbox>>,
) -> Vec<String> {
    mailbox.lock().unwrap().drain(window.label())
}

fn send_navigation(app: &AppHandle, action: &deeplink::Action) {
    let intent = format!("{action:?}");
    let targets = app
        .state::<Mutex<navigation::Mailbox>>()
        .lock()
        .unwrap()
        .enqueue(&intent);
    // 不持有队列锁调用窗口 API。
    for label in targets {
        if let Err(error) = app.emit_to(
            label,
            "deeplink://navigate",
            serde_json::json!({"action": intent}),
        ) {
            log_line(&format!("[deeplink] 导航投递失败 {label}: {error}"));
        }
    }
}

#[tauri::command]
fn get_settings(state: State<SharedEngine>) -> Settings {
    state.lock().unwrap().settings.clone()
}

#[tauri::command]
fn save_settings(
    state: State<SharedEngine>,
    app: AppHandle,
    new_settings: Settings,
) -> Result<Settings, String> {
    patch_settings(
        state,
        app,
        serde_json::to_value(new_settings).map_err(|error| error.to_string())?,
    )
}

#[tauri::command]
fn patch_settings(
    state: State<SharedEngine>,
    app: AppHandle,
    patch: serde_json::Value,
) -> Result<Settings, String> {
    let settings = {
        let mut engine = state.lock().unwrap();
        let settings = engine.settings.patched(patch)?;
        if !background_test_requested() {
            settings
                .try_save_to(&crate::settings::config_dir())
                .map_err(|error| error.to_string())?;
        }
        engine.settings = settings.clone();
        settings
    };
    apply_window_appearance(&app, &settings.appearance);
    let _ = app.emit("settings://changed", &settings);
    Ok(settings)
}

/// 系统材质和 Web 内容使用同一外观；跟随系统时取消强制主题。
fn apply_window_appearance(app: &AppHandle, mode: &str) {
    let theme = match mode {
        "dark" => Some(tauri::Theme::Dark),
        "light" => Some(tauri::Theme::Light),
        _ => None,
    };
    for label in ["island", "sidebar", "workbench"] {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.set_theme(theme);
        }
    }
}

/// Native UI regression always runs without showing windows or registering user controls.
fn background_test_requested() -> bool {
    std::env::args().any(|a| {
        matches!(
            a.as_str(),
            "--background-test" | "--memory-smoke" | "--ui-smoke"
        )
    })
}

/// Test instances that must not collide with a running user app get their own bundle
/// identifier (single-instance hands off by identifier). Covers background tests plus
/// `--dock-smoke`, which otherwise forwards to the resident instance and exits before
/// any Dock transition — the regression then observes nothing and fails with an empty
/// sequence. Deliberately *not* part of `background_test_requested`: the Dock
/// regression needs the real reveal/conceal path, which that predicate disables.
fn isolated_instance_requested() -> bool {
    background_test_requested() || std::env::args().any(|a| a == "--dock-smoke")
}

// Tauri executes run_on_main_thread inline when already on the main thread.
// Leave IPC/deep-link callbacks first, then build/destroy windows without their runtime locks.
fn queue_window_task<F: FnOnce() + Send + 'static>(app: &AppHandle, task: F) {
    let handle = app.clone();
    std::thread::spawn(move || {
        let _ = handle.run_on_main_thread(task);
    });
}

/// Called only from setup or a main-thread task: window creation must be serialized.
fn ensure_window(app: &AppHandle, label: &str) -> tauri::Result<tauri::WebviewWindow> {
    if let Some(window) = app.get_webview_window(label) {
        return Ok(window);
    }
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == label)
        .ok_or_else(|| tauri::Error::WindowNotFound)?;
    let window = tauri::WebviewWindowBuilder::from_config(app, config)?
        .visible(false)
        .build()?;
    let appearance = app
        .state::<SharedEngine>()
        .lock()
        .unwrap()
        .settings
        .appearance
        .clone();
    let _ = window.set_theme(match appearance.as_str() {
        "dark" => Some(tauri::Theme::Dark),
        "light" => Some(tauri::Theme::Light),
        _ => None,
    });
    #[cfg(target_os = "macos")]
    if label == "workbench" {
        use tauri::utils::{config::WindowEffectsConfig, WindowEffect};
        let _ = window.set_effects(WindowEffectsConfig {
            effects: vec![WindowEffect::LiquidGlassRegular, WindowEffect::Sidebar],
            ..Default::default()
        });
    }
    if label == "workbench" {
        app.state::<Mutex<window_lifecycle::WorkbenchLease>>()
            .lock()
            .unwrap()
            .created();
    }
    log_line(&format!("[window] created {label}"));
    Ok(window)
}

fn show_resident_window(
    app: &AppHandle,
    mode: crate::models::ShellMode,
    edge: crate::models::DockEdge,
    width: f64,
) -> tauri::Result<()> {
    let label = match mode {
        crate::models::ShellMode::Island => "island",
        crate::models::ShellMode::Sidebar => "sidebar",
    };
    let window = ensure_window(app, label)?;
    if label == "sidebar" {
        place_sidebar_window(app, edge, width);
    }
    if !background_test_requested() {
        let _ = window.show();
    }
    for other in ["island", "sidebar"] {
        if other != label {
            if let Some(w) = app.get_webview_window(other) {
                let _ = w.hide();
                let _ = w.emit("ui://visibility", false);
            }
        }
    }
    let _ = window.emit("ui://visibility", !background_test_requested());
    Ok(())
}

#[tauri::command]
fn open_agent_session(
    state: State<SharedEngine>,
    agent: String,
    event: Option<String>,
    expected_url: Option<String>,
) -> Result<session_navigation::Target, String> {
    let target = state
        .lock()
        .unwrap()
        .navigation_target(&agent, event.as_deref())?;
    if expected_url
        .as_deref()
        .is_some_and(|url| target.url.as_deref() != Some(url))
    {
        return Err("对应会话已更新，请重新点击".into());
    }
    session_navigation::launch(&target)?;
    Ok(target)
}

#[tauri::command]
fn get_engine_state(state: State<SharedEngine>) -> models::EngineState {
    state.lock().unwrap().state()
}

#[tauri::command]
fn window_is_visible(window: tauri::WebviewWindow) -> bool {
    window.is_visible().unwrap_or(false)
}

/// 把侧边栏窗口按记忆值摆好（贴左/贴右、宽度、铺满工作区高度）。
///
/// 与灵动岛那条路分开：灵动岛是「锚点 × 工作区」的浮层，侧边栏是一整列，
/// 两者共用一个 `place_with` 只会让两边都别扭。
/// 命令行里的 `--shell=<mode>`（也接受 `--shell <mode>`）。给不了就返回 `None`，
/// 于是「用户没写」与「用户写了 island」不会混为一谈。
/// `--shell=workbench` / `--shell workbench`（**只认 workbench**，其余走
/// [`shell_arg_override`] 那条形态枚举）。与设置无关，只影响这一次运行。
fn boot_arg_is(name: &str) -> bool {
    let args: Vec<String> = std::env::args().collect();
    let mut index = 0;
    while index < args.len() {
        let matched = args[index]
            .strip_prefix("--shell=")
            .is_some_and(|v| v == name)
            || (args[index] == "--shell" && args.get(index + 1).is_some_and(|v| v == name));
        if matched {
            return true;
        }
        index += 1;
    }
    false
}

fn shell_arg_override() -> Option<crate::models::ShellMode> {
    let args: Vec<String> = std::env::args().collect();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if let Some(value) = arg.strip_prefix("--shell=") {
            return Some(crate::models::ShellMode::parse(value));
        }
        if arg == "--shell" {
            if let Some(value) = args.get(index + 1) {
                return Some(crate::models::ShellMode::parse(value));
            }
        }
        index += 1;
    }
    None
}

/// 按用户选的**屏幕跟随模式**取工作区（`Settings::screen_follow_mode`，四档）。
///
/// 收在一处而不是让每个调用点各取一遍：跟随模式换了以后，六个调用点要一起换，
/// 而漏一个的症状是「岛跟过来了、侧边栏没跟」——**两边都不会报错**。
/// 锁拿不到时用空串：`work_area_for_mode` 认不出的值一律按「跟随光标」处理，
/// 那是最不像出错的回落。
fn work_area_for(
    win: &tauri::WebviewWindow,
    state: &State<'_, SharedEngine>,
) -> (f64, f64, f64, f64, f64) {
    placement::work_area_for_mode(win, &follow_mode(state))
}

fn follow_mode(state: &State<'_, SharedEngine>) -> String {
    state
        .lock()
        .map(|e| e.settings.screen_follow_mode.clone())
        .unwrap_or_default()
}

/// 全局热键的组合键（Swift `GlobalHotKeyManager` 同口径）。
/// `Cmd+Shift+I`（macOS）/ `Ctrl+Shift+I`（其他平台）。
///
/// 选这组是因为它与本应用既有的快捷键不冲突，且在系统设置里可被用户看到与修改。
/// 注意：插件的注册单位**就是组合键本身**（没有独立的 id），
/// 所以「幂等」只能靠 `is_registered` 读回系统状态来判断。
pub const HOTKEY_ACCEL: &str = if cfg!(target_os = "macos") {
    "CmdOrCtrl+Shift+I"
} else {
    "Ctrl+Shift+I"
};

/// 解析热键组合键。解析失败是**配置级错误**（用户改了常量），
/// 与运行期无关，所以单独给一条可断言的错误文本。
pub fn parse_hotkey() -> Result<tauri_plugin_global_shortcut::Shortcut, String> {
    HOTKEY_ACCEL
        .parse()
        .map_err(|error| format!("组合键 {HOTKEY_ACCEL} 解析失败: {error}"))
}

/// 让系统里的全局热键注册状态与设置对齐。返回 `Ok(())` 表示**无需动作**或已成功。
///
/// **幂等**：已经在想要的状态就什么都不做。这一点很重要——注册是系统级副作用，
/// 若每 5 秒无条件重注册一次，用户电脑上的这个热键会反复失效又恢复，
/// 而现象是「偶尔按了没反应」，极难归因。
pub fn apply_hotkey(app: &AppHandle, want: bool) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let shortcut = parse_hotkey()?;
    let manager = app.global_shortcut();
    if want == manager.is_registered(shortcut.clone()) {
        return Ok(());
    }
    if want {
        manager
            .register(shortcut)
            .map_err(|error| format!("注册 {HOTKEY_ACCEL} 失败: {error}"))
    } else {
        manager
            .unregister(shortcut)
            .map_err(|error| format!("注销 {HOTKEY_ACCEL} 失败: {error}"))
    }
}

/// 开机自启（设置项 `launch_at_login`）。
///
/// 走 Tauri 官方 `autostart` 插件。**注册失败要如实返回 false**，界面上那一栏
/// 随即写「系统拒绝了」——静默失败的话，用户下次开机发现没启动，
/// 只会以为这个开关坏了，而不会想到是系统权限没给。
#[tauri::command]
fn set_launch_at_login(app: AppHandle, enabled: bool) -> bool {
    if background_test_requested() {
        return enabled;
    }
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    let outcome = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    if let Err(error) = outcome {
        log_line(&format!(
            "[launchAtLogin] {} 失败: {error}",
            if enabled { "启用" } else { "关闭" }
        ));
    }
    // 「现在到底注册着没有」以插件的读数为准，而不是以上面那次调用的返回值——
    // 两者会不一致的情形正是最需要如实告诉用户的那一种
    manager.is_enabled().unwrap_or(false)
}

#[tauri::command]
fn launch_at_login_state(app: AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// 托盘徽标文案（Swift `MenuBarBadgeMode` 同名三档）。
///
/// 三档的取舍是「菜单栏那一格要不要说话」：
/// · `iconOnly`（默认）只留图标——菜单栏最贵的是被字占掉的长度
/// · `activeCount` 显示正在工作的 Agent 数（`⚡️ 2`）
/// · `tokenUsage` 显示今日 token（`120k`）
///
/// **纯函数**：它决定一个会天天出现在用户眼前的字符串，而它能拿到的只有
/// 一个 `&AgentSnapshot` 列表与一个 24h 总量。写成纯函数是为了能离线断言——
/// 「没有 Agent 时写什么」「用量怎么缩写」这两件事在真机上很难稳定复现。
pub fn tray_badge_text(mode: &str, active: usize, tokens24h: i64) -> Option<String> {
    match mode {
        "activeCount" => Some(format!("⚡️ {active}")),
        "tokenUsage" => {
            // 缩写复用引擎那一套口径（`tokens::compact`）：菜单栏上写 `120.0k`
            // 而别处也是 `120.0k`——同一件事两种说法是不允许的。
            // 顺带回答一个查过的问题：菜单栏那格比卡片窄，会不会被挤走？
            // 不会——`compact` 对任何小于 1e18 的量级都给出 ≤ 8 字符
            // （`{:.1}k` / `{:.2}M` / `{:.2}B` 各自的小数位定死了整数位长度）。
            // 再往上的量级是数据源异常，那时该修的是数据源，不是菜单栏。
            let text = crate::tokens::compact(tokens24h);
            (!text.is_empty()).then_some(text)
        }
        _ => None, // iconOnly 与一切认不出的值：只留图标
    }
}

fn place_sidebar_window(app: &AppHandle, edge: crate::models::DockEdge, width: f64) {
    let Some(win) = app.get_webview_window("sidebar") else {
        return; // 配置里没有这个窗口（旧包）：如实什么都不做，而不是 panic
    };
    // 侧边栏这一路只有 AppHandle：从 Tauri 的状态表里取设置，锁不到就退回默认档
    let state = app.state::<SharedEngine>();
    let wa = work_area_for(&win, &state);
    let (left, top, w, h) = placement::sidebar_frame(edge, width, wa);
    let _ = win.set_size(LogicalSize::new(w, h));
    let _ = win.set_position(LogicalPosition::new(left, top));
}

/// 首次切换时创建另一形态，此后复用窗口；创建操作在回调结束后排队。
#[tauri::command]
fn set_shell_mode(state: State<SharedEngine>, app: AppHandle, mode: String) -> String {
    let mode = crate::models::ShellMode::parse(&mode);
    let (edge, width);
    {
        let mut e = state.lock().unwrap();
        e.settings.shell_mode = mode.as_str().to_string();
        let s = e.settings.clone();
        if !background_test_requested() {
            s.save();
        }
        edge = crate::models::DockEdge::parse(&e.settings.sidebar_edge);
        width = e.settings.sidebar_width;
    }
    let handle = app.clone();
    queue_window_task(&app, move || {
        if let Err(error) = show_resident_window(&handle, mode, edge, width) {
            log_line(&format!("[window] switch failed: {error}"));
        }
    });
    mode.as_str().to_string()
}

/// 侧边栏自己报尺寸变化（用户拉了宽度）时存下来并重新摆位。
/// 宽度钳在可用区间内：拉到 20px 会让它变成一条缝且再也拉不回来。
#[tauri::command]
fn set_sidebar_width(state: State<SharedEngine>, app: AppHandle, width: f64) -> f64 {
    let width = placement::clamp_sidebar_width(width);
    let edge;
    {
        let mut e = state.lock().unwrap();
        e.settings.sidebar_width = width;
        let s = e.settings.clone();
        if !background_test_requested() {
            s.save();
        }
        edge = crate::models::DockEdge::parse(&e.settings.sidebar_edge);
    }
    place_sidebar_window(&app, edge, width);
    width
}

/// 侧边栏贴左/贴右（拖到哪边就贴哪边）。上下两档按**右侧**回落，
/// 与 `sidebar_frame` 同一条规则——两处不一致会让「拖到上面」变成不可预测的行为。
#[tauri::command]
fn set_sidebar_edge(state: State<SharedEngine>, app: AppHandle, edge: String) -> String {
    let edge = if edge.trim().eq_ignore_ascii_case("left") {
        "left"
    } else {
        "right"
    };
    let width;
    {
        let mut e = state.lock().unwrap();
        e.settings.sidebar_edge = edge.to_string();
        let s = e.settings.clone();
        if !background_test_requested() {
            s.save();
        }
        width = e.settings.sidebar_width;
    }
    place_sidebar_window(&app, crate::models::DockEdge::parse(edge), width);
    edge.to_string()
}

/// 侧边栏初始化：窗口创建后调一次（它初始是隐藏的，尺寸与位置要按记忆值摆）
#[tauri::command]
fn place_sidebar(state: State<SharedEngine>, app: AppHandle) {
    let (edge, width) = {
        let e = state.lock().unwrap();
        (
            crate::models::DockEdge::parse(&e.settings.sidebar_edge),
            e.settings.sidebar_width,
        )
    };
    place_sidebar_window(&app, edge, width);
}

#[tauri::command]
fn set_dock_edge(state: State<SharedEngine>, app: AppHandle, edge: String) {
    let anchor;
    {
        let mut e = state.lock().unwrap();
        e.settings.dock_edge = edge.clone();
        anchor = e.settings.dock_anchor;
        let s = e.settings.clone();
        if !background_test_requested() {
            s.save();
        }
    }
    reposition(&app, state, &DockEdge::parse(&edge), anchor, true);
}

#[tauri::command]
fn place_island(
    window: tauri::WebviewWindow,
    state: State<SharedEngine>,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let (edge, anchor) = {
        let e = state.lock().unwrap();
        (
            DockEdge::parse(&e.settings.dock_edge),
            e.settings.dock_anchor,
        )
    };
    let wa = work_area_for(&window, &state);
    let (left, top) = placement::place_with(edge, anchor, width, height, wa);
    // ⚠️ **只记「请求去哪儿」，不要在这里回读窗口几何当证据。**
    // `outer_size()` / `outer_position()` 在本应用里**恒等于 tauri.conf.json 的配置值**、
    // 且从不随 `set_size` 变化（2026-09-29 实测：窗口真被缩到 18×132 贴到右沿之后，
    // 它依然报 372×520 与 (1098,216)）。曾据此得出「窗口压根没被缩」的错误结论，
    // v0.0.256 已撤回。**验窗口几何只能截图**：
    // `screencapture -x -o -R <x>,<y>,<w>,<h>`，并做一次「杀掉进程再拍同一块」的对照。
    log_line(&format!(
        "[place] 请求 {width}×{height} 算得 ({left},{top})"
    ));
    #[cfg(target_os = "macos")]
    {
        // Tao's setters each enqueue another AppKit task, even on the main thread.
        // Commit origin, size and view display together, and acknowledge execution.
        let native_window = window.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        window
            .run_on_main_thread(move || {
                let result = (|| -> Result<(), String> {
                    use objc2::MainThreadMarker;
                    use objc2_app_kit::{NSScreen, NSWindow};
                    use objc2_foundation::{NSPoint, NSRect, NSSize};
                    let mtm = MainThreadMarker::new().ok_or("AppKit requires the main thread")?;
                    // Tauri uses a top-left desktop origin; AppKit uses the primary
                    // screen's bottom-left origin, including for secondary displays.
                    let primary = NSScreen::screens(mtm)
                        .firstObject()
                        .ok_or("No primary screen")?;
                    let frame = NSRect::new(
                        NSPoint::new(left, primary.frame().size.height - top - height),
                        NSSize::new(width, height),
                    );
                    let ptr = native_window.ns_window().map_err(|e| e.to_string())?;
                    if ptr.is_null() {
                        return Err("Missing native island window".into());
                    }
                    // SAFETY: the retained Tauri window owns this NSWindow; access is
                    // confined to its main thread. The island is borderless.
                    let native = unsafe { &*ptr.cast::<NSWindow>() };
                    if native.frame() != frame {
                        native.setFrame_display(frame, true);
                    }
                    Ok(())
                })();
                let _ = tx.send(result);
            })
            .map_err(|e| e.to_string())?;
        return rx.recv().map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        window
            .set_size(LogicalSize::new(width, height))
            .map_err(|e| e.to_string())?;
        window
            .set_position(LogicalPosition::new(left, top))
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[tauri::command]
fn snap_nearest_edge(
    window: tauri::WebviewWindow,
    state: State<SharedEngine>,
    width: f64,
    height: f64,
) -> Result<String, String> {
    if ui_smoke_requested() {
        let _ = window.emit("test://snap", ());
    }
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().unwrap_or(1.0);
    // 统一回逻辑坐标：outer_position 是物理像素，而 place_with 吃逻辑坐标。
    // 原先这里拿物理中心点去问一个返回逻辑工作区的函数，2x 屏上距离全算错。
    let cx = (pos.x as f64 + size.width as f64 / 2.0) / scale;
    let cy = (pos.y as f64 + size.height as f64 / 2.0) / scale;
    let (wx, wy, ww, wh, _s) = placement::work_area_at_logical(&window, cx, cy);

    let d_top = cy - wy;
    let d_bottom = wy + wh - cy;
    let d_left = cx - wx;
    let d_right = wx + ww - cx;
    let min = d_top.min(d_bottom).min(d_left).min(d_right);
    let (edge_str, anchor) = if (min - d_top).abs() < f64::EPSILON {
        ("top", (cx - wx) / ww)
    } else if (min - d_bottom).abs() < f64::EPSILON {
        ("bottom", (cx - wx) / ww)
    } else if (min - d_left).abs() < f64::EPSILON {
        ("left", (cy - wy) / wh)
    } else {
        ("right", (cy - wy) / wh)
    };

    {
        let mut e = state.lock().unwrap();
        e.settings.dock_edge = edge_str.to_string();
        e.settings.dock_anchor = anchor.clamp(0.0, 1.0);
        let s = e.settings.clone();
        if !background_test_requested() {
            s.save();
        }
    }
    let edge = DockEdge::parse(edge_str);
    let wa = work_area_for(&window, &state);
    let (left, top) = placement::place_with(edge, anchor.clamp(0.0, 1.0), width, height, wa);
    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    window
        .set_position(LogicalPosition::new(left, top))
        .map_err(|e| e.to_string())?;
    Ok(edge_str.to_string())
}

#[tauri::command]
fn reposition_now(
    window: tauri::WebviewWindow,
    state: State<SharedEngine>,
    width: f64,
    height: f64,
) {
    let (edge, anchor) = {
        let e = state.lock().unwrap();
        (
            DockEdge::parse(&e.settings.dock_edge),
            e.settings.dock_anchor,
        )
    };
    let wa = work_area_for(&window, &state);
    let (left, top) = placement::place_with(edge, anchor, width, height, wa);
    let _ = window.set_position(LogicalPosition::new(left, top));
}

fn reposition(
    app: &AppHandle,
    _state: State<SharedEngine>,
    edge: &DockEdge,
    anchor: f64,
    _expanded: bool,
) {
    if let Some(win) = app.get_webview_window("island") {
        let size = win.outer_size().unwrap_or_default();
        let scale = win.scale_factor().unwrap_or(1.0);
        let w = size.width as f64 / scale;
        let h = size.height as f64 / scale;
        let wa = work_area_for(&win, &_state);
        let (left, top) = placement::place_with(*edge, anchor, w, h, wa);
        let _ = win.set_position(LogicalPosition::new(left, top));
    }
}

#[tauri::command]
fn get_report(state: State<SharedEngine>, agent_id: String) -> Option<models::TokenReport> {
    let mut e = state.lock().unwrap();
    if agent_id.is_empty() {
        // 聚合全部启用档案（分析页口径）
        let profiles: Vec<crate::models::AgentProfile> = crate::registry::builtin()
            .into_iter()
            .filter(|p| {
                !e.settings.disabled_agents.contains(&p.id)
                    && (!p.token_roots.is_empty()
                        || p.session_database.as_ref().is_some_and(|db| {
                            matches!(
                                db.schema,
                                models::SessionSchema::MiniMaxRuntime
                                    | models::SessionSchema::OpenCode
                                    | models::SessionSchema::DimTasks
                            )
                        }))
            })
            .collect();
        if profiles.is_empty() {
            return None;
        }
        let mut agg_tokens24 = 0i64;
        let mut agg_total = 0i64;
        let mut agg_cost24 = 0f64;
        let mut agg_cost_total = 0f64;
        // 只要有任一档案的成本是估的，聚合值就不是记录值（见 models.rs 的 `cost_estimated`）
        let mut agg_cost_estimated = false;
        let mut hourly: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
        let mut models: std::collections::HashMap<String, (i64, f64, bool)> =
            std::collections::HashMap::new();
        let mut total_models: std::collections::HashMap<String, (i64, f64, bool)> =
            std::collections::HashMap::new();
        for p in &profiles {
            if let Some(r) = e.get_report(&p.id) {
                agg_tokens24 += r.usage.tokens24h;
                agg_total += r.usage.tokens_total;
                agg_cost24 += r.usage.cost24h;
                agg_cost_total += r.usage.cost_total;
                agg_cost_estimated = agg_cost_estimated || r.usage.cost_estimated;
                for (ts, v) in r.hourly30d {
                    *hourly.entry(ts).or_insert(0) += v;
                }
                for m in r.models_total {
                    let entry = total_models.entry(m.model).or_insert((0, 0.0, false));
                    entry.0 += m.tokens;
                    entry.1 += m.cost;
                    entry.2 |= m.cost_estimated;
                }
                for m in r.models24h {
                    let e2 = models.entry(m.model).or_insert((0, 0.0, false));
                    e2.0 += m.tokens;
                    e2.1 += m.cost;
                    e2.2 = e2.2 || m.cost_estimated;
                }
            }
        }
        let mut hourly30d: Vec<(i64, i64)> = hourly.into_iter().collect();
        hourly30d.sort_by_key(|kv| kv.0);
        let mut models24h: Vec<models::ModelUsage> = models
            .into_iter()
            .map(
                |(model, (tokens, cost, cost_estimated))| models::ModelUsage {
                    model,
                    tokens,
                    cost,
                    cost_estimated,
                },
            )
            .collect();
        models24h.sort_by(|a, b| b.tokens.cmp(&a.tokens));
        let mut models_total: Vec<models::ModelUsage> = total_models
            .into_iter()
            .map(
                |(model, (tokens, cost, cost_estimated))| models::ModelUsage {
                    model,
                    tokens,
                    cost,
                    cost_estimated,
                },
            )
            .collect();
        models_total.sort_by(|a, b| b.tokens.cmp(&a.tokens));
        return Some(models::TokenReport {
            usage: models::TokenUsage {
                tokens24h: agg_tokens24,
                tokens_total: agg_total,
                cost24h: agg_cost24,
                cost_total: agg_cost_total,
                cost_estimated: agg_cost_estimated,
            },
            models24h,
            models_total,
            hourly30d,
        });
    }
    e.get_report(&agent_id)
}

/// 外发状态快照（设置页用）。**不含任何密钥值**——`has_secret` 是布尔，
/// 凭据只读钥匙串条目名，值永远不经过这里（ADR 0009）。
/// 预览不读取密钥；{key} 保持掩码，实际发送在策略放行后由工作线程读取钥匙串。
#[tauri::command]
fn remote_preview(
    state: State<SharedEngine>,
    args: Option<crate::render::PreviewArgs>,
) -> crate::render::Preview {
    let engine = state.lock().unwrap();
    let settings = &engine.settings;
    let args = args.unwrap_or_default();
    let (channel, _) = crate::remote::resolve_kind(Some(settings.remote_kind.as_str()));
    let empty = crate::remote::ChannelConfig::default();
    let config = settings
        .remote_channels
        .get(channel.as_str())
        .unwrap_or(&empty);
    let kind =
        crate::remote::EventKind::parse(&args.kind).unwrap_or(crate::remote::EventKind::Attention);
    let agent_name = if args.agent_name.is_empty() {
        "AgentIsland"
    } else {
        args.agent_name.as_str()
    };
    let mut inputs = crate::render::Inputs::new(agent_name, kind, args.seconds);
    inputs.action_detail = args.action_detail;
    inputs.message = args.message;
    crate::render::preview(&inputs, channel, config, None)
}

#[tauri::command]
fn remote_status(state: State<SharedEngine>) -> crate::remote::Status {
    let engine = state.lock().unwrap();
    let settings = &engine.settings;
    let now = crate::tokens::now_ms();
    // 「密钥存过没有」查钥匙串的**存在性**（不带 kSecReturnData：值不进界面内存）。
    // 条目名由通道种类唯一决定，所以这里必须先解析出通道。
    let (channel, _) = crate::remote::resolve_kind(Some(settings.remote_kind.as_str()));
    let has_secret = crate::secret::exists(&crate::secret::default_secret_name(channel));
    let mut snapshot = crate::remote::status(
        Some(settings.remote_kind.as_str()),
        &settings.remote_channels,
        &settings.remote_policy,
        has_secret,
        crate::remote::Now::at(now),
        // 在场信号：锁屏 / 显示器睡眠 / 无输入时长（`power::presence_signals` 采集）。
        // 「只在人不在时发」靠的其实是第三条——远程桌面连着 Mac 时
        // 会话既不锁屏也不熄屏，只有无输入这条管用。
        &power::presence_signals(),
    );
    // 节流状态在 notifier 里，不在判定层：这里补上
    snapshot.throttled = engine.notifier.throttle_keys();
    snapshot
}

#[tauri::command]
fn remote_recent(state: State<SharedEngine>) -> Vec<crate::notifier::Recent> {
    state.lock().unwrap().notifier.recent_view()
}

#[tauri::command]
async fn remote_send_test(state: State<'_, SharedEngine>) -> Result<String, String> {
    let (notifier, policy, channel, config) = {
        let engine = state.lock().unwrap();
        let channel = crate::remote::resolve_kind(Some(&engine.settings.remote_kind)).0;
        (
            engine.notifier.clone(),
            engine.settings.remote_policy.clone(),
            channel,
            engine
                .settings
                .remote_channels
                .get(channel.as_str())
                .cloned()
                .unwrap_or_default(),
        )
    };
    tauri::async_runtime::spawn_blocking(move || {
        let now = crate::tokens::now_ms();
        let mut inputs =
            crate::render::Inputs::new("AgentIsland", crate::remote::EventKind::Attention, 0.0);
        inputs.agent_id = "agentisland-test".into();
        let outcome = notifier.attempt(
            &inputs,
            &policy,
            channel,
            &config,
            crate::remote::Now::at(now),
            &power::presence_signals(),
            true,
        );
        crate::notifier::Attempt {
            at_ms: now,
            title: "测试通知".into(),
            outcome,
            tries: 1,
        }
        .short_text()
    })
    .await
    .map_err(|error| error.to_string())
}

/// 存密钥：**由用户自己录入，只进钥匙串**（ADR 0009：本仓不存任何凭据）。
///
/// 返回 `WriteResult` 而不是 bool：ad-hoc 签名下每次出包代码标识都变，钥匙串可能弹
/// 「允许访问」甚至直接拒绝——**拒绝的理由必须原样显示给用户**，静默吞掉就等于
/// 用户以为存上了，之后每次外发都失败且没有任何线索。
///
/// 刻意**不提供「读回密钥」的命令**：界面只需要「存过没有」（`remote_status.secret_set`）
/// 与掩码回显，值没有理由回到 webview。
#[tauri::command]
fn remote_secret_set(
    state: State<SharedEngine>,
    value: Option<String>,
) -> crate::secret::WriteResult {
    let channel = {
        let engine = state.lock().unwrap();
        crate::remote::resolve_kind(Some(engine.settings.remote_kind.as_str())).0
    };
    let value = value.unwrap_or_default();
    if value.trim().is_empty() {
        // 空值当「清除」处理：这也是用户最容易做出的动作（把输入框清空再点保存）
        let name = crate::secret::default_secret_name(channel);
        return if crate::secret::delete(&name) {
            crate::secret::WriteResult::Ok
        } else {
            crate::secret::WriteResult::Refused {
                reason: "钥匙串里本来就没有这一条".to_string(),
            }
        };
    }
    if channel == crate::remote::Channel::FeishuBot
        && crate::im::feishu_credentials(&value).is_none()
    {
        return crate::secret::WriteResult::Refused {
            reason: "请填写有效的飞书 HTTPS 群机器人 Webhook 地址".into(),
        };
    }
    if channel == crate::remote::Channel::QqOneBot
        && value.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return crate::secret::WriteResult::Refused {
            reason: "访问令牌不能包含空格或换行".into(),
        };
    }
    crate::secret::write(&crate::secret::default_secret_name(channel), &value)
}

/// 删密钥：与「留空保存」同一个结果，单独给一个命令是为了界面能把「清除」做得明确
#[tauri::command]
fn remote_secret_delete(state: State<SharedEngine>) -> bool {
    let channel = {
        let engine = state.lock().unwrap();
        crate::remote::resolve_kind(Some(engine.settings.remote_kind.as_str())).0
    };
    crate::secret::delete(&crate::secret::default_secret_name(channel))
}

/// 月末预估（对应 Swift CLI `tokens` 的招牌输出与分析页顶部那块）。
///
/// 用引擎缓存的总量（`grand_total`）而不是重新聚合：那张卡上写的 24h 用量与
/// 这里外推的基准必须是**同一个数**，否则界面会自己跟自己打架。
#[tauri::command]
fn token_forecast(state: State<SharedEngine>) -> crate::forecast::ForecastReport {
    let engine = state.lock().unwrap();
    let settings = engine.settings.normalized();
    crate::forecast::evaluate(
        engine.grand_total.tokens24h,
        engine.grand_total.cost24h,
        settings.daily_token_budget,
        crate::tokens::now_ms(),
    )
}

/// 导出运维审计报告（Markdown / CSV）。对应 Swift 侧「导出到剪贴板」那条路。
///
/// 用的是**这一拍的快照**与面板同源的 `grand_total`（口径差异会在报告里写明），
/// 不重新聚合：报告与岛对不上时，用户怀疑的是面板。
#[tauri::command]
fn audit_report_markdown(state: State<SharedEngine>) -> crate::models::Export {
    let engine = state.lock().unwrap();
    crate::audit::markdown_export(
        &engine.snapshots,
        &engine.recent_events(),
        Some(&engine.grand_total),
        crate::tokens::now_ms(),
    )
}

#[tauri::command]
fn audit_report_csv(state: State<SharedEngine>) -> crate::models::Export {
    let engine = state.lock().unwrap();
    crate::audit::csv_export(&engine.snapshots, crate::tokens::now_ms())
}

/// 某个 Agent 的派生进程树（详情页「谁在吃 CPU」那一块）。
///
/// 现场扫一拍进程表：树是**当下**的结构，用引擎里那份快照拼不出「谁派生了谁」——
/// 快照里只有匹配到档案的那些进程，中间夹着的 npm / node 不在其中。
#[tauri::command]
fn agent_process_tree(
    state: State<SharedEngine>,
    agent_id: String,
) -> Option<crate::trees::TreeReport> {
    let pid = {
        let engine = state.lock().unwrap();
        engine.snapshots.iter().find(|s| s.id == agent_id)?.pid?
    };
    let mut monitor = crate::procmon::ProcessMonitor::new();
    monitor.refresh();
    Some(crate::trees::build_tree(pid, &monitor.table()))
}

/// 无头自检（对应 Swift 的 `agentisland selftest`）。
///
/// 它是**唯一**一条不读引擎状态、自己造合成输入跑一遍核心判定的命令：
/// 界面坏掉、采集读不到、设置被写坏的时候，这条仍然能回答「判定逻辑本身还对不对」。
#[tauri::command]
fn run_selftest() -> crate::selftest::Report {
    crate::selftest::run()
}

/// Token 消费报表（Markdown / CSV）。`range` 取 `day` / `week` / `month`，缺省与写错都按 `day`。
///
/// 与审计报告同一套约定：返回 `{filename, content}`（同一拍），
/// 累计历史那一行用引擎缓存的总量（与面板汇总栏同源，避免报告与岛对不上）。
#[tauri::command]
fn token_report_markdown(
    state: State<SharedEngine>,
    range: Option<String>,
) -> crate::models::Export {
    let engine = state.lock().unwrap();
    let range = range
        .as_deref()
        .and_then(crate::report::TokenTimeRange::parse)
        .unwrap_or(crate::report::TokenTimeRange::Day);
    let timeline = crate::report::build_timeline(range, crate::tokens::now_ms());
    crate::report::markdown_export(&timeline, &engine.grand_total, crate::tokens::now_ms())
}

#[tauri::command]
fn token_report_csv(state: State<SharedEngine>, range: Option<String>) -> crate::models::Export {
    let engine = state.lock().unwrap();
    let range = range
        .as_deref()
        .and_then(crate::report::TokenTimeRange::parse)
        .unwrap_or(crate::report::TokenTimeRange::Day);
    let timeline = crate::report::build_timeline(range, crate::tokens::now_ms());
    let _ = &engine;
    crate::report::csv_export(&timeline, crate::tokens::now_ms())
}

// MARK: - Codex 档位（Phase 2）

#[tauri::command]
fn provider_scan_tools() -> Vec<crate::provider::ToolScan> {
    crate::provider::scan_tools()
}

#[tauri::command]
fn provider_list_profiles() -> Result<Vec<crate::provider::CodexProfile>, String> {
    crate::provider::ProviderStore::at_default().list_checked()
}

#[tauri::command]
fn provider_export_file() -> Result<String, String> {
    let directory = dirs::download_dir().ok_or("下载目录不可用")?;
    crate::provider::ProviderStore::at_default()
        .export_to_directory(&directory)
        .map(|path| path.to_string_lossy().into_owned())
}
#[tauri::command]
fn provider_preview_import(text: String) -> Result<crate::provider::ImportPreview, String> {
    crate::provider::ProviderStore::at_default().preview_import(&text)
}
#[tauri::command]
fn provider_import_bundle(text: String, revision: String) -> Result<usize, String> {
    let _lock = PROVIDER_WRITE_LOCK.lock().map_err(|_| "档位写入锁不可用")?;
    crate::provider::ProviderStore::at_default().import_bundle(&text, &revision)
}
#[tauri::command]
async fn mcp_inspect() -> Result<crate::mcp_config::Snapshot, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let target = crate::provider::codex_config_path().ok_or("找不到 Codex 配置目录")?;
        crate::mcp_config::inspect(&target)
    })
    .await
    .map_err(|_| "MCP 后台读取未完成，请重试".to_string())?
}
#[tauri::command]
async fn mcp_preview(
    operation: crate::mcp_config::Operation,
    revision: String,
) -> Result<crate::mcp_config::Preview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let target = crate::provider::codex_config_path().ok_or("找不到 Codex 配置目录")?;
        crate::mcp_config::preview(&target, &operation, &revision)
    })
    .await
    .map_err(|_| "MCP 后台预览未完成，请重试".to_string())?
}
#[tauri::command]
async fn mcp_apply(
    operation: crate::mcp_config::Operation,
    revision: String,
    plan_id: String,
) -> Result<crate::mcp_config::Applied, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = PROVIDER_WRITE_LOCK.lock().map_err(|_| "配置写入锁不可用")?;
        let target = crate::provider::codex_config_path().ok_or("找不到 Codex 配置目录")?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "系统时间不可用")?
            .as_millis();
        crate::mcp_config::apply(
            &target,
            &crate::provider::ProviderStore::at_default().backups_dir(),
            &operation,
            &revision,
            &plan_id,
            i64::try_from(now).map_err(|_| "系统时间超出范围")?,
        )
    })
    .await
    .map_err(|_| "MCP 后台操作未完成，请刷新核对配置和备份".to_string())?
}
#[tauri::command]
async fn skills_inspect() -> Result<crate::skills_config::Snapshot, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let target = crate::provider::codex_config_path().ok_or("找不到 Codex 配置目录")?;
        let home = dirs::home_dir().ok_or("用户目录不可用")?;
        crate::skills_config::inspect(&target, &home)
    })
    .await
    .map_err(|_| "Skills 后台读取未完成".to_string())?
}
#[tauri::command]
async fn skills_preview(
    operation: crate::skills_config::Operation,
    revision: String,
) -> Result<crate::skills_config::Preview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let target = crate::provider::codex_config_path().ok_or("找不到 Codex 配置目录")?;
        let home = dirs::home_dir().ok_or("用户目录不可用")?;
        crate::skills_config::preview(&target, &home, &operation, &revision)
    })
    .await
    .map_err(|_| "Skills 后台预览未完成".to_string())?
}
#[tauri::command]
async fn skills_apply(
    operation: crate::skills_config::Operation,
    revision: String,
    plan_id: String,
) -> Result<crate::mcp_config::Applied, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = PROVIDER_WRITE_LOCK.lock().map_err(|_| "配置写入锁不可用")?;
        let target = crate::provider::codex_config_path().ok_or("找不到 Codex 配置目录")?;
        let home = dirs::home_dir().ok_or("用户目录不可用")?;
        crate::skills_config::apply(
            &target,
            &home,
            &crate::provider::ProviderStore::at_default().backups_dir(),
            &operation,
            &revision,
            &plan_id,
            crate::tokens::now_ms(),
        )
    })
    .await
    .map_err(|_| "Skills 后台操作未完成，请刷新核对配置和备份".to_string())?
}
#[tauri::command]
fn provider_capabilities() -> crate::capabilities::CapabilityInventory {
    crate::capabilities::inventory()
}

#[tauri::command]
fn skills_package_capability() -> bool {
    cfg!(target_os = "macos")
}
#[tauri::command]
async fn skills_package_choose(
    app: tauri::AppHandle,
    target: Option<String>,
) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "macos")]
    {
        let target: crate::skills_package::Tool = serde_json::from_value(
            serde_json::Value::String(target.unwrap_or_else(|| "codex".into())),
        )
        .map_err(|_| "技能安装目标不受支持")?;
        let (tx, rx) = std::sync::mpsc::channel();
        app.run_on_main_thread(move || {
            let _ = tx.send(crate::skills_picker::choose());
        })
        .map_err(|_| "目录选择暂不可用")?;
        tauri::async_runtime::spawn_blocking(move || {
            let path = rx.recv().map_err(|_| "目录选择未完成")??;
            let Some(path) = path else {
                return Ok(serde_json::Value::Null);
            };
            let state = app.state::<SkillPackageStore>();
            let result = state
                .inner
                .lock()
                .map_err(|_| "技能安装服务不可用")?
                .preview_for(&path, target, crate::tokens::now_ms())?;
            serde_json::to_value(result).map_err(|_| "技能预览不可序列化".into())
        })
        .await
        .map_err(|_| "技能预览未完成，请重新选择目录".to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, target);
        Err("本平台技能安装尚未接入".into())
    }
}
#[tauri::command]
async fn skills_package_inventory(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    #[cfg(unix)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<SkillPackageStore>();
            let inventory = state
                .inner
                .lock()
                .map_err(|_| "技能服务不可用")?
                .inventory()?;
            serde_json::to_value(inventory).map_err(|_| "技能来源不可序列化".into())
        })
        .await
        .map_err(|_| "技能来源读取未完成".to_string())?
    }
    #[cfg(not(unix))]
    {
        let _ = app;
        Err("本平台技能同步尚未接入".into())
    }
}
#[tauri::command]
async fn skills_package_sync_preview(
    app: tauri::AppHandle,
    id: String,
    generation: String,
    target: String,
) -> Result<serde_json::Value, String> {
    #[cfg(unix)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            let target: crate::skills_package::Tool =
                serde_json::from_value(serde_json::Value::String(target))
                    .map_err(|_| "技能同步目标不受支持")?;
            let state = app.state::<SkillPackageStore>();
            let preview = state
                .inner
                .lock()
                .map_err(|_| "技能服务不可用")?
                .sync_preview(&id, &generation, target, crate::tokens::now_ms())?;
            serde_json::to_value(preview).map_err(|_| "技能同步预览不可序列化".into())
        })
        .await
        .map_err(|_| "技能同步预览未完成".to_string())?
    }
    #[cfg(not(unix))]
    {
        let _ = (app, id, generation, target);
        Err("本平台技能同步尚未接入".into())
    }
}
#[tauri::command]
async fn skills_package_edit_read(
    app: tauri::AppHandle,
    id: String,
    generation: String,
) -> Result<serde_json::Value, String> {
    #[cfg(unix)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<SkillPackageStore>();
            let value = state
                .inner
                .lock()
                .map_err(|_| "技能服务不可用")?
                .edit_read(&id, &generation, crate::tokens::now_ms())?;
            serde_json::to_value(value).map_err(|_| "正文读取不可序列化".into())
        })
        .await
        .map_err(|_| "正文读取未完成".to_string())?
    }
    #[cfg(not(unix))]
    {
        let _ = (app, id, generation);
        Err("本平台技能编辑尚未接入".into())
    }
}
#[tauri::command]
async fn skills_package_edit_preview(
    app: tauri::AppHandle,
    ticket: String,
    body: String,
) -> Result<serde_json::Value, String> {
    #[cfg(unix)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<SkillPackageStore>();
            let value = state
                .inner
                .lock()
                .map_err(|_| "技能服务不可用")?
                .edit_preview(&ticket, body, crate::tokens::now_ms())?;
            serde_json::to_value(value).map_err(|_| "正文预览不可序列化".into())
        })
        .await
        .map_err(|_| "正文预览未完成".to_string())?
    }
    #[cfg(not(unix))]
    {
        let _ = (app, ticket, body);
        Err("本平台技能编辑尚未接入".into())
    }
}
#[tauri::command]
fn skills_package_edit_close(
    ticket: String,
    state: State<'_, SkillPackageStore>,
) -> Result<(), String> {
    #[cfg(unix)]
    {
        state
            .inner
            .lock()
            .map_err(|_| "技能服务不可用")?
            .edit_close(&ticket);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (ticket, state);
        Err("本平台技能编辑尚未接入".into())
    }
}
#[tauri::command]
async fn skills_package_apply(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<serde_json::Value, String> {
    #[cfg(unix)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<SkillPackageStore>();
            let result = state
                .inner
                .lock()
                .map_err(|_| "技能安装服务不可用")?
                .apply(&plan_id, crate::tokens::now_ms())?;
            serde_json::to_value(result).map_err(|_| "技能结果不可序列化".into())
        })
        .await
        .map_err(|_| "技能安装后台操作未完成，请刷新核对安装记录".to_string())?
    }
    #[cfg(not(unix))]
    {
        let _ = (app, plan_id);
        Err("本平台技能安装尚未接入".into())
    }
}
#[tauri::command]
fn skills_package_cancel(
    plan_id: String,
    state: State<'_, SkillPackageStore>,
) -> Result<(), String> {
    #[cfg(unix)]
    {
        state
            .inner
            .lock()
            .map_err(|_| "技能安装服务不可用")?
            .cancel(&plan_id);
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (plan_id, state);
        Ok(())
    }
}
#[tauri::command]
async fn skills_package_recoveries(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    #[cfg(unix)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<SkillPackageStore>();
            let result = state
                .inner
                .lock()
                .map_err(|_| "技能安装服务不可用")?
                .recoveries()?;
            serde_json::to_value(result).map_err(|_| "技能记录不可序列化".into())
        })
        .await
        .map_err(|_| "技能安装记录读取未完成".to_string())?
    }
    #[cfg(not(unix))]
    {
        let _ = app;
        Err("本平台技能安装尚未接入".into())
    }
}
#[tauri::command]
async fn skills_package_restore(
    app: tauri::AppHandle,
    id: String,
    revision: String,
) -> Result<serde_json::Value, String> {
    #[cfg(unix)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<SkillPackageStore>();
            let result = state
                .inner
                .lock()
                .map_err(|_| "技能安装服务不可用")?
                .restore(&id, &revision)?;
            serde_json::to_value(result).map_err(|_| "技能恢复结果不可序列化".into())
        })
        .await
        .map_err(|_| "技能恢复后台操作未完成，请刷新核对安装记录".to_string())?
    }
    #[cfg(not(unix))]
    {
        let _ = (app, id, revision);
        Err("本平台技能安装尚未接入".into())
    }
}
#[tauri::command]
async fn skills_package_restore_preview(
    app: tauri::AppHandle,
    id: String,
    revision: String,
) -> Result<serde_json::Value, String> {
    #[cfg(unix)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<SkillPackageStore>();
            let result = state
                .inner
                .lock()
                .map_err(|_| "技能安装服务不可用")?
                .restore_preview(&id, &revision)?;
            serde_json::to_value(result).map_err(|_| "技能恢复预览不可序列化".into())
        })
        .await
        .map_err(|_| "技能恢复预览未完成，请刷新记录".to_string())?
    }
    #[cfg(not(unix))]
    {
        let _ = (app, id, revision);
        Err("本平台技能安装尚未接入".into())
    }
}
#[tauri::command]
async fn skills_package_trash_preview(
    app: tauri::AppHandle,
    id: String,
) -> Result<serde_json::Value, String> {
    #[cfg(unix)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<SkillPackageStore>();
            let result = state
                .inner
                .lock()
                .map_err(|_| "技能安装服务不可用")?
                .trash_preview(&id, crate::tokens::now_ms())?;
            serde_json::to_value(result).map_err(|_| "技能记录清理预览不可序列化".into())
        })
        .await
        .map_err(|_| "技能记录清理预览未完成，请刷新记录".to_string())?
    }
    #[cfg(not(unix))]
    {
        let _ = (app, id);
        Err("本平台技能记录清理尚未接入".into())
    }
}
#[tauri::command]
async fn skills_package_trash(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<serde_json::Value, String> {
    #[cfg(unix)]
    {
        tauri::async_runtime::spawn_blocking(move || {
            let state = app.state::<SkillPackageStore>();
            let result = state
                .inner
                .lock()
                .map_err(|_| "技能安装服务不可用")?
                .trash(&plan_id, crate::tokens::now_ms())?;
            serde_json::to_value(result).map_err(|_| "技能记录清理结果不可序列化".into())
        })
        .await
        .map_err(|_| "技能记录清理后台未完成，请核对废纸篓与安装记录".to_string())?
    }
    #[cfg(not(unix))]
    {
        let _ = (app, plan_id);
        Err("本平台技能记录清理尚未接入".into())
    }
}
#[tauri::command]
fn provider_preview_backup(name: String) -> Result<crate::provider::BackupPreview, String> {
    let target = crate::provider::codex_config_path().ok_or("找不到 Codex 配置目录")?;
    crate::provider::preview_backup(
        &target,
        &crate::provider::ProviderStore::at_default().backups_dir(),
        &name,
    )
}

#[tauri::command]
fn provider_save_profile(
    profile: crate::provider::CodexProfile,
) -> Result<crate::provider::CodexProfile, String> {
    let _lock = PROVIDER_WRITE_LOCK.lock().map_err(|_| "档位写入锁不可用")?;
    crate::provider::ProviderStore::at_default().save(profile)
}

#[tauri::command]
fn provider_delete_profile(id: String) -> Result<(), String> {
    let _lock = PROVIDER_WRITE_LOCK.lock().map_err(|_| "档位写入锁不可用")?;
    crate::provider::ProviderStore::at_default().delete(&id)
}

#[tauri::command]
fn provider_status() -> crate::provider::ProviderStatus {
    let store = crate::provider::ProviderStore::at_default();
    let path = crate::provider::codex_config_path();
    let profiles = store.list_checked();
    let mut status = crate::provider::inspect_codex_config(
        path.as_deref(),
        profiles.as_deref().unwrap_or(&[]),
        &store,
    );
    if let Err(error) = profiles {
        status.record_error = Some(error);
    }
    status
}

// Serialize our three config/record mutations across windows. Other tools are
// protected by the observed revision check, never automatically overwritten.
static PROVIDER_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn apply_provider_choice(
    profile: crate::provider::CodexProfile,
    revision: &str,
) -> Result<crate::provider::ProviderApplyResult, String> {
    let store = crate::provider::ProviderStore::at_default();
    let target =
        crate::provider::codex_config_path().ok_or_else(|| "找不到 Codex 配置目录".to_string())?;
    store.apply_with_receipt(&target, &profile, revision, crate::tokens::now_ms())
}

#[tauri::command]
fn provider_apply_profile(
    id: String,
    revision: String,
    expected_profile: crate::provider::CodexProfile,
) -> Result<crate::provider::ProviderApplyResult, String> {
    let _lock = PROVIDER_WRITE_LOCK.lock().map_err(|_| "档位写入锁不可用")?;
    let profile =
        crate::provider::ProviderStore::at_default().profile_for_apply(&id, &expected_profile)?;
    apply_provider_choice(profile, &revision)
}

#[tauri::command]
fn provider_reapply(revision: String) -> Result<crate::provider::ProviderApplyResult, String> {
    let _lock = PROVIDER_WRITE_LOCK.lock().map_err(|_| "档位写入锁不可用")?;
    let profile = crate::provider::ProviderStore::at_default()
        .last_applied()?
        .ok_or("没有上次应用记录")?;
    apply_provider_choice(profile, &revision)
}

#[tauri::command]
fn provider_keep_current(revision: String) -> Result<(), String> {
    let _lock = PROVIDER_WRITE_LOCK.lock().map_err(|_| "档位写入锁不可用")?;
    let target = crate::provider::codex_config_path().ok_or("找不到 Codex 配置目录")?;
    crate::provider::verify_config_revision(&target, &revision)?;
    crate::provider::ProviderStore::at_default().keep_current()
}

#[tauri::command]
fn provider_list_backups() -> Vec<crate::provider::BackupInfo> {
    crate::provider::list_backups(&crate::provider::ProviderStore::at_default().backups_dir())
}

/// 按**名字**还原（界面只能选我们列出的备份；传别的名字一律拒绝）
#[tauri::command]
fn provider_restore_backup(
    name: String,
    revision: String,
    backup_revision: String,
) -> Result<crate::provider::ProviderApplyResult, String> {
    let _lock = PROVIDER_WRITE_LOCK.lock().map_err(|_| "档位写入锁不可用")?;
    let store = crate::provider::ProviderStore::at_default();
    let target =
        crate::provider::codex_config_path().ok_or_else(|| "找不到 Codex 配置目录".to_string())?;
    store.restore_with_receipt(&target, &name, &revision, &backup_revision)
}

type WorkspaceApplied = workspace_journal::Applied;
fn workspace_profile_write(
    app: &AppHandle,
    ctx: workspace_journal::Context,
    recovery: Option<&str>,
    write: impl FnOnce() -> Result<provider::ProviderApplyResult, String>,
) -> Result<WorkspaceApplied, String> {
    let store = app.state::<WorkspaceStore>();
    let store = store.lock().map_err(|_| "工作空间服务不可用")?;
    store.check_profile_context(&ctx)?;
    store.journal().execute(&ctx, recovery, write)
}
#[tauri::command]
fn workspace_apply_profile(
    app: AppHandle,
    context: workspace_journal::Context,
    id: String,
    revision: String,
    expected_profile: provider::CodexProfile,
) -> Result<WorkspaceApplied, String> {
    if context.profile_id != id || context.recovery_operation_id.is_some() {
        return Err("工作空间应用目标不匹配".into());
    }
    workspace_profile_write(&app, context, None, || {
        provider_apply_profile(id, revision, expected_profile)
    })
}
#[tauri::command]
fn workspace_restore_profile(
    app: AppHandle,
    context: workspace_journal::Context,
    name: String,
    revision: String,
    backup_revision: String,
) -> Result<WorkspaceApplied, String> {
    workspace_profile_write(&app, context, Some(&name), || {
        provider_restore_backup(name.clone(), revision, backup_revision)
    })
}
#[tauri::command]
async fn workspace_operations(
    app: AppHandle,
    id: String,
) -> Result<Vec<workspace_journal::Record>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if uuid::Uuid::parse_str(&id).is_err() {
            return Err("工作空间身份无效".into());
        }
        let store = app.state::<WorkspaceStore>();
        let store = store.lock().map_err(|_| "工作空间服务不可用")?;
        Ok(store
            .journal()
            .list()?
            .into_iter()
            .filter(|r| r.workspace_id == id)
            .collect())
    })
    .await
    .map_err(|_| "操作记录读取未完成".to_string())?
}

// Local task commands share one writer and never invoke source-agent actions.
type TaskStore = Mutex<crate::tasks::Store>;
fn with_tasks<T>(
    state: State<TaskStore>,
    f: impl FnOnce(&crate::tasks::Store) -> Result<T, crate::tasks::Error>,
) -> Result<T, crate::tasks::Error> {
    let store = state.lock().map_err(|_| crate::tasks::Error {
        code: "unavailable".into(),
        message: "任务服务暂不可用".into(),
    })?;
    f(&store)
}
fn publish_tasks(
    app: &AppHandle,
    result: Result<crate::tasks::Data, crate::tasks::Error>,
) -> Result<crate::tasks::Data, crate::tasks::Error> {
    if let Ok(data) = &result {
        let _ = app.emit("tasks://changed", data.revision);
    }
    result
}
#[tauri::command]
fn tasks_attention_summary(
    state: State<TaskStore>,
) -> Result<crate::task_attention::Summary, crate::tasks::Error> {
    with_tasks(state, |s| {
        s.load().map(|d| crate::task_attention::summarize(&d))
    })
}
#[tauri::command]
fn task_show_workbench(
    app: AppHandle,
    store: State<TaskStore>,
    id: Option<String>,
    run_id: Option<String>,
    expected_revision: Option<u64>,
) -> Result<(), String> {
    let intent = if let Some(id) = id {
        let data = with_tasks(store, |s| s.load()).map_err(|e| e.message)?;
        if expected_revision != Some(data.revision) {
            return Err("任务已更新，请重新点击".into());
        }
        let t = data
            .tasks
            .iter()
            .find(|t| t.id == id && t.archived_ms.is_none())
            .ok_or("任务已归档或不可用")?;
        if t.current_run_id != run_id {
            return Err("这次运行已更新，请重新点击".into());
        }
        format!(
            "Task({})",
            serde_json::to_string(&id).map_err(|_| "任务导航不可用")?
        )
    } else {
        "Tasks".into()
    };
    let live = app
        .state::<Mutex<navigation::Mailbox>>()
        .lock()
        .map_err(|_| "任务导航不可用")?
        .enqueue_for("workbench", &intent);
    if live {
        let _ = app.emit_to(
            "workbench",
            "deeplink://navigate",
            serde_json::json!({"action":intent}),
        );
    }
    reveal_workbench_window(&app);
    Ok(())
}
#[tauri::command]
fn tasks_snapshot(state: State<TaskStore>) -> Result<crate::tasks::Data, crate::tasks::Error> {
    with_tasks(state, |s| s.load())
}
#[tauri::command]
fn task_project_create(
    app: AppHandle,
    state: State<TaskStore>,
    name: String,
    expected_revision: u64,
) -> Result<crate::tasks::Data, crate::tasks::Error> {
    publish_tasks(
        &app,
        with_tasks(state, |s| s.project_create(&name, expected_revision)),
    )
}
#[tauri::command]
fn task_create(
    app: AppHandle,
    state: State<TaskStore>,
    title: String,
    project_id: Option<String>,
    expected_revision: u64,
) -> Result<crate::tasks::Data, crate::tasks::Error> {
    publish_tasks(
        &app,
        with_tasks(state, |s| {
            s.create(
                &title,
                project_id,
                expected_revision,
                crate::tokens::now_ms(),
            )
        }),
    )
}
#[tauri::command]
fn task_update(
    app: AppHandle,
    state: State<TaskStore>,
    id: String,
    title: String,
    project_id: Option<String>,
    expected_revision: u64,
) -> Result<crate::tasks::Data, crate::tasks::Error> {
    publish_tasks(
        &app,
        with_tasks(state, |s| {
            s.update(
                &id,
                &title,
                project_id,
                expected_revision,
                crate::tokens::now_ms(),
            )
        }),
    )
}
#[tauri::command]
fn task_archive(
    app: AppHandle,
    state: State<TaskStore>,
    id: String,
    archived: bool,
    expected_revision: u64,
) -> Result<crate::tasks::Data, crate::tasks::Error> {
    publish_tasks(
        &app,
        with_tasks(state, |s| {
            s.archive(&id, archived, expected_revision, crate::tokens::now_ms())
        }),
    )
}
#[tauri::command]
fn task_record_progress(
    app: AppHandle,
    state: State<TaskStore>,
    id: String,
    status: crate::tasks::RunStatus,
    kind: Option<crate::tasks::AttentionKind>,
    artifact_title: Option<String>,
    expected_revision: u64,
) -> Result<crate::tasks::Data, crate::tasks::Error> {
    publish_tasks(
        &app,
        with_tasks(state, |s| {
            s.record(
                &id,
                status,
                kind,
                artifact_title,
                expected_revision,
                crate::tokens::now_ms(),
            )
        }),
    )
}
#[tauri::command]
fn task_mark_handled(
    app: AppHandle,
    state: State<TaskStore>,
    id: String,
    run_id: String,
    attention_id: String,
    expected_revision: u64,
) -> Result<crate::tasks::Data, crate::tasks::Error> {
    publish_tasks(
        &app,
        with_tasks(state, |s| {
            s.handled(
                &id,
                &run_id,
                &attention_id,
                expected_revision,
                crate::tokens::now_ms(),
            )
        }),
    )
}

#[tauri::command]
fn task_sources(state: State<SharedEngine>) -> Vec<crate::task_sources::Choice> {
    state
        .lock()
        .unwrap()
        .task_sources
        .values()
        .cloned()
        .collect()
}

type SessionCatalogStore = Mutex<crate::session_catalog::Store>;
#[tauri::command]
async fn session_catalog_read(app: AppHandle) -> Result<session_catalog::Catalog, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<SessionCatalogStore>()
            .lock()
            .map_err(|_| "会话目录不可用".to_string())
            .map(|mut store| store.read())
    })
    .await
    .map_err(|_| "会话目录读取未完成".to_string())?
}
#[tauri::command]
async fn session_catalog_open(
    app: AppHandle,
    generation: String,
    source: tasks::Source,
) -> Result<session_navigation::Target, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let target = app
            .state::<SessionCatalogStore>()
            .lock()
            .map_err(|_| "会话目录不可用")?
            .target(&generation, &source)?;
        session_navigation::launch(&target)?;
        Ok(target)
    })
    .await
    .map_err(|_| "会话打开未完成".to_string())?
}

#[tauri::command]
fn session_open_observed(
    state: State<SharedEngine>,
    source: tasks::Source,
) -> Result<session_navigation::Target, String> {
    let target = crate::task_sources::selected_target(
        state.lock().unwrap().task_sources.get(&source.agent_id),
        &source,
    )?;
    session_navigation::launch(&target)?;
    Ok(target)
}
#[tauri::command]
fn task_link_source(
    app: AppHandle,
    state: State<SharedEngine>,
    store: State<TaskStore>,
    id: String,
    agent: String,
    session_id: String,
    expected_revision: u64,
) -> Result<crate::tasks::Data, crate::tasks::Error> {
    let source = state
        .lock()
        .unwrap()
        .task_sources
        .get(&agent)
        .filter(|c| c.source.session_id == session_id)
        .map(|c| c.source.clone())
        .ok_or_else(|| crate::tasks::Error {
            code: "source_expired".into(),
            message: "来源会话已更新，请刷新来源列表".into(),
        })?;
    publish_tasks(
        &app,
        with_tasks(store, |s| {
            s.link(&id, source, expected_revision, crate::tokens::now_ms())
        }),
    )
}
#[tauri::command]
fn task_open_source(
    store: State<TaskStore>,
    id: String,
    run_id: Option<String>,
    artifact_id: Option<String>,
    expected_revision: u64,
) -> Result<session_navigation::Target, String> {
    let data = with_tasks(store, |s| s.load()).map_err(|e| e.message)?;
    let source = data
        .navigation_source(
            &id,
            run_id.as_deref(),
            artifact_id.as_deref(),
            expected_revision,
        )
        .map_err(|e| e.message)?;
    let target = crate::task_sources::stored_target(source)?;
    session_navigation::launch(&target)?;
    Ok(target)
}

#[tauri::command]
async fn task_artifact_content(
    app: AppHandle,
    id: String,
    run_id: String,
    artifact_id: String,
    expected_revision: u64,
) -> Result<crate::task_artifacts::Content, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let store = app.state::<TaskStore>();
        let snapshot = store
            .lock()
            .map_err(|_| "任务服务暂不可用")?
            .load()
            .map_err(|e| e.message)?;
        let source = snapshot
            .navigation_source(&id, Some(&run_id), Some(&artifact_id), expected_revision)
            .map_err(|e| e.message)?;
        let profile = crate::registry::builtin()
            .into_iter()
            .find(|p| p.id == source.agent_id)
            .ok_or("来源工具未支持内容读取")?;
        #[cfg(target_os = "macos")]
        let runtime = app
            .state::<SharedEngine>()
            .lock()
            .map_err(|_| "采集状态不可用")?
            .claude_plan_runtime
            .clone();
        #[cfg(target_os = "macos")]
        let result = if let Some(runtime) = runtime {
            runtime
                .lock()
                .map_err(|_| "采集状态不可用")?
                .with_cache(|capture| {
                    crate::task_artifacts::read_with_capture(
                        &snapshot,
                        &id,
                        &run_id,
                        &artifact_id,
                        expected_revision,
                        &profile,
                        capture,
                    )
                })?
        } else {
            crate::task_artifacts::read(
                &snapshot,
                &id,
                &run_id,
                &artifact_id,
                expected_revision,
                &profile,
            )?
        };
        #[cfg(not(target_os = "macos"))]
        let result = crate::task_artifacts::read(
            &snapshot,
            &id,
            &run_id,
            &artifact_id,
            expected_revision,
            &profile,
        )?;
        if store
            .lock()
            .map_err(|_| "任务服务暂不可用")?
            .load()
            .map_err(|e| e.message)?
            .revision
            != expected_revision
        {
            return Err("任务已更新，请刷新后重试".into());
        }
        Ok(result)
    })
    .await
    .map_err(|_| "来源内容读取未完成，请重试".to_string())?
}

// Claude plan controls return metadata only; body reads use the existing exact artifact command.
enum ClaudePlanOperation {
    Status,
    Preview(claude_hook_config::Action, String, Option<String>),
    Apply(String),
    Cancel,
    Resume(String),
    Pause,
}
async fn claude_plan_operation(
    app: AppHandle,
    operation: ClaudePlanOperation,
) -> Result<serde_json::Value, serde_json::Value> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, operation);
        Err(
            serde_json::json!({"code":"unsupported","notice":"本平台尚未支持方案采集。","applied":false}),
        )
    }
    #[cfg(target_os="macos")]
    tauri::async_runtime::spawn_blocking(move||{
        let error=|notice:&str|serde_json::json!({"code":"unavailable","notice":notice,"applied":false});
        let runtime=app.state::<SharedEngine>().lock().map_err(|_|error("采集状态不可用。"))?.claude_plan_runtime.clone().ok_or_else(||error("本次运行未开放方案采集。"))?;
        let mut runtime=runtime.lock().map_err(|_|error("采集状态不可用。"))?;
        let convert=|e:claude_hook_config::Error|serde_json::to_value(e).unwrap_or_else(|_|error("采集结果未核实。"));
        match operation {
            ClaudePlanOperation::Status=>serde_json::to_value(runtime.status()).map_err(|_|error("采集状态不可用。")),
            ClaudePlanOperation::Preview(action,revision,backup)=>serde_json::to_value(runtime.preview(action,&revision,backup.as_deref()).map_err(convert)?).map_err(|_|error("预览未核实。")),
            ClaudePlanOperation::Apply(id)=>serde_json::to_value(runtime.apply(&id).map_err(convert)?).map_err(|_|error("执行结果未核实。")),
            ClaudePlanOperation::Cancel=>{runtime.cancel();Ok(serde_json::json!({"cancelled":true}))},
            ClaudePlanOperation::Resume(revision)=>{runtime.resume(&revision).map_err(convert)?;serde_json::to_value(runtime.status()).map_err(|_|error("采集状态不可用。"))},
            ClaudePlanOperation::Pause=>{runtime.pause().map_err(convert)?;serde_json::to_value(runtime.status()).map_err(|_|error("采集状态不可用。"))},
        }
    }).await.map_err(|_|serde_json::json!({"code":"uncertain","notice":"采集操作未完整返回，请重新读取。","applied":true}))?
}
#[tauri::command]
async fn claude_plan_status(app: AppHandle) -> Result<serde_json::Value, serde_json::Value> {
    claude_plan_operation(app, ClaudePlanOperation::Status).await
}
#[tauri::command]
async fn claude_plan_preview(
    app: AppHandle,
    action: claude_hook_config::Action,
    revision: String,
    backup_id: Option<String>,
) -> Result<serde_json::Value, serde_json::Value> {
    claude_plan_operation(
        app,
        ClaudePlanOperation::Preview(action, revision, backup_id),
    )
    .await
}
#[tauri::command]
async fn claude_plan_apply(
    app: AppHandle,
    plan_id: String,
) -> Result<serde_json::Value, serde_json::Value> {
    claude_plan_operation(app, ClaudePlanOperation::Apply(plan_id)).await
}
#[tauri::command]
async fn claude_plan_cancel(app: AppHandle) -> Result<serde_json::Value, serde_json::Value> {
    claude_plan_operation(app, ClaudePlanOperation::Cancel).await
}
#[tauri::command]
async fn claude_plan_resume(
    app: AppHandle,
    revision: String,
) -> Result<serde_json::Value, serde_json::Value> {
    claude_plan_operation(app, ClaudePlanOperation::Resume(revision)).await
}
#[tauri::command]
async fn claude_plan_pause(app: AppHandle) -> Result<serde_json::Value, serde_json::Value> {
    claude_plan_operation(app, ClaudePlanOperation::Pause).await
}

// MARK: - 待办（Phase 3）

#[tauri::command]
fn todos_list() -> crate::todos::TodoList {
    crate::todos::TodoStore::at_default().snapshot(crate::tokens::now_ms())
}

#[tauri::command]
fn todos_add(text: String) -> Result<crate::todos::TodoList, String> {
    crate::todos::TodoStore::at_default().add(&text, crate::tokens::now_ms())
}

#[tauri::command]
fn todos_toggle(id: String) -> Result<crate::todos::TodoList, String> {
    crate::todos::TodoStore::at_default().toggle(&id)
}

#[tauri::command]
fn todos_remove(id: String) -> Result<crate::todos::TodoList, String> {
    crate::todos::TodoStore::at_default().remove(&id)
}

#[tauri::command]
fn todos_clear_done() -> Result<crate::todos::TodoList, String> {
    crate::todos::TodoStore::at_default().clear_done()
}

#[tauri::command]
fn clear_latest_event(state: State<SharedEngine>) {
    // 确认这一条、推下一条：覆盖式清除会把同一拍里排队的告警一起丢掉
    state.lock().unwrap().ack_latest_event();
}

/// 界面上的「终止这个 Agent」。
///
/// **不再自己发信号**——走 [`crate::cleaner`] 那一条，与 CLI 的 `clean`
/// 同一套规则。此前这里是裸 `kill -9`：没有身份复核、没有进程树、
/// 没有僵尸判定，而且「kill 完之后才复核」的那段代码建了个 ProcessMonitor
/// 就丢掉了（`let _ = pm;`），什么也没做。
///
/// 界面和 CLI 各杀一套的后果不是「有两份代码」，而是**两把不同的刀**：
/// CLI 里那把知道怎么避开 PID 复用，界面上那把不知道，而用户看不出区别。
#[tauri::command]
fn terminate_agent(state: State<SharedEngine>, pid: Option<u32>, agent_id: String) {
    let Some(pid) = pid else { return };
    if pid < crate::cleaner::MIN_TARGET_PID {
        return;
    }
    let profile = {
        let _guard = state.lock().unwrap();
        crate::registry::builtin()
            .into_iter()
            .find(|p| p.id == agent_id)
    };
    let Some(profile) = profile else { return };
    let monitor = crate::procmon::ProcessMonitor::new();
    let table = monitor.table();

    // 身份复核：这个 pid 上的进程**确实是这个档案**吗？
    // 用档案的 `process_names` 去比对当前进程，而不是拿进程表里的名字与它自己比——
    // 后者是恒真式，复核就成了摆设。
    let Some(hit) = crate::cleaner::find(&table, pid) else {
        return; // pid 已经不在了
    };
    if !crate::procmon::profile_matches(&profile, &hit.name, &hit.exe_path, "") {
        return;
    }
    if hit.is_zombie {
        return;
    }
    // SIGTERM + 优雅期，仍在就 SIGKILL；不再是无条件 `-9`。
    crate::cleaner::terminate(pid, std::time::Duration::from_millis(300));
}

/// 运维报告文本（工作台的「报告」面板用）。
///
/// **与 CLI 的 `agentisland report` 走同一对函数**（`audit::markdown_export` /
/// `audit::csv_export`）。界面自己拼一份报告的话，两边的表头与口径迟早漂移，
/// 而报告是给人拿去对账的，对不上比没有更糟。
#[tauri::command]
fn report_text(state: State<SharedEngine>, format: String) -> Result<String, String> {
    let snapshots = {
        let e = state.lock().unwrap();
        let s = e.state();
        (s.snapshots.clone(), s.grand_total.clone())
    };
    let now = crate::tokens::now_ms();
    match format.as_str() {
        "csv" => Ok(crate::audit::csv_export(&snapshots.0, now).content),
        "md" | "markdown" => {
            Ok(crate::audit::markdown_export(&snapshots.0, &[], Some(&snapshots.1), now).content)
        }
        other => Err(format!("--format 只认 md 与 csv（收到 {other}）")),
    }
}

/// 隐藏工作台窗口（**只隐藏，不销毁**——内容与滚���位置都留着）。
///
/// 与关窗口分开是有意的：工作台是「随时瞄一眼」的面板，
/// 隐藏再打开时用户期望的是回到刚才的位置，而不是一个刚加载完的空白页。
#[tauri::command]
fn hide_workbench(app: AppHandle) {
    conceal_workbench_window(&app);
}

#[tauri::command]
fn show_workbench(app: AppHandle) {
    reveal_workbench_window(&app);
}

/// 把工作台窗口叫到前面。**托盘、深链、前端三处共用这一段**——
/// 三处各写一份显隐规则的话，迟早只有一处会带 `unminimize`。
#[cfg(target_os = "macos")]
fn acquire_startup_guard() -> std::io::Result<std::fs::File> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    // single-instance 的 Unix listener 异步绑定；串行化冷启动直到 setup 完成，
    // 防止两次同时打开都在 socket 就绪前通过插件检测。CLI 不经过此入口。
    let path = std::env::temp_dir().join(format!("dev.agentisland.startup-{}.lock", unsafe {
        libc::geteuid()
    }));
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(file);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn set_dock_presence(app: &AppHandle, visible: bool) {
    #[cfg(target_os = "macos")]
    if let Err(error) = app.set_activation_policy(if visible {
        tauri::ActivationPolicy::Regular
    } else {
        tauri::ActivationPolicy::Accessory
    }) {
        log_line(&format!("[window] Dock 显隐失败: {error}"));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, visible);
}

#[tauri::command]
fn set_workbench_draft(
    window: tauri::WebviewWindow,
    lease: State<Mutex<window_lifecycle::WorkbenchLease>>,
    dirty: bool,
) {
    if window.label() == "workbench" {
        lease.lock().unwrap().dirty = dirty;
    }
}

fn schedule_workbench_release(app: &AppHandle, window: tauri::WebviewWindow) {
    let epoch = app
        .state::<Mutex<window_lifecycle::WorkbenchLease>>()
        .lock()
        .unwrap()
        .hidden();
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(90));
        let app = handle.clone();
        queue_window_task(&handle, move || {
            let may_release = app
                .state::<Mutex<window_lifecycle::WorkbenchLease>>()
                .lock()
                .unwrap()
                .can_release(epoch);
            if !may_release || window.is_visible().unwrap_or(true) {
                return;
            }
            if window.destroy().is_ok() {
                app.state::<Mutex<navigation::Mailbox>>()
                    .lock()
                    .unwrap()
                    .reset("workbench");
                memory::reclaim_idle_pages();
                log_line("[window] released hidden workbench");
            }
        });
    });
}

fn conceal_workbench_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("workbench") {
        if window.hide().is_ok() {
            let _ = window.emit("ui://visibility", false);
            set_dock_presence(app, false);
            schedule_workbench_release(app, window);
        }
    }
}

// 单色胶囊符号交给 macOS 作为 template 着色，避免缩小应用图标到菜单栏。
fn menu_bar_icon() -> tauri::image::Image<'static> {
    let mut rgba = vec![0u8; 44 * 44 * 4];
    for y in 0..44 {
        for x in 0..44 {
            let px = (x as f64 + 0.5) / 2.0;
            let py = (y as f64 + 0.5) / 2.0;
            let dx = (px - 11.0).abs() - 5.0;
            let distance = dx.max(0.0).hypot(py - 11.0);
            if distance <= 4.0 {
                // 右侧状态灯留白；小尺寸只保留胶囊与一个圆点。
                let hole = (px - 16.0).hypot(py - 11.0) < 1.5;
                rgba[(y * 44 + x) * 4 + 3] = if hole { 0 } else { 255 };
            }
        }
    }
    tauri::image::Image::new_owned(rgba, 44, 44)
}

fn reveal_workbench_window(app: &AppHandle) {
    let handle = app.clone();
    queue_window_task(app, move || match ensure_window(&handle, "workbench") {
        Ok(window) => {
            if !background_test_requested() {
                set_dock_presence(&handle, true);
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
            let _ = window.emit("ui://visibility", true);
        }
        Err(error) => log_line(&format!("[window] workbench creation failed: {error}")),
    });
}

#[tauri::command]
fn collapse_to_tray(window: tauri::WebviewWindow) {
    let _ = window.emit_to("island", "ui://collapse", ());
}

// 抑制未使用告警（collapse_to_tray 参数保留给后续窗口控制）
#[allow(unused)]
#[derive(serde::Serialize, Clone)]
struct TokenReportPub {
    #[serde(flatten)]
    inner: models::TokenReport,
}

/// 写一行诊断日志。**打开失败要自报，不能吞掉**。
///
/// 此前是 `if let Ok(mut f) = …open(&path)`，失败时什么都不发生——于是
/// 「打包后的 App 日志一行都没有」这件事本身查不出来：没有日志就没有线索，
/// 没有线索就只能猜是不是代码没跑到。v0.0.222 我为「工作台为什么没动静」
/// 卡了很久，根因就在这里：**通道坏了，而它坏得毫无声响**。
///
/// 写不出去时退回 stderr：GUI 应用的 stdout 通常没人看，但**总比静默好**——
/// 从终端 `open` 出来的那一次就能看见。
/// 日志封顶：超过 [`LOG_CAP_BYTES`] 就**整个重来**，并在首行写明截断前有多大。
///
/// 为什么需要：日志是**纯 append、无轮转**的，而这个应用是常驻的日历应用
/// ——天天开着，文件只会一直长。v0.0.243 起每次启动还要多写 6-9KB
/// （三个窗口 × 三轮 `[eval]` 自省 + 冒烟结果），于是涨得更快。
///
/// 整份重来而不是只留尾部：排障时要的是「**这一次运行**从头到尾」，
/// 而 `[run]` 标记本来就在每次启动时写，所以截断后不会把两次运行混在一起。
const LOG_CAP_BYTES: u64 = 2 * 1024 * 1024;

/// 超过封顶就把文件清空，返回截断前的字节数（没截断则 `None`）。
fn truncate_log_if_oversized(path: &std::path::Path) -> Option<u64> {
    let size = std::fs::metadata(path).ok()?.len();
    if size <= LOG_CAP_BYTES {
        return None;
    }
    std::fs::write(path, b"").ok()?;
    Some(size)
}

pub(crate) fn log_line(msg: &str) {
    let filename = if background_test_requested() {
        format!("agentisland-test-{}.log", std::process::id())
    } else {
        "agentisland-tauri.log".to_string()
    };
    let path = std::env::temp_dir().join(filename);
    if let Some(before) = truncate_log_if_oversized(&path) {
        let line = format!("[run] 上一份日志 {before} 字节，已按 {LOG_CAP_BYTES} 字节封顶清空");
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            let _ = writeln!(f, "{line}");
        }
    }
    // 说明：日志文件是**累积**的（同一路径、按运行叠加），所以每次启动
    // 都先写一行 `[run]` 标记。没有它就没法把「这一次跑出来的行」与
    // 「历史遗留的行」分开——我为此误判过好几轮：把累积日志里的
    // `[page] / [webview]` 当成当次运行的证据。

    use std::io::Write;
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(mut f) => {
            let _ = writeln!(f, "{}", msg);
        }
        Err(error) => {
            eprintln!("[log] 写不进 {}：{error}", path.display());
            eprintln!("[log] {msg}");
        }
    }
}

fn engine_loop(shared: SharedEngine, app: AppHandle) {
    let mut source_signature = Vec::<String>::new();
    loop {
        // 徽标文本在临界区里**算好**，写托盘留到锁外（理由见 `apply_tray_badge`）。
        let (state, interval, badge, observations) = {
            let mut e = shared.lock().unwrap();
            let mut events = Vec::new();
            if let Some(rx) = e.event_rx.as_ref() {
                while let Ok(ev) = rx.try_recv() {
                    events.push(ev);
                }
            }
            for ev in events {
                e.push_event(ev);
            }
            let badge_mode = e.settings.menu_bar_badge_mode.clone();
            e.tick();
            let s = e.state();
            let working = s
                .snapshots
                .iter()
                .filter(|snap| {
                    matches!(
                        snap.level,
                        models::ActivityLevel::Working | models::ActivityLevel::Attention
                    )
                })
                .count();
            // 跟随 `menu_bar_badge_mode`（iconOnly 时不设标题）
            let badge = tray_badge_text(&badge_mode, working, s.grand_total.tokens24h);
            let active = s.snapshots.iter().any(|snap| {
                matches!(
                    snap.level,
                    models::ActivityLevel::Working | models::ActivityLevel::Attention
                )
            });
            // 全闲置走**独立字段**（Swift `idleSampleInterval`，默认 5s）。
            // 此前这里写死 `sample_interval × 2.5`——那是另一个公式，
            // 于是两侧的耗电量与「岛多久变灰」对不上。
            // 节电降频（Swift `BatterySaver` 同口径）：**只拉长闲置那一档**——
            // 有活动时降频等于直接漏掉工作态，那是拿正确性换电量
            let interval = if active {
                e.settings.sample_interval
            } else {
                let base = e.settings.idle_sample_interval.clamp(0.5, 600.0);
                if power::should_throttle(
                    e.settings.battery_saver_enabled,
                    power::is_low_power_mode(),
                    power::is_on_battery(),
                ) {
                    base * power::BATTERY_THROTTLE_FACTOR
                } else {
                    base
                }
            };
            (
                s,
                interval,
                badge,
                e.task_sources
                    .values()
                    .map(|c| c.observation.clone())
                    .collect::<Vec<_>>(),
            )
        };
        // 锁已释放，才轮到托盘——顺序不能反，见 `apply_tray_badge` 的注释。
        memory::reclaim_idle_pages();
        apply_tray_badge(&app, badge);
        let mut signature: Vec<_> = observations
            .iter()
            .map(|o| {
                format!(
                    "{}:{}:{:?}",
                    o.source.agent_id, o.source.session_id, o.status
                )
            })
            .collect();
        signature.sort();
        if signature != source_signature {
            source_signature = signature;
            let _ = app.emit("tasks://sources_changed", ());
        }
        if !observations.is_empty() {
            let result = app
                .state::<TaskStore>()
                .lock()
                .map_err(|_| crate::tasks::Error {
                    code: "unavailable".into(),
                    message: "任务服务暂不可用".into(),
                })
                .and_then(|store| store.sync(&observations, crate::tokens::now_ms()));
            // Owned data leaves the writer lock before touching any WebView.
            match result {
                Ok(Some(data)) => {
                    let _ = app.emit("tasks://changed", data.revision);
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = app.emit("tasks://error", error);
                }
            }
        }
        for label in ["island", "sidebar", "workbench"] {
            if let Some(window) = app.get_webview_window(label) {
                if window.is_visible().unwrap_or(false) {
                    let _ = window.emit("engine://tick", &state);
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_secs_f64(interval));
    }
}

/// 把托盘徽标写上去。**只能在释放引擎锁之后调用**，所以唯一调用点在 `engine_loop`。
///
/// macOS 的托盘是 AppKit 的 `NSStatusItem`，Tauri 写它必须回到主线程，而且是
/// **同步等结果**的（实测栈：`mpsc::recv` → `Condvar::wait` → `Thread::park`
/// → `_dispatch_semaphore_wait_slow`）。引擎循环是后台线程，于是：
///
/// ```text
/// 引擎线程：持锁 → 等主线程执行完托盘写入
/// 主线程  ：要锁（setup 读设置、任何命令都要）→ 等引擎线程放锁
/// ```
///
/// 互等 ⇒ 应用永远停在「启动中」。而 Tauri 的 `setup` 没返回，事件循环就不会转，
/// WKWebView 于是从不导航：窗口在、尺寸对、资源嵌好了，`on_page_load` 一次不响，
/// 界面全白。改 URL / 改 CSP / 换 SDK 全都不起作用，因为它们都不在这条链上。
///
/// 取证：`docs/research/2026-09-29-blank-ui-deadlock.md`
fn apply_tray_badge(app: &AppHandle, badge: Option<String>) {
    let Some(tray) = app.tray_by_id("main") else {
        return;
    };
    let _ = match badge {
        Some(text) => tray.set_title(Some(&text)),
        None => tray.set_title::<&str>(None),
    };
}

/// 嵌进二进制的资源清单（`[boot] 资源表` 逐个验它真在里面）。
///
/// 覆盖 `app/ui/` 下的**全部**文件——少列一个，那个文件缺失时就查不出来，
/// 而少一个 CSS 的后果是**整个形态无样式渲染、零报错**。
const EMBEDDED_ASSET_SAMPLE: &[&str] = &[
    "assets/agents/LICENSE",
    "assets/agents/LICENSE.OpenViking",
    "assets/agents/LICENSE.DeepSeekHarness",
    "assets/agents/official-sources.json",
    "assets/agents/dim.png",
    "assets/agents/zcode.png",
    "assets/agents/qoder.png",
    "assets/agents/workbuddy.png",
    "assets/agents/chatgpt.png",
    "assets/agents/workbuddyai.png",
    "assets/agents/dsh.svg",
    "assets/agents/trae.png",
    "assets/agents/traework.png",
    "assets/agents/doubaowork.png",
    "assets/agents/mimodesktop.png",
    "assets/agents/minimaxcode.png",
    "assets/agents/vscode.png",
    "assets/agents/aider.svg",
    "assets/agents/ima.svg",
    "assets/agents/continue.svg",
    "assets/agents/openviking.svg",
    "assets/agents/customagent.svg",
    "assets/agents/README.md",
    "assets/agents/antigravity.svg",
    "assets/agents/claude.svg",
    "assets/agents/cline.svg",
    "assets/agents/codex.svg",
    "assets/agents/cursor.svg",
    "assets/agents/deepseek.svg",
    "assets/agents/goose.svg",
    "assets/agents/hermesagent.svg",
    "assets/agents/openai.svg",
    "assets/agents/opencode.svg",
    "assets/agents/qoder.svg",
    "assets/agents/roocode.svg",
    "assets/agents/trae.svg",
    "assets/agents/windsurf.svg",
    "assets/agents/xiaomimimo.svg",
    "css/agents.css",
    "css/island.css",
    "css/panels.css",
    "css/sidebar.css",
    "css/tokens.css",
    "css/workbench.css",
    "index.html",
    "js/agent-icons.js",
    "js/agent-actions.js",
    "js/island-navigation.js",
    "js/main.js",
    "js/navigation.js",
    "js/window-lifecycle.js",
    "js/page-host.js",
    "js/tasks-page.js",
    "js/sessions-page.js",
    "js/report-panel.js",
    "js/usage-trend.js",
    "js/workspaces-page.js",
    "js/workspace-flow.js",
    "js/prompts-page.js",
    "js/quick-navigation.js",
    "js/window-layout-page.js",
    "js/models-page.js",
    "js/connections-page.js",
    "js/mcp-page.js",
    "js/skills-package-page.js",
    "js/claude-plan-page.js",
    "js/artifact-lifetime.js",
    "js/task-attention.js",
    "js/shell.js",
    "js/tauri.js",
    "js/views.js",
    "probe.html",
];

/// **`[boot] 资源表` 的名单必须盖全 `app/ui/`。**
///
/// 少列一个的后果不是「少一条日志」：那个文件没嵌进二进制时，
/// **没有任何东西会报错**——少一个 CSS 就是**整个形态无样式渲染**，
/// 页面照样出、DOM 照样在，只有像素不对。
#[cfg(test)]
mod embedded_assets_sentinel {
    use super::EMBEDDED_ASSET_SAMPLE;
    use std::path::{Path, PathBuf};

    fn ui_files() -> Vec<String> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui");
        let mut out = Vec::new();
        fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
            let Ok(rd) = std::fs::read_dir(dir) else {
                return;
            };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, base, out);
                } else if let Ok(rel) = p.strip_prefix(base) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        walk(&root, &root, &mut out);
        out.sort();
        out
    }

    /// 名单里**不能有**目录里不存在的条目（否则资源表永远查不出「少了谁」）。
    #[test]
    fn the_sample_list_matches_the_ui_directory_exactly() {
        let mut on_disk = ui_files();
        let mut listed: Vec<String> = EMBEDDED_ASSET_SAMPLE
            .iter()
            .map(|s| s.to_string())
            .collect();
        on_disk.sort();
        listed.sort();
        assert_eq!(
            on_disk, listed,
            "app/ui 下的文件与 `[boot] 资源表` 名单对不上。\n\
             **新加文件却忘了进名单 = 它没嵌进二进制时永远查不出来**——\
             少一个 CSS 的后果是那个形态整个无样式渲染，而且**零报错**。"
        );
    }
}

#[tauri::command]
fn log_from_ui(message: String) {
    log_line(&format!("[webview] {}", message));
}

/// 装在真实 webview 里的**错误陷阱**：先把 `error` / `unhandledrejection`
/// 收进 `window.__aiErrs`，后面的快照才有得读。
///
/// 为什么单独一份而不是并进快照：模块求值阶段的错误**早于**第一次快照发生。
/// 陷阱晚一步装，`__aiErrs` 就永远是空的——而「空的错误列表」与「没有错误」
/// 在日志上长得一模一样，这正是我此前多轮误判的形状。
const WEBVIEW_TRAP_JS: &str = r#"(function () {
  if (window.__aiTrap) { return 'already'; }
  window.__aiTrap = true;
  window.__aiErrs = [];
  function push(s) { if (window.__aiErrs.length < 20) { window.__aiErrs.push(String(s)); } }
  window.addEventListener('error', function (e) {
    push('ERR ' + (e.message || '') + ' @' + (e.filename || '') + ':' + (e.lineno || 0));
  });
  window.addEventListener('unhandledrejection', function (e) {
    var r = e.reason;
    push('REJ ' + ((r && r.message) || r));
  });
  return 'trap-installed';
})()"#;

/// 一次 webview 自省。返回的 JSON 覆盖三个**互斥**分支：
///
/// | 现象 | 结论 |
/// | :--- | :--- |
/// | 回调一行都不触发 | webview 里 JS 引擎压根没在跑 |
/// | `ready=complete` 且 `invoke=false` | 页面活着，是全局 Tauri API 没注入 |
/// | `errs` 非空 | 页面活着但脚本抛了错，原文就在这里 |
///
/// 附带一份 `performance` 资源时序。「资源表里有」与「webview 真的去取过、
/// 并取成功了」是两件事——前者只证明二进制里嵌了字节。
const WEBVIEW_PROBE_JS: &str = r#"(function () {
  var out;
  try {
    var t = window.__TAURI__;
    var root = document.getElementById('root');
    out = {
      href: String(location.href),
      ready: document.readyState,
      tauri: !!t,
      invoke: !!(t && t.core && t.core.invoke),
      errs: window.__aiErrs || [],
      rootChildren: root ? root.children.length : -1,
      rootHtmlLen: root ? root.innerHTML.length : -1,
      // 窗口在屏幕上的几何。⚠️ **这些字段在本项目的 WKWebView 里是空的**：
      // 三个窗口（含隐藏的）一律报 `outerWidth/outerHeight = 0`、
      // `screenX/screenY` 恒为 `0/956`（= 屏幕高度，像填充物的默认值）。
      // 拿它下「窗口被推出屏幕了」这种结论会踩空——权威值在 Rust 侧，
      // 见 setup 里 place 之后的 `[boot] 落位`。
      win: {
        x: screenX, y: screenY, w: outerWidth, h: outerHeight,
        iw: innerWidth, ih: innerHeight, dpr: devicePixelRatio
      },
      res: (performance.getEntriesByType('resource') || []).map(function (e) {
        return e.name.replace('tauri://localhost/', '') + '|' + e.responseStatus + '|' + Math.round(e.duration);
      }),
      nav: (performance.getEntriesByType('navigation') || []).map(function (e) {
        return e.name + '|' + e.responseStatus + '|' + Math.round(e.duration) + '|' + e.transferSize;
      })
    };
  } catch (e) {
    out = { throw: String((e && e.message) || e) };
  }
  // **不经过 Tauri IPC 的兜底通道**：`eval_with_callback` 的回调万一在某个
  // 平台上不回值，这一下仍然会落到本机 webhook 服务器的 `[http]` 日志里。
  // 载荷放进 **path 而不是 query**——那边记日志前会 `split('?')` 把 query 丢掉。
  try {
    fetch('http://127.0.0.1:42000/__probe__/' + out.ready
      + '-tauri' + (out.tauri ? 1 : 0) + '-invoke' + (out.invoke ? 1 : 0)
      + '-errs' + (out.errs || []).length, { mode: 'no-cors' }).catch(function () {});
  } catch (e2) { /* 兜底也失败就算了，回调那条路还在 */ }
  return JSON.stringify(out);
})()"#;

/// 往每个窗口塞一次陷阱、读一次快照。
///
/// 分三个时刻（启动即刻 / +1.5s / +5s）是有意为之：模块求值、首个事件、
/// 首个 tick 各在不同时刻，**只取一个快照就等于在猜哪一步坏了**。
fn probe_webviews(app: &tauri::AppHandle, delay_ms: u64) {
    if delay_ms > 0 {
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            probe_webviews(&app, 0);
        });
        return;
    }
    for label in ["island", "sidebar", "workbench"] {
        let Some(w) = app.get_webview_window(label) else {
            continue;
        };
        // 陷阱必须先于快照下单：eval 是异步派发，同一 webview 上保序。
        if let Err(e) = w.eval(WEBVIEW_TRAP_JS) {
            log_line(&format!("[eval] {label} 陷阱装不上：{e}"));
        }
        let tag = label.to_string();
        if let Err(e) = w.eval_with_callback(WEBVIEW_PROBE_JS, move |result| {
            log_line(&format!("[eval] {tag} {result}"));
        }) {
            log_line(&format!("[eval] {label} 快照取不到：{e}"));
        }
    }
}

/// UI 冒烟：把每个窗口里**所有可点的元素**都点一遍，逐步记下 DOM 与错误数。
///
/// 为什么现在才做：v0.0.242 之前界面从来没渲染过，任何 UI 验证都无从谈起。
/// 而侧边栏的 Provider 页历史上就出过「导航项在、注水函数在、页面函数压根没定义」
/// 的白屏——那一类 bug 只有真的点进去才会现形。
///
/// 它是**驱动**而不是断言：结果落在日志里，由 `scripts/ui-smoke.sh` 判定。
/// 这样驱动脚本本身不用改，判定标准收紧时也不用动 UI。
const UI_SMOKE_JS: &str = r#"(function () {
  if (window.__uiSmoke) { return 'already'; }
  window.__uiSmoke = { state: 'running', found: {}, steps: [] };
  var wait = function (ms) { return new Promise(function (r) { setTimeout(r, ms); }); };
  var root = function () { return document.getElementById('root'); };
  var snap = function (label) {
    var r = root();
    window.__uiSmoke.steps.push({
      label: label,
      html: r ? r.innerHTML.length : -1,
      kids: r ? r.children.length : -1,
      errs: (window.__aiErrs || []).length
    });
  };
  // 元素取不到**不算失败**：窗口形态不同，可点的东西本就不同。
  // 但「一个都没取到」必须显形（found 里是 0），否则等于静默通过。
  var clickAll = function (selector) {
    var els = Array.prototype.slice.call(document.querySelectorAll(selector));
    var chain = Promise.resolve();
    window.__uiSmoke.found[selector] = els.length;
    els.forEach(function (el, index) {
      chain = chain.then(function () {
        var key = el.dataset.nav || el.dataset.agent || el.dataset.reportFormat
          || (el.textContent || '').trim().slice(0, 20) || ('#' + index);
        try {
          el.click();
        } catch (error) {
          window.__uiSmoke.steps.push({
            label: selector + '[' + key + '] 点不动',
            err: String((error && error.message) || error)
          });
          return;
        }
        return wait(240).then(async function () {
          var remoteRoot = document.querySelector('[data-remote-root]');
          if (remoteRoot) {
            var status = await window.__TAURI__.core.invoke('remote_status');
            if (remoteRoot.textContent.indexOf(status.secretName) < 0) throw new Error('远程通知密钥条目名未显示');
            if (!remoteRoot.querySelector('[data-remote-test]') || !remoteRoot.querySelector('[data-remote-history]')) throw new Error('远程测试或发送记录入口缺失');
            var advanced = remoteRoot.querySelector('[data-remote-advanced]');
            if (!advanced || advanced.open) throw new Error('远程高级设置应默认折叠');
            if (remoteRoot.querySelectorAll('[data-remote-kind] option').length < 7) throw new Error('IM 渠道缺失');
          }
          snap(selector + ' → ' + key);
        });
      });
    });
    return chain;
  };
  // **等 DOM 稳下来再点**，而不是靠固定延时。
  //
  // 原来的写法是「等到 root 非空」，而收起态的窄条已经让 root 非空了——
  // 于是这个等待等于没等。改成「连续两次采样长度不变」才算稳，
  // 灵动岛展开那一下（renderCard + place_island + resize）才等得到。
  var settle = function () {
    var last = -1, stable = 0;
    var step = function (i) {
      if (i >= 45) { return Promise.resolve(false); }
      var r = root();
      var now = r ? r.innerHTML.length : -1;
      if (now > 0 && now === last) {
        if (++stable >= 2) { return Promise.resolve(true); }
      } else {
        stable = 0;
      }
      last = now;
      return wait(200).then(function () { return step(i + 1); });
    };
    return step(0);
  };
  (async function () {
    try {
      // **岛的 boot 第一件事就是 `place_island`。**
      //
      // 它在 `boot()` 里是 `await` 的，而它后面才轮到两个事件订阅与 `--expand`
      // 的定时器——所以它一旦不 resolve，整条 boot 链就停在那里，
      // 表现为「岛永远是那条 101B 的窄条、一个可点元素都没有」，
      // 而且**没有任何报错**。
      window.__uiSmoke.bootArgs = 'n/a（非灵动岛）';
      if (document.documentElement.className.indexOf('shell-island') >= 0) {
        // 只问**只读**的 `get_boot_args`：它在 main.js 里是 `.catch(() => ({expand:false}))`
        // **静默兜底**的，一旦失败岛就无声地不展开，而什么都不会报。
        //
        // 曾经顺手在这里也调一次 `place_island`「顺便验一下它通不通」——
        // **那是自伤**：它会把岛窗口改成 330×120，在测量途中改掉了被测状态。
        // 诊断动作只要动了一点状态，它测出来的就不是原来那个东西了
        // （同类的还有：在 +2.6s 补发一次 `tray://toggle`，把刚展开的岛又收了回去）。
        window.__uiSmoke.bootArgs = await Promise.race([
          window.__TAURI__.core.invoke('get_boot_args')
            .then(function (a) { return JSON.stringify(a); })
            .catch(function (e) { return 'rejected: ' + e; }),
          new Promise(function (r) { setTimeout(function () { r('**3s 未 resolve**'); }, 3000); })
        ]);
      }
      await settle();
      // 在真正的 WebView 中验证全部本地标识，而非只检查 DOM 中出现了类名。
      var identityModule = await import(new URL('js/agent-icons.js', location.href).href);
      var identityProbe = document.createElement('div');
      identityProbe.setAttribute('aria-hidden', 'true');
      identityProbe.style.cssText = 'position:fixed;left:-10000px;top:0;pointer-events:none;';
      identityProbe.innerHTML = Object.keys(identityModule.agentIdentities).map(function (id) {
        return identityModule.agentIcon({ id: id }, 'wb-agent-glyph');
      }).join('') + identityModule.agentIcon({ id: 'custom-probe', name: '自定义 Agent' });
      if (identityProbe.querySelectorAll('.agent-mark, .agent-brand-image').length !== identityProbe.children.length)
        throw new Error('Agent 缺少图形标识');
      document.body.appendChild(identityProbe);
      try {
        Array.from(identityProbe.children).forEach(function (avatar) {
          var bounds = avatar.getBoundingClientRect();
          if (Math.abs(bounds.width-32)>0.5 || Math.abs(bounds.height-32)>0.5) throw new Error('工作台图标容器大小不一致');
        });
        await Promise.all(Array.from(identityProbe.querySelectorAll('.agent-mark, .agent-brand-image')).map(async function (mark) {
          if (mark instanceof HTMLImageElement) {
            await Promise.race([mark.decode(), wait(3000).then(function () { throw new Error('产品原始图标加载超时'); })]);
            if (mark.naturalWidth < 16 || mark.getBoundingClientRect().width < 16) throw new Error('产品原始图标尺寸无效');
            if (/\.png$/.test(mark.src)) {
              var canvas=document.createElement('canvas');canvas.width=mark.naturalWidth;canvas.height=mark.naturalHeight;
              var context=canvas.getContext('2d');context.drawImage(mark,0,0);
              var pixels=context.getImageData(0,0,canvas.width,canvas.height).data;
              var left=canvas.width,right=-1,top=canvas.height,bottom=-1;
              for(var y=0;y<canvas.height;y++) for(var x=0;x<canvas.width;x++) {
                if(pixels[(y*canvas.width+x)*4+3]>=32) {left=Math.min(left,x);right=Math.max(right,x);top=Math.min(top,y);bottom=Math.max(bottom,y);}
              }
              var bounds=mark.getBoundingClientRect();
              var visible=Math.max((right-left+1)/canvas.width*bounds.width,(bottom-top+1)/canvas.height*bounds.height);
              if(Math.abs(visible-28)>1) throw new Error('工作台原色图标可见尺寸不一致');
            }
            return;
          }
          var mask = getComputedStyle(mark).maskImage || getComputedStyle(mark).webkitMaskImage;
          var match = mask.match(/^url\(["']?(.*?)["']?\)$/);
          if (!match || mark.getBoundingClientRect().width < 16) throw new Error('Agent 图标样式或遮罩缺失');
          var image = new Image();
          image.src = match[1];
          await Promise.race([image.decode(), wait(3000).then(function () { throw new Error('Agent 图标加载超时'); })]);
        }));
        window.__uiSmoke.agentIcons = identityProbe.children.length;
      } finally { identityProbe.remove(); }
      // 灵动岛额外验一件具体的事：**卡片有没有出现过，多久出现的**。
      // 「DOM 长度稳定」分不清「一直是窄条」与「卡片闪过又被收回去了」，
      // 而这两种要查的地方完全不同。-1 表示 6 秒内一次都没出现过。
      if (document.documentElement.className.indexOf('shell-island') >= 0) {
        var t0 = Date.now(), appeared = -1;
        while (Date.now() - t0 < 6000) {
          if (document.querySelector('.card')) { appeared = Date.now() - t0; break; }
          await wait(100);
        }
        window.__uiSmoke.cardAfterMs = appeared;
      }
      window.__uiSmoke.cards = document.querySelectorAll('.card').length;
      snap('起点');
      // **启动路由有没有生效**：`--route=xxx` 是托盘 / CLI 用的真实入口，
      // 而冒烟平时靠**逐个点击**覆盖页面——那是另一条分支。
      //
      // ⚠️ 必须在**点击循环之前**读：第一版放在最后，于是读到的是
      // 「最后点的那一项」，**根本观察不到启动时的落点**——
      // 于是把启动路由短路掉它照样报「生效」。
      //
      // 而这恰恰是「变异验证抓到的」与「我想当然以为没问题的」之间的差别。
      var activeNav = document.querySelector('.sb-item.is-active');
      window.__uiSmoke.activeNav = activeNav
        ? (activeNav.textContent || '').trim().slice(0, 12) : '';
      window.__uiSmoke.firstNav = (function () {
        var f = document.querySelector('.sb-item');
        return f ? (f.textContent || '').trim().slice(0, 12) : '';
      })();
      // 在实际 WebView 中派发按钮按下/松开，确认不会走拖拽定位命令。
      var islandHeader = document.querySelector('.header[data-drag]');
      var islandButton = islandHeader && islandHeader.querySelector('button');
      if (islandButton) {
        var snapCalls = 0;
        var unlistenSnap = await window.__TAURI__.event.listen('test://snap', function () { snapCalls++; });
        try {
          islandButton.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0, screenX: 100, screenY: 100 }));
          islandButton.dispatchEvent(new MouseEvent('mouseup', { bubbles: true, button: 0, screenX: 100, screenY: 100 }));
          await wait(180);
          if (snapCalls) throw new Error('按钮点击触发拖拽吸附：' + snapCalls);
          snap('按钮按下/松开未触发定位命令');
        } finally { unlistenSnap(); }
      }
      await clickAll('[data-nav]');
      await clickAll('[data-wb-nav]');
      var overview = document.querySelector('[data-wb-nav="overview"]');
      if (overview) { overview.click(); await wait(300); }
      await clickAll('[data-analytics]');
      await clickAll('[data-agent]');
      await clickAll('[data-back]');
      if (document.querySelector('[data-island-settings]')) {
        await clickAll('[data-island-settings]');
        if (!document.querySelector('[data-settings-root]') || document.querySelector('[data-report-root]')) throw new Error('灵动岛设置入口未进入设置页');
        await clickAll('[data-back]');
      }
      var usageNav = document.querySelector('.wb-nav [data-wb-nav="tokenAnalytics"]');
      if (usageNav) {
        usageNav.click(); await wait(300);
        var usageReport = document.querySelector('[data-usage-report]');
        if (!usageReport) throw new Error('用量页缺少报告入口');
        if (!usageReport.open) usageReport.querySelector('summary').click();
      }
      await clickAll('[data-report-format]');
      var reportPanel = document.querySelector('[data-report-panel]');
      if (reportPanel) {
        var reportWait = Date.now();
        while (reportPanel.getAttribute('aria-busy') === 'true' && Date.now() - reportWait < 6000) await wait(100);
        var reportStatus = reportPanel.querySelector('[data-report-status]');
        if (!reportStatus || !/报告已生成|暂无报告内容/.test(reportStatus.textContent)) throw new Error('报告生成未完成');
        snap('用量页报告生成与状态反馈');
      }
      await clickAll('[data-usage-range]');
      await clickAll('[data-search]');
      // **真打一个字进去**：点开搜索框不等于搜索能用。
      //
      // `[data-search]` 只把输入框显示出来；真正干活的是 `#searchInput` 的
      // `input` 事件 → 写 `st.searchText` → 重画并过滤列表。这三步任何一步断掉，
      // 界面都只是「多了一个输入框」而**不报任何错**——点它的人看不出搜索坏了。
      //
      // 这里派发真实的 `input` 事件（与用户敲键盘走的是同一条路），
      // 再验过滤**真的跑了**：空态里会出现「未找到匹配「…」的智能体」。
      //
      // ⚠️ 边界：本机没有正在跑的 Agent，所以**只有「无匹配」这一支可观测**；
      // 「有匹配时列表变短」那一支要等真有 Agent 时才验得到。
      //
      // ⚠️ 位置也重要：这一段必须排在 `clickAll('[data-collapse]')` **之前**——
      // 收起之后卡片连同输入框一起没了，第一版就栽在这儿（点开过、找不到输入框，
      // 于是「输入框不存在」和「我没在正确时机找」看起来一模一样）。
      window.__uiSmoke.search = { inputFound: false, filtered: false };
      var searchInput = document.querySelector('#searchInput');
      if (searchInput) {
        window.__uiSmoke.search.inputFound = true;
        var probe = 'zz-不存在的名字';
        searchInput.value = probe;
        searchInput.dispatchEvent(new Event('input', { bubbles: true }));
        await wait(240);
        // ⚠️ **这里刻意不做通过/失败的判定。**
        //
        // 试过两次都失败，两次的原因都记在这儿，因为它们**不是**同一类问题：
        // ① 判据写成 `indexOf('未找到匹配') >= 0 || indexOf('empty') >= 0`——
        //    后半句会匹配 HTML 里**任何**含 "empty" 的子串 ⇒ 假阳性。
        // ② 收紧成只认「未找到匹配」之后，**去掉输入事件它照样通过**。
        //    真因不是判据松，而是**这台机器上没有正在跑的 Agent**：
        //    `visible` 本来就是空的，于是搜索框一打开空态就渲染，
        //    跟打了什么字**无法区分**。
        //
        // 所以「过滤真的生效了」这件事**在本机不可验**。一个证不了失败的门禁
        // 比没有门禁更糟——它看起来像覆盖。所以这里只**跑这一步**：
        // 它仍能抓到崩溃（异常会进 `errs`），但不假装验过了什么。
        var html = root().innerHTML;
        window.__uiSmoke.search.emptyStateRendered = html.indexOf('未找到匹配') >= 0;
        window.__uiSmoke.search.verdict = '未验：本机没有在跑的 Agent，空态与「过滤生效」无法区分';
        window.__uiSmoke.search.value = searchInput.value;
        snap('搜索过滤「' + probe + '」');
        // 还原，别把「搜索开着」的状态留给后面的人看
        searchInput.value = '';
        searchInput.dispatchEvent(new Event('input', { bubbles: true }));
        await wait(120);
      }
      if (document.documentElement.classList.contains('shell-sidebar')) {
        var mainModule = await import(new URL('js/main.js', location.href).href);
        var viewModule = await import(new URL('js/views.js', location.href).href);
        var state = mainModule.getState(), originalEngine = state.engine, originalRoute = state.route;
        try {
          state.route = 'list';
          state.engine = { snapshots: [{id:'workbuddy', name:'WorkBuddy', level:'completed', level_label:'已完成', process_running:true, last_activity_text:'', token_usage:null}], grand_total:{}, latest_event:null };
          viewModule.renderSidebar();
          var row = document.querySelector('.sb-agent');
          if (!row || document.querySelectorAll('.sb-agent').length !== 1) throw new Error('单条完成记录未显示');
          var icon = row.querySelector('.agent-avatar').getBoundingClientRect();
          var name = row.querySelector('.name').getBoundingClientRect();
          var meta = row.querySelector('.meta').getBoundingClientRect();
          if (name.left < icon.right + 6 || Math.abs(name.left-meta.left)>1 || meta.top < name.bottom) throw new Error('单条完成记录图标与文字错位');
          snap('单条已完成记录的原生行布局');
        } finally { state.engine=originalEngine; state.route=originalRoute; viewModule.renderSidebar(); }
      }
      await clickAll('[data-theme]');
      await clickAll('[data-collapse]');
      // **工作台监控栏的空态到底在不在**。
      //
      // 拍图时左栏看着是空的，而代码里写着 `还没有检测到运行中的智能体`。
      // 分不清是「没进 DOM」还是「11px + opacity .5 太暗」——前者是 bug、后者是观感，
      // 修法完全不同。直接问 DOM，不靠眼睛。
      var monitorBox = document.querySelector('[data-wb-monitor]');
      var emptyEl = document.querySelector('.wb-empty');
      window.__uiSmoke.emptyState = {
        monitorBox: !!monitorBox,
        runningAgents: monitorBox ? monitorBox.querySelectorAll('[data-agent]').length : -1,
        emptyEl: !!emptyEl,
        text: emptyEl ? (emptyEl.textContent || '').slice(0, 30) : '',
        // 元素在但看不见 ⇒ 尺寸或颜色有问题，一并量出来
        box: emptyEl ? emptyEl.getBoundingClientRect().height : -1
      };
      window.__uiSmoke.state = 'done';
    } catch (error) {
      window.__uiSmoke.state = 'threw';
      window.__uiSmoke.fatal = String((error && error.message) || error);
    }
    window.__uiSmoke.errs = window.__aiErrs || [];
    // **驱动自己把结果发出去**。
    //
    // 原来是由 Rust 在固定时刻（+10.6s）eval 一次读回，而驱动的耗时是**可变的**
    // （灵动岛多出「等展开」与「搜索」两段）。机器一忙，读回就落在驱动跑完之前，
    // 于是日志里留下 `"state":"running"`、判据随之变红——实测偶发。
    //
    // 定时读回不该和被测对象赛跑。**让被测对象自己报告**就没有竞态了；
    // 固定读回保留着，只当诊断用。
    try {
      var shell = document.documentElement.className.indexOf('shell-sidebar') >= 0
        ? 'sidebar' : (document.documentElement.className.indexOf('shell-workbench') >= 0 ? 'workbench' : 'island');
      window.__TAURI__.core.invoke('log_from_ui', {
        message: 'SMOKE_RESULT ' + shell + ' ' + JSON.stringify({
          state: window.__uiSmoke.state,
          // 驱动抛异常时 `steps`/`found` 都是空的，没有 fatal 这条输出就是
          // 「threw 但不知道为什么」——冒烟门禁红得没有可操作性。
          fatal: window.__uiSmoke.fatal,
          found: window.__uiSmoke.found,
          steps: window.__uiSmoke.steps,
          bootArgs: window.__uiSmoke.bootArgs,
          agentIcons: window.__uiSmoke.agentIcons,
          cards: window.__uiSmoke.cards,
          search: window.__uiSmoke.search,
          emptyState: window.__uiSmoke.emptyState,
          activeNav: window.__uiSmoke.activeNav,
          firstNav: window.__uiSmoke.firstNav,
          errs: window.__uiSmoke.errs
        })
      }).catch(function () {});
    } catch (e) { /* 发不出去就算了，定时读回还在 */ }
  })();
  return 'started';
})()"#;

/// 读回冒烟结果。`eval` 的返回值是脚本的完成值，所以这里直接序列化。
const UI_SMOKE_READ_JS: &str = "JSON.stringify(window.__uiSmoke || { state: 'never-ran' })";

/// 冒烟调度：1.2s 后开跑（等 boot 把 DOM 建起来），8s 后读结果。
///
/// 每个窗口**各跑一遍**同一个脚本——它按各窗口自己的 DOM 走，
/// 所以侧边栏点导航、灵动岛点分析页、工作台点报告格式，三条路一次覆盖。
fn schedule_ui_smoke(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        // 灵动岛必须**展开**了才点得到卡片里的东西：收起态下 root 里只有一个
        // 101B 的窄条，`[data-analytics]` / `[data-agent]` / `[data-search]` /
        // `[data-theme]` / `[data-collapse]` 一个都点不到。
        //
        // 展开走 `--expand` 启动参数（`get_boot_args` → `boot.expand` → 开机 1.5s 后
        // `expand()`），也就是深链 `Expand` 与演示模式用的**同一条产品路径**。
        // 曾经试过在这里 `emit_to("island", "tray://toggle")`，不可靠：页面会加载
        // 两次（日志里「Tauri API 就绪」出现 6 次 = 两轮 × 三窗口），定时发出的
        // 事件正好落在两次加载的间隙里，那一刻没有监听者——事件石沉大海。
        // 启动参数没有这个竞态。
        // 展开只走 `--expand` 启动参数这一条路（产品路径，与深链 `Expand`、演示模式同一条）。
        //
        // 曾经在这里补发一次 `tray://toggle`「保险一下」——**那是把测量动作变成了扰动**：
        // 此时 `state.expanded` 已经是 true，toggle 语义于是把岛又收了回去，
        // 于是「已展开」明明打过、`cards` 却是 0。
        // 诊断动作只要改了一点被测状态，它测出来的就不是原来那个东西了。
        std::thread::sleep(std::time::Duration::from_millis(2600));

        for label in ["island", "sidebar", "workbench"] {
            if let Some(w) = app.get_webview_window(label) {
                let _ = w.eval(UI_SMOKE_JS);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(8000));
        for label in ["island", "sidebar", "workbench"] {
            let Some(w) = app.get_webview_window(label) else {
                continue;
            };
            let tag = format!("{label}/smoke");
            let tag2 = tag.clone();
            if let Err(e) = w.eval_with_callback(UI_SMOKE_READ_JS, move |result| {
                log_line(&format!("[smoke] {tag2} {result}"));
            }) {
                log_line(&format!("[smoke] {tag} 读不到：{e}"));
            }
        }
    });
}

/// `--ui-smoke`：只在这一次运行里打开 UI 冒烟，与设置无关。
fn ui_smoke_requested() -> bool {
    std::env::args().any(|a| a == "--ui-smoke")
}

/// 深链投递的事件落进事件队列。
///
/// 标记为**外部投递**（`externally_delivered`）：它来自一条 URL，
/// 而 URL 可以由任何进程拼出来。岛内必须能把它与本机自发的事件区分开，
/// 而且按本仓的既有纪律，外部投递**一律不外发**——
/// 深链 / `/notify` / 没令牌的自报都不是用户自己敲的，不该从机器上再飞出去。
#[tauri::command]
fn notify_external(
    state: State<SharedEngine>,
    agent: String,
    kind: String,
    message: String,
    detail: String,
) -> bool {
    let mut engine = state.lock().unwrap();
    // 投递目标必须能解析到已知档案：深链的 agent 参数是任意字符串
    let Some(profile) = crate::registry::builtin()
        .into_iter()
        .find(|p| p.id.eq_ignore_ascii_case(agent.trim()))
    else {
        log_line(&format!("[deeplink] notify 投递给未知档案：{agent}"));
        return false;
    };
    let event_type = match kind.as_str() {
        "attention" | "confirm" | "wait" => "attention",
        "costspike" | "cost" | "budget" | "alert" => "costSpike",
        _ => "completed",
    };
    engine.push_event(models::AgentTaskEvent {
        id: format!("deeplink-{}", tokens::now_ms()),
        agent_id: profile.id.clone(),
        agent_name: profile.name.clone(),
        event_type: event_type.to_string(),
        timestamp: tokens::now_ms(),
        message: if message.is_empty() {
            None
        } else {
            Some(message)
        },
        detail: if detail.is_empty() {
            None
        } else {
            Some(detail)
        },
        duration: 0.0,
        externally_delivered: true,
    });
    true
}

/// 处理一条 `agentisland://` 深链。返回是否真的做了动作。
///
/// **执行面只做窗口可见性与路由，不做两件事**（与 Swift 侧同一条纪律）：
/// · **不写剪贴板**——`open agentisland://export` 无需任何确认，
///   而 `clearContents` 会让一个 npm postinstall 或 `.command` 脚本
///   静默销毁用户正准备粘贴的密码。自动化请走显式的 `agentisland report -o`。
/// · **不杀进程**——`clean` 只把用户带到工作台的清理区，点不点由人决定。
pub fn handle_deep_link(app: &AppHandle, url: &str) -> bool {
    let Some(action) = deeplink::parse(url) else {
        log_line(&format!("[deeplink] 认不出的指令：{url}"));
        return false;
    };

    // `notify` 要投递到**已知档案**。档案表来自 `registry::builtin()`（自由函数），
    // **刻意不经过引擎**：深链处理可能由任意线程调进来，
    // 在这里锁引擎多一层拿不到任何东西，却添一条死锁的路。
    if let deeplink::Action::Notify { .. } = &action {
        let known: Vec<String> = crate::registry::builtin()
            .iter()
            .map(|p| p.id.clone())
            .collect();
        let Some((agent, kind, message, detail)) = deeplink::resolve_notify(&action, &known) else {
            // 拒绝理由要写进日志：用户看到「什么都没发生」时，
            // 唯一能查的就是这一行
            log_line(&format!("[deeplink] 拒绝投递给未知智能体：{url}"));
            return false;
        };
        let _ = app.emit(
            "deeplink://notify",
            serde_json::json!({ "agent": agent, "kind": kind, "message": message, "detail": detail }),
        );
        return true;
    }

    if action.reveals_window() {
        // 导航交给已订阅的三个窗口，各自决定显隐与页面；
        // 启动期间的意图由 Mailbox 保存至前端就绪，
        // 免得 Rust 侧再写一份显隐规则（两份规则迟早只改一处）
        send_navigation(app, &action);
        if matches!(action, deeplink::Action::Workbench) {
            reveal_workbench_window(app);
        }
    } else if let deeplink::Action::Settings(tab) = &action {
        // 设置是独立窗口：先把它显示出来，岛保持当前形态
        if let Some(win) = app.get_webview_window("settings") {
            let _ = win.show();
            let _ = win.set_focus();
        }
        let _ = app.emit("deeplink://settings", serde_json::json!({ "tab": tab }));
    }
    true
}

fn main() {
    // `--selftest` / `selftest`：无头自检（对应 Swift 的 `agentisland selftest`）。
    // 放在最前面：它不该启动 UI、采集或任何后台线程——自检的全部价值是
    // 「在这台机器上，判定逻辑本身还对不对」，被采集的副作用搅进来就不再是那个问题的答案。
    // CLI 子命令（`agentisland status` / `doctor` / `tokens` / `state` …）。
    // **必须在建引擎、开线程、起窗口之前**：`status` 的全部价值是「快」，
    // 而拉起 Tauri 再退出比它自己采完一拍慢一个量级。
    let argv: Vec<String> = std::env::args().collect();
    if let Some(code) = claude_plan_receiver::cli(&argv) {
        std::process::exit(code);
    }
    // Explicit diagnostic mode: no WebViews, hooks, settings writes or remote delivery.
    if argv.iter().any(|a| a == "--memory-core-probe") {
        let options = match resource_diagnostics::Options::parse(&argv) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        };
        if argv.iter().any(|a| a == "--process-only") {
            let mut monitor = procmon::ProcessMonitor::new();
            for sample in 0..options.samples {
                let start = std::time::Instant::now();
                monitor.refresh();
                println!(
                    "{}",
                    serde_json::json!({"schema_version":1,"sample":sample+1,"recorded_ms":tokens::now_ms(),"mode":"process_only","process_us":resource_diagnostics::elapsed(Some(start))})
                );
                if sample + 1 < options.samples {
                    std::thread::sleep(std::time::Duration::from_millis(options.interval_ms));
                }
            }
            return;
        }
        let (_tx, rx) = mpsc::channel();
        let mut settings = Settings::load();
        settings.remote_policy.master_enabled = false;
        let mut engine = ActivityEngine::new(settings, rx);
        engine.refresh_usage = !argv.iter().any(|a| a == "--without-usage");
        engine.enable_resource_diagnostics();
        for sample in 0..options.samples {
            engine.tick();
            let reclaim = !argv.iter().any(|a| a == "--retain-allocator-pages");
            let start = std::time::Instant::now();
            if reclaim {
                memory::reclaim_idle_pages();
            }
            let reclaim_us = resource_diagnostics::elapsed(Some(start));
            println!(
                "{}",
                serde_json::json!({"schema_version":1,"sample":sample+1,"recorded_ms":tokens::now_ms(),"mode":"core","usage_enabled":engine.refresh_usage,"reclaim_enabled":reclaim,"reclaim_us":reclaim_us,"tick":engine.resource_diagnostics()})
            );
            if sample + 1 < options.samples {
                std::thread::sleep(std::time::Duration::from_millis(options.interval_ms));
            }
        }
        return;
    }
    if let Some(code) = cli::try_run(&argv) {
        std::process::exit(code);
    }
    let (tx, rx) = mpsc::channel::<models::AgentTaskEvent>();
    let mut settings = Settings::load();
    if background_test_requested() {
        settings.remote_policy.master_enabled = false;
    }
    let mut engine = ActivityEngine::new(settings, rx);
    let demo = std::env::args().any(|a| a == "--demo");
    engine.demo_mode = demo;
    #[cfg(target_os = "macos")]
    if !demo && !isolated_instance_requested() {
        if let Ok(mut runtime) = crate::claude_plan_runtime::Runtime::at_default() {
            if runtime.resume_saved().is_err() {
                log_line("[claude-plan] 采集重启状态未核实，未声明运行。");
            }
            engine.claude_plan_runtime = Some(Arc::new(Mutex::new(runtime)));
        }
    }
    let shared: SharedEngine = Arc::new(Mutex::new(engine));

    std::panic::set_hook(Box::new(|info| {
        log_line(&format!("[panic] {}", info));
    }));
    // **每次启动一行 run 标记**：日志文件是累积的，只有它能把当次运行切出来。
    // 少了这一行，「某类日志没出现」这个结论就不可信——而我恰好靠它下了好几轮结论。
    log_line(&format!("[run] pid={} 启动", std::process::id()));
    log_line("=== boot ===");

    #[cfg(target_os = "macos")]
    let startup_guard = acquire_startup_guard().expect("应用冷启动互斥锁应当可用");

    let mut context = tauri::generate_context!();
    if isolated_instance_requested() {
        context
            .config_mut()
            .identifier
            .push_str(&format!(".test.p{}", std::process::id()));
    }
    tauri::Builder::default()
        // 必须早于其他插件：第二次打开复用现有应用，不再创建窗口与托盘。
        .plugin(tauri_plugin_single_instance::init(|_app, _args, _cwd| {
            // 普通重复打开只复用常驻实例；工作台由菜单或明确深链唤回。
        }))
        // **页面加载完成时记一笔。**
        //
        // 这条钩子的用途很具体：界面是一个 webview，而「webview 里到底发生了什么」
        // 此前**没有任何 Rust 侧证据**——`[webview]` 日志是 JS 自己写的，
        // JS 不跑它就必然是空的，于是「没日志」无法区分「页面没加载」与
        // 「加载了但脚本没执行」。这条钩子把两件事分开：
        // 它响了 ⇒ 页面确实加载了；它不响 ⇒ 资源或路径有问题。
        .on_page_load(|webview, payload| {
            let label = webview.label();
            crate::log_line(&format!("[page] {label} 加载完成 url={}", payload.url()));
        })
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_deep_link::init())
        .manage(shared.clone())
        .manage(WindowLayoutStore::new(
            window_layout_service::Store::with_journal(window_layout_journal::Store::at_default()),
        ))
        .manage(WindowRuleStore::new(
            window_layout_rules::Store::at_default(),
        ))
        .manage(WorkspaceStore::new(workspaces::Store::at_default()))
        .manage(SessionCatalogStore::new(
            session_catalog::Store::at_default(),
        ))
        .manage(PromptStore::new(prompts::Store::at_default()))
        .manage(SkillPackageStore::new())
        .manage(ConnectionStore::new(connections::Store::at_default()))
        .manage(Mutex::new(crate::tasks::Store::at_default()))
        .manage(Mutex::new(navigation::Mailbox::default()))
        .manage(Mutex::new(window_lifecycle::WorkbenchLease::default()))
        .on_window_event(|window, event| {
            // 标准关闭按钮收起工作台，托盘和重复打开仍可唤回原窗口。
            if window.label() == "workbench" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    conceal_workbench_window(window.app_handle());
                }
            }
        })
        .setup(move |app| {
            set_dock_presence(app.handle(), false);
            let (saved_mode, edge, width) = {
                let e = shared.lock().unwrap();
                (
                    crate::models::ShellMode::parse(&e.settings.shell_mode),
                    crate::models::DockEdge::parse(&e.settings.sidebar_edge),
                    e.settings.sidebar_width,
                )
            };
            let startup_mode = shell_arg_override().unwrap_or(saved_mode);
            show_resident_window(app.handle(), startup_mode, edge, width)?;
            if ui_smoke_requested() {
                for label in ["island", "sidebar", "workbench"] {
                    ensure_window(app.handle(), label)?;
                }
            }

            // 原生显隐回归：只在明确测试参数下执行，普通启动没有定时切窗。
            #[cfg(target_os = "macos")]
            if std::env::args().any(|arg| arg == "--dock-smoke") {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    reveal_workbench_window(&handle);
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    conceal_workbench_window(&handle);
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    reveal_workbench_window(&handle);
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    if let Some(window) = handle.get_webview_window("workbench") {
                        let _ = window.close();
                    }
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    handle.exit(0);
                });
            }
            let appearance = shared.lock().unwrap().settings.appearance.clone();
            apply_window_appearance(app.handle(), &appearance);
            // 全局热键：按设置里的开关注册。
            //
            // 刻意**不在设置变化时重注册**：热键注册要在主线程做，而设置改完
            // 立刻生效是本轮的承诺之一——那就在每拍检查一次「开关状态与
            // 当前注册状态是否一致」，不一致才动。注册失败只记日志，
            // 不打断引擎循环：热键是锦上添花，不该让它把监控整个拖停。
            if !isolated_instance_requested() {
                // 热键回调：与托盘同一个动作（展开 / 收起），不另发明一套
                if let Ok(shortcut) = parse_hotkey() {
                    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
                    let _ = app
                        .global_shortcut()
                        .on_shortcut(shortcut, move |app, _, event| {
                            if matches!(event.state(), ShortcutState::Pressed) {
                                let _ = app.emit("tray://toggle", ());
                            }
                        });
                } else {
                    log_line("[globalHotKey] 组合键解析失败，未注册");
                }
                // 注册状态对齐：每 5 秒比一次「设置要什么」与「系统里是什么」，
                // 不一致才动。`apply_hotkey` 幂等，所以多跑几次没有副作用
                let shared3 = shared.clone();
                let handle3 = app.handle().clone();
                std::thread::spawn(move || loop {
                    let want = shared3
                        .lock()
                        .map(|e| e.settings.global_hot_key_enabled)
                        .unwrap_or(false);
                    if let Err(error) = apply_hotkey(&handle3, want) {
                        log_line(&format!("[globalHotKey] {error}"));
                    }
                    std::thread::sleep(std::time::Duration::from_secs(5));
                });
            }
            // 深链：冷启动那条 URL 已经被 `get_current` 收走了，
            // 这里取出来消费掉——否则用户从 Raycast 冷启动应用时，
            // 岛会开起来但**什么都不发生**，而那正是他点进来的原因。
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let app_handle = app.handle().clone();
                // 插件返回 `Result`：取不到不是「没有深链」而是「不知道」，
                // 所以失败也要留痕——冷启动路径上这是最常见的静默失败
                match app_handle.deep_link().get_current() {
                    Ok(Some(urls)) => {
                        for url in urls {
                            handle_deep_link(&app_handle, url.as_str());
                        }
                    }
                    Ok(None) => {}
                    Err(error) => log_line(&format!("[deeplink] 取冷启动 URL 失败: {error}")),
                }
            }

            // 运行期收到的新深链
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let app_handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    // 一次事件可能带多条 URL（批量粘贴时会发生），逐条消费
                    let handle = app_handle.clone();
                    let urls = event.urls();
                    // Emission/window operations must not re-enter the plugin callback.
                    std::thread::spawn(move || {
                        for url in urls {
                            handle_deep_link(&handle, url.as_str());
                        }
                    });
                });
            }

            // 引擎采样线程
            let shared2 = shared.clone();
            let handle = app.handle().clone();
            std::thread::spawn(move || engine_loop(shared2, handle));

            // 本地 Webhook（Rust 端 127.0.0.1:42000，与 Swift 的 41999 分开以免静默抢端口）
            if !isolated_instance_requested() {
                let shared_for_webhook = shared.clone();
                std::thread::spawn(move || {
                    let _server = webhook::LocalEventServer::start(tx, shared_for_webhook);
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(3600));
                    }
                });

                // 托盘
                let toggle =
                    MenuItem::with_id(app, "toggle", "展开 / 收起灵动岛", true, None::<&str>)?;
                // 工作台是**第三个窗口**，不能只靠深链进：深链在这台机器上被
                // Swift 版抢走的可能性是真实存在的（两个应用都声明了同一个 scheme），
                // 所以它必须有一条不经过 URL scheme 的入口。
                let workbench =
                    MenuItem::with_id(app, "workbench", "打开工作台", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "退出 AgentIsland", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&toggle, &workbench, &quit])?;
                TrayIconBuilder::with_id("main")
                    .icon(if cfg!(target_os = "macos") {
                        menu_bar_icon()
                    } else {
                        app.default_window_icon().unwrap().clone()
                    })
                    .icon_as_template(true)
                    .tooltip("AgentIsland")
                    .menu(&menu)
                    .on_menu_event(|app, event| {
                        match event.id.as_ref() {
                            "toggle" => {
                                let _ = app.emit("tray://toggle", ());
                            }
                            // 与深链那条路径共用同一个命令，不另写一份显隐规则
                            "workbench" => reveal_workbench_window(app),
                            "quit" => app.exit(0),
                            _ => {}
                        }
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let tauri::tray::TrayIconEvent::Click {
                            button: tauri::tray::MouseButton::Left,
                            ..
                        } = event
                        {
                            let _ = tray.app_handle().emit("tray://toggle", ());
                        }
                    })
                    .build(app)?;
            }

            // **启动痕迹：把真实建出来的窗口逐个记下来。**
            //
            // 为什么加这条：v0.0.230 之前，Rust 端「界面不回调」这件事只能从
            // 「没有 [webview] 日志」反推——而那条日志**本来就是 webview 写的**，
            // webview 不跑 ⇒ 它必然是空的 ⇒ 这条证据无法自证。
            // 从 Rust 侧记一份「我建了哪些窗口」，至少能把「窗口没建出来」
            // 与「窗口建了但里面没跑 JS」分开。
            let labels: Vec<String> = app.webview_windows().keys().cloned().collect();
            log_line(&format!("[boot] 建出的窗口：{labels:?}"));
            for label in &labels {
                if let Some(w) = app.get_webview_window(label) {
                    // **尺寸与位置也要记**：配置里 island 是 372×520，
                    // 而实测到的是 0×0 —— 「窗口建出来」和「窗口按配置摆好」是两件事，
                    // 只记 url 与 visible 看不见后者。
                    log_line(&format!(
                        "[boot] {label} url={:?} visible={} size={:?} pos={:?}",
                        w.url(),
                        w.is_visible().unwrap_or(false),
                        w.outer_size(),
                        w.outer_position()
                    ));
                }
            }

            // **直接问一遍嵌进来的资源里到底有什么。**
            //
            // 为什么非问不可：此前「资源已嵌入」这个判断只是**在二进制里搜到了
            // 资源路径字符串**——而那些路径同样来自内嵌的 tauri.conf.json，
            // 搜到它们**不能证明资源本体在里面**。界面白屏时，最可能的原因就是
            // `frontendDist` 压根没被打进去（`generate_context!` 在找不到目录时
            // 会安静地嵌一个空的资源表），而那时从外面看**一切正常**。
            //
            // 资源表是**压缩**的，所以正文搜不到；只能问解析器本人。
            {
                let resolver = app.asset_resolver();
                let mut seen: Vec<String> = Vec::new();
                // 名单要**盖全** `app/ui/` 下的每一个文件：少列一个，
                // 那个文件没嵌进去时就**没人会知道**——而少一个 CSS 的后果是
                // 那个形态整个无样式渲染，照样零报错。
                // 「名单与目录一致」由 `embedded_assets_cover_every_ui_file` 守着。
                for key in EMBEDDED_ASSET_SAMPLE {
                    match resolver.get(key.to_string()) {
                        Some(asset) => {
                            seen.push(format!("{key}={}B/{}", asset.bytes.len(), asset.mime_type))
                        }
                        None => seen.push(format!("{key}=**缺失**")),
                    }
                }
                log_line(&format!("[boot] 资源表：{seen:?}"));
            }

            // **webview 自省**（上面那两个常量的注释解释了为什么非它不可）。
            // 三个时刻各来一次，作用域是 app handle 而不是局部 `app`，
            // 因为后面两次是从新建线程里发的。
            probe_webviews(app.handle(), 0);
            probe_webviews(app.handle(), 1500);
            probe_webviews(app.handle(), 5000);
            // ⚠️ **这一行不能当几何证据**：`outer_size()` / `outer_position()`
            // 在本应用里恒等于 tauri.conf.json 的配置值、从不随 `set_size` 变化。
            // 它唯一的用处是「窗口在不在、可见不可见」（那两项是真的），
            // 以及**提醒想要位置的人去截图**。位置与尺寸请看 `[place] 请求…算得…`
            // 再用 `screencapture` 对那一小块拍一张。
            {
                let app = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1200));
                    for label in ["island", "sidebar", "workbench"] {
                        let Some(w) = app.get_webview_window(label) else {
                            continue;
                        };
                        let visible = w.is_visible().unwrap_or(false);
                        log_line(&format!(
                            "[boot] 落位 {label}：pos={:?} size={:?} visible={visible}",
                            w.outer_position(),
                            w.outer_size()
                        ));
                    }
                });
            }
            if ui_smoke_requested() {
                log_line("[smoke] 已按 --ui-smoke 打开界面冒烟");
                schedule_ui_smoke(app.handle());
            }

            if std::env::args().any(|a| a == "--memory-smoke") {
                window_smoke::schedule(app.handle());
            }
            if boot_arg_is("workbench") {
                reveal_workbench_window(app.handle());
            }
            #[cfg(target_os = "macos")]
            drop(startup_guard);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_boot_args,
            connections_list,
            connection_test,
            connection_save,
            connection_remove,
            window_layout_capabilities,
            window_layout_candidates,
            window_layout_preview,
            window_layout_apply,
            window_layout_history,
            window_layout_history_remove,
            window_layout_recovery_preview,
            window_layout_undo,
            window_layout_open_permissions,
            window_layout_rules_list,
            prompts_list,
            prompt_save,
            prompt_remove,
            prompt_preview,
            prompt_apply,
            prompt_backups,
            prompt_preview_restore,
            prompt_restore,
            workspaces_list,
            workspace_catalog,
            workspace_save,
            workspace_remove,
            workspace_preview,
            workspace_apply_profile,
            workspace_restore_profile,
            workspace_operations,
            window_layout_save_rule,
            window_layout_remove_rule,
            window_layout_resolve_rule,
            get_engine_state,
            open_agent_session,
            window_is_visible,
            set_workbench_draft,
            drain_navigation,
            get_settings,
            save_settings,
            patch_settings,
            remote_status,
            remote_recent,
            remote_send_test,
            remote_preview,
            token_forecast,
            audit_report_markdown,
            audit_report_csv,
            agent_process_tree,
            run_selftest,
            task_sources,
            session_open_observed,
            session_catalog_read,
            session_catalog_open,
            task_link_source,
            task_open_source,
            task_artifact_content,
            claude_plan_status,
            claude_plan_preview,
            claude_plan_apply,
            claude_plan_cancel,
            claude_plan_resume,
            claude_plan_pause,
            tasks_attention_summary,
            task_show_workbench,
            tasks_snapshot,
            task_project_create,
            task_create,
            task_update,
            task_archive,
            task_record_progress,
            task_mark_handled,
            todos_list,
            todos_add,
            todos_toggle,
            todos_remove,
            todos_clear_done,
            provider_scan_tools,
            provider_list_profiles,
            provider_export_file,
            provider_preview_import,
            provider_import_bundle,
            provider_preview_backup,
            provider_capabilities,
            skills_inspect,
            skills_preview,
            skills_apply,
            skills_package_capability,
            skills_package_choose,
            skills_package_inventory,
            skills_package_sync_preview,
            skills_package_edit_read,
            skills_package_edit_preview,
            skills_package_edit_close,
            skills_package_apply,
            skills_package_cancel,
            skills_package_recoveries,
            skills_package_restore,
            skills_package_restore_preview,
            skills_package_trash_preview,
            skills_package_trash,
            mcp_inspect,
            mcp_preview,
            mcp_apply,
            provider_save_profile,
            provider_delete_profile,
            provider_status,
            provider_apply_profile,
            provider_reapply,
            provider_keep_current,
            provider_list_backups,
            provider_restore_backup,
            token_report_markdown,
            token_report_csv,
            remote_secret_set,
            remote_secret_delete,
            set_dock_edge,
            set_shell_mode,
            set_launch_at_login,
            notify_external,
            launch_at_login_state,
            set_sidebar_width,
            set_sidebar_edge,
            place_sidebar,
            place_island,
            snap_nearest_edge,
            reposition_now,
            get_report,
            clear_latest_event,
            log_from_ui,
            terminate_agent,
            report_text,
            hide_workbench,
            show_workbench,
            collapse_to_tray
        ])
        .run(context)
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tray_badge_tests {
    use super::tray_badge_text;

    #[test]
    fn the_badge_says_one_of_three_things_and_never_invents_a_fourth() {
        // iconOnly（默认）与一切认不出的值：只留图标，不设标题
        assert_eq!(tray_badge_text("iconOnly", 3, 123_456), None);
        assert_eq!(tray_badge_text("", 3, 123_456), None);
        assert_eq!(tray_badge_text("whatever", 3, 123_456), None);

        // activeCount：正在工作的 Agent 数
        assert_eq!(
            tray_badge_text("activeCount", 2, 0),
            Some("⚡️ 2".to_string())
        );
        assert_eq!(
            tray_badge_text("activeCount", 0, 0),
            Some("⚡️ 0".to_string())
        );

        // tokenUsage：复用引擎那一套缩写，菜单栏上不该写 `120.00M`
        assert_eq!(
            tray_badge_text("tokenUsage", 0, 120_000),
            Some("120.0k".to_string())
        );
        assert_eq!(
            tray_badge_text("tokenUsage", 0, 2_500_000),
            Some("2.50M".to_string())
        );
    }

    /// 菜单栏那一格的长度不是小事：它会把旁边的菜单挤走。
    ///
    /// 范围**只取真实量级**：上界给到 1e12 而不是 `i64::MAX`。
    /// 写用例时试过 `i64::MAX / 4`，`compact` 会给它 `2305843009.21B`（14 字符）——
    /// 但那是 2.3×10^18 个 token，人和模型都造不出来。那种量级是**数据源异常**，
    /// 该修的是数据源；为它写一套菜单栏专用的收窄逻辑，是拿复杂度换一个不存在的问题。
    /// 这条断言的职责因此是「**真实量级下不许变长**」，不是「任何输入下都不许变长」。
    #[test]
    fn the_badge_never_gets_long_enough_to_push_the_menubar_around() {
        for mode in ["iconOnly", "activeCount", "tokenUsage"] {
            for tokens in [
                0i64,
                999,
                1_000,
                120_000,
                2_500_000,
                999_999_999,
                1_000_000_000_000,
            ] {
                for active in [0usize, 1, 9, 99, 1_000] {
                    if let Some(text) = tray_badge_text(mode, active, tokens) {
                        assert!(
                            text.chars().count() <= 12,
                            "{mode} active={active} tokens={tokens} ⇒ 徽标过长：{text}"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod hotkey_tests {
    use super::{parse_hotkey, HOTKEY_ACCEL};

    /// 热键组合键必须**真的能被插件解析**。
    ///
    /// 这条不是形式检查：`HOTKEY_ACCEL` 是一句字符串常量，写错一个字符
    /// （比如 `CmdOrCtrl` 拼成 `CmdOrContorl`）编译照过、运行期才在
    /// `setup` 里失败——而那时候应用已经起来了，用户只看到「按了没反应」。
    #[test]
    fn the_hotkey_accelerator_is_something_the_plugin_can_actually_parse() {
        let shortcut = parse_hotkey().expect("热键组合键必须能解析");
        // 解析结果里至少要有修饰键：裸字母键会把用户的整个输入法顶掉
        assert!(
            !shortcut.mods.is_empty(),
            "{HOTKEY_ACCEL} 没有修饰键——那不是全局热键，是全局劫持"
        );
        // 主键的形状钉住：`I`。写成 Debug 全文比较，键一改就红
        assert_eq!(
            format!("{:?}", shortcut.key),
            "KeyI",
            "{HOTKEY_ACCEL} 的主键应当是 I"
        );
    }

    /// 组合键的**形状**要写进断言：改了它，用户肌肉记忆里的快捷键就变了，
    /// 而这件事不该只在 CHANGELOG 里留一句。
    #[test]
    fn the_hotkey_shape_is_pinned_so_a_silent_change_cannot_happen() {
        let shortcut = parse_hotkey().expect("热键组合键必须能解析");
        assert!(
            shortcut
                .mods
                .contains(tauri_plugin_global_shortcut::Modifiers::SHIFT),
            "{HOTKEY_ACCEL} 应当带 Shift（与既有快捷键区分）"
        );
        let cmd_or_ctrl = tauri_plugin_global_shortcut::Modifiers::SUPER
            | tauri_plugin_global_shortcut::Modifiers::CONTROL;
        assert!(
            shortcut.mods.intersects(cmd_or_ctrl),
            "{HOTKEY_ACCEL} 应当带 Cmd 或 Ctrl"
        );
    }
}

/// Rust 包、Tauri 配置与交付文档的版本保持一致。
#[cfg(test)]
mod version_pinning {
    use std::path::Path;

    #[test]
    fn the_cargo_version_matches_the_changelog() {
        let text = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../CHANGELOG.md"),
        )
        .unwrap();
        let version = text
            .lines()
            .find_map(|line| line.strip_prefix("## ["))
            .unwrap()
            .split(']')
            .next()
            .unwrap();
        assert_eq!(env!("CARGO_PKG_VERSION"), version);
    }

    /// `tauri.conf.json` 的 `version` 也要跟上——**它决定用户看到的
    /// `CFBundleShortVersionString`**。
    ///
    /// 这条是被「交付物整个换成 Rust 端」这件事逼出来的：换过去之后，用户
    /// 「关于本机」的窗口里写着 **0.1.0**，而 tag 是 0.0.232。
    /// 而原来的 `version_pinning` 只查 Cargo.toml 与 CHANGELOG，
    /// **漏了这一处**——三处版本号里最容易漂的一处，反而没被钉住。
    #[test]
    fn the_tauri_bundle_reports_the_same_version_as_everything_else() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let conf = std::fs::read_to_string(root.join("tauri.conf.json"))
            .expect("应当读得到 tauri.conf.json");
        // 只取顶层那个 "version"（bundle 段里没有同名键，缩进不同）
        let want = env!("CARGO_PKG_VERSION");
        let found = conf
            .lines()
            .find_map(|line| line.trim().strip_prefix("\"version\": \""))
            .and_then(|rest| rest.split('"').next());
        assert_eq!(
            found,
            Some(want),
            "tauri.conf.json 的 version 与 Cargo.toml 不一致——它是 `CFBundleShortVersionString` 的来源"
        );
    }
}

/// UI 的静态哨兵：**每个 `pageXxx` / `hydrateXxx` / `renderXxx` 调用点都必须有定义**。
///
/// 这条是被真 bug 逼出来的：侧边栏的 Provider 页**只有注水函数、没有页面本身**——
/// 导航项在、`hydrateProvider` 在、`renderProviderPage` 在，可是渲染页面的
/// `pageProvider()` 从没被定义过。于是点「Codex 档位」直接抛 `ReferenceError`，
/// 那一页就是白屏。
///
/// 静态检查与冒烟都发现不了：它们只看「有没有报错」，而这一页**从来没被测过**
/// （谁会去点它），运行时错误也只落在那一个窗口的前端控制台里。
///
/// 所以这里直接扫源码：调用的名字必须在某个 `app/ui/js/*.js` 里被定义出来。
#[cfg(test)]
mod ui_symbol_sentinel {
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};

    fn ui_js_files() -> Vec<PathBuf> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/js");
        let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("应当读得到 app/ui/js")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "js"))
            .collect();
        out.sort();
        out
    }

    /// 收集被**定义**出来的名字：`export function X` / `function X` / `export const X`
    fn defined_symbols(text: &str) -> HashSet<String> {
        let mut out = HashSet::new();
        for line in text.lines() {
            let line = line.trim_start();
            for prefix in [
                "export function ",
                "export async function ",
                "function ",
                "async function ",
            ] {
                if let Some(rest) = line.strip_prefix(prefix) {
                    if let Some(name) = rest.split(['(', '<', ' ']).next() {
                        if !name.is_empty() {
                            out.insert(name.to_string());
                        }
                    }
                }
            }
            for prefix in ["export const ", "const "] {
                if let Some(rest) = line.strip_prefix(prefix) {
                    if let Some(name) = rest.split([' ', '=']).next() {
                        if !name.is_empty() {
                            out.insert(name.to_string());
                        }
                    }
                }
            }
        }
        out
    }

    /// 只扫这一族名字：它们是「页面 / 注水 / 渲染」这一层的约定。
    /// 把所有标识符都扫进来会把内置对象和 DOM 全卷进来，噪声压过信号。
    const PREFIXES: [&str; 3] = ["page", "hydrate", "render"];

    #[test]
    fn session_directory_preserves_source_identity_and_history() {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test-sessions.mjs");
        let result = std::process::Command::new("node")
            .arg(script)
            .output()
            .expect("node is required");
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[test]
    fn every_page_and_hydrate_call_has_a_definition() {
        let files = ui_js_files();
        let mut defined: HashSet<String> = HashSet::new();
        let mut sources: Vec<(String, String)> = Vec::new();
        for path in &files {
            let text = std::fs::read_to_string(path).expect("应当读得到 ui/js 下的 js");
            defined.extend(defined_symbols(&text));
            sources.push((
                path.file_name().unwrap().to_string_lossy().into_owned(),
                text,
            ));
        }

        let mut missing: Vec<String> = Vec::new();
        for (file, text) in &sources {
            for line in text.lines() {
                // 跳过注释行：文档里提到某个函数名不算调用
                let trimmed = line.trim_start();
                if trimmed.starts_with("//")
                    || trimmed.starts_with("*")
                    || trimmed.starts_with("/*")
                {
                    continue;
                }
                let mut rest = trimmed;
                while let Some(at) = rest.find(|c: char| c.is_alphanumeric() || c == '_') {
                    // 取标识符及其前缀位置
                    let start = at;
                    let tail = &rest[start..];
                    let name: String = tail
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    let ends_with_paren = tail[name.len()..].starts_with('(');
                    if ends_with_paren
                        && PREFIXES.iter().any(|p| name.starts_with(p))
                        && !defined.contains(&name)
                    {
                        missing.push(format!("{file}: 调用了 `{name}()` 但没有任何定义"));
                    }
                    rest = &tail[name.len()..];
                }
            }
        }
        assert!(
            missing.is_empty(),
            "UI 调用了没有定义的页面/注水函数——点进去就是白屏：\n{}",
            missing.join("\n")
        );
    }

    /// 工作台必须**复用**既有页面函数，而不是自己再抄一份。
    ///
    /// 抄一遍的后果不是「多写了几行」，而是侧边栏改一处措辞、工作台留在旧话上，
    /// 而没有任何断言会响。所以这里钉住「工作台调用的名字都在别处定义过」。
    #[test]
    fn the_workbench_reuses_pages_instead_of_reimplementing_them() {
        let views = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/js/views.js"),
        )
        .expect("应当读得到 views.js");
        let start = views
            .find("export function renderWorkbench(")
            .expect("views.js 里应当有 renderWorkbench");
        let body = &views[start..];
        for name in ["pageAnalytics", "pageProvider", "pageTodo", "pageReport"] {
            // 页面可作为 section() 的参数；检查调用，不绑定模板插值写法。
            assert!(
                body.contains(&format!("{name}(")),
                "工作台应当复用 `{name}(…)`——它现在要么没被调用，要么被换成了另一份实现"
            );
        }
    }

    /// 三种形态都要能判定，且**认不出的值退回默认形态**。
    /// URL 上的东西是外部输入，四个分支只会多一个「拼错了却渲染出别的东西」的面。
    #[test]
    fn three_shells_are_recognised_with_a_default_fallback() {
        let shell = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/js/shell.js"),
        )
        .expect("应当读得到 shell.js");
        for name in ["island", "sidebar", "workbench"] {
            assert!(
                shell.contains(&format!("'{name}'")),
                "shell.js 不认 `{name}` 这个形态"
            );
        }
        assert!(
            shell.contains("KNOWN.includes(raw)"),
            "认不出的 shell 值必须退回默认形态，而不是被当成一种新形态"
        );
    }

    /// UI 调用的每个 `invoke('X')` 都必须有一个 `#[tauri::command] fn X`。
    ///
    /// 同 [`every_page_and_hydrate_call_has_a_definition`] 的理由：命令名拼错、
    /// 或者调了一个还没写的命令，运行时只会得到一个被 `.catch(() => null)`
    /// 吞掉的 null——**界面表现为「这一块空着」，不报错**。
    /// 那个 `.catch` 是为了不让某个源读不到时整个界面崩掉，
    /// 代价就是它同时把「我调错了」也一起吞了。
    #[test]
    fn every_invoke_has_a_backing_command() {
        let mut commands: HashSet<String> = HashSet::new();
        for entry in std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
            .unwrap_or_else(|_| panic!("应当读得到 src"))
        {
            let path = entry.expect("目录项应当可读").path();
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let mut lines = text.lines().peekable();
            while let Some(line) = lines.next() {
                if line.trim() != "#[tauri::command]" {
                    continue;
                }
                // 命令函数名在下一行：`fn name(` / `fn name<T>(`
                if let Some(next) = lines.peek() {
                    let trimmed = next
                        .trim()
                        .strip_prefix("fn ")
                        .or_else(|| next.trim().strip_prefix("async fn "))
                        .unwrap_or("");
                    if let Some(name) = trimmed.split(['(', '<']).next() {
                        if !name.is_empty() {
                            commands.insert(name.to_string());
                        }
                    }
                }
            }
        }
        assert!(
            !commands.is_empty(),
            "一条 #[tauri::command] 都没扫到——下面的断言会永远为真，等于没有守护"
        );

        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/js");
        let mut missing: Vec<String> = Vec::new();
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("应当读得到 app/ui/js")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "js"))
            .collect();
        files.sort();
        for path in &files {
            let text = std::fs::read_to_string(path).expect("应当读得到 ui/js 下的 js");
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            for line in text.lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") || trimmed.starts_with("*") {
                    continue;
                }
                // **只看 `invoke(` 后面的第一个参数**。
                // 早期版本把文件里所有引号字符串都当命令名，于是
                // `invoke('save_settings', { compact_view: … })` 里的字段名
                // 全被报成「不存在的命令」——一条刷满噪声的守护等于没有。
                let mut rest = trimmed;
                while let Some(at) = rest.find("invoke(") {
                    let after = &rest[at + "invoke(".len()..];
                    let after = after.trim_start();
                    let Some(inner) = after.strip_prefix('\'') else {
                        rest = &rest[at + "invoke(".len()..];
                        continue;
                    };
                    let Some(end) = inner.find('\'') else { break };
                    let word = &inner[..end];
                    // 允许大写：IPC 层对命令名是**精确匹配**，
                    // 所以 `get_Settings` 这种大小写拼错是真实会发生的错误，
                    // 而一条只认小写的过滤规则会把它悄悄放过去
                    //（第一版就栽在这里：我用 `todos_listX` 做变异，守护没响，
                    //  一度以为规则有洞——其实是规则自己把大写挡在了门外）。
                    let looks_like_command = word.contains('_')
                        && word.contains(|c: char| c.is_ascii_lowercase())
                        && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                    if looks_like_command && !commands.contains(word) {
                        missing.push(format!(
                            "{name}: `invoke('{word}')` 没有对应的 #[tauri::command]"
                        ));
                    }
                    rest = &inner[end + 1..];
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "UI 调了不存在的命令——运行时只会拿到一个被 catch 吞掉的 null，界面表现为「这块空着」：\n{}",
            missing.join("\n")
        );
    }

    /// **每个 UI 的 `.js` 都必须能按 ES 模块解析。**
    ///
    /// 这条守护是被一个**已经发布 29 个版本**的 bug 逼出来的：
    /// v0.0.200 那次提交误删了 `views.js` 里 `pageProvider()` 的**函数头**，
    /// 函数体剩下一个顶层 `return`——而顶层 `return` 在 ES 模块里是语法错误，
    /// 于是 `views.js` 整个加载不了，**灵动岛 / 侧边栏 / 工作台三个形态全是空白**。
    ///
    /// **它潜伏 29 个版本的唯一原因**：一直用 `node --check app/ui/js/views.js`
    /// 验语法，而那是按**脚本**解析的——脚本模式不报「顶层 return」，
    /// 所以它一路绿灯。模块是**严格模式**，两者判定不同。
    ///
    /// 正确做法：把文件按 `.mjs` 交给 `node --check`（或直接 `import()`）。
    /// 本用例就是那么做的：`node --check <临时 .mjs>`，非 0 即红。
    #[test]
    fn every_ui_js_file_parses_as_an_es_module() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/js");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("应当读得到 app/ui/js")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "js"))
            .collect();
        files.sort();
        assert!(
            !files.is_empty(),
            "一个 ui/js/*.js 都没找到——检查本身失效了"
        );

        let mut broken = Vec::new();
        for path in &files {
            // **必须换成 `.mjs`**：Node 按扩展名决定解析模式，
            // 保持 `.js` 就退回脚本模式，而脚本模式恰好不报这一类错误。
            let tmp = std::env::temp_dir().join(format!(
                "agentisland-parse-{}-{:p}.mjs",
                path.file_name().unwrap().to_string_lossy(),
                path
            ));
            if std::fs::copy(path, &tmp).is_err() {
                broken.push(format!("{}: 复制到临时文件失败", path.display()));
                continue;
            }
            let output = std::process::Command::new("node")
                .arg("--check")
                .arg(&tmp)
                .output();
            let _ = std::fs::remove_file(&tmp);
            match output {
                Ok(out) if out.status.success() => {}
                Ok(out) => {
                    let text = String::from_utf8_lossy(&out.stderr);
                    let first = text
                        .lines()
                        .find(|l| l.contains("SyntaxError") || l.contains("Error"))
                        .unwrap_or("")
                        .trim();
                    broken.push(format!(
                        "{}: {}",
                        path.file_name().unwrap().to_string_lossy(),
                        first
                    ));
                }
                // 没有 node 就**明说没验**，不能当作通过——
                // 静默跳过等于把这条守护变成一个永远为真的断言。
                Err(error) => broken.push(format!(
                    "{}: 跑不了 node（{error}）——本条守护本次**没有真的验**",
                    path.file_name().unwrap().to_string_lossy()
                )),
            }
        }
        assert!(
            broken.is_empty(),
            "UI 模块解析失败 ⇒ 整个 webview 是空白的（`node --check` 报的是**脚本**模式，\
             按 ES 模块解析才能发现这一类）：\n{}",
            broken.join("\n")
        );
    }

    /// **每个在 `tauri.conf.json` 里声明的窗口，都必须出现在 capability 的 `windows` 里。**
    ///
    /// 少一个的后果**不是报错，是静默失效**：那个窗口的 `invoke` 与 `listen`
    /// 全被权限层拒绝，于是它一片空白、且点什么都不动。
    /// 计划里 v0.0.188 已经为侧边档栽过一次（`listen('engine://tick')` 被拒 ⇒ 永不刷新）；
    /// v0.0.222 加工作台窗口时又栽了一次——**同一个坑，第二次**。
    ///
    /// 所以它必须有守护：两份清单（「有哪些窗口」与「哪些窗口有权限」）
    /// 是两个事实，**必须由一处推导或由一条断言钉住**，不能靠人记得同步。
    #[test]
    fn every_declared_window_has_ipc_permission() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let conf = std::fs::read_to_string(root.join("tauri.conf.json"))
            .expect("应当读得到 tauri.conf.json");
        let caps = std::fs::read_to_string(root.join("capabilities/default.json"))
            .expect("应当读得到 capabilities/default.json");

        // 从 tauri.conf.json 里取出所有 "label": "..."
        let labels: Vec<String> = conf
            .lines()
            .filter_map(|line| {
                let rest = line.trim().strip_prefix("\"label\": \"")?;
                Some(rest.split('"').next().unwrap_or("").to_string())
            })
            .filter(|s| !s.is_empty())
            .collect();
        assert!(
            !labels.is_empty(),
            "一个窗口标签都没从 tauri.conf.json 里读到——解析失效了"
        );

        for label in &labels {
            assert!(
                caps.contains(&format!("\"{label}\"")),
                "窗口 `{label}` 不在 capabilities/default.json 的 windows 里 ⇒ \
                 它的 invoke 与 listen 会被**静默拒绝**，界面一片空白且点不动。 \
                 （v0.0.188 侧边栏栽过一次，v0.0.222 工作台又栽了一次）"
            );
        }
    }
}

/// **构建环境的两个「静默失效」守护。**
///
/// 这两条各对应一个**已经付出过代价**的坑，而且两个都不报任何错——
/// 症状一律是「界面空白」，看起来像前端代码写错了，于是在代码里找了十几轮。
///
/// | # | 坑 | 症状 |
/// | :-- | :--- | :--- |
/// | 1 | `SDKROOT` 钉死在旧版本 | webview **根本不发起导航**（`on_page_load` 一次都不触发） |
/// | 2 | `security.csp` 写成 `null` | 页面加载了，但 **JS 一行都不执行**（`[webview]` 永远 0 行） |
///
/// 两个都在**构建配置**里，而排查时眼睛盯着代码——这个错配是它们难查的全部原因。
#[cfg(test)]
mod build_env_sentinel {
    use std::path::Path;

    /// 最小 DOM 桩。**故意不完整**：`querySelector` 恒为 null、元素只有壳子。
    /// 它只够让模块**加载**与 boot 的同步段跑起来，不足以渲染任何东西。
    const DOM_STUB: &str = r#"
    const noop = () => {};
    const mk = () => ({
      className: 'shell-island', style: {}, dataset: {}, children: [], innerHTML: '',
      classList: { contains: () => false, add: noop, remove: noop, toggle: noop },
      appendChild: noop, setAttribute: noop, removeAttribute: noop, addEventListener: noop,
      querySelector: () => null, querySelectorAll: () => [],
      getBoundingClientRect: () => ({ width: 0, height: 0 }), focus: noop, remove: noop,
    });
    globalThis.document = {
      documentElement: mk(), body: mk(), getElementById: () => mk(), createElement: mk,
      querySelector: () => null, querySelectorAll: () => [], addEventListener: noop,
      createTextNode: () => mk(),
    };
    globalThis.window = { innerWidth: 400, innerHeight: 800, devicePixelRatio: 2, getComputedStyle: () => ({}) };
    globalThis.location = { search: '?shell=island' };
    Object.defineProperty(globalThis, 'navigator', { value: { clipboard: { writeText: async () => {} } }, configurable: true });
    globalThis.addEventListener = noop; globalThis.removeEventListener = noop;
    globalThis.matchMedia = () => ({ matches: false, addEventListener: noop, removeEventListener: noop });
    globalThis.requestAnimationFrame = (f) => setTimeout(f, 0);
    globalThis.cancelAnimationFrame = (h) => clearTimeout(h);
    globalThis.localStorage = { getItem: () => null, setItem: noop, removeItem: noop };
    globalThis.innerWidth = 400; globalThis.innerHeight = 800;
    "#;

    #[test]
    fn workbench_cached_drafts_and_request_lifetimes() {
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test-window-lifecycle.mjs");
        let output = std::process::Command::new("node")
            .arg(script)
            .output()
            .expect("草稿回归需要 node");
        assert!(
            output.status.success(),
            "草稿生命周期回归失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn model_directory_preserves_configured_target_scope() {
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test-model-directory.mjs");
        let output = std::process::Command::new("node")
            .arg(script)
            .output()
            .expect("模型目录回归需要 node");
        assert!(
            output.status.success(),
            "模型目录回归失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn quick_navigation_search_is_scoped_and_escaped() {
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test-quick-navigation.mjs");
        let output = std::process::Command::new("node")
            .arg(script)
            .output()
            .expect("快捷导航回归需要 node");
        assert!(
            output.status.success(),
            "快捷导航回归失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn prompts_ui_keeps_body_out_of_lists_and_escapes_labels() {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test-prompts.mjs");
        let output = std::process::Command::new("node")
            .arg(script)
            .output()
            .expect("提示词回归需要 node");
        assert!(
            output.status.success(),
            "提示词回归失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn workspace_reference_actions_escape_labels_and_disable_missing_sources() {
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test-workspaces.mjs");
        let output = std::process::Command::new("node")
            .arg(script)
            .output()
            .expect("工作空间回归需要 node");
        assert!(
            output.status.success(),
            "工作空间回归失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn functional_ui_settings_and_remote_feedback_regression() {
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test-functional-ui.mjs");
        let output = std::process::Command::new("node")
            .arg(script)
            .output()
            .expect("功能回归需要 node");
        assert!(
            output.status.success(),
            "功能回归失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn agent_identities_cover_registry_and_all_ui_locations() {
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test-agent-identities.mjs");
        let output = std::process::Command::new("node")
            .arg(script)
            .output()
            .expect("身份图标回归需要 node");
        assert!(
            output.status.success(),
            "身份图标回归失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn island_navigation_settles_after_asynchronous_report_retarget() {
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test-island-motion.mjs");
        let output = std::process::Command::new("node")
            .arg(script)
            .output()
            .expect("导航动效回归需要 node");
        assert!(
            output.status.success(),
            "导航动效回归失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn island_agent_return_waits_for_viewport_commit() {
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test-island-return.mjs");
        let output = std::process::Command::new("node")
            .arg(script)
            .output()
            .expect("返回动效回归需要 node");
        assert!(
            output.status.success(),
            "返回动效回归失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn island_clicks_do_not_trigger_drag_placement() {
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test-island-click.mjs");
        let output = std::process::Command::new("node")
            .arg(script)
            .output()
            .expect("点击回归需要 node");
        assert!(
            output.status.success(),
            "点击定位回归失败：{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn island_boot_handles_deep_link_navigation() {
        let script =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test-island-deeplink.mjs");
        for (shell, cold_start) in [
            ("island", "0"),
            ("island", "1"),
            ("workbench", "0"),
            ("workbench", "1"),
        ] {
            let output = std::process::Command::new("node")
                .arg(&script)
                .env("TEST_COLD_NAVIGATION", cold_start)
                .env("TEST_NAVIGATION_SHELL", shell)
                .output()
                .expect("深链 UI 回归需要 node");
            assert!(
                output.status.success(),
                "深链 UI 回归失败（shell={shell}, cold={cold_start}）：\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn macos_bundle_starts_without_a_dock_entry() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let out = std::process::Command::new("python3")
            .arg("-c")
            .arg("import plistlib,json,sys; from pathlib import Path; p=Path(sys.argv[1]); c=json.loads((p/'tauri.conf.json').read_text()); assert plistlib.loads((p/c['bundle']['macOS']['infoPlist']).read_bytes())['LSUIElement'] is True; assert 'icons/icon.icns' in c['bundle']['icon']; assert (p/'icons/icon.icns').stat().st_size > 0")
            .arg(root).output().expect("bundle 验证需要 python3");
        assert!(
            out.status.success(),
            "macOS 常驻启动或 Dock 资产配置错误：{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn tauri_conf() -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(root.join("tauri.conf.json")).expect("应当读得到 tauri.conf.json")
    }

    /// **`csp` 不能是 `null`。**
    ///
    /// 写成 `null` 时 Tauri/wry 会施加一条会拦掉本项目脚本的策略，页面照常加载、
    /// 画面全白，而**没有任何一行错误**。实测：改成显式 CSP 后，
    /// `[webview] Tauri API 就绪` 从 0 行变成 3 行（三个窗口各一行）。
    #[test]
    fn the_csp_is_explicit_rather_than_null() {
        let conf = tauri_conf();
        assert!(
            !conf.contains("\"csp\": null"),
            "tauri.conf.json 的 csp 又是 null 了——那会让 webview 里的 JS 一行都不跑，\
             而症状只是「界面空白」，不报任何错。v0.0.236 的教训。"
        );
        assert!(
            conf.contains("\"csp\": \""),
            "csp 应当是一段显式策略（写清这份界面允许什么），而不是让它缺省"
        );
    }

    /// **`build-app.sh` 里不许再出现钉死的 SDK 路径。**
    ///
    /// 拿 26.5 的 SDK 去链 WKWebView、跑在 macOS 27 上 ⇒ webview 静默不导航。
    /// 错配不报错，所以脚本里**必须**是「挑最新的」，而不是「钉一个」。
    #[test]
    fn the_packaging_script_never_pins_an_sdk_version() {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/build-app.sh");
        let text = std::fs::read_to_string(&script).expect("应当读得到 scripts/build-app.sh");
        // 候选列表里**出现** SDK 名是正常的（那是排序清单）；被钉死的是
        // 「写死某一个并直接 export」——那正是坑的形状。
        let pinned = text.lines().any(|line| {
            let line = line.trim();
            line.starts_with("export SDKROOT=")
                && !line.contains("candidate")
                && line.contains("MacOSX")
        });
        assert!(
            !pinned,
            "build-app.sh 又把 SDKROOT 钉死在某一个版本上——跑在更新的 macOS 上时 \
             webview 会静默不发起导航。应当从候选清单里挑最新的（v0.0.235 的教训）。"
        );
    }

    /// **`views.js` 必须能在 Node 里真的求值**（不只是 `node --check` 的语法检查）。
    ///
    /// 语法层的检查在 `every_ui_js_file_parses_as_an_es_module`；这里管第三件：
    /// **顶层求值就抛异常**。那种异常的表现是**整个界面空白、且浏览器控制台
    /// 之外没有任何痕迹**——而「页面没加载」与「脚本抛了」在界面上一模一样。
    /// v0.0.229 那次 `pageProvider` 的函数头被删，正是这一类。
    ///
    /// **只验 `views.js`，不验 `main.js` 的 boot**：boot() 在 import 时就执行，
    /// 而本测试的 DOM 桩是**故意不完整**的（`querySelector` 恒为 null），
    /// boot 摸到真实元素必然炸——那是**桩的失败，不是代码的失败**，
    /// 混在一起报红只会让人学会忽略它。main.js 的 boot 路径要验需要真 DOM 或屏幕。
    #[test]
    fn the_view_module_evaluates() {
        let ui = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/js");
        let stub = std::env::temp_dir().join("agentisland-dom-stub.mjs");
        std::fs::write(&stub, DOM_STUB).expect("应当写得出 DOM 桩");

        // 末尾的 `/` 不是装饰：`new URL('views.js', base)` 在没有尾斜杠的 base 下
        // 会解析到**上一级**目录，于是报「找不到 app/ui/views.js」——
        // 一个和代码毫无关系的失败。
        let script = format!(
            "await import('{}');\
             const base = new URL('file://{}/');\
             const m = await import(new URL('views.js', base).href);\
             console.log('EVALUATED:' + Object.keys(m).length);",
            stub.canonicalize().expect("桩文件应当存在").display(),
            ui.canonicalize().expect("app/ui/js 应当存在").display()
        );
        let out = std::process::Command::new("node")
            .arg("--input-type=module")
            .arg("--eval")
            .arg(&script)
            .output();
        let _ = std::fs::remove_file(&stub);

        match out {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(
                    stdout.contains("EVALUATED"),
                    "views.js 在求值时抛异常 ⇒ 整个界面会空白而外部毫无痕迹。\n\
                     stdout: {stdout}\nstderr: {stderr}"
                );
            }
            Err(error) => {
                panic!("跑不了 node（{error}）——本条守护**没有真的验**，不是通过")
            }
        }
    }
}

/// **主线程派发死锁**的守护。
///
/// 规矩：任何会回到主线程并**同步等结果**的 Tauri 调用，都不许出现在持有
/// `SharedEngine` 锁的临界区里。引擎循环踩过这条——它就是界面全白的**确切原因**，
/// 而现象（窗口建好、尺寸正确、资源嵌好、脚本一行不跑）看起来像前端坏了，
/// 于是前一轮全部力气都花在 URL / CSP / SDK 上，全都不在这条链上。
///
/// 命令侧（`set_shell_mode` / `set_sidebar_width` / …）一直是对的：它们用
/// `{ let e = state.lock()…; }` 作用域块，在碰窗口之前就把锁放了。
/// 这条守护盯的就是引擎循环——那个当时漏掉的地方。
#[cfg(test)]
mod main_thread_dispatch_sentinel {
    /// 本文件源码。编译期嵌入，所以**永远**与正在跑的代码一致。
    const SRC: &str = include_str!("main.rs");

    fn engine_loop_body() -> &'static str {
        let start = SRC
            .find("fn engine_loop(")
            .expect("engine_loop 改名或没了？守护需要跟着改");
        let rest = &SRC[start + 1..];
        let end = rest.find("\nfn ").map(|i| i + 1).unwrap_or(rest.len());
        &rest[..end]
    }

    /// **托盘写入必须排在引擎锁的临界区之外。**
    ///
    /// 变异验证：把 `apply_tray_badge(&app, badge);` 挪回 `{ … }` 块里，
    /// 本条会失败——它不是靠注释通过的。
    #[test]
    fn tray_writes_happen_outside_the_engine_lock() {
        let body = engine_loop_body();
        let lock_at = body
            .find("shared.lock()")
            .expect("engine_loop 不再锁引擎了？守护需要跟着改");
        let badge_at = body
            .find("apply_tray_badge(")
            .expect("engine_loop 不再写托盘徽标了？徽标会静默消失");
        assert!(
            lock_at < badge_at,
            "写托盘必须排在拿锁之后，实际是 {lock_at} vs {badge_at}"
        );

        let critical = &body[lock_at..badge_at];
        for forbidden in ["set_title", "tray_by_id"] {
            assert!(
                !critical.contains(forbidden),
                "托盘是 AppKit 的 NSStatusItem，写它会被同步派发到主线程并等结果。\n\
                 它出现在引擎锁与 `apply_tray_badge` 之间 ⇒ 引擎线程持锁等主线程、\n\
                 主线程要锁 ⇒ 互等 ⇒ setup 不返回 ⇒ 事件循环不转 ⇒ WKWebView 不导航 ⇒ 界面全白。\n\
                 修法：把计算放进锁内（`tray_badge_text`），把写入挪到锁外（`apply_tray_badge`）。"
            );
        }
    }

    /// 徽标不能为了躲开死锁就被悄悄丢掉——写入点仍在引擎循环里、且每轮都调。
    #[test]
    fn the_badge_is_still_written_every_tick() {
        let body = engine_loop_body();
        assert_eq!(
            body.matches("apply_tray_badge(&app").count(),
            1,
            "引擎循环里应当恰好有一处写入托盘徽标"
        );
        assert!(
            body.find("apply_tray_badge(").unwrap_or(usize::MAX)
                < body.find("app.emit(").unwrap_or(usize::MAX),
            "徽标写入排在事件推送之前"
        );
    }
}

/// **「只显示在线」这条规则的契约**。
///
/// 前端那条判据（`app/ui/js/views.js` 的 `isVisible`）等价于
/// `process_running`，而全站可见性都建立在一条 Rust 侧的不变式上：
///
/// > `decide_level` 在 `!process_running` 时**无条件**返回 `Offline`。
///
/// 这条不变式就是「`level !== 'offline'` 蕴含 `process_running`」的依据。
/// 它一旦被破坏，前端那条判据就会与界面别处显示的东西对不上，
/// 而**没有任何别的测试会红**——所以它自己得被钉住。
#[cfg(test)]
mod level_contract_sentinel {
    use super::Settings;
    use crate::engine::ActivityEngine;
    use crate::filemon::FileActivityResult;
    use std::sync::mpsc::channel;

    fn engine() -> ActivityEngine {
        ActivityEngine::new(Settings::default(), channel().1)
    }

    /// 进程没在跑 ⇒ 任何情况下都只能是 `Offline`。
    ///
    /// 把探测信号设成最强的 attention 也一样：进程不在，状态就不该是「在等你」。
    #[test]
    fn a_process_that_is_not_running_is_always_offline() {
        use crate::models::ActivityLevel;
        use crate::session::{SessionProbe, Signal};
        let mut e = engine();
        let Some(profile) = crate::registry::builtin()
            .into_iter()
            .find(|p| p.id == "claude")
        else {
            return;
        };
        let mut e = e;
        let attention = SessionProbe {
            signal: Some(Signal::Attention("fp".into(), "等你批准".into())),
            subagent_count: 0,
            health: None,
        };
        let level = e.decide_level(
            &profile,
            1_000_000,
            false, // process_running
            Some(99.0),
            0.0,
            30.0,
            1.0,
            &FileActivityResult {
                latest_write: Some(
                    std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(999),
                ),
                latest_file: Some("/tmp/x.jsonl".into()),
                active_sessions: 3,
            },
            &attention,
            Some(4242),
        );
        assert_eq!(
            level,
            ActivityLevel::Offline,
            "进程没在跑却给出了非 Offline —— 前端 `isVisible` 会与界面别处对不上"
        );
    }

    /// 反向：进程在跑时**可能**是 Offline（那由别的分支决定），
    /// 但契约只要求「非 Offline ⇒ 在跑」，所以这里只守住进程在跑时不误判。
    #[test]
    fn a_running_process_is_not_forced_offline_by_this_contract() {
        use crate::session::SessionProbe;
        let mut e = engine();
        let Some(profile) = crate::registry::builtin()
            .into_iter()
            .find(|p| p.id == "claude")
        else {
            return;
        };
        let level = e.decide_level(
            &profile,
            1_000_000,
            true,
            None,
            0.0,
            30.0,
            1.0,
            &FileActivityResult {
                latest_write: None,
                latest_file: None,
                active_sessions: 0,
            },
            &SessionProbe::default(),
            Some(4242),
        );
        assert_ne!(
            level,
            crate::models::ActivityLevel::Offline,
            "进程在跑却被判成 Offline —— 刚启动的 Agent 会一直显示离线"
        );
    }
}

/// **「只显示在线」只能有一处判据。**
///
/// 前端此前把同一条规则拼成两种写法、散在 5 个调用点
/// （`s.process_running` 与 `snap.process_running || snap.level !== 'offline'`）。
/// 今天它们可证明等价（见 [`level_contract_sentinel`]），所以第二种是**冗余**；
/// 而冗余长得像有意为之——哪天 `decide_level` 改了，那两处会显示出别处藏着的条目，
/// 而且**没有任何别的测试会红**。
///
/// 这条只认一种形状：`.snapshots.filter(isVisible)`。
/// 任何内联箭头（`filter((s) => …)`）一律算违规。
#[cfg(test)]
mod visibility_rule_sentinel {
    use std::path::Path;

    fn views_js() -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/js/views.js");
        std::fs::read_to_string(&p).expect("应当读得到 app/ui/js/views.js")
    }

    /// **过滤谓词里不许再出现 `process_running`。**
    ///
    /// 规则要收得够紧：`filter((snap) => snap.level === 'attention')`
    /// （数有几个 Agent 在等你）是**另一个问题**，不该被这条守护捎带上。
    /// 所以盯的不是「所有 filter」，而是**这条被统一的判据本身有没有回流**。
    #[test]
    fn the_visibility_rule_never_reappears_inline() {
        let src = views_js();
        let mut offenders: Vec<String> = Vec::new();
        for (number, line) in src.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("*") || trimmed.starts_with("/*") {
                continue;
            }
            // `isVisible` 的定义本身不算回流
            if line.contains("export const isVisible") || line.contains("function isVisible") {
                continue;
            }
            if line.contains(".filter(") && line.contains("process_running") {
                offenders.push(format!("第 {} 行：{}", number + 1, trimmed));
            }
        }
        assert!(
            offenders.is_empty(),
            "「只显示在线」的判据又自己拼了一遍：\n{}\n\
             全站只能有 `isVisible` 一处；各拼各的迟早漂（第二种拼法曾与第一种并存数十个版本）。",
            offenders.join("\n")
        );
    }

    /// 反向：共享判据必须**真的在被用**。收成一处之后若没人调它，
    /// 就等于把「显示哪些条目」这件事删掉了。
    #[test]
    fn the_predicate_is_actually_used_at_every_display_site() {
        let src = views_js();
        let uses = src.matches("filter(isVisible)").count();
        assert!(
            uses >= 4,
            "只查到 {uses} 处 `filter(isVisible)`——灵动岛两处 + 侧边栏 + 工作台至少 4 处。"
        );
    }

    /// `isVisible` 只能**定义一次**。定义两份等于把「唯一判据」又说了一遍。
    #[test]
    fn the_predicate_is_defined_exactly_once() {
        let src = views_js();
        let definitions = src
            .lines()
            .filter(|l| l.contains("export const isVisible") || l.contains("function isVisible"))
            .count();
        assert_eq!(definitions, 1, "`isVisible` 被定义了 {definitions} 次");
    }
}

/// **采样时钟只能由引擎盖章，探测层不许自己取当前时间。**
///
/// 这条规矩写在 `engine.rs` 的 `probe_cached_multi` 上（`now: i64` 注释）。
/// 破它的代价不是难看，是**测不出来**：探测层拿行里的 epoch 与另一个钟比，
/// 测试里写死的历史时间戳必然落到保质期外，那一支永远走不到。
/// v0.0.258 前后一共有两个地方破过（`probe_opencode`、`probe_status_index`）。
///
/// 这条守护只看**生产段**（第一个测试模块 `mod tests;` 之前的部分），
/// 并允许 `#[cfg(test)]` 覆盖的零星例外——`FIXED_NOW_MS` 那种测试常量就住在那里。
/// 假设：本文件的测试模块都在生产代码之后（现状如此）；若哪天不是了，
/// 本条会误报——**那时要改的是这条守护，不是把探测层改回去取墙上时钟**。
#[cfg(test)]
mod sampling_clock_sentinel {
    fn session_rs() -> String {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/session.rs");
        std::fs::read_to_string(&p).expect("应当读得到 src/session.rs")
    }

    /// 变异验证：第一版只看**调用点**的 `SystemTime::now()` 字面量，
    /// 于是 `now_secs()` 这个住在 `#[cfg(test)]` 区域（被当成「测试常量」放行）
    /// 的辅助函数可以**被生产代码调**而完全隐形——**间接调用看不见**。
    /// 所以除了字面量，还必须盯住「取墙上时钟的辅助函数被调用」。
    #[test]
    fn the_probe_layer_never_reads_the_wall_clock() {
        const CLOCK_READS: [&str; 3] = ["SystemTime::now()", "Utc::now()", "now_secs()"];
        let src = session_rs();
        // 边界 = **第一个内联测试模块**（带花括号的那个）。
        //
        // ⚠️ 第一版错用 `split("mod tests;")`，而那句在**第 8 行**
        // ——它是独立测试文件的声明，于是「生产段」只剩 7 行，
        // 守护**结构上就抓不到任何东西**、也永远不会红。
        // 第三次栽在同一件事上：造门禁 ≠ 门禁有效。
        let boundary = src
            .lines()
            .position(|l| l.starts_with("mod ") && l.trim_end().ends_with('{'))
            .unwrap_or(0);
        assert!(
            boundary > 200,
            "session.rs 的内联测试模块没找到（边界落在第 {boundary} 行）——             守护的有效范围已经失效，**先修守护**，别当成探测层出了问题"
        );
        let production: String = src.lines().take(boundary).collect::<Vec<_>>().join("\n");
        let lines: Vec<&str> = production.lines().collect();
        let mut offenders: Vec<String> = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            // 注释里提到这些名字是**散文**，不是调用——「文档说这里曾经取过墙上时钟」
            // 会被当成「这里在取墙上时钟」，那是误报。
            let t = line.trim_start();
            if t.starts_with("//") || t.starts_with("/*") || t.starts_with('*') {
                continue;
            }
            let hit = CLOCK_READS.iter().find(|p| line.contains(*p));
            let Some(needle) = hit else { continue };
            // 往前 4 行里若有 `#[cfg(test)]`，那是测试常量，允许
            let covered = lines[i.saturating_sub(4)..i]
                .iter()
                .any(|l| l.contains("#[cfg(test)]"));
            if !covered {
                offenders.push(format!("第 {} 行：{}", i + 1, line.trim()));
            }
        }
        assert!(
            offenders.is_empty(),
            "探测层不许自己取当前时间 —— 时钟由引擎盖章传进来：\n{}\n\
             破它的后果不是难看，是**测不出来**：测试里写死的时间戳必然落到保质期外，\
             那一支永远走不到。\n\
             （辅助函数也算：定义放在 cfg(test) 区、被生产代码调用，一样是破规矩。）",
            offenders.join("\n")
        );
    }
}
#[cfg(test)]
mod log_cap_tests {
    use super::{truncate_log_if_oversized, LOG_CAP_BYTES};

    /// 没超封顶就**一个字都不许动**。
    ///
    /// ⚠️ 夹具大小**必须写死**、不能由 `LOG_CAP_BYTES` 算出来：
    /// 第一版用 `vec![b'x'; LOG_CAP_BYTES + 1]`，
    /// 于是**把常量改小夹具跟着变小、测试照样全绿**——
    /// 钉的是「常量与夹具的关系」而不是行为，**永远不可能失败**。
    /// 同一类错这一轮犯过好几次，只是这次长在测试自己身上。
    #[test]
    fn a_small_log_is_left_byte_for_byte_intact() {
        let dir = crate::testutil::Sandbox::new("log-cap-small");
        let path = dir.path().join("small.log");
        let body = "[run] pid=1 启动\n[boot] 一些内容\n";
        std::fs::write(&path, body).unwrap();
        assert_eq!(truncate_log_if_oversized(&path), None, "没超封顶就不该截断");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            body,
            "内容必须原样保留"
        );
    }

    #[test]
    fn an_oversized_log_is_wiped_and_the_old_size_is_reported() {
        let dir = crate::testutil::Sandbox::new("log-cap-big");
        let path = dir.path().join("big.log");
        let body = vec![b'x'; (LOG_CAP_BYTES + 1) as usize];
        std::fs::write(&path, &body).unwrap();
        let before = truncate_log_if_oversized(&path).expect("超了封顶就该返回原大小");
        assert_eq!(before, body.len() as u64, "报出来的应是**截断前**的字节数");
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0, "应当被清空");
    }

    /// **封顶的取值本身要有上下界。**
    ///
    /// 上一条用常量算夹具大小，于是把常量调到 `u64::MAX`（或 64）两条仍全绿——
    /// 变异验证当场证伪了它们「能失败」。这条把取值钉在合理区间，
    /// 让那类变异**重新变得可抓**。
    #[test]
    fn the_cap_stays_in_a_range_that_actually_caps_something() {
        assert!(
            LOG_CAP_BYTES <= 16 * 1024 * 1024,
            "封顶 {LOG_CAP_BYTES} 字节大得没意义：等于没有封顶"
        );
        assert!(
            LOG_CAP_BYTES > 64 * 1024,
            "封顶太小会把正常运行要的日志也清掉"
        );
    }
}

/// 守护：**macOS 专属的 crate 不许挂在共享的 `[dependencies]` 里。**
///
/// 起因是 v0.0.267 补的 Windows CI **第一次跑就红**，而报错不在本仓任何一行：
///
/// ```text
/// error[E0433]: cannot find `unix` in `os`
///   --> core-foundation-0.10.1/src/filedescriptor.rs:19
/// error[E0432]: unresolved import `libc::PATH_MAX`
///   --> core-foundation-0.10.1/src/url.rs:23
/// ```
///
/// `security-framework` 是本仓的直接依赖（钥匙串），它拉进 `core-foundation`
/// （Apple Core Foundation 绑定，内部用 `std::os::unix`），而**没有任何 target 门控**
/// ⇒ Windows 依赖图里也有它 ⇒ 构建死在**依赖自己的源码**上。
///
/// 为什么本机看不见：macOS 构建需要它，所以「能编过」完全正常。
/// `cargo test`、`cargo build`、编译器警告——**全都照不到这条路径**。
/// 只有 `cargo tree --target x86_64-pc-windows-msvc` 或真正的 Windows runner 能看见。
///
/// 所以这里用 `Cargo.toml` 静态判定：读表、检查这些名字在哪个表里。
/// 它不替代 Windows CI（CI 才是权威判据），但它在**每台 macOS 机器上、每次 `cargo test`
/// 都会跑**，而 CI 只在 push 时跑。
#[cfg(test)]
mod windows_dep_gating_sentinel {
    use std::path::Path;
    use toml_edit::DocumentMut;

    /// 内部是 macOS 专属实现的 crate。列在这里的名字**必须**在
    /// `[target.'cfg(target_os = "macos")'.dependencies]` 下，不得在共享表里。
    ///
    /// ⚠️ 这张表是**逐条核实过的**，不是「看起来像 macOS 的都算」——
    /// 按名字猜会把无害的 crate 误报，久而久之就没人看这条守护了。
    const MACOS_ONLY: [&str; 2] = ["security-framework", "security-framework-sys"];

    // 路径按 `.` 分段，**段名就是键名本身**：`target` → `cfg(target_os = "macos")` → `dependencies`。
    // ⚠️ 别把 TOML 的引号语法带进来：键名 `cfg(target_os = "macos")` **不含**那对引号，
    // 写成 `target."cfg(...)".dependencies` 会按字面量去找一个不存在的键，永远返回空集——
    // 而空集恰好能让「不许在共享表里」那条恒绿。
    const MACOS_TABLE: &str = r#"target.cfg(target_os = "macos").dependencies"#;

    fn cargo_toml() -> DocumentMut {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        std::fs::read_to_string(&path)
            .expect("应当读得到 Cargo.toml")
            .parse()
            .expect("Cargo.toml 应当能解析")
    }

    /// 取一张表的键集合。表不存在时返回空集（`toml_edit` 的索引不会 panic）。
    fn keys(doc: &DocumentMut, path: &str) -> Vec<String> {
        let mut node: &dyn toml_edit::TableLike = doc.as_table();
        for segment in path.split('.') {
            node = match node.get(segment).and_then(|i| i.as_table_like()) {
                Some(t) => t,
                None => return Vec::new(),
            };
        }
        node.iter().map(|(k, _)| k.to_string()).collect()
    }

    #[test]
    fn no_macos_only_crate_sits_in_the_shared_dependency_table() {
        let doc = cargo_toml();
        let shared = keys(&doc, "dependencies");
        let offenders: Vec<&str> = MACOS_ONLY
            .iter()
            .copied()
            .filter(|name| shared.iter().any(|k| k == name))
            .collect();
        assert!(
            offenders.is_empty(),
            "这些 crate 挂在共享的 [dependencies] 里，会被**无条件**拉进所有平台的依赖图：\n  {}\n\
             报错会落在依赖自己的源码上（`core-foundation` 内部 `use std::os::unix::…`），\n\
             本机（macOS）永远看不见——只有 `cargo tree --target x86_64-pc-windows-msvc`\n\
             或真正的 Windows runner 能发现。改放到 [{}] 下面。",
            offenders.join("\n  "),
            MACOS_TABLE,
        );
    }

    /// 反向：门控之后**不能把 macOS 那边的能力弄丢**。
    /// 只查「不在共享表里」不够——全删掉也满足，那就把钥匙串悄悄弄没了。
    #[test]
    fn the_macos_only_crates_are_still_declared_for_macos() {
        let doc = cargo_toml();
        let macos = keys(&doc, &MACOS_TABLE);
        let missing: Vec<&str> = MACOS_ONLY
            .iter()
            .copied()
            .filter(|name| !macos.iter().any(|k| k == name))
            .collect();
        assert!(
            missing.is_empty(),
            "{} 里缺了 {:?}——钥匙串在 macOS 上就没有实现了。\n  该表当前的键：{:?}",
            MACOS_TABLE,
            missing,
            macos,
        );
    }

    /// 正例控制：证明上面两条真的在读 Cargo.toml 的表，而不是恒绿。
    /// 拿一段内联的 TOML 走同一个 `keys()`。
    #[test]
    fn the_table_reader_actually_finds_keys() {
        let doc: DocumentMut = r#"
[dependencies]
a = "1"
b = "2"

[target.'cfg(target_os = "macos")'.dependencies]
security-framework = "3"
"#
        .parse()
        .expect("内联 TOML 应当可解析");
        assert_eq!(keys(&doc, "dependencies"), vec!["a", "b"]);
        assert_eq!(keys(&doc, &MACOS_TABLE), vec!["security-framework"]);
        assert!(keys(&doc, "target.\"cfg(windows)\".dependencies").is_empty());
    }
}

/// 守护：**生产代码里的 POSIX-only 用法只能变少，不能变多。**
///
/// 起因是 v0.0.269 的实测结论：**这个应用在 Windows 上编不过**——
/// Windows CI 一次就报出 17 个错误、横跨 13 个文件，而 **macOS 上一切正常**。
/// 缺口是一整层从未移植的 POSIX 实现（`std::os::unix` 权限位、
/// `libc::localtime_r` / `getppid` / `kill` / `gethostname`、`SIGKILL` …）。
///
/// 「macOS 上正常」完全证明不了什么——那条路需要它们。
/// CI 是权威判据，但它只在 push 时跑、还要等几分钟；
/// 这条守护**在每台 macOS 机器的每次 `cargo test` 里都跑**，且秒级出结果。
///
/// **它只管方向，不保证正确。** 判定条件是「总数 ≤ 钉死的值」：
/// 新增会被抓住，减少不会报错。端口做对了会自然下降；
/// 想一次清零就重写这个常量——那是自觉，不是门禁能替你做的事。
#[cfg(test)]
mod posix_port_ratchet {
    use std::path::{Path, PathBuf};

    /// Windows 上**不存在**的标识符。这张表不是「看起来像 POSIX 的都算」，
    /// 而是 v0.0.269 的 Windows CI **实际报出来的那些**——按名字猜会误报，
    /// 误报久了就没人看这条守护了。
    const POSIX_ONLY: [&str; 6] = [
        "std::os::unix",
        "libc::localtime_r",
        "libc::getppid",
        "libc::kill",
        "libc::gethostname",
        "libc::SIGKILL",
    ];

    /// 钉死的上限。改它只有一个正当理由：**你把 Windows 那一层真的做完了**，
    /// 或者有意识地决定不做、并把 `app/README.md` 的口径一并改掉。
    ///
    /// v0.0.270：四处本地日历调用归入 `localclock` 的平台门控，基线从 8 降至 **4**。
    /// 对照 v0.0.269 首次 Windows CI 的 17 个**编译错误**：
    /// 两者不等价、也不必相等——CI 还报了 `placement.rs` 的
    /// 「`fallback_work_area` 不在作用域内」那类 **cfg 作用域**问题，
    /// 以及 `filemon.rs` / `procmon.rs` / `engine.rs` / `main.rs` 的文件级 `#[cfg(unix)] use`，
    /// **那些一条都不在 `POSIX_ONLY` 这张表里**。
    ///
    /// ⚠️ 所以 **4 不是「Windows 还差多少」的全部**，它只是这一类
    /// （`std::os::unix` + POSIX libc）的数量。别拿它当完成度指标。
    const PINNED_MAX: usize = 4;

    /// 该处**是否已经被 `#[cfg(unix)]` 正确门控**。
    ///
    /// 门控过的在 Windows 上根本不会编译，所以**不算缺口**。漏算的后果是守护误报，
    /// 而误报久了就没人看它了——比漏算更糟。
    ///
    /// 判据是**向上 4 行内**出现 `#[cfg(unix)]` / `#[cfg(not(unix))]`。这是启发式：
    /// 实测仓里两种写法都覆盖得到——
    /// `#[cfg(unix)]\n{ use std::os::unix::…; }`（属性在上两行）与
    /// `#[cfg(unix)]\npub fn f() { use std::os::unix::…; }`（在上三行）。
    /// 宁可少算（少算只是让上限偏松），不要误报。
    fn is_unix_gated(lines: &[&str], at: usize) -> bool {
        let mut looked = 0;
        for up in (0..at).rev() {
            let t = lines[up].trim();
            if t.is_empty() {
                continue;
            }
            looked += 1;
            if t.contains("cfg(unix)") || t.contains("cfg(not(unix))") {
                return true;
            }
            if looked >= 4 {
                break;
            }
        }
        false
    }

    /// 取一个文件的**生产段**：到第一个**内联** `#[cfg(test)]` 模块为止。
    ///
    /// ⚠️ `#[cfg(test)] mod tests;` 这种**声明**不算边界——它的内容在别的文件里
    /// （`session.rs` 第 7 行就是这种情况，而它的生产代码一直到 900 多行）。
    /// 用「第一个 `#[cfg(test)]`」当边界会把整个 `session.rs` 误判成测试代码，
    /// 于是读出 0 处、上面那条守护就**恒绿**。
    fn production_section(text: &str) -> &str {
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.trim() != "#[cfg(test)]" {
                continue;
            }
            // 往后跳过别的属性行（`#[path = "…"]` 之类）
            let mut j = i + 1;
            while j < lines.len() && lines[j].trim_start().starts_with("#[") {
                j += 1;
            }
            let next = lines.get(j).unwrap_or(&"").trim();
            if next.starts_with("mod ") && next.ends_with(';') {
                continue; // 声明式：内容在别的文件里，不切
            }
            let mut byte_end = 0;
            for l in &lines[..i] {
                byte_end += l.len() + 1;
            }
            return &text[..byte_end.min(text.len())];
        }
        text
    }

    fn rusted_files() -> Vec<PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut out: Vec<PathBuf> = walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "rs"))
            .map(|e| e.path().to_path_buf())
            .collect();
        out.sort();
        assert!(
            out.len() > 20,
            "只找到 {} 个 .rs，采集器本身失效了",
            out.len()
        );
        out
    }

    fn hits() -> Vec<String> {
        let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        for path in rusted_files() {
            let text = std::fs::read_to_string(&path).expect("源文件应可读");
            let rel = path
                .strip_prefix(&src_root)
                .unwrap_or(&path)
                .display()
                .to_string();
            let lines: Vec<&str> = production_section(&text).lines().collect();
            for (i, line) in lines.iter().enumerate() {
                for needle in POSIX_ONLY {
                    // 按**出现次数**计，而不是「这行有没有」——同一函数用两次算两处
                    for _ in 0..line.matches(needle).count() {
                        if is_unix_gated(&lines, i) {
                            continue;
                        }
                        found.push(format!("{rel}:{}  {needle}", i + 1));
                    }
                }
            }
        }
        found
    }

    #[test]
    fn the_posix_surface_is_not_allowed_to_grow() {
        let found = hits();
        assert!(
            found.len() <= PINNED_MAX,
            "生产代码里的 POSIX-only 用法变成 {} 处，超过钉死的上限 {PINNED_MAX}。\n\
             Windows 上它们**全部编译不过**（v0.0.269 实测 17 个错误 / 13 个文件）。\n\
             新增一处之前先问：它在 `#[cfg(unix)]` / `#[cfg(windows)]` 后面吗？\n  {}",
            found.len(),
            found.join("\n  "),
        );
    }

    fn count_in(text: &str) -> usize {
        let lines: Vec<&str> = production_section(text).lines().collect();
        let mut n = 0;
        for (i, line) in lines.iter().enumerate() {
            for needle in POSIX_ONLY {
                for _ in 0..line.matches(needle).count() {
                    if !is_unix_gated(&lines, i) {
                        n += 1;
                    }
                }
            }
        }
        n
    }

    /// 正例控制 + 边界规则控制。
    ///
    /// 这几处都是「守护最容易悄悄坏掉」的地方：
    /// 采集器读到 0 处、边界切错（把生产段当成测试段）、
    /// 或把已门控的误算进来，都会让上面那条**恒绿或误报**。
    #[test]
    fn the_ratchet_actually_reads_production_code() {
        // 内联测试模块**之后**的内容不计入
        let inline =
            "fn a() { let _ = libc::getppid(); }\n#[cfg(test)]\nmod t { let _ = libc::getppid(); }\n";
        assert_eq!(count_in(inline), 1, "内联测试模块里的那处不该计入");

        // 声明式 `mod tests;` **不是**边界——它的内容在别的文件里
        let declared = "#[cfg(test)]\nmod tests;\nfn b() { let _ = libc::getppid(); }\n";
        assert_eq!(count_in(declared), 1, "声明式模块不该把后面的生产代码切掉");

        // `#[path = …]` 夹在中间也要能跨过去
        let with_path =
            "fn c() { let _ = libc::getppid(); }\n#[cfg(test)]\n#[path = \"x.rs\"]\nmod y;\n";
        assert_eq!(count_in(with_path), 1, "带 #[path] 的声明不该成为边界");

        // 已门控的**不算缺口**——它在 Windows 上不会编译
        let gated = "fn f() {\n    #[cfg(unix)]\n    {\n        use std::os::unix::fs::PermissionsExt;\n    }\n}\n";
        assert_eq!(
            count_in(gated),
            0,
            "已门控的不该计入（否则这条守护会一直误报）"
        );
        let gated_item =
            "#[cfg(unix)]\npub fn g() {\n    use std::os::unix::fs::PermissionsExt;\n}\n";
        assert_eq!(count_in(gated_item), 0, "函数级门控也不该计入");

        // 整个仓当前确实有命中（不是 0）
        assert!(
            hits().len() > 0,
            "读到 0 处——采集器或边界规则坏了，上面那条会恒绿"
        );
    }
}

/// 守护：**shell 脚本里的 `$VAR` 后面不许紧跟非 ASCII 字符。**
///
/// 起因是 v0.0.266 那次发版**真的卡住了**：`release.sh:61` 在这台机器上
/// 报 `DEV_DIR?: unbound variable`，`set -u` 之下整条发版链中止。
///
/// 机理：bash 的变量名在**多字节 locale** 下允许高位字节，于是
/// `$DEV_DIR）`（全角右括号紧跟）被当成变量名 `DEV_DIR` + `）`，
/// 那个变量不存在，`set -u` 立刻退出。
///
/// ⚠️ **反直觉的地方**：不是 `LC_ALL=C` 才炸，恰恰是 **UTF-8 locale 才炸**——
/// 实测 `C` locale 下正常、`en_US.UTF-8` 下失败。所以「本机跑过」不证明它安全，
/// 换个 locale（或 CI、或别人机器）就炸。引号**救不了**（出问题的那行本来就在双引号里），
/// 只有 `${VAR}` 花括号能定住名字。
///
/// 这已经是**同一个坑第三次**：`build-app.sh` 的注释里明写着
/// 「`$APP_DIR（` ⇒ 报 `APP_DIR...: unbound variable`。这个坑踩了两次」。
/// 修过一次、没留下门禁，于是换个文件又出现 6 处。
#[cfg(test)]
mod shell_quoting_sentinel {
    use std::path::{Path, PathBuf};

    /// 仓里的 shell 脚本目录（`app/src-tauri` 往上两层）。
    fn scripts_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts")
    }

    /// 扫一行，返回所有「`$VAR` 紧跟非 ASCII」的位置。
    ///
    /// 跳过的三类，都不是 bug：
    /// - 整行注释——`build-app.sh` 里那行注释**正是在描述这个坑**；
    /// - `$` 后面跟 `(`（命令替换 `$(...)`）或字母数字以外的东西（`$1`/`$?`/`$@`）；
    /// - 被反斜杠转义的 `\$VAR`（那是要输出字面量 `$`）。
    ///
    /// `${VAR}` 天然不命中：名字后面紧跟的是 `}`，是 ASCII。
    fn scan_line(line: &str) -> Vec<String> {
        fn name_end(bytes: &[u8], from: usize) -> usize {
            let mut i = from;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            i
        }
        let bytes = line.as_bytes();
        let mut hits = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b'$' {
                i += 1;
                continue;
            }
            // `\$` 是转义，不算变量引用
            let escaped = i > 0 && bytes[i - 1] == b'\\' && !bytes[..i].ends_with(b"\\\\");
            let start = i + 1;
            if start >= bytes.len() {
                break;
            }
            let first = bytes[start];
            if !(first.is_ascii_alphabetic() || first == b'_') {
                i = start;
                continue;
            }
            if escaped {
                i = name_end(bytes, start);
                continue;
            }
            let end = name_end(bytes, start);
            let name = &line[start..end];
            let next = line[end..].chars().next();
            if let Some(c) = next {
                if !c.is_ascii() {
                    hits.push(format!("${name}{c}（`{c}` 是 U+{:04X}）", c as u32));
                }
            }
            i = end;
        }
        hits
    }

    fn scan_all_scripts() -> Vec<String> {
        let dir = scripts_dir();
        let mut hits = Vec::new();
        let entries =
            std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("应当读得到 scripts/：{e}"));
        let mut paths: Vec<PathBuf> = entries
            .map(|e| e.expect("目录项应可读").path())
            .filter(|p| p.extension().is_some_and(|x| x == "sh"))
            .collect();
        paths.sort();
        assert!(
            !paths.is_empty(),
            "scripts/ 下一个 .sh 都没有——守护本身失效了"
        );
        for path in paths {
            let text = std::fs::read_to_string(&path).expect("脚本应可读");
            for (n, line) in text.lines().enumerate() {
                if line.trim_start().starts_with('#') {
                    continue;
                }
                for hit in scan_line(line) {
                    let name = path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    hits.push(format!("{name}:{}: {hit}  |  {}", n + 1, line.trim()));
                }
            }
        }
        hits
    }

    #[test]
    fn no_shell_script_expands_a_variable_straight_into_a_multibyte_character() {
        let hits = scan_all_scripts();
        assert!(
            hits.is_empty(),
            "这些 `$VAR` 后面紧跟了非 ASCII 字符——UTF-8 locale 下 bash 会把那个字符\n\
             并进变量名，`set -u` 于是报 `VAR?: unbound variable` 并中止脚本。\n\
             改写成 `${{VAR}}`（花括号）即可，引号无效。命中：\n  {}",
            hits.join("\n  ")
        );
    }

    /// 扫描器自己得能看见坏样例。
    ///
    /// 只会返回「没有命中」的扫描器等于没有扫描器——这条拿真实坏写法钉住检测逻辑，
    /// 顺带把三类该放过的也钉住（注释、命令替换、转义）。
    #[test]
    fn the_scanner_sees_a_bare_name_next_to_a_full_width_paren() {
        let bad = r#"    echo "!! 工具链里没有 SwiftUIMacros（$DEV_DIR）⇒ 自动 SKIP_SWIFT=1""#;
        let hits = scan_line(bad);
        assert_eq!(hits.len(), 1, "应恰好报一处：{hits:?}");
        assert!(
            hits[0].starts_with("$DEV_DIR）"),
            "报出的应是 `$DEV_DIR）`：{hits:?}"
        );
    }

    #[test]
    fn braced_expansion_and_the_three_legitimate_shapes_are_left_alone() {
        for ok in [
            r#"echo "工具链里没有（${DEV_DIR}）⇒ 跳过""#, // 花括号定住了名字
            r#"echo "（$(wc -l < "$HITS") 条）""#,        // 命令替换，不是变量
            r#"echo "第 $1 行、第 $? 行、第 $@ 些""#,     // 位置参数，不是变量名
            r#"echo "字面量 \$DEV_DIR 不是变量""#,        // 转义
            r#"echo "工具链是 $DEV_DIR 跳过 Swift 那段""#, // 紧跟的是 ASCII 空格
        ] {
            assert_eq!(scan_line(ok), Vec::<String>::new(), "不该命中：{ok}");
        }
    }

    // MARK: 同一类里另一种死法——`set -u` 引用了从没定义过的变量

    /// 扫一行，返回所有「裸展开了一个本文件里没定义、也没兜底的变量」。
    ///
    /// `assigned` 是**全文件**收集到的赋值（含 `read` 的多个目标、
    /// `for` 的循环变量、`scripts/common.sh` 里共享的常量）。
    /// `optional` 是别处写过 `${VAR:-…}` 的那些——作者已经表明它可缺，**不报**。
    fn undefined_in_line(line: &str, assigned: &[String], optional: &[String]) -> Vec<String> {
        fn name_end(b: &[u8], from: usize) -> usize {
            let mut i = from;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            i
        }
        let known = |n: &str| assigned.iter().any(|a| a == n) || optional.iter().any(|a| a == n);
        let line = strip_comment(line);
        let bytes = line.as_bytes();
        let mut hits = Vec::new();
        let mut i = 0;
        let mut in_single = false;
        while i < bytes.len() {
            match bytes[i] {
                // **单引号里的 `$` 是字面量**，不展开。
                // 实测踩过：`test-scan-secrets.sh` 里那段 awk 程序整体包在单引号中，
                // 里面的 `${rest%%:*}` 是给用户看的样例文本，不是脚本自己的变量。
                b'\'' => {
                    in_single = !in_single;
                    i += 1;
                    continue;
                }
                b'\\' if !in_single => {
                    i += 2; // 转义掉下一个字符
                    continue;
                }
                _ => {}
            }
            if in_single || bytes[i] != b'$' {
                i += 1;
                continue;
            }
            // `${NAME}` 的名字从 `{` 之后开始；`$NAME` 从 `$` 之后开始
            let braced = bytes.get(i + 1) == Some(&b'{');
            let start = i + if braced { 2 } else { 1 };
            if start >= bytes.len() || !(bytes[start].is_ascii_alphabetic() || bytes[start] == b'_')
            {
                i = start;
                continue;
            }
            let end = name_end(bytes, start);
            let name = &line[start..end];
            // 名字后面紧跟 `:` 或 `=` ⇒ `${NAME:-…}` / `${NAME:=…}`，有兜底。
            // ⚠️ 紧跟 `}` **不算**兜底——那是普通的花括号展开，正是要报的那种。
            let guarded = matches!(bytes.get(end), Some(b':' | b'='));
            if !guarded && !known(name) {
                hits.push(format!("${name}"));
            }
            i = end;
        }
        hits
    }

    /// 去掉行内注释。`#` 在引号里不算注释开头。
    fn strip_comment(line: &str) -> &str {
        let b = line.as_bytes();
        let (mut in_s, mut in_d) = (false, false);
        for i in 0..b.len() {
            match b[i] {
                b'\\' => continue,
                b'\'' if !in_d => in_s = !in_s,
                b'"' if !in_s => in_d = !in_d,
                b'#' if !in_s && !in_d => return &line[..i],
                _ => {}
            }
        }
        line
    }

    /// 收集一个文件里定义过的变量名。
    fn assigned_in(text: &str) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for raw in text.lines() {
            let line = strip_comment(raw);
            let b = line.as_bytes();
            // `NAME=` 出现在「词首」才算赋值
            let mut i = 0;
            while i < b.len() {
                if !(b[i].is_ascii_alphabetic() || b[i] == b'_') {
                    i += 1;
                    continue;
                }
                if i > 0 && !matches!(b[i - 1], b' ' | b'\t' | b';' | b'&' | b'|' | b'(') {
                    i += 1;
                    continue;
                }
                let mut j = i;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                if b.get(j) == Some(&b'=') {
                    names.push(line[i..j].to_string());
                }
                i = j;
            }
            // `read -r a b c`：只剥掉 `-x` / `--long` 选项，剩下的到分隔符为止全是变量名。
            // ⚠️ 必须按**整行**找 `read` 这个词：早先版本在空白切分出的 token 里 `find`，
            // 却拿那个偏移去切整行，于是切到别处、`read` 的目标一个都没收进来。
            let bytes = line.as_bytes();
            let mut i = 0;
            while i + 4 <= bytes.len() {
                let is_word = (i == 0
                    || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
                    && &bytes[i..i + 4] == b"read"
                    && bytes.get(i + 4).is_some_and(|c| c.is_ascii_whitespace());
                if !is_word {
                    i += 1;
                    continue;
                }
                let mut j = i + 4;
                // 剥选项：`-r` / `-a` / `--color` 一律以 `-` 开头
                loop {
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j] == b'-' {
                        while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        continue;
                    }
                    break;
                }
                let stop = j;
                let mut k = j;
                while k < bytes.len() && !matches!(bytes[k], b';' | b'|' | b'&' | b'<' | b'>') {
                    k += 1;
                }
                for tok in line[stop..k].split_whitespace() {
                    if tok.starts_with('-') || tok.is_empty() {
                        continue;
                    }
                    let ok = tok.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                        && tok
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
                    if ok {
                        names.push(tok.to_string());
                    }
                }
                i = k.max(i + 4);
            }

            // `for NAME in …` / `for ((NAME=…` 的循环变量
            let words: Vec<&str> = line.split_whitespace().collect();
            for (k, w) in words.iter().enumerate() {
                if *w != "for" && *w != "select" {
                    continue;
                }
                let Some(next) = words.get(k + 1) else {
                    continue;
                };
                let next = next.trim_start_matches("((");
                if !next.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
                    continue;
                }
                let name: String = next
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    names.push(name);
                }
            }
        }
        names
    }

    /// 别处写过 `${VAR:-` / `${VAR:=` 的变量：作者已表明它可缺。
    fn optional_in(text: &str) -> Vec<String> {
        let mut out = Vec::new();
        let b = text.as_bytes();
        let mut i = 0;
        while i + 1 < b.len() {
            if b[i] == b'$' && b[i + 1] == b'{' {
                let mut j = i + 2;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                if j > i + 2 && matches!(b.get(j), Some(b':' | b'=')) {
                    out.push(text[i + 2..j].to_string());
                    i = j;
                    continue;
                }
            }
            i += 1;
        }
        out
    }

    /// bash 自己提供的、或从 `scripts/common.sh` 共享来的，不算「该脚本没定义」。
    ///
    /// ⚠️ 这里**只能列 bash 内建 + `SCRIPT_DIR` 这类本脚本自己算出来的**。
    /// 早先版本把 `APP_NAME` 也列了进来——那等于把这条守护要抓的 bug
    /// 直接豁免掉：变异删掉 `common.sh` 里的定义，它照样全绿。
    /// `APP_NAME` 该由 `common.sh` 提供，走 `common_names` 那条路进来。
    fn ambient_names() -> Vec<String> {
        [
            "BASH_SOURCE",
            "PATH",
            "HOME",
            "PWD",
            "TMPDIR",
            "USER",
            "LANG",
            "VERSION",
            "HOSTNAME",
            "RANDOM",
            "SECONDS",
            "LINENO",
            "PIPESTATUS",
            "UID",
            "EUID",
            "SHELL",
            "TERM",
            "IFS",
            "REPLY",
            "OSTYPE",
            "PPID",
            "FUNCNAME",
            "DEVELOPER_DIR",
            "SDKROOT",
            "SCRIPT_DIR",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn no_set_u_script_expands_a_variable_it_never_defines() {
        let dir = scripts_dir();
        let common_text = std::fs::read_to_string(dir.join("common.sh")).expect("读得到 common.sh");
        let common_names = assigned_in(&common_text);
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
            .expect("读得到 scripts/")
            .map(|e| e.expect("目录项可读").path())
            .filter(|p| p.extension().is_some_and(|x| x == "sh"))
            .collect();
        paths.sort();
        assert!(
            !paths.is_empty(),
            "scripts/ 下一个 .sh 都没有——守护本身失效了"
        );

        let mut problems: Vec<String> = Vec::new();
        for path in &paths {
            let text = std::fs::read_to_string(path).expect("脚本应可读");
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name == "common.sh" {
                continue; // 它就是来源
            }
            let mut defined = ambient_names();
            defined.extend(common_names.iter().cloned());
            defined.extend(assigned_in(&text));
            let optional = optional_in(&text);
            for (n, line) in text.lines().enumerate() {
                for hit in undefined_in_line(line, &defined, &optional) {
                    problems.push(format!(
                        "{name}:{}: {hit} 从未定义，也没有 :- 兜底（该脚本有 set -u）\n      {}",
                        n + 1,
                        line.trim()
                    ));
                }
            }
        }
        assert!(
            problems.is_empty(),
            "这些变量在该脚本里从未赋值、也没写 ${{VAR:-}} 兜底，而脚本开头是 `set -euo pipefail`——\n\
             跑到那一行会直接 `unbound variable` 中止。实测就是这样卡死过一次发版。\n  {}",
            problems.join("\n  ")
        );
    }

    /// 正例控制：扫描器必须看得见真问题。
    ///
    /// v0.0.266 之前 `release.sh` 里的 `${APP_NAME}` 就是这个形状——
    /// 它是 `build-app.sh` 的变量，跨进程取不到。
    #[test]
    fn the_undefined_variable_scanner_sees_a_real_one() {
        let bad = r#"    echo "!! 产不出 dist/${APP_NAME}-Swift.app 那条回退路""#;
        let hits = undefined_in_line(bad, &["SCRIPT_DIR".to_string()], &[]);
        assert_eq!(hits, vec!["$APP_NAME".to_string()], "应报出 $APP_NAME");
        // 给它一个定义之后就不报了——证明这条判据确实在看「有没有定义」
        assert!(undefined_in_line(bad, &["APP_NAME".to_string()], &[]).is_empty());
    }
}

#[cfg(test)]
mod single_tray_tests {
    #[test]
    fn configuration_does_not_create_a_second_tray() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert!(
            config["app"].get("trayIcon").is_none(),
            "setup 创建带菜单的 main 托盘，配置不能再自动创建一个托盘"
        );
    }
}

#[cfg(test)]
mod background_lifecycle_tests {
    #[test]
    fn all_windows_are_declared_but_only_created_on_demand() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let windows = config["app"]["windows"].as_array().unwrap();
        assert_eq!(windows.len(), 3);
        for window in windows {
            assert_eq!(
                window["create"].as_bool().unwrap_or(true),
                false,
                "{} must not eagerly create a WebView",
                window["label"]
            );
        }
    }
}
