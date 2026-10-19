// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The window's shape, applied to a real window: **VM-only, `#[ignore]`d**.
//!
//! GUI must never run on the host, so this test refuses unless it can prove
//! it runs inside the headless test VM — the shared gate in
//! [`rustamp_app::window::require_test_vm`]. There is deliberately **no**
//! override. Run it only inside the headless test VM:
//!
//! ```text
//! cargo test -p rustamp-app --test window_region -- --ignored
//! ```
//!
//! It cuts a real window to the shape a skin's `region.txt` asks for, reads
//! the shape back and clears it again. That is the half the headless tests
//! cannot cover: the operating system's window region itself. The
//! rectangles and the section choice are covered headless, in
//! [`rustamp_app::region`].

use std::sync::{Arc, Mutex};

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Window, WindowId};

/// The shape a skin asks for: the left and the right part of the window,
/// with a gap in the middle.
fn skin_shape() -> rustamp_skin::Region {
    rustamp_skin::Region::parse(
        "[Normal]\r\nNumPoints=4,4\r\n\
         PointList=0,0,80,0,80,116,0,116,195,0,275,0,275,116,195,116\r\n",
    )
}

/// What the operating system answered about the window's region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RegionReading {
    /// A pixel the skin covers is inside the region.
    covered: Option<bool>,
    /// A pixel in the gap is outside the region.
    gap: Option<bool>,
    /// A pixel of the right-hand part is inside the region.
    covered_right: Option<bool>,
    /// After the region is cleared the window has no region at all.
    cleared: Option<bool>,
}

/// Cuts a real window to the skin's shape and reads the shape back.
#[test]
#[ignore = "VM-only: opens a window; run inside the headless test VM"]
fn the_window_region_cuts_the_window_to_the_skins_shape() {
    // The gate, before any window exists.
    rustamp_app::window::require_test_vm();

    let rects = rustamp_app::region::main_rects(&skin_shape(), false, false);
    assert!(!rects.is_empty(), "the skin asks for a shape");

    struct Probe {
        rects: Vec<rustamp_platform::window_region::RegionRect>,
        seen: Arc<Mutex<Option<RegionReading>>>,
    }

    impl ApplicationHandler for Probe {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            // The product's own shell, so the probe sees the window the
            // player ships with: borderless, because the title bar is part
            // of the skin.
            let window: Window = event_loop
                .create_window(rustamp_ui::skinned_window_attributes(
                    "Rustamp region (ignored probe)",
                    (275, 116),
                    rustamp_ui::shell::TaskbarRole::Secondary,
                ))
                .expect("create the window");
            let raw = match window.window_handle().map(|handle| handle.as_raw()) {
                Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get(),
                _ => 0,
            };
            assert!(
                rustamp_platform::window_region::apply_window_region(raw, &self.rects),
                "the operating system must take the region"
            );
            let covered = rustamp_platform::window_region::window_region_contains(raw, 40, 10);
            let gap = rustamp_platform::window_region::window_region_contains(raw, 130, 10);
            let covered_right =
                rustamp_platform::window_region::window_region_contains(raw, 230, 10);
            // No rectangle at all: the window is a plain rectangle again.
            assert!(
                rustamp_platform::window_region::apply_window_region(raw, &[]),
                "the operating system must take the empty region"
            );
            let cleared = rustamp_platform::window_region::window_region_contains(raw, 40, 10);
            *self.seen.lock().unwrap() = Some(RegionReading {
                covered,
                gap,
                covered_right,
                cleared,
            });
            event_loop.exit();
        }

        fn window_event(
            &mut self,
            _event_loop: &ActiveEventLoop,
            _id: WindowId,
            _event: WindowEvent,
        ) {
        }
    }

    let seen = Arc::new(Mutex::new(None));
    let mut probe = Probe {
        rects,
        seen: Arc::clone(&seen),
    };
    // Off the main thread on purpose: this test runs on a harness worker
    // thread, so `EventLoop::new()` would panic. The product keeps the
    // default; see [`rustamp_app::window::build_event_loop`].
    let event_loop = rustamp_app::window::build_event_loop(true).expect("create the event loop");
    event_loop.run_app(&mut probe).expect("run the event loop");

    let reading = seen.lock().unwrap().expect("the probe ran");
    assert_eq!(
        reading.covered,
        Some(true),
        "a pixel the skin covers must be inside the window's region"
    );
    assert_eq!(
        reading.gap,
        Some(false),
        "a pixel outside the skin's shape must be outside the window's region"
    );
    assert_eq!(
        reading.covered_right,
        Some(true),
        "the second part of the shape must be inside the window's region"
    );
    assert_eq!(
        reading.cleared, None,
        "a cleared region leaves the window a plain rectangle: it has no region at all"
    );
}
