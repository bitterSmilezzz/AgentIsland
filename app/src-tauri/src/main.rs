// 始终隐藏控制台：日志统一走 %TEMP%gentisland-tauri.log（log_from_ui + panic hook）
#![cfg_attr(windows, windows_subsystem = "windows")]

mod atomicfile;
mod cleaner;
mod cli;
mod cost;
mod deeplink;
mod duration;
mod selfreport;
#[cfg(test)]
mod testutil;
mod todos;
mod engine;
mod filemon;
mod health;
mod installed;
mod models;
mod notifier;
mod observability;
mod placement;
mod power;
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
/// 命令行里的 `--shell=<mode>`（也接受 `--shell <mode>`）。给不了就返回 `None`，
/// 于是「用户没写」与「用户写了 island」不会混为一谈。
/// `--shell=workbench` / `--shell workbench`（**只认 workbench**，其余走
/// [`shell_arg_override`] 那条形态枚举）。与设置无关，只影响这一次运行。
fn boot_arg_is(name: &str) -> bool {
    let args: Vec<String> = std::env::args().collect();
    let mut index = 0;
    while index < args.len() {
        let matched = args[index].strip_prefix("--shell=").is_some_and(|v| v == name)
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
fn work_area_for(win: &tauri::WebviewWindow, state: &State<'_, SharedEngine>) -> (f64, f64, f64, f64, f64) {
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
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    let outcome = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    if let Err(error) = outcome {
        log_line(&format!("[launchAtLogin] {} 失败: {error}", if enabled { "启用" } else { "关闭" }));
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
pub fn tray_badge_text(
    mode: &str,
    active: usize,
    tokens24h: i64,
) -> Option<String> {
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
    let wa = work_area_for(&window, &state);
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
fn reposition_now(window: tauri::WebviewWindow, state: State<SharedEngine>, width: f64, height: f64) {
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

fn reposition(app: &AppHandle, _state: State<SharedEngine>, edge: &DockEdge, anchor: f64, _expanded: bool) {
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
        // 在场信号：锁屏 / 显示器睡眠 / 无输入时长（`power::presence_signals` 采集）。
        // 「只在人不在时发」靠的其实是第三条——远程桌面连着 Mac 时
        // 会话既不锁屏也不熄屏，只有无输入这条管用。
        &power::presence_signals(),
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

// MARK: - Codex 档位（Phase 2）

#[tauri::command]
fn provider_scan_tools() -> Vec<crate::provider::ToolScan> {
    crate::provider::scan_tools()
}

#[tauri::command]
fn provider_list_profiles() -> Vec<crate::provider::CodexProfile> {
    crate::provider::ProviderStore::at_default().list()
}

#[tauri::command]
fn provider_save_profile(
    profile: crate::provider::CodexProfile,
) -> Result<crate::provider::CodexProfile, String> {
    crate::provider::ProviderStore::at_default().save(profile)
}

#[tauri::command]
fn provider_delete_profile(id: String) -> Result<(), String> {
    crate::provider::ProviderStore::at_default().delete(&id)
}

#[tauri::command]
fn provider_status() -> crate::provider::ProviderStatus {
    let store = crate::provider::ProviderStore::at_default();
    let profiles = store.list();
    let path = crate::provider::codex_config_path();
    let installed = path.as_ref().is_some_and(|p| p.exists());
    let active_provider_id = path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| crate::provider::active_provider_id(&text));
    let active_profile_id = active_provider_id.as_ref().and_then(|provider| {
        profiles
            .iter()
            .find(|profile| &profile.provider_id == provider)
            .map(|profile| profile.id.clone())
    });
    crate::provider::ProviderStatus {
        installed,
        config_path: path.map(|p| p.to_string_lossy().to_string()),
        active_provider_id,
        active_profile_id,
        profile_count: profiles.len(),
        limitations: crate::provider::PROVIDER_LIMITATIONS,
    }
}

/// 切换档位：**先备份、再原子写**。写失败时原文件逐字节不动（`atomicfile` 保证）。
#[tauri::command]
fn provider_apply_profile(id: String) -> Result<crate::provider::ProviderApplyResult, String> {
    let store = crate::provider::ProviderStore::at_default();
    let profile = store
        .list()
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("没有这个档位：{id}"))?;
    let target = crate::provider::codex_config_path()
        .ok_or_else(|| "找不到 Codex 配置目录".to_string())?;
    let backup = crate::provider::apply_codex_profile(
        &target,
        &store.backups_dir(),
        &profile,
        crate::tokens::now_ms(),
    )
    .map_err(|e| format!("切换档位失败：{e}"))?;
    Ok(crate::provider::ProviderApplyResult {
        config_path: target.to_string_lossy().to_string(),
        backup_name: backup
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        limitations: crate::provider::PROVIDER_LIMITATIONS,
    })
}

#[tauri::command]
fn provider_list_backups() -> Vec<crate::provider::BackupInfo> {
    crate::provider::list_backups(&crate::provider::ProviderStore::at_default().backups_dir())
}

/// 按**名字**还原（界面只能选我们列出的备份；传别的名字一律拒绝）
#[tauri::command]
fn provider_restore_backup(name: String) -> Result<(), String> {
    let store = crate::provider::ProviderStore::at_default();
    let target = crate::provider::codex_config_path()
        .ok_or_else(|| "找不到 Codex 配置目录".to_string())?;
    crate::provider::restore_backup_by_name(&target, &store.backups_dir(), &name)
        .map_err(|e| format!("还原失败：{e}"))
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
        "md" | "markdown" => Ok(crate::audit::markdown_export(
            &snapshots.0,
            &[],
            Some(&snapshots.1),
            now,
        )
        .content),
        other => Err(format!("--format 只认 md 与 csv（收到 {other}）")),
    }
}

/// 隐藏工作台窗口（**只隐藏，不销毁**——内容与滚���位置都留着）。
///
/// 与关窗口分开是有意的：工作台是「随时瞄一眼」的面板，
/// 隐藏再打开时用户期望的是回到刚才的位置，而不是一个刚加载完的空白页。
#[tauri::command]
fn hide_workbench(app: AppHandle) {
    if let Some(window) = app.get_webview_window("workbench") {
        let _ = window.hide();
    }
}

#[tauri::command]
fn show_workbench(app: AppHandle) {
    reveal_workbench_window(&app);
}

/// 把工作台窗口叫到前面。**托盘、深链、前端三处共用这一段**——
/// 三处各写一份显隐规则的话，迟早只有一处会带 `unminimize`。
fn reveal_workbench_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("workbench") {
        let _ = window.show();
        // 隐藏后再打开时窗口可能是最小化的；只 `show` 会得到一个「在 Dock 里但看不见」的窗口
        let _ = window.unminimize();
        let _ = window.set_focus();
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

/// 写一行诊断日志。**打开失败要自报，不能吞掉**。
///
/// 此前是 `if let Ok(mut f) = …open(&path)`，失败时什么都不发生——于是
/// 「打包后的 App 日志一行都没有」这件事本身查不出来：没有日志就没有线索，
/// 没有线索就只能猜是不是代码没跑到。v0.0.222 我为「工作台为什么没动静」
/// 卡了很久，根因就在这里：**通道坏了，而它坏得毫无声响**。
///
/// 写不出去时退回 stderr：GUI 应用的 stdout 通常没人看，但**总比静默好**——
/// 从终端 `open` 出来的那一次就能看见。
fn log_line(msg: &str) {
    let path = std::env::temp_dir().join("agentisland-tauri.log");
    // 说明：日志文件是**累积**的（同一路径、按运行叠加），所以每次启动
    // 都先写一行 `[run]` 标记。没有它就没法把「这一次跑出来的行」与
    // 「历史遗留的行」分开——我为此误判过好几轮：把累积日志里的
    // `[page] / [webview]` 当成当次运行的证据。

    use std::io::Write;
    match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
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
            let badge_mode = e.settings.menu_bar_badge_mode.clone();
            e.tick();
            let s = e.state();
            // 托盘徽标：跟随 `menu_bar_badge_mode`（iconOnly 时不设标题）
            if let Some(tray) = app.tray_by_id("main") {
                let active = s
                    .snapshots
                    .iter()
                    .filter(|snap| {
                        matches!(snap.level, models::ActivityLevel::Working | models::ActivityLevel::Attention)
                    })
                    .count();
                if let Some(text) = tray_badge_text(&badge_mode, active, s.grand_total.tokens24h) {
                    let _ = tray.set_title(Some(&text));
                } else {
                    let _ = tray.set_title::<&str>(None);
                }
            }
            let active = s
                .snapshots
                .iter()
                .any(|snap| matches!(snap.level, models::ActivityLevel::Working | models::ActivityLevel::Attention));
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
        message: if message.is_empty() { None } else { Some(message) },
        detail: if detail.is_empty() { None } else { Some(detail) },
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
        // 两个窗口都建好了才谈「显示哪个」——这里只发意图，
        // 由前端按 `shell_mode` 决定显示岛还是侧边栏，
        // 免得 Rust 侧再写一份显隐规则（两份规则迟早只改一处）
        let _ = app.emit("deeplink://navigate", serde_json::json!({
            "action": format!("{action:?}"),
        }));
    } else if matches!(action, deeplink::Action::Workbench) {
        // 工作台是独立窗口：把它叫到前面。**同时**把意图发给前端，
        // 这样「已开着工作台时收到一个 agent 深链」会推进内容而不是白等一次显隐。
        reveal_workbench_window(app);
        let _ = app.emit("deeplink://navigate", serde_json::json!({
            "action": format!("{action:?}"),
        }));
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
    if let Some(code) = cli::try_run(&argv) {
        std::process::exit(code);
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
    // **每次启动一行 run 标记**：日志文件是累积的，只有它能把当次运行切出来。
    // 少了这一行，「某类日志没出现」这个结论就不可信——而我恰好靠它下了好几轮结论。
    log_line(&format!("[run] pid={} 启动", std::process::id()));
    log_line("=== boot ===");

    tauri::Builder::default()
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
        .setup(move |app| {
            // 全局热键：按设置里的开关注册。
            //
            // 刻意**不在设置变化时重注册**：热键注册要在主线程做，而设置改完
            // 立刻生效是本轮的承诺之一——那就在每拍检查一次「开关状态与
            // 当前注册状态是否一致」，不一致才动。注册失败只记日志，
            // 不打断引擎循环：热键是锦上添花，不该让它把监控整个拖停。
            {
                // 热键回调：与托盘同一个动作（展开 / 收起），不另发明一套
                if let Ok(shortcut) = parse_hotkey() {
                    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
                    let _ = app.global_shortcut().on_shortcut(shortcut, move |app, _, event| {
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
                    for url in event.urls() {
                        handle_deep_link(&app_handle, url.as_str());
                    }
                });
            }

            // 引擎采样线程
            let shared2 = shared.clone();
            let handle = app.handle().clone();
            std::thread::spawn(move || engine_loop(shared2, handle));

            // 本地 Webhook（Rust 端 127.0.0.1:42000，与 Swift 的 41999 分开以免静默抢端口）
            let shared_for_webhook = shared.clone();
            std::thread::spawn(move || {
                let _server = webhook::LocalEventServer::start(tx, shared_for_webhook);
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(3600));
                }
            });

            // 托盘
            let toggle = MenuItem::with_id(app, "toggle", "展开 / 收起灵动岛", true, None::<&str>)?;
            // 工作台是**第三个窗口**，不能只靠深链进：深链在这台机器上被
            // Swift 版抢走的可能性是真实存在的（两个应用都声明了同一个 scheme），
            // 所以它必须有一条不经过 URL scheme 的入口。
            let workbench = MenuItem::with_id(app, "workbench", "打开工作台", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出 AgentIsland", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle, &workbench, &quit])?;
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().unwrap().clone())
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
                    if let tauri::tray::TrayIconEvent::Click { button: tauri::tray::MouseButton::Left, .. } = event {
                        let _ = tray.app_handle().emit("tray://toggle", ());
                    }
                })
                .build(app)?;

            // **启动痕迹：把真实建出来的窗口逐个记下来。**
            //
            // 为什么加这条：v0.0.230 之前，Rust 端「界面不回调」这件事只能从
            // 「没有 [webview] 日志」反推——而那条日志**本来就是 webview 写的**，
            // webview 不跑 ⇒ 它必然是空的 ⇒ 这条证据无法自证。
            // 从 Rust 侧记一份「我建了哪些窗口」，至少能把「窗口没建出来」
            // 与「窗口建了但里面没跑 JS」分开。
            let labels: Vec<String> = app
                .webview_windows()
                .keys()
                .cloned()
                .collect();
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
                for key in [
                    "index.html",
                    "js/main.js",
                    "js/views.js",
                    "css/tokens.css",
                    "css/workbench.css",
                ] {
                    match resolver.get(key.to_string()) {
                        Some(asset) => seen.push(format!("{key}={}B/{}", asset.bytes.len(), asset.mime_type)),
                        None => seen.push(format!("{key}=**缺失**")),
                    }
                }
                log_line(&format!("[boot] 资源表：{seen:?}"));
            }

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
            // `--shell=sidebar` / `--shell=island`：**只影响这一次运行，不写回设置**。
            // 加它是为了能在不改用户 settings.json 的前提下验证另一个形态
            // （改了设置去验证，验证完还得记得改回来，那是最容易留下脏状态的做法）
            let mode = shell_arg_override().unwrap_or(mode);
            if mode == crate::models::ShellMode::Sidebar {
                place_sidebar_window(app.handle(), sidebar_edge, sidebar_width);
                if let Some(sidebar) = app.get_webview_window("sidebar") {
                    let _ = sidebar.show();
                }
                let _ = win.hide();
            }
            // `--shell=workbench`：同样**只影响这一次运行，不写回设置**。
            // 存在的理由与 `--shell=sidebar` 一样——验证第三个形态不该靠改用户的设置，
            // 而工作台平时是按需才出现的（托盘 / 深链），没有启动参数就没法无头验证它。
            if boot_arg_is("workbench") {
                reveal_workbench_window(app.handle());
                let _ = win.hide();
                if let Some(sidebar) = app.get_webview_window("sidebar") {
                    let _ = sidebar.hide();
                }
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
            todos_list,
            todos_add,
            todos_toggle,
            todos_remove,
            todos_clear_done,
            provider_scan_tools,
            provider_list_profiles,
            provider_save_profile,
            provider_delete_profile,
            provider_status,
            provider_apply_profile,
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
        .run(tauri::generate_context!())
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
        assert_eq!(tray_badge_text("activeCount", 2, 0), Some("⚡️ 2".to_string()));
        assert_eq!(tray_badge_text("activeCount", 0, 0), Some("⚡️ 0".to_string()));

        // tokenUsage：复用引擎那一套缩写，菜单栏上不该写 `120.00M`
        assert_eq!(tray_badge_text("tokenUsage", 0, 120_000), Some("120.0k".to_string()));
        assert_eq!(tray_badge_text("tokenUsage", 0, 2_500_000), Some("2.50M".to_string()));
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
            for tokens in [0i64, 999, 1_000, 120_000, 2_500_000, 999_999_999, 1_000_000_000_000] {
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
        assert_eq!(format!("{:?}", shortcut.key), "KeyI", "{HOTKEY_ACCEL} 的主键应当是 I");
    }

    /// 组合键的**形状**要写进断言：改了它，用户肌肉记忆里的快捷键就变了，
    /// 而这件事不该只在 CHANGELOG 里留一句。
    #[test]
    fn the_hotkey_shape_is_pinned_so_a_silent_change_cannot_happen() {
        let shortcut = parse_hotkey().expect("热键组合键必须能解析");
        assert!(
            shortcut.mods.contains(tauri_plugin_global_shortcut::Modifiers::SHIFT),
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

/// **版本位的唯一真相源**：四个地方写着同一个版本号。
///
/// Swift 侧 `AppVersion.string`、CHANGELOG 首条、README 的「本文档描述 vX.Y.Z」
/// 三处早就在 `scripts/release.sh` 里做过一致性预检；`Cargo.toml` 是**第四处**，
/// 此前一直是 0.1.0，而 `raycast` 清单要把版本写进去——照抄 Cargo 的会得到 0.1.0。
///
/// 写法是「读源码」而不是各写各的：`AppVersion.string` 是 Swift 源文本，
/// `CARGO_PKG_VERSION` 是编译期常量。四处对不上时这条用例红，
/// 而发版脚本会在 commit 之前就挡住前两处。
#[cfg(test)]
mod version_pinning {
    use std::path::Path;

    #[test]
    fn the_cargo_version_matches_the_swift_app_version() {
        let manifest = env!("CARGO_MANIFEST_DIR");
        let swift = std::path::Path::new(manifest)
            .join("../../Sources/AgentIslandCore/AppVersion.swift");
        let text = std::fs::read_to_string(&swift).expect("应当读得到 AppVersion.swift");
        let want = text
            .split("string = \"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("AppVersion.string 应当是 \"X.Y.Z\" 形状");
        assert_eq!(
            env!("CARGO_PKG_VERSION"),
            want,
            "Cargo.toml 的版本与 AppVersion.string 不一致——raycast 清单会把 Cargo 的那个写进去"
        );
    }

    /// `tauri.conf.json` 的 `version` 也要跟上——**它决定用户看到的
    /// `CFBundleShortVersionString`**。
    ///
    /// 这条是被「交付物整个换成 Rust 端」这件事逼出来的：换过去之后，用户
    /// 「关于本机」的窗口里写着 **0.1.0**，而 tag 是 0.0.232。
    /// 而原来的 `version_pinning` 只查 Cargo.toml 与 AppVersion.swift，
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
            for prefix in ["export function ", "export async function ", "function ", "async function "] {
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
                if trimmed.starts_with("//") || trimmed.starts_with("*") || trimmed.starts_with("/*") {
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
            // 只查**调用形状** `${name(`，不查 `${name()}`：实际调用都带参
            // （`${pageAnalytics(eng)}`），按无参写会把「调用了」误判成「没调用」。
            assert!(
                body.contains(&format!("${{{name}(")),
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
            .unwrap_or_else(|_| panic!("应当读得到 src")) {
            let path = entry.expect("目录项应当可读").path();
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let mut lines = text.lines().peekable();
            while let Some(line) = lines.next() {
                if line.trim() != "#[tauri::command]" {
                    continue;
                }
                // 命令函数名在下一行：`fn name(` / `fn name<T>(`
                if let Some(next) = lines.peek() {
                    let trimmed = next.trim().strip_prefix("fn ").unwrap_or("");
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
                        && word
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_');
                    if looks_like_command && !commands.contains(word) {
                        missing.push(format!("{name}: `invoke('{word}')` 没有对应的 #[tauri::command]"));
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
        assert!(!files.is_empty(), "一个 ui/js/*.js 都没找到——检查本身失效了");

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
        assert!(!labels.is_empty(), "一个窗口标签都没从 tauri.conf.json 里读到——解析失效了");

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

