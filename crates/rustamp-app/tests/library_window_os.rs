// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The media library window, opened for real: **VM-only, `#[ignore]`d**.
//!
//! GUI must never run on the host, so this test refuses unless it can prove
//! it runs inside the headless test VM — the shared gate in
//! [`rustamp_app::window::require_test_vm`]. There is deliberately **no**
//! override. Run it only inside the headless test VM:
//!
//! ```text
//! cargo test -p rustamp-app --test library_window_os -- --ignored
//! ```
//!
//! It opens the library window through the product's own **modern** path
//! ([`rustamp_app::modern_shell::modern_window_attributes`]) and reads back
//! the client size and the outer size.
//!
//! The outer size is the frame check: the library draws the skin's title bar
//! itself, like the classic windows, so it is borderless and its outer size
//! equals its client size (`tests/eq_window.rs`). A system caption here would be
//! a second title bar.
//!
//! The loop is built off the main thread on purpose: tests run on harness
//! worker threads, so `EventLoop::new()` would panic. See
//! [`rustamp_app::window::build_event_loop`].

use std::sync::{Arc, Mutex};

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use rustamp_app::library_window::LIBRARY_CLIENT_SIZE;
use rustamp_app::modern_shell::modern_window_attributes;

// The shared test-VM gate lives in `rustamp_app::window::require_test_vm`:
// this file keeps no copy of it (an ignored test that can open a window on
// the host is a loaded gun, so the gate has deliberately **no** override).

/// Opens the real media library window and checks its client size.
#[test]
#[ignore = "VM-only: opens the media library window; run inside the headless test VM"]
fn the_library_window_opens_with_its_client_size_and_no_system_frame() {
    // The gate, before any window exists.
    rustamp_app::window::require_test_vm();

    struct Probe {
        seen: Arc<Mutex<Option<(u32, u32)>>>,
        seen_outer: Arc<Mutex<Option<(u32, u32)>>>,
    }

    impl ApplicationHandler for Probe {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            // The product's own modern path, so this probe sees the frame
            // the player ships with: a native caption, because the media
            // library is not skinned.
            let window: Window = event_loop
                .create_window(modern_window_attributes(
                    "Rustamp Library (ignored probe)",
                    LIBRARY_CLIENT_SIZE,
                    rustamp_ui::shell::TaskbarRole::Secondary,
                ))
                .expect("create the library window");
            let size = window.inner_size();
            *self.seen.lock().unwrap() = Some((size.width, size.height));
            let outer = window.outer_size();
            *self.seen_outer.lock().unwrap() = Some((outer.width, outer.height));
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
    let seen_outer = Arc::new(Mutex::new(None));
    let mut probe = Probe {
        seen: Arc::clone(&seen),
        seen_outer: Arc::clone(&seen_outer),
    };
    // Off the main thread on purpose: this test runs on a harness worker
    // thread, so `EventLoop::new()` would panic. The product keeps the
    // default; see `rustamp_app::window::build_event_loop`.
    let event_loop = rustamp_app::window::build_event_loop(true).expect("create the event loop");
    event_loop.run_app(&mut probe).expect("run the event loop");
    assert_eq!(
        *seen.lock().unwrap(),
        Some(LIBRARY_CLIENT_SIZE),
        "the client area must be the size the app draws in"
    );
    let (outer_width, outer_height) = seen_outer.lock().unwrap().expect("read the outer size");
    assert_eq!(
        (outer_width, outer_height),
        LIBRARY_CLIENT_SIZE,
        "the media library draws its own skinned title bar, so the system adds none"
    );
}
