use objc2::{rc::Weak, runtime::AnyObject};
use std::time::{Duration, Instant};
use tao::{
    event_loop::{ControlFlow, EventLoop},
    platform::{macos::WindowExtMacOS, run_return::EventLoopExtRunReturn},
    window::WindowBuilder,
};

// A zeroing weak reference can clear when dealloc starts even if a custom
// dealloc omits its superclass and never frees the allocation. Observe both.
fn native_allocations() -> [usize; 3] {
    let output = std::process::Command::new("/usr/bin/heap")
        .args(["-s", &std::process::id().to_string()])
        .output()
        .expect("native heap observation unavailable");
    assert!(output.status.success(), "native heap observation failed");
    let summary = String::from_utf8(output.stdout).expect("invalid heap summary");
    let mut counts = [0; 3];
    for line in summary.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() < 5 || fields[4] != "ObjC" {
            continue;
        }
        let slot = match fields[3] {
            "TaoView" => 1,
            "TaoWindowDelegate" => 2,
            name if name.ends_with("TaoWindow") => 0,
            _ => continue,
        };
        counts[slot] += fields[0].parse::<usize>().expect("invalid class count");
    }
    assert!(counts.iter().all(|n| *n > 0), "native classes not observed");
    counts
}

fn settle(event_loop: &mut EventLoop<()>) {
    let end = Instant::now() + Duration::from_millis(100);
    event_loop.run_return(|_, _, flow| {
        *flow = if Instant::now() >= end {
            ControlFlow::Exit
        } else {
            ControlFlow::WaitUntil(end)
        };
    });
}

fn main() {
    // Keep this native regression out of the user's desktop.
    let allow_desktop = std::env::args().skip(1).collect::<Vec<_>>() == ["--allow-desktop"];
    assert!(
        std::env::args().count() == 1 || allow_desktop,
        "unknown probe arguments"
    );
    let proof = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "kern.hv_vmm_present"])
        .output()
        .unwrap();
    assert!(
        allow_desktop
            || (proof.status.success() && String::from_utf8_lossy(&proof.stdout).trim() == "1"),
        "requires an isolated macOS VM"
    );
    let mut event_loop = EventLoop::new();
    let _anchor = WindowBuilder::new()
        .with_title("Lifetime probe anchor")
        .with_visible(false)
        .build(&event_loop)
        .unwrap();
    settle(&mut event_loop);
    let baseline = native_allocations();
    println!("{{\"phase\":\"baseline\",\"window_view_delegate_allocations\":{baseline:?}}}");
    let mut retired = Vec::new();
    let mut retired_views = Vec::new();
    for cycle in 1..=10 {
        let window = WindowBuilder::new()
            .with_title("Disposable lifetime probe")
            .with_visible(false)
            .build(&event_loop)
            .unwrap();
        // A zeroing weak reference observes native deallocation without retaining it.
        let weak = Weak::new(unsafe { &*window.ns_window().cast::<AnyObject>() });
        retired.push(weak);
        retired_views.push(Weak::new(unsafe { &*window.ns_view().cast::<AnyObject>() }));
        drop(window);
        settle(&mut event_loop);
        println!(
            "{{\"cycle\":{cycle},\"retained_native_windows\":{}}}",
            retired.iter().filter(|w| w.load().is_some()).count()
        );
    }
    if retired.iter().any(|w| w.load().is_some())
        || retired_views.iter().any(|w| w.load().is_some())
    {
        std::process::exit(1)
    }
    let released = native_allocations();
    println!("{{\"phase\":\"released\",\"window_view_delegate_allocations\":{released:?}}}");
    assert_eq!(released, baseline, "native allocations remain after drop");
    println!("WINDOW_LIFETIME_PASS: 10 retired windows, views and delegates freed");
}
