use crate::models::DockEdge;

/// 屏幕工作区（任务栏/Dock/菜单栏以外）+ DPI 缩放。
/// Win32 返回物理像素；Tauri set_position/set_size 用逻辑坐标（DIP），必须换算。
///
/// ⚠️ **本文件里下面两个 `#[cfg(windows)]` 函数在 macOS 构建上零引用**，
/// 而「零引用」不等于死代码——它们是 Windows 端的取工作区入口。
/// 判据是 `#[cfg]`：跨平台构建够不到的那一支是**平台代码**，
/// 而同平台可达却没人调用的才是死代码（那个叫 `place`，v0.0.227 已删）。
/// **不要用「零引用」一刀切去删平台函数**。
#[cfg(windows)]
pub fn work_area_under_cursor() -> (f64, f64, f64, f64, f64) {
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    unsafe {
        let mut pt = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut pt) == 0 {
            return (0.0, 0.0, 1536.0, 864.0, 1.0);
        }
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        work_area_of_monitor(hmon)
    }
}

#[cfg(windows)]
pub fn work_area_at(x: i32, y: i32) -> (f64, f64, f64, f64, f64) {
    use windows_sys::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST};
    use windows_sys::Win32::Foundation::POINT;
    unsafe {
        let pt = POINT { x, y };
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        work_area_of_monitor(hmon)
    }
}

#[cfg(windows)]
fn work_area_of_monitor(hmon: windows_sys::Win32::Graphics::Gdi::HMONITOR) -> (f64, f64, f64, f64, f64) {
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};
    use windows_sys::Win32::UI::HiDpi::GetDpiForMonitor;

    unsafe {
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let mut dpi_x: u32 = 96;
        let mut dpi_y: u32 = 96;
        if GetMonitorInfoW(hmon, &mut mi) != 0 {
            let _ = GetDpiForMonitor(hmon, 0 /*MDT_EFFECTIVE_DPI*/, &mut dpi_x, &mut dpi_y);
        }
        let scale = dpi_x as f64 / 96.0;
        let w = mi.rcWork;
        (
            w.left as f64 / scale,
            w.top as f64 / scale,
            (w.right - w.left) as f64 / scale,
            (w.bottom - w.top) as f64 / scale,
            scale,
        )
    }
}

// MARK: - 非 Windows（macOS / Linux）：走 Tauri 的 monitor API

/// 从 Tauri 窗口取「光标所在显示器」的工作区。
///
/// **为什么必须用窗口而不是继续硬编码**：原先这里返回 `(0, 0, 1440, 900)`，
/// 于是任何非 1440×900 的屏幕上灵动岛都贴错边；且 `work_area_at` 直接忽略坐标，
/// 多显示器场景（含不同 DPI）必然算错。这是 [ADR 0010](../docs/adr/0010-swift-freeze-and-rust-prerequisites.md)
/// 里 M2 的「placement.rs 按 OS 真工作区」。
///
/// 底层 `Monitor::work_area()` 在 macOS 用的是 `NSScreen.visibleFrame()`
/// （`tauri-runtime-wry/src/monitor/macos.rs`，已实测读码确认：`visibleFrame` 与
/// `frame` 的差就是 Dock 与菜单栏），所以拿到的**天然是工作区而非全屏近似**，
/// 不会出现灵动岛被 Dock 压住。
///
/// 两个签名上的坑（编译器教的，别再改回去）：
/// ① `Monitor::work_area()` 返回 `PhysicalRect<i32,u32>`——**物理像素**，
///    而 `set_position` 吃逻辑坐标，所以必须除 scale；
/// ② `monitor_from_point(x, y)` 吃的是**逻辑 f64** 坐标，不是物理 i32。
///    传物理值在 2x 屏上会点到另一块屏。
#[cfg(not(windows))]
pub fn work_area_under_window(window: &tauri::WebviewWindow) -> (f64, f64, f64, f64, f64) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let s = if scale > 0.0 { scale } else { 1.0 };
    // 不引入全局鼠标钩子：拿「窗口自身的中心」当参考点。对贴边这个用途更稳——
    // 窗口此刻就在目标屏上，而光标可能停在另一块屏。
    let center = window
        .outer_position()
        .ok()
        .zip(window.outer_size().ok())
        .map(|(p, sz)| {
            (
                (p.x as f64 + sz.width as f64 / 2.0) / s,
                (p.y as f64 + sz.height as f64 / 2.0) / s,
            )
        });
    let monitor = match center {
        Some((lx, ly)) => window
            .monitor_from_point(lx, ly)
            .ok()
            .flatten()
            .or_else(|| window.primary_monitor().ok().flatten()),
        None => window.primary_monitor().ok().flatten(),
    };
    match monitor {
        Some(m) => physical_rect_to_logical(m.work_area(), s),
        None => fallback_work_area(),
    }
}

