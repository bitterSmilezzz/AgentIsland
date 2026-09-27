// 始终隐藏控制台：日志统一走 %TEMP%gentisland-tauri.log（log_from_ui + panic hook）
#![cfg_attr(windows, windows_subsystem = "windows")]

mod atomicfile;
mod cost;
mod duration;
mod engine;
mod filemon;
mod health;
mod models;
mod notifier;
mod observability;
mod placement;
mod procmon;
mod provider;
mod registry;
mod remote;
mod render;
mod report;
mod resilience;
mod session;
mod secret;
mod selftest;
mod settings;
mod smtp;
mod sqlite;
mod audit;
mod budget;
mod forecast;
mod tokens;
mod trees;
mod transport;
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

/// 把侧边栏窗口按记忆值摆好（贴左/贴右、宽度、铺满工作区高度）。
///
/// 与灵动岛那条路分开：灵动岛是「锚点 × 工作区」的浮层，侧边栏是一整列，
/// 两者共用一个 `place_with` 只会让两边都别扭。
fn place_sidebar_window(app: &AppHandle, edge: crate::models::DockEdge, width: f64) {
    let Some(win) = app.get_webview_window("sidebar") else {
        return; // 配置里没有这个窗口（旧包）：如实什么都不做，而不是 panic
    };
    let wa = placement::work_area_under_window(&win);
    let (left, top, w, h) = placement::sidebar_frame(edge, width, wa);
    let _ = win.set_size(LogicalSize::new(w, h));
    let _ = win.set_position(LogicalPosition::new(left, top));
}

/// 切换形态。**两个窗口都建好了**，这里只改显示哪一个——
/// 「切回去」因此不需要重建窗口，也就不会有「切十次留十个窗口」这种事。
#[tauri::command]
fn set_shell_mode(state: State<SharedEngine>, app: AppHandle, mode: String) -> String {
    let mode = crate::models::ShellMode::parse(&mode);
    let (edge, width);
    {
        let mut e = state.lock().unwrap();
        e.settings.shell_mode = mode.as_str().to_string();
        let s = e.settings.clone();
        s.save();
        edge = crate::models::DockEdge::parse(&e.settings.sidebar_edge);
        width = e.settings.sidebar_width;
    }
    match mode {
        crate::models::ShellMode::Island => {
            if let Some(sidebar) = app.get_webview_window("sidebar") {
                let _ = sidebar.hide();
            }
            if let Some(island) = app.get_webview_window("island") {
                let _ = island.show();
            }
        }
        crate::models::ShellMode::Sidebar => {
            place_sidebar_window(&app, edge, width);
            if let Some(sidebar) = app.get_webview_window("sidebar") {
                let _ = sidebar.show();
            }
            if let Some(island) = app.get_webview_window("island") {
                let _ = island.hide();
            }
        }
    }
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
        s.save();
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
        s.save();
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
        // 只要有任一档案的成本是估的，聚合值就不是记录值（见 models.rs 的 `cost_estimated`）
        let mut agg_cost_estimated = false;
        let mut hourly: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
        let mut models: std::collections::HashMap<String, (i64, f64, bool)> = std::collections::HashMap::new();
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
            .map(|(model, (tokens, cost, cost_estimated))| models::ModelUsage {
                model,
                tokens,
                cost,
                cost_estimated,
            })
            .collect();
        models24h.sort_by(|a, b| b.tokens.cmp(&a.tokens));
        return Some(models::TokenReport {
            usage: models::TokenUsage {
                tokens24h: agg_tokens24,
                tokens_total: agg_total,
                cost24h: agg_cost24,
                cost_total: agg_cost_total,
                cost_estimated: agg_cost_estimated,
            },
            models24h: models24h.clone(),
            models_total: models24h,
            hourly30d,
        });
    }
    e.get_report(&agent_id)
}

/// 外发状态快照（设置页用）。**不含任何密钥值**——`has_secret` 是布尔，
/// 凭据只读钥匙串条目名，值永远不经过这里（ADR 0009）。
/// 钥匙串读取本轮未接：ad-hoc 签名下每次出包代码标识都变，取「存在性」也可能弹窗，
/// 所以先如实报 `false`，而不是假装查过。
/// 「发送预览」：**真发之前**让界面看见哪些字节会离开这台机器。
/// 密钥本轮一律传 `None`（Rust 侧还没有钥匙串），预览里 `{key}` 显示成掩码，
/// 而不是假装查到了值。
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
    let kind = crate::remote::EventKind::parse(&args.kind)
        .unwrap_or(crate::remote::EventKind::Attention);
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
        // Rust 还没接 macOS 的在场信号层（锁屏 / 显示器睡眠 / 无输入时长）：
        // 传 unavailable ⇒ 按 fail-open 判成「人不在」，依据写在 awayReason 里
        &crate::remote::PresenceSignals::unavailable(),
    );
    // 节流状态在 notifier 里，不在判定层：这里补上
    snapshot.throttled = engine.notifier.throttle_keys();
    snapshot
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
fn agent_process_tree(state: State<SharedEngine>, agent_id: String) -> Option<crate::trees::TreeReport> {
    let pid = {
        let engine = state.lock().unwrap();
        engine
            .snapshots
            .iter()
            .find(|s| s.id == agent_id)?
            .pid?
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

#[tauri::command]
fn clear_latest_event(state: State<SharedEngine>) {
    // 确认这一条、推下一条：覆盖式清除会把同一拍里排队的告警一起丢掉
    state.lock().unwrap().ack_latest_event();
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
    // `--selftest` / `selftest`：无头自检（对应 Swift 的 `agentisland selftest`）。
    // 放在最前面：它不该启动 UI、采集或任何后台线程——自检的全部价值是
    // 「在这台机器上，判定逻辑本身还对不对」，被采集的副作用搅进来就不再是那个问题的答案。
    if std::env::args().any(|arg| arg == "--selftest" || arg == "selftest") {
        let report = selftest::run();
        print!("{}", report.text());
        std::process::exit(report.exit_code());
    }
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

            // 上次关在侧边栏形态 ⇒ 这次仍开侧边栏。**两个窗口都已经建好**，
            // 这里只是决定显示哪一个——直接复用命令那条路径，避免「启动」与「切换」
            // 两处各写一份显隐规则（两份规则迟早只改一处）
            let (mode, sidebar_edge, sidebar_width) = {
                let e = shared.lock().unwrap();
                (
                    crate::models::ShellMode::parse(&e.settings.shell_mode),
                    crate::models::DockEdge::parse(&e.settings.sidebar_edge),
                    e.settings.sidebar_width,
                )
            };
            if mode == crate::models::ShellMode::Sidebar {
                place_sidebar_window(app.handle(), sidebar_edge, sidebar_width);
                if let Some(sidebar) = app.get_webview_window("sidebar") {
                    let _ = sidebar.show();
                }
                let _ = win.hide();
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_boot_args,
            get_settings,
            save_settings,
            remote_status,
            remote_preview,
            token_forecast,
            audit_report_markdown,
            audit_report_csv,
            agent_process_tree,
            run_selftest,
            token_report_markdown,
            token_report_csv,
            remote_secret_set,
            remote_secret_delete,
            set_dock_edge,
            set_shell_mode,
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
            collapse_to_tray
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
