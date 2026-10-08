//! Explicit, visible regression probe. Never enabled by normal application startup.
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager};

static PROBE_STARTED: AtomicBool = AtomicBool::new(false);

// eval may succeed against the initial blank document. Wait for the real
// synthetic page and start once in that document, rather than guessing load time.
const START_SCRIPT: &str = r#"(() => {
  if (window.__agentIslandMotionProbeStarted || document.readyState !== 'complete'
      || !window.__TAURI__?.core?.invoke || !document.querySelector('.card [data-agent]')) return;
  window.__agentIslandMotionProbeStarted = true;
  import('/js/motion-probe.js').then(module => module.runMotionProbe()).catch(async () => {
    const invoke = window.__TAURI__.core.invoke;
    await invoke('log_from_ui', {message:'MOTION_START_FAILED: probe import or startup rejected'});
    await invoke('motion_probe_finish', {passed:false});
  });
})()"#;

pub fn requested() -> bool {
    std::env::args().any(|arg| arg == "--motion-smoke")
}

pub fn edge_from(mut args: impl Iterator<Item = String>) -> String {
    args.find_map(|arg| arg.strip_prefix("--motion-edge=").map(str::to_owned))
        .filter(|edge| matches!(edge.as_str(), "top" | "bottom" | "left" | "right"))
        .unwrap_or_else(|| "top".into())
}

#[tauri::command]
pub fn motion_probe_frame(window: tauri::WebviewWindow) -> Result<serde_json::Value, String> {
    if !requested() || window.label() != "island" {
        return Err("Native motion probe is not enabled".into());
    }
    PROBE_STARTED.store(true, Ordering::Relaxed);
    #[cfg(target_os = "macos")]
    {
        let (send, receive) = std::sync::mpsc::channel();
        let native_window = window.clone();
        window
            .run_on_main_thread(move || {
                let result = (|| -> Result<serde_json::Value, String> {
                    use objc2::MainThreadMarker;
                    use objc2_app_kit::{NSScreen, NSWindow};
                    let mtm = MainThreadMarker::new().ok_or("AppKit requires main thread")?;
                    let primary = NSScreen::screens(mtm)
                        .firstObject()
                        .ok_or("No primary screen")?;
                    let pointer = native_window
                        .ns_window()
                        .map_err(|error| error.to_string())?;
                    if pointer.is_null() {
                        return Err("Missing native window".into());
                    }
                    // SAFETY: Tauri retains the window, and this closure is on AppKit's main thread.
                    let native = unsafe { &*pointer.cast::<NSWindow>() };
                    let frame = native.frame();
                    Ok(serde_json::json!({
                        "x": frame.origin.x,
                        "y": primary.frame().size.height-frame.origin.y-frame.size.height,
                        "width": frame.size.width,
                        "height": frame.size.height,
                        "visible": native.isVisible(),
                    }))
                })();
                let _ = send.send(result);
            })
            .map_err(|error| error.to_string())?;
        return receive
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
    }
    #[cfg(not(target_os = "macos"))]
    Err("Native motion probe requires macOS".into())
}

#[tauri::command]
pub fn motion_probe_finish(app: AppHandle, passed: bool) -> Result<(), String> {
    if !requested() {
        return Err("Native motion probe is not enabled".into());
    }
    app.exit(if passed { 0 } else { 2 });
    Ok(())
}

pub fn schedule(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        for _ in 0..60 {
            if PROBE_STARTED.load(Ordering::Relaxed) {
                return;
            }
            if let Some(window) = app.get_webview_window("island") {
                if window.eval(START_SCRIPT).is_err() {
                    app.exit(2);
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        if !PROBE_STARTED.load(Ordering::Relaxed) {
            app.exit(2);
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn edge_is_a_closed_test_configuration() {
        for edge in ["top", "bottom", "left", "right"] {
            assert_eq!(
                super::edge_from([format!("--motion-edge={edge}")].into_iter()),
                edge
            );
        }
        assert_eq!(
            super::edge_from(["--motion-edge=unknown".into()].into_iter()),
            "top"
        );
        assert_eq!(super::edge_from(std::iter::empty()), "top");
    }
}