/// 按用户选的**屏幕跟随模式**取工作区（Swift `ScreenFollowMode` 同名四档）。
///
/// 四档的取舍是「用户想让它待在哪」：
/// · `followMouse`（默认）跟着光标走——多屏用户最常见的诉求
/// · `mainScreen` 固定主屏（带菜单栏的那块）
/// · `builtInScreen` 优先笔记本内置屏（外接时别把它甩到副屏上）
/// · `externalScreen` 优先外接屏
///
/// **认不出的值按 `followMouse` 走**，与 `Settings::normalized()` 的回落一致。
/// 取不到目标屏时**逐级回落**（目标屏 → 主屏 → 兜底工作区），
/// 宁可落在一块能用的屏上，也不要算出落在屏幕之外的坐标——
/// 那会让窗口整个消失，而界面上没有任何提示。
pub fn work_area_for_mode(
    window: &tauri::WebviewWindow,
    mode: &str,
) -> (f64, f64, f64, f64, f64) {
    match mode {
        "mainScreen" => window
            .primary_monitor()
            .ok()
            .flatten()
            .map(|m| {
                let scale = window.scale_factor().unwrap_or(1.0);
                physical_rect_to_logical(m.work_area(), if scale > 0.0 { scale } else { 1.0 })
            })
            .unwrap_or_else(fallback_work_area),
        "builtInScreen" | "externalScreen" => {
            let want_builtin = mode == "builtInScreen";
            match named_monitor(window, want_builtin) {
                Some(m) => {
                    let scale = window.scale_factor().unwrap_or(1.0);
                    physical_rect_to_logical(m.work_area(), if scale > 0.0 { scale } else { 1.0 })
                }
                // 这一档的目标屏现在没插着：退回跟随光标，而不是落在主屏上假装是内置屏
                None => work_area_under_window(window),
            }
        }
        // `followMouse` 与一切认不出的值
        _ => work_area_under_window(window),
    }
}

/// 找内置屏 / 外接屏。判据是**工作区原点**：`x=0 且 y=0` 的是主屏，
/// 而 macOS 的内置屏在多屏布局里通常不是原点那块（外接屏挂在左边或上边时更常见）。
/// 这不是完美判据（外接屏被摆到主位时也会命中），但它不依赖任何平台专有 API，
/// 且拿不到时的回落是安全的。
fn named_monitor(window: &tauri::WebviewWindow, want_builtin: bool) -> Option<tauri::Monitor> {
    let monitors = window.available_monitors().ok()?;
    let primary = window.primary_monitor().ok().flatten();
    let primary_origin = primary.as_ref().map(|m| (m.position().x, m.position().y));
    monitors.into_iter().find(|m| {
        let is_primary = primary_origin == Some((m.position().x, m.position().y));
        // 主屏当内置屏用：它就是这块机器上最「本体」的那块
        if want_builtin {
            is_primary
        } else {
            !is_primary
        }
    })
}

/// 任意逻辑坐标所在的显示器工作区。多显示器 + 不同 DPI 时按坐标选屏，
/// 不能一律取主屏——否则副屏上的贴边会算到主屏的坐标空间里。
#[cfg(not(windows))]
pub fn work_area_at_logical(window: &tauri::WebviewWindow, x: f64, y: f64) -> (f64, f64, f64, f64, f64) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let s = if scale > 0.0 { scale } else { 1.0 };
    let monitor = window
        .monitor_from_point(x, y)
        .ok()
        .flatten()
        .or_else(|| window.primary_monitor().ok().flatten());
    match monitor {
        Some(m) => physical_rect_to_logical(m.work_area(), s),
        None => fallback_work_area(),
    }
}

/// `PhysicalRect<i32,u32>` → 逻辑坐标五元组：物理像素除 scale。
#[cfg(not(windows))]
fn physical_rect_to_logical(
    rect: &tauri::PhysicalRect<i32, u32>,
    scale: f64,
) -> (f64, f64, f64, f64, f64) {
    let s = if scale > 0.0 { scale } else { 1.0 };
    (
        rect.position.x as f64 / s,
        rect.position.y as f64 / s,
        rect.size.width as f64 / s,
        rect.size.height as f64 / s,
        s,
    )
}

