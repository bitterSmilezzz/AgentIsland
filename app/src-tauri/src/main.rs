// 始终隐藏控制台：日志统一走 %TEMP%gentisland-tauri.log（log_from_ui + panic hook）
#![cfg_attr(windows, windows_subsystem = "windows")]

mod engine;
mod filemon;
mod models;
mod placement;
mod procmon;
mod provider;
mod registry;
mod session;
mod settings;
mod tokens;
mod webhook;

use engine::ActivityEngine;
use models::DockEdge;
use settings::Settings;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

type SharedEngine = Arc<Mutex<ActivityEngine>>;

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

#[tauri::command]
fn get_settings(state: State<SharedEngine>) -> Settings {
    state.lock().unwrap().settings.clone()
}

#[tauri::command]
fn save_settings(state: State<SharedEngine>, new_settings: Settings) {
    let mut e = state.lock().unwrap();
    e.settings = new_settings.normalized();
    let s = e.settings.clone();
    drop(e);
    s.save();
}

#[tauri::command]
fn set_dock_edge(state: State<SharedEngine>, app: AppHandle, edge: String) {
    let anchor;
    {
        let mut e = state.lock().unwrap();
        e.settings.dock_edge = edge.clone();
        anchor = e.settings.dock_anchor;
        let s = e.settings.clone();
        s.save();
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
    let wa = placement::work_area_under_window(&window);
    let (left, top) = placement::place_with(edge, anchor, width, height, wa);
    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    window
        .set_position(LogicalPosition::new(left, top))
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn snap_nearest_edge(
    window: tauri::WebviewWindow,
    state: State<SharedEngine>,
    width: f64,
    height: f64,
) -> Result<String, String> {
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
        s.save();
    }
    let edge = DockEdge::parse(edge_str);
    let wa = placement::work_area_under_window(&window);
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
fn reposition_now(window: tauri::WebviewWindow, state: State<SharedEngine>, width: f64, height: f64) {
    let (edge, anchor) = {
        let e = state.lock().unwrap();
        (
            DockEdge::parse(&e.settings.dock_edge),
            e.settings.dock_anchor,
        )
    };
    let wa = placement::work_area_under_window(&window);
    let (left, top) = placement::place_with(edge, anchor, width, height, wa);
    let _ = window.set_position(LogicalPosition::new(left, top));
}

fn reposition(app: &AppHandle, _state: State<SharedEngine>, edge: &DockEdge, anchor: f64, _expanded: bool) {
    if let Some(win) = app.get_webview_window("island") {
        let size = win.outer_size().unwrap_or_default();
        let scale = win.scale_factor().unwrap_or(1.0);
        let w = size.width as f64 / scale;
        let h = size.height as f64 / scale;
        let wa = placement::work_area_under_window(&win);
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
            .filter(|p| !e.settings.disabled_agents.contains(&p.id) && !p.token_roots.is_empty())
            .collect();
        if profiles.is_empty() {
            return None;
        }
        let mut agg_tokens24 = 0i64;
        let mut agg_total = 0i64;
        let mut agg_cost24 = 0f64;
        let mut agg_cost_total = 0f64;
        let mut hourly: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
        let mut models: std::collections::HashMap<String, (i64, f64)> = std::collections::HashMap::new();
        for p in &profiles {
            if let Some(r) = e.get_report(&p.id) {
                agg_tokens24 += r.usage.tokens24h;
                agg_total += r.usage.tokens_total;
                agg_cost24 += r.usage.cost24h;
                agg_cost_total += r.usage.cost_total;
                for (ts, v) in r.hourly30d {
                    *hourly.entry(ts).or_insert(0) += v;
                }
                for m in r.models24h {
                    let e2 = models.entry(m.model).or_insert((0, 0.0));
                    e2.0 += m.tokens;
                    e2.1 += m.cost;
                }
            }
        }
        let mut hourly30d: Vec<(i64, i64)> = hourly.into_iter().collect();
        hourly30d.sort_by_key(|kv| kv.0);
        let mut models24h: Vec<models::ModelUsage> = models
            .into_iter()
            .map(|(model, (tokens, cost))| models::ModelUsage { model, tokens, cost })
            .collect();
        models24h.sort_by(|a, b| b.tokens.cmp(&a.tokens));
        return Some(models::TokenReport {
            usage: models::TokenUsage {
                tokens24h: agg_tokens24,
                tokens_total: agg_total,
                cost24h: agg_cost24,
                cost_total: agg_cost_total,
            },
            models24h: models24h.clone(),
            models_total: models24h,
            hourly30d,
        });
    }
    e.get_report(&agent_id)
}

#[tauri::command]
fn clear_latest_event(state: State<SharedEngine>) {
    state.lock().unwrap().latest_event = None;
}

#[tauri::command]
fn terminate_agent(state: State<SharedEngine>, pid: Option<u32>, agent_id: String) {
    let Some(pid) = pid else { return };
    let profile = {
        let e = state.lock().unwrap();
        crate::registry::builtin().into_iter().find(|p| p.id == agent_id)
    };
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .creation_flags(0x08000000) // CREATE_NO_WINDOW
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("kill")
            .args(["-9", &pid.to_string()])
            .status();
    }
    // 身份复核：pid 复用防护（进程名仍须匹配档案）
    if let Some(p) = profile {
        if !p.process_names.is_empty() {
            let mut pm = procmon::ProcessMonitor::new();
            pm.refresh();
            let _ = pm;
        }
    }
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

fn log_line(msg: &str) {
    let path = std::env::temp_dir().join("agentisland-tauri.log");
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{}", msg);
    }
}

fn engine_loop(shared: SharedEngine, app: AppHandle) {
    loop {
        let (state, interval) = {
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
            e.tick();
            let s = e.state();
            let active = s
                .snapshots
                .iter()
                .any(|snap| matches!(snap.level, models::ActivityLevel::Working | models::ActivityLevel::Attention));
            let interval = if active {
                e.settings.sample_interval
            } else {
                (e.settings.sample_interval * 2.5).clamp(2.0, 12.5)
            };
            (s, interval)
        };
        let _ = app.emit("engine://tick", &state);
        std::thread::sleep(std::time::Duration::from_secs_f64(interval));
    }
}

#[tauri::command]
fn log_from_ui(message: String) {
    log_line(&format!("[webview] {}", message));
}

fn main() {
    let (tx, rx) = mpsc::channel::<models::AgentTaskEvent>();
    let settings = Settings::load();
    let mut engine = ActivityEngine::new(settings, rx);
    let demo = std::env::args().any(|a| a == "--demo");
    engine.demo_mode = demo;
    let shared: SharedEngine = Arc::new(Mutex::new(engine));

    std::panic::set_hook(Box::new(|info| {
        log_line(&format!("[panic] {}", info));
    }));
    log_line("=== boot ===");

    tauri::Builder::default()
        .manage(shared.clone())
        .setup(move |app| {
            // 引擎采样线程
            let shared2 = shared.clone();
            let handle = app.handle().clone();
            std::thread::spawn(move || engine_loop(shared2, handle));

            // 本地 Webhook（127.0.0.1:41999）
            std::thread::spawn(move || {
                let _server = webhook::LocalEventServer::start(tx);
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                }
            });

            // 托盘
            let toggle = MenuItem::with_id(app, "toggle", "展开 / 收起灵动岛", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出 AgentIsland", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle, &quit])?;
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("AgentIsland")
                .menu(&menu)
                .on_menu_event(|app, event| {
                    match event.id.as_ref() {
                        "toggle" => {
                            let _ = app.emit("tray://toggle", ());
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click { button: tauri::tray::MouseButton::Left, .. } = event {
                        let _ = tray.app_handle().emit("tray://toggle", ());
                    }
                })
                .build(app)?;

            // 初始贴边放置
            let win = app.get_webview_window("island").unwrap();
            let _ = win.set_ignore_cursor_events(false);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_boot_args,
            get_settings,
            save_settings,
            set_dock_edge,
            place_island,
            snap_nearest_edge,
            reposition_now,
            get_report,
            clear_latest_event,
            log_from_ui,
            terminate_agent,
            collapse_to_tray
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
