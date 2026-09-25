use crate::models::DockEdge;

/// 屏幕工作区（任务栏以外）+ DPI 缩放。
/// Win32 返回物理像素；Tauri set_position/set_size 用逻辑坐标（DIP），必须换算。
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

#[cfg(not(windows))]
pub fn work_area_under_cursor() -> (f64, f64, f64, f64, f64) {
    // macOS/Linux 由各自的实现补齐；先用全屏近似
    let (w, h) = (1440.0, 900.0);
    (0.0, 0.0, w, h, 1.0)
}

#[cfg(not(windows))]
pub fn work_area_at(_x: i32, _y: i32) -> (f64, f64, f64, f64, f64) {
    work_area_under_cursor()
}

/// 统一锚点模型（与 Windows 端 IslandWindow.cs 同源）：
/// 细条中心 = 卡片中心 = anchor × 工作区（水平边沿 X，垂直边沿 Y）。
/// 返回窗口应放置的逻辑左上角。
pub fn place(edge: DockEdge, anchor: f64, width: f64, height: f64) -> (f64, f64) {
    let (wx, wy, ww, wh, _s) = work_area_under_cursor();
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
    }
    if wh > height {
        top = top.clamp(wy, wy + wh - height);
    }
    (left, top)
}
