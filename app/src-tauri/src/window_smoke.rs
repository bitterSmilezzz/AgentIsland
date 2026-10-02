//! Explicit native regression mode for lazy creation and the real hidden-window lease.
use tauri::{AppHandle, Emitter, Manager};
use std::time::Duration;

pub fn schedule(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(30));
        if app.webview_windows().len() != 1 || app.get_webview_window("workbench").is_some() {
            super::log_line("[memory-smoke] FAIL eager windows");
            app.exit(1); return;
        }
        super::log_line("[memory-smoke] PASS resident only");
        super::reveal_workbench_window(&app);
        std::thread::sleep(Duration::from_secs(12));
        if app.get_webview_window("workbench").is_none() {
            super::log_line("[memory-smoke] FAIL lazy creation");
            app.exit(1); return;
        }
        super::log_line("[memory-smoke] PASS lazy workbench");
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || super::conceal_workbench_window(&handle));
        std::thread::sleep(Duration::from_secs(100));
        if app.get_webview_window("workbench").is_some() {
            super::log_line("[memory-smoke] FAIL hidden workbench was retained");
            app.exit(1); return;
        }
        super::log_line("[memory-smoke] PASS hidden lease released");
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let _ = handle.emit("deep-link://new-url", vec!["agentisland://workbench"]);
        });
        std::thread::sleep(Duration::from_secs(8));
        if app.get_webview_window("workbench").is_none() {
            super::log_line("[memory-smoke] FAIL reopening after release");
            app.exit(1); return;
        }
        super::log_line("[memory-smoke] PASS recreated workbench");
        app.exit(0);
    });
}