/// 最后的兜底：取不到任何显示器时给 0×0，

/// `Monitor::work_area()` 返回 `Rect { position: Position, size: Size }`，
/// 其中 `Position`/`Size` 是带 Physical/Logical 两种单位的枚举
/// （`tauri-runtime/src/dpi.rs:10-15`）。Tauri 的 macOS 实现填的是物理像素，
/// 所以这里**按枚举分支决定要不要除 scale**，而不是无脑除——除错一次就是整个贴边偏一半屏幕。
/// 最后的兜底：取不到任何显示器时给 0×0，
/// 而不是「看起来像真的」的 1440×900——假尺寸会让贴边静默错位，
/// 而 0×0 至少会在 clamp 里露出来。
#[cfg(not(windows))]
fn fallback_work_area() -> (f64, f64, f64, f64, f64) {
    (0.0, 0.0, 0.0, 0.0, 1.0)
}

/// 侧边栏宽度的上下界。下界是「还能读一列信息」，上界是「别盖住整块屏」
pub const MIN_SIDEBAR_WIDTH: f64 = 280.0;
pub const MAX_SIDEBAR_WIDTH: f64 = 720.0;
/// 420 而不是 360：360 减去 132 的导航只剩 228px，档位页那类表单会挤成一条
/// （截图上量出来的）。宽度可拉且**记忆**，所以这个值只决定第一次打开的样子。
pub const DEFAULT_SIDEBAR_WIDTH: f64 = 420.0;

/// 把侧边栏宽度钳进可用区间。
///
/// 三个非正常值各有明确去处：NaN 回落默认值（NaN 的比较全为假，`clamp` 拿它没办法），
/// 正负无穷按**取到边界**处理（`+inf` 的语义是「要多宽有多宽」⇒ 上界，
/// 与 `clamp` 的直觉一致，而不是悄悄变成默认值）。
pub fn clamp_sidebar_width(width: f64) -> f64 {
    if width.is_nan() {
        return DEFAULT_SIDEBAR_WIDTH;
    }
    if width.is_infinite() {
        return if width > 0.0 {
            MAX_SIDEBAR_WIDTH
        } else {
            MIN_SIDEBAR_WIDTH
        };
    }
    width.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH)
}

