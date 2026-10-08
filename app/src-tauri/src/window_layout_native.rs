//! macOS AX adapter. Enumeration never prompts or mutates; writes require explicit IPC.
use crate::window_layout::{DisplayArea, WindowCandidate};
use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct Capabilities {
    pub supported: bool,
    pub permission_granted: bool,
    pub reason: Option<String>,
}

pub fn capabilities() -> Capabilities {
    #[cfg(target_os = "macos")]
    {
        let trusted = mac::trusted();
        Capabilities {
            supported: true,
            permission_granted: trusted,
            reason: (!trusted).then(|| "需要辅助功能权限，用于调整你选择的工具窗口".into()),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Capabilities {
            supported: false,
            permission_granted: false,
            reason: Some("当前平台尚未接入窗口排列".into()),
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac::{displays, enumerate, NativeWindow};

#[cfg(not(target_os = "macos"))]
#[derive(Clone)]
pub struct NativeWindow {
    pub candidate: WindowCandidate,
}
#[cfg(not(target_os = "macos"))]
impl crate::window_layout_execution::Handle for NativeWindow {
    fn inspect(&self) -> Result<WindowCandidate, crate::window_layout_execution::InspectError> {
        Err(crate::window_layout_execution::InspectError::Unavailable(
            "当前平台尚未接入窗口排列".into(),
        ))
    }
    fn write(&self, _: crate::window_layout::Rect) -> crate::window_layout_execution::WriteResult {
        crate::window_layout_execution::WriteResult {
            actual: None,
            error: Some("当前平台尚未接入窗口排列".into()),
        }
    }
}
#[cfg(not(target_os = "macos"))]
pub fn displays() -> Result<Vec<DisplayArea>, String> {
    Err("当前平台尚未接入窗口排列".into())
}
#[cfg(not(target_os = "macos"))]
pub struct Probe {
    pub windows: Vec<NativeWindow>,
    pub warnings: Vec<String>,
}
#[cfg(not(target_os = "macos"))]
pub fn enumerate() -> Result<Probe, String> {
    Err("当前平台尚未接入窗口排列".into())
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use crate::window_layout::Rect;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSRunningApplication, NSScreen};
    use objc2_foundation::NSString;
    use sha2::{Digest, Sha256};
    use std::{
        ffi::c_void,
        time::{Duration, Instant},
    };
    type Ref = *const c_void;
    #[repr(C)]
    #[derive(Default)]
    struct Pair {
        a: f64,
        b: f64,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Bounds {
        origin: Pair,
        size: Pair,
    }
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> u8;
        fn AXUIElementCreateApplication(pid: i32) -> Ref;
        fn AXUIElementGetTypeID() -> usize;
        fn AXUIElementCopyAttributeValue(element: Ref, attribute: Ref, value: *mut Ref) -> i32;
        fn AXUIElementCopyAttributeValues(
            element: Ref,
            attribute: Ref,
            index: isize,
            count: isize,
            value: *mut Ref,
        ) -> i32;
        fn AXUIElementGetAttributeValueCount(
            element: Ref,
            attribute: Ref,
            count: *mut isize,
        ) -> i32;
        fn AXUIElementIsAttributeSettable(element: Ref, attribute: Ref, value: *mut u8) -> i32;
        fn AXUIElementSetMessagingTimeout(element: Ref, timeout: f32) -> i32;
        fn AXValueGetTypeID() -> usize;
        fn AXValueGetValue(value: Ref, kind: u32, out: *mut c_void) -> u8;
        fn AXValueCreate(kind: u32, value: *const c_void) -> Ref;
        fn AXUIElementSetAttributeValue(element: Ref, attribute: Ref, value: Ref) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRetain(value: Ref) -> Ref;
        fn CFRelease(value: Ref);
        fn CFGetTypeID(value: Ref) -> usize;
        fn CFStringCreateWithCString(alloc: Ref, text: *const i8, encoding: u32) -> Ref;
        fn CFStringGetTypeID() -> usize;
        fn CFStringGetCString(value: Ref, buffer: *mut i8, length: isize, encoding: u32) -> u8;
        fn CFArrayGetTypeID() -> usize;
        fn CFArrayGetCount(value: Ref) -> isize;
        fn CFArrayGetValueAtIndex(value: Ref, index: isize) -> Ref;
        fn CFBooleanGetTypeID() -> usize;
        fn CFBooleanGetValue(value: Ref) -> u8;
        fn CFEqual(a: Ref, b: Ref) -> u8;
        fn CFDictionaryGetTypeID() -> usize;
        fn CFDictionaryGetValue(dict: Ref, key: Ref) -> Ref;
        fn CFNumberGetTypeID() -> usize;
        fn CFNumberGetValue(number: Ref, kind: isize, value: *mut c_void) -> u8;
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(options: u32, relative: u32) -> Ref;
        fn CGRectMakeWithDictionaryRepresentation(dict: Ref, rect: *mut Bounds) -> u8;
        static kCGWindowOwnerPID: Ref;
        static kCGWindowNumber: Ref;
        static kCGWindowLayer: Ref;
        static kCGWindowBounds: Ref;
    }
    // CF ownership travels through integer storage. AX reads occur on the probe worker;
    // only immutable retained references are transferred, never borrowed array elements.
    struct Owned(usize);
    impl Clone for Owned {
        fn clone(&self) -> Self {
            Self(unsafe { CFRetain(self.ptr()) } as usize)
        }
    }
    impl Owned {
        fn take(value: Ref) -> Option<Self> {
            (!value.is_null()).then_some(Self(value as usize))
        }
        fn ptr(&self) -> Ref {
            self.0 as Ref
        }
        fn attribute(&self, name: &str) -> Result<Self, i32> {
            let key = string(name);
            let mut value = std::ptr::null();
            let code = unsafe { AXUIElementCopyAttributeValue(self.ptr(), key.ptr(), &mut value) };
            if code != 0 {
                if !value.is_null() {
                    unsafe { CFRelease(value) };
                }
                return Err(code);
            }
            Self::take(value).ok_or(-25212)
        }
        fn settable(&self, name: &str) -> Result<bool, i32> {
            let key = string(name);
            let mut yes = 0;
            let code = unsafe { AXUIElementIsAttributeSettable(self.ptr(), key.ptr(), &mut yes) };
            if code == 0 {
                Ok(yes != 0)
            } else {
                Err(code)
            }
        }
    }
    impl Drop for Owned {
        fn drop(&mut self) {
            unsafe { CFRelease(self.ptr()) };
        }
    }
    fn string(text: &str) -> Owned {
        let text = std::ffi::CString::new(text).expect("static AX attribute");
        Owned::take(unsafe {
            CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), 0x08000100)
        })
        .expect("CFString allocation")
    }
    fn text(value: &Owned) -> Option<String> {
        if unsafe { CFGetTypeID(value.ptr()) != CFStringGetTypeID() } {
            return None;
        }
        let mut buffer = [0u8; 2048];
        if unsafe {
            CFStringGetCString(
                value.ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len() as isize,
                0x08000100,
            )
        } == 0
        {
            return None;
        }
        std::ffi::CStr::from_bytes_until_nul(&buffer)
            .ok()?
            .to_str()
            .ok()
            .map(|s| s.chars().take(160).collect())
    }
    fn boolean(value: &Owned) -> Option<bool> {
        (unsafe { CFGetTypeID(value.ptr()) == CFBooleanGetTypeID() })
            .then(|| unsafe { CFBooleanGetValue(value.ptr()) != 0 })
    }
    fn pair(value: &Owned, kind: u32) -> Option<Pair> {
        if unsafe { CFGetTypeID(value.ptr()) != AXValueGetTypeID() } {
            return None;
        }
        let mut p = Pair::default();
        (unsafe { AXValueGetValue(value.ptr(), kind, (&mut p as *mut Pair).cast()) } != 0)
            .then_some(p)
    }
    pub fn trusted() -> bool {
        unsafe { AXIsProcessTrusted() != 0 }
    }
    pub fn displays() -> Result<Vec<DisplayArea>, String> {
        let mtm = MainThreadMarker::new().ok_or("屏幕读取必须在主线程执行")?;
        let screens = NSScreen::screens(mtm);
        let first = screens.firstObject().ok_or("没有可用屏幕")?;
        let frame = first.frame();
        let top = frame.origin.y + frame.size.height;
        Ok(screens
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let visible = s.visibleFrame();
                let frame = s.frame();
                let key = format!(
                    "{i}:{}:{}:{}:{}:{}",
                    frame.origin.x,
                    frame.origin.y,
                    frame.size.width,
                    frame.size.height,
                    s.backingScaleFactor()
                );
                DisplayArea {
                    screen_id: format!("{:x}", Sha256::digest(key.as_bytes())),
                    rect: Rect {
                        x: visible.origin.x,
                        y: top - visible.origin.y - visible.size.height,
                        width: visible.size.width,
                        height: visible.size.height,
                    },
                    scale: s.backingScaleFactor(),
                }
            })
            .collect())
    }
    #[derive(Clone)]
    pub struct NativeWindow {
        pub candidate: WindowCandidate,
        // Retained AX object, PID and launch identity remain private to the server.
        element: Owned,
        pid: i32,
        launched: u64,
        bundle: String,
        cg_id: Option<u32>,
    }
    pub struct Probe {
        pub windows: Vec<NativeWindow>,
        pub warnings: Vec<String>,
    }
    pub fn enumerate() -> Result<Probe, String> {
        objc2::rc::autoreleasepool(|_| enumerate_pooled())
    }
    fn enumerate_pooled() -> Result<Probe, String> {
        if !trusted() {
            return Err("请先允许辅助功能权限，再读取工具窗口".into());
        }
        let mut result = Probe {
            windows: Vec::new(),
            warnings: Vec::new(),
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut seen = std::collections::HashSet::new();
        for profile in crate::registry::builtin() {
            for bundle in &profile.bundle_ids {
                for app in NSRunningApplication::runningApplicationsWithBundleIdentifier(
                    &NSString::from_str(bundle),
                )
                .iter()
                {
                    if Instant::now() >= deadline {
                        result.warnings.push("部分工具读取超时，可刷新重试".into());
                        return Ok(result);
                    }
                    let pid = app.processIdentifier();
                    if !seen.insert(pid) || app.isTerminated() {
                        continue;
                    }
                    let Some(launched) = app
                        .launchDate()
                        .map(|d| d.timeIntervalSince1970().to_bits())
                    else {
                        result
                            .warnings
                            .push(format!("{}：无法核实启动身份", profile.name));
                        continue;
                    };
                    let Some(root) = Owned::take(unsafe { AXUIElementCreateApplication(pid) })
                    else {
                        continue;
                    };
                    if unsafe { AXUIElementSetMessagingTimeout(root.ptr(), 0.15) } != 0 {
                        result
                            .warnings
                            .push(format!("{}：无法设置读取超时", profile.name));
                        continue;
                    }
                    let key = string("AXWindows");
                    let mut array = std::ptr::null();
                    let mut count = 0;
                    let code = unsafe {
                        AXUIElementGetAttributeValueCount(root.ptr(), key.ptr(), &mut count)
                    };
                    if code != 0 {
                        result
                            .warnings
                            .push(format!("{}：{}", profile.name, error_reason(code)));
                        continue;
                    }
                    if count <= 0 {
                        continue;
                    }
                    if count > 32 {
                        result
                            .warnings
                            .push(format!("{}：仅读取前 32 个窗口", profile.name));
                    }
                    let code = unsafe {
                        AXUIElementCopyAttributeValues(
                            root.ptr(),
                            key.ptr(),
                            0,
                            count.min(32),
                            &mut array,
                        )
                    };
                    let array = Owned::take(array);
                    if code != 0 {
                        result
                            .warnings
                            .push(format!("{}：{}", profile.name, error_reason(code)));
                        continue;
                    }
                    let Some(array) =
                        array.filter(|v| unsafe { CFGetTypeID(v.ptr()) == CFArrayGetTypeID() })
                    else {
                        result
                            .warnings
                            .push(format!("{}：窗口列表不可读", profile.name));
                        continue;
                    };
                    for i in 0..unsafe { CFArrayGetCount(array.ptr()) }.min(32) {
                        if Instant::now() >= deadline {
                            result.warnings.push("部分窗口读取超时，可刷新重试".into());
                            return Ok(result);
                        }
                        let raw = unsafe { CFArrayGetValueAtIndex(array.ptr(), i) };
                        if raw.is_null() || unsafe { CFGetTypeID(raw) != AXUIElementGetTypeID() } {
                            continue;
                        }
                        let element = Owned::take(unsafe { CFRetain(raw) }).unwrap();
                        if unsafe { AXUIElementSetMessagingTimeout(element.ptr(), 0.15) } != 0 {
                            result
                                .warnings
                                .push(format!("{}：无法设置窗口读取超时", profile.name));
                            continue;
                        }
                        let Some(rect) = rect(&element) else {
                            result
                                .warnings
                                .push(format!("{}：窗口位置或尺寸不可读", profile.name));
                            continue;
                        };
                        let movable = element.settable("AXPosition");
                        let resizable = element.settable("AXSize");
                        let minimized = element
                            .attribute("AXMinimized")
                            .ok()
                            .as_ref()
                            .and_then(boolean);
                        let fullscreen = element
                            .attribute("AXFullScreen")
                            .ok()
                            .as_ref()
                            .and_then(boolean);
                        let restriction = if minimized == Some(true) {
                            Some("窗口已最小化".into())
                        } else if fullscreen == Some(true) {
                            Some("全屏窗口".into())
                        } else if minimized.is_none() || fullscreen.is_none() {
                            Some("无法核实窗口最小化或全屏状态".into())
                        } else if movable.is_err() || resizable.is_err() {
                            Some("无法核实窗口调整能力".into())
                        } else {
                            None
                        };
                        let candidate = WindowCandidate {
                            window_id: uuid::Uuid::new_v4().to_string(),
                            agent_id: profile.id.clone(),
                            application: profile.name.clone(),
                            title: element
                                .attribute("AXTitle")
                                .ok()
                                .as_ref()
                                .and_then(text)
                                .unwrap_or_else(|| "未命名窗口".into()),
                            screen_id: None,
                            rect,
                            movable: movable.unwrap_or(false),
                            resizable: resizable.unwrap_or(false),
                            restriction,
                            minimum_size: None,
                        };
                        let mut window = NativeWindow {
                            candidate,
                            element,
                            pid,
                            launched,
                            bundle: bundle.clone(),
                            cg_id: None,
                        };
                        match window.visible_identity() {
                            Ok(id) => window.cg_id = Some(id),
                            Err(reason) => {
                                if window.candidate.restriction.is_none() {
                                    window.candidate.restriction = Some(reason);
                                }
                            }
                        }
                        result.windows.push(window);
                    }
                }
            }
        }
        Ok(result)
    }
    fn rect(element: &Owned) -> Option<Rect> {
        let p = pair(&element.attribute("AXPosition").ok()?, 1)?;
        let s = pair(&element.attribute("AXSize").ok()?, 2)?;
        let rect = Rect {
            x: p.a,
            y: p.b,
            width: s.a,
            height: s.b,
        };
        rect.valid().then_some(rect)
    }

    fn windows_for(pid: i32) -> Result<Owned, String> {
        let root =
            Owned::take(unsafe { AXUIElementCreateApplication(pid) }).ok_or("无法读取工具窗口")?;
        if unsafe { AXUIElementSetMessagingTimeout(root.ptr(), 0.15) } != 0 {
            return Err("无法设置读取超时".into());
        }
        let key = string("AXWindows");
        let mut count = 0;
        let code = unsafe { AXUIElementGetAttributeValueCount(root.ptr(), key.ptr(), &mut count) };
        if code != 0 {
            return Err(error_reason(code).into());
        }
        if !(1..=128).contains(&count) {
            return Err("窗口列表为空或过多，无法核实身份".into());
        }
        let mut array = std::ptr::null();
        let code =
            unsafe { AXUIElementCopyAttributeValues(root.ptr(), key.ptr(), 0, count, &mut array) };
        let array = Owned::take(array);
        if code != 0 {
            return Err(error_reason(code).into());
        }
        array
            .filter(|v| unsafe { CFGetTypeID(v.ptr()) == CFArrayGetTypeID() })
            .ok_or_else(|| "窗口列表不可读".into())
    }
    fn number(dict: Ref, key: Ref) -> Option<i32> {
        let value = unsafe { CFDictionaryGetValue(dict, key) };
        if value.is_null() || unsafe { CFGetTypeID(value) != CFNumberGetTypeID() } {
            return None;
        }
        let mut n = 0i32;
        (unsafe { CFNumberGetValue(value, 3, (&mut n as *mut i32).cast()) } != 0).then_some(n)
    }
    fn visible_windows() -> Result<Vec<(i32, u32, Rect)>, String> {
        // Metadata only: no names, screenshots or deprecated workspace IDs.
        let list = Owned::take(unsafe { CGWindowListCopyWindowInfo(1 | 16, 0) })
            .ok_or("当前桌面窗口列表不可用")?;
        if unsafe { CFGetTypeID(list.ptr()) != CFArrayGetTypeID() } {
            return Err("当前桌面窗口列表不可读".into());
        }
        let count = unsafe { CFArrayGetCount(list.ptr()) };
        if count > 4096 {
            return Err("当前桌面窗口过多，无法核实可见性".into());
        }
        let mut result = Vec::new();
        for i in 0..count {
            let dict = unsafe { CFArrayGetValueAtIndex(list.ptr(), i) };
            if dict.is_null() || unsafe { CFGetTypeID(dict) != CFDictionaryGetTypeID() } {
                continue;
            }
            if number(dict, unsafe { kCGWindowLayer }) != Some(0) {
                continue;
            }
            let Some(pid) = number(dict, unsafe { kCGWindowOwnerPID }) else {
                continue;
            };
            let Some(id) = number(dict, unsafe { kCGWindowNumber }).filter(|id| *id > 0) else {
                continue;
            };
            let bounds = unsafe { CFDictionaryGetValue(dict, kCGWindowBounds) };
            if bounds.is_null() || unsafe { CFGetTypeID(bounds) != CFDictionaryGetTypeID() } {
                continue;
            }
            let mut b = Bounds::default();
            if unsafe { CGRectMakeWithDictionaryRepresentation(bounds, &mut b) } == 0 {
                continue;
            }
            let r = Rect {
                x: b.origin.a,
                y: b.origin.b,
                width: b.size.a,
                height: b.size.b,
            };
            if r.valid() {
                result.push((pid, id as u32, r));
            }
        }
        Ok(result)
    }
    impl NativeWindow {
        fn launch_valid(&self) -> Result<(), crate::window_layout_execution::InspectError> {
            use crate::window_layout_execution::InspectError::{Gone, Unavailable};
            if !trusted() {
                return Err(Unavailable("辅助功能权限不可用".into()));
            }
            let app = NSRunningApplication::runningApplicationWithProcessIdentifier(self.pid)
                .ok_or_else(|| Gone("工具已退出".into()))?;
            if app.isTerminated()
                || app
                    .bundleIdentifier()
                    .is_none_or(|b| b.to_string() != self.bundle)
                || app
                    .launchDate()
                    .is_none_or(|d| d.timeIntervalSince1970().to_bits() != self.launched)
            {
                return Err(Gone("工具启动身份已变化".into()));
            }
            if app.isHidden() {
                return Err(Unavailable("工具已隐藏，请先显示窗口".into()));
            }
            Ok(())
        }
        fn visible_identity(&self) -> Result<u32, String> {
            let current = rect(&self.element).ok_or("窗口位置不可读")?;
            let list = windows_for(self.pid)?;
            let mut same_geometry = 0;
            let mut member = false;
            let deadline = Instant::now() + Duration::from_millis(600);
            for i in 0..unsafe { CFArrayGetCount(list.ptr()) } {
                if Instant::now() >= deadline {
                    return Err("窗口身份核实超时，请重试".into());
                }
                let raw = unsafe { CFArrayGetValueAtIndex(list.ptr(), i) };
                if raw.is_null() || unsafe { CFGetTypeID(raw) != AXUIElementGetTypeID() } {
                    continue;
                }
                let window = Owned::take(unsafe { CFRetain(raw) }).unwrap();
                if unsafe { AXUIElementSetMessagingTimeout(window.ptr(), 0.1) } != 0 {
                    return Err("无法设置身份读取超时".into());
                }
                if unsafe { CFEqual(raw, self.element.ptr()) } != 0 {
                    member = true;
                }
                let r = rect(&window).ok_or("无法完整核实工具窗口身份")?;
                if crate::window_layout_execution::matches(r, current) {
                    same_geometry += 1;
                }
            }
            crate::window_visibility::match_id(
                self.pid,
                current,
                member,
                same_geometry,
                &visible_windows()?,
            )
        }
        fn inspect_pooled(
            &self,
        ) -> Result<WindowCandidate, crate::window_layout_execution::InspectError> {
            use crate::window_layout_execution::InspectError::{Gone, Unavailable};
            self.launch_valid()?;
            // InvalidUIElement is terminal; transport failures remain retryable.
            let position = self.element.attribute("AXPosition").map_err(|e| {
                if e == -25202 {
                    Gone("窗口已关闭".into())
                } else {
                    Unavailable(error_reason(e).into())
                }
            })?;
            if pair(&position, 1).is_none() {
                return Err(Unavailable("窗口位置不可读".into()));
            }
            let id = self.visible_identity().map_err(Unavailable)?;
            if self.cg_id != Some(id) {
                return Err(Gone("窗口系统身份已变化，请重新读取".into()));
            }
            let mut candidate = self.candidate.clone();
            candidate.rect =
                rect(&self.element).ok_or_else(|| Unavailable("窗口位置或尺寸不可读".into()))?;
            candidate.movable = self
                .element
                .settable("AXPosition")
                .map_err(|_| Unavailable("无法核实移动能力".into()))?;
            candidate.resizable = self
                .element
                .settable("AXSize")
                .map_err(|_| Unavailable("无法核实缩放能力".into()))?;
            let minimized = self
                .element
                .attribute("AXMinimized")
                .ok()
                .as_ref()
                .and_then(boolean);
            let fullscreen = self
                .element
                .attribute("AXFullScreen")
                .ok()
                .as_ref()
                .and_then(boolean);
            candidate.restriction = match (minimized, fullscreen) {
                (Some(true), _) => Some("窗口已最小化".into()),
                (_, Some(true)) => Some("全屏窗口".into()),
                (None, _) | (_, None) => Some("无法核实窗口最小化或全屏状态".into()),
                _ => None,
            };
            Ok(candidate)
        }
        fn set_pair(&self, attribute: &str, kind: u32, pair: Pair) -> Result<(), String> {
            self.launch_valid()
                .map_err(crate::window_layout_execution::InspectError::reason)?;
            let key = string(attribute);
            let value = Owned::take(unsafe { AXValueCreate(kind, (&pair as *const Pair).cast()) })
                .ok_or("无法创建窗口坐标值")?;
            let code =
                unsafe { AXUIElementSetAttributeValue(self.element.ptr(), key.ptr(), value.ptr()) };
            if code == 0 {
                Ok(())
            } else {
                Err(error_reason(code).into())
            }
        }
        fn settle_geometry(
            &self,
            target: Rect,
            part: crate::window_layout_geometry::Part,
        ) -> crate::window_layout_readback::Observation {
            let start = Instant::now();
            crate::window_layout_readback::settle(
                || {
                    self.launch_valid()
                        .map_err(crate::window_layout_execution::InspectError::reason)?;
                    rect(&self.element).ok_or_else(|| "调整后窗口位置不可读".into())
                },
                |actual| part.accepts(actual, target),
                || start.elapsed(),
                std::thread::sleep,
            )
        }
    }
    impl crate::window_layout_geometry::Driver for NativeWindow {
        fn read(&self) -> Result<Rect, String> {
            self.launch_valid()
                .map_err(crate::window_layout_execution::InspectError::reason)?;
            if self.visible_identity()? != self.cg_id.unwrap_or(0) {
                return Err("窗口可见性或身份已变化".into());
            }
            rect(&self.element).ok_or_else(|| "调整后窗口位置不可读".into())
        }
        fn resize(&self, target: Rect) -> Result<(), String> {
            self.set_pair(
                "AXSize",
                2,
                Pair {
                    a: target.width,
                    b: target.height,
                },
            )
        }
        fn move_to(&self, target: Rect) -> Result<(), String> {
            self.set_pair(
                "AXPosition",
                1,
                Pair {
                    a: target.x,
                    b: target.y,
                },
            )
        }
        fn settle(
            &self,
            target: Rect,
            part: crate::window_layout_geometry::Part,
        ) -> crate::window_layout_readback::Observation {
            self.settle_geometry(target, part)
        }
    }
    impl crate::window_layout_execution::Handle for NativeWindow {
        fn collision_domain(&self) -> Option<crate::window_layout_plan::Domain> {
            Some((self.pid, self.launched))
        }
        fn inspect(&self) -> Result<WindowCandidate, crate::window_layout_execution::InspectError> {
            objc2::rc::autoreleasepool(|_| self.inspect_pooled())
        }
        fn write(&self, target: Rect) -> crate::window_layout_execution::WriteResult {
            objc2::rc::autoreleasepool(|_| {
                let result = self
                    .inspect_pooled()
                    .map_err(crate::window_layout_execution::InspectError::reason)
                    .and_then(|c| {
                        crate::window_layout_execution::verify(&c, c.rect, target)?;
                        if !target.valid() {
                            return Err("目标窗口尺寸无效".into());
                        }
                        crate::window_layout_geometry::adjust(self, target)?;
                        Ok(())
                    });
                if let Err(reason) = result {
                    let actual = if self.launch_valid().is_ok() {
                        rect(&self.element)
                    } else {
                        None
                    };
                    return crate::window_layout_execution::WriteResult {
                        actual,
                        error: Some(reason),
                    };
                }
                // Poll only on the worker; never reissue setters while waiting.
                let observation =
                    self.settle_geometry(target, crate::window_layout_geometry::Part::Complete);
                let identity = self
                    .inspect_pooled()
                    .map_err(crate::window_layout_execution::InspectError::reason);
                match identity {
                    Ok(candidate) => crate::window_layout_execution::WriteResult {
                        actual: Some(candidate.rect),
                        error: observation.error,
                    },
                    Err(reason) => crate::window_layout_execution::WriteResult {
                        actual: observation.actual,
                        error: Some(reason),
                    },
                }
            })
        }
    }
    fn error_reason(code: i32) -> &'static str {
        match code {
            -25204 => "工具未响应辅助功能读取",
            -25202 => "窗口已失效",
            -25205 | -25208 => "工具未提供窗口读取能力",
            -25211 => "辅助功能权限不可用",
            _ => "窗口列表读取失败",
        }
    }
}