/// 侧边栏的几何：贴左或贴右、**铺满工作区高度**、宽度取记忆值。
///
/// 与细条那套「锚点 × 工作区」不是一回事：侧边栏不是浮在中间的卡片，而是一整列，
/// 所以它不用锚点、也不需要垂直居中。`Top`/`Bottom` 传进来按**右侧**处理——
/// 侧边栏没有上下两档，与其静默给一个奇怪的几何，不如按明确的一条回落。
///
/// 返回 `(left, top, width, height)`，逻辑坐标。
pub fn sidebar_frame(edge: DockEdge, width: f64, wa: (f64, f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    let (wx, wy, ww, wh, _scale) = wa;
    // 宽度既受记忆值也受工作区约束：窗口比工作区宽会让它有一截永远在屏幕外。
    // 工作区比下界还窄时**以工作区为准**（此时侧边栏铺满宽度）——
    // 下界是「还能读一列信息」的期望值，不是可以无视屏幕的许可
    let width = clamp_sidebar_width(width).min(ww);
    let left = match edge {
        DockEdge::Left => wx,
        // 右侧，以及上下两档的回落
        _ => wx + ww - width,
    };
    (left, wy, width, wh)
}

/// 统一锚点模型（与 Windows 端 IslandWindow.cs 同源）：
/// 细条中心 = 卡片中心 = anchor × 工作区（水平边沿 X，垂直边沿 Y）。
/// 返回窗口应放置的逻辑左上角。
///
/// `wa` 是 `(工作区 x, y, w, h, scale)`。**width/height 也是逻辑坐标**，
/// 与 `wa` 同口径，不要在这里混物理像素。
pub fn place_with(edge: DockEdge, anchor: f64, width: f64, height: f64, wa: (f64, f64, f64, f64, f64)) -> (f64, f64) {
    let (wx, wy, ww, wh, _s) = wa;
    let (mut left, mut top);
    match edge {
        DockEdge::Top => {
            left = wx + anchor * ww - width / 2.0;
            top = wy;
        }
        DockEdge::Bottom => {
            left = wx + anchor * ww - width / 2.0;
            top = wy + wh - height;
        }
        DockEdge::Left => {
            left = wx;
            top = wy + anchor * wh - height / 2.0;
        }
        DockEdge::Right => {
            left = wx + ww - width;
            top = wy + anchor * wh - height / 2.0;
        }
    }
    if ww > width {
        left = left.clamp(wx, wx + ww - width);
    } else {
        // 窗口比工作区还宽：无法完整放进工作区。此时仍要钳回左边界，
        // 否则居中算式会给出负值，窗口有一截飞出屏幕外——**用户可能再也拖不回来**。
        // 宁可让它从左边开始溢出，也不让它凭空消失。
        left = left.max(wx);
    }
    if wh > height {
        top = top.clamp(wy, wy + wh - height);
    } else {
        top = top.max(wy);
    }
    (left, top)
}

#[cfg(test)]
mod sidebar_tests {
    use super::*;

    /// 一块 1440×900、scale 2 的工作区
    fn wa() -> (f64, f64, f64, f64, f64) {
        (0.0, 0.0, 1440.0, 900.0, 2.0)
    }

    #[test]
    fn the_sidebar_spans_the_work_area_height_on_whichever_side_it_is_docked() {
        let (left, top, w, h) = sidebar_frame(DockEdge::Right, 360.0, wa());
        assert_eq!((left, top, w, h), (1080.0, 0.0, 360.0, 900.0));

        let (left, top, w, h) = sidebar_frame(DockEdge::Left, 360.0, wa());
        assert_eq!((left, top, w, h), (0.0, 0.0, 360.0, 900.0));

        // 工作区原点不在 (0,0) 时也要贴着它（多显示器、菜单栏留空都走这条）
        let wa2 = (100.0, 50.0, 1000.0, 700.0, 1.0);
        let (left, top, w, h) = sidebar_frame(DockEdge::Right, 300.0, wa2);
        assert_eq!((left, top, w, h), (800.0, 50.0, 300.0, 700.0));
    }

    #[test]
    fn top_and_bottom_fall_back_to_the_right_side_instead_of_producing_odd_geometry() {
        // 侧边栏没有上下两档：静默给一个居中或越界的几何比明确回落更难查
        let right = sidebar_frame(DockEdge::Right, 360.0, wa());
        assert_eq!(sidebar_frame(DockEdge::Top, 360.0, wa()), right);
        assert_eq!(sidebar_frame(DockEdge::Bottom, 360.0, wa()), right);
    }

    #[test]
    fn the_width_is_clamped_and_never_wider_than_the_work_area() {
        // 记忆值超出上下界 ⇒ 钳回区间
        assert_eq!(sidebar_frame(DockEdge::Right, 5_000.0, wa()).2, MAX_SIDEBAR_WIDTH);
        assert_eq!(sidebar_frame(DockEdge::Right, 10.0, wa()).2, MIN_SIDEBAR_WIDTH);

        // 工作区比下界还窄（小屏/分屏）：宽度以工作区为上限，
        // 否则窗口会有一截永远在屏幕外，用户再也拖不回来
        let narrow = (0.0, 0.0, 220.0, 600.0, 1.0);
        let (left, _, w, _) = sidebar_frame(DockEdge::Right, 360.0, narrow);
        assert_eq!(w, 220.0, "不该比工作区还宽");
        assert_eq!(left, 0.0, "右边贴着工作区右缘（此时等于铺满）");
        let (left, _, w, _) = sidebar_frame(DockEdge::Left, 360.0, narrow);
        assert_eq!((left, w), (0.0, 220.0));
    }

    #[test]
    fn the_width_clamp_handles_nan_and_infinities_explicitly() {
        assert_eq!(clamp_sidebar_width(360.0), 360.0);
        assert_eq!(clamp_sidebar_width(0.0), MIN_SIDEBAR_WIDTH);
        assert_eq!(clamp_sidebar_width(-1.0), MIN_SIDEBAR_WIDTH);
        assert_eq!(clamp_sidebar_width(9_999.0), MAX_SIDEBAR_WIDTH);
        assert_eq!(clamp_sidebar_width(f64::NAN), DEFAULT_SIDEBAR_WIDTH);
        assert_eq!(clamp_sidebar_width(f64::INFINITY), MAX_SIDEBAR_WIDTH);
        assert_eq!(clamp_sidebar_width(f64::NEG_INFINITY), MIN_SIDEBAR_WIDTH);
    }

    #[test]
    fn the_default_width_is_inside_the_allowed_range() {
        // 默认值漂出区间的话，第一次打开侧边栏就会被「归一化」成别的宽度
        assert!(DEFAULT_SIDEBAR_WIDTH >= MIN_SIDEBAR_WIDTH);
        assert!(DEFAULT_SIDEBAR_WIDTH <= MAX_SIDEBAR_WIDTH);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WA: (f64, f64, f64, f64, f64) = (100.0, 40.0, 1600.0, 1000.0, 2.0);

    /// 四边各算一次，锚点 0.5 时窗口应居中于该边。
    /// 工作区原点不是 (0,0)（模拟有 Dock 的桌面），专治「忘了加 wx/wy」。
    #[test]
    fn place_with_centers_on_each_edge() {
        let (w, h) = (300.0, 400.0);

        let (l, t) = place_with(DockEdge::Top, 0.5, w, h, WA);
        assert!((l - (WA.0 + WA.2 / 2.0 - w / 2.0)).abs() < 1e-6, "top 水平未居中: {l}");
        assert!((t - WA.1).abs() < 1e-6, "top 未贴工作区上沿: {t}");

        let (l, t) = place_with(DockEdge::Bottom, 0.5, w, h, WA);
        assert!((t - (WA.1 + WA.3 - h)).abs() < 1e-6, "bottom 未贴工作区下沿: {t}");

        let (l, t) = place_with(DockEdge::Left, 0.5, w, h, WA);
        assert!((l - WA.0).abs() < 1e-6, "left 未贴工作区左沿: {l}");
        assert!((t - (WA.1 + WA.3 / 2.0 - h / 2.0)).abs() < 1e-6, "left 垂直未居中: {t}");

        let (l, t) = place_with(DockEdge::Right, 0.5, w, h, WA);
        assert!((l - (WA.0 + WA.2 - w)).abs() < 1e-6, "right 未贴工作区右沿: {l}");
    }

    /// 锚点 0/1 与 0.5 必须真的不同，否则「拖拽换位置」会被静默钳成居中。
    #[test]
    fn anchor_moves_along_edge() {
        let (w, h) = (300.0, 400.0);
        let a = place_with(DockEdge::Top, 0.0, w, h, WA).0;
        let b = place_with(DockEdge::Top, 0.5, w, h, WA).0;
        let c = place_with(DockEdge::Top, 1.0, w, h, WA).0;
        assert!(a < b && b < c, "锚点未生效: {a} {b} {c}");
    }

    /// 窗口比工作区还大时：clamp 会让 left/top 退到工作区原点，
    /// 且**不能出现 NaN**（`ww > width` 不成立时若仍 clamp 会得到 NaN）。
    #[test]
    fn oversized_window_clamps_to_origin_without_nan() {
        let (l, t) = place_with(DockEdge::Top, 0.5, 5000.0, 5000.0, WA);
        assert!(l.is_finite() && t.is_finite(), "出现非有限值: {l} {t}");
        assert!((l - WA.0).abs() < 1e-6, "超大窗口未退到工作区左边界: {l}");
        assert!((t - WA.1).abs() < 1e-6, "超大窗口未退到工作上边界: {t}");
    }

    /// 反向锚点（用户可能把 dock_anchor 写成 -0.2 之类）必须被 clamp 进工作区，
    /// 否则窗口飞到屏幕外，用户再也拖不回来。
    #[test]
    fn out_of_range_anchor_stays_inside_work_area() {
        let (w, h) = (300.0, 400.0);
        for anchor in [-5.0, -0.1, 1.1, 9.0] {
            let (l, t) = place_with(DockEdge::Top, anchor, w, h, WA);
            assert!(l >= WA.0 - 1e-6 && l <= WA.0 + WA.2 - w + 1e-6, "锚点 {anchor} 越界: {l}");
            assert!(t >= WA.1 - 1e-6 && t <= WA.1 + WA.3 - h + 1e-6, "锚点 {anchor} 越界: {t}");
        }
    }

    /// 兜底工作区是 0×0：此时 clamp 不再生效（`ww > width` 为假），
    /// 位置应稳定在工作区原点而不是 NaN——这正是 `place_with` 敢用 0×0 兜底的原因。
    #[test]
    fn zero_sized_fallback_is_not_nan() {
        let (l, t) = place_with(DockEdge::Bottom, 0.5, 300.0, 400.0, (0.0, 0.0, 0.0, 0.0, 1.0));
        assert!(l.is_finite() && t.is_finite(), "0×0 兜底出现非有限值: {l} {t}");
    }
}
