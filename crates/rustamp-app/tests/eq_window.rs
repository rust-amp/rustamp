// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The equalizer window, opened for real: **VM-only, `#[ignore]`d**.
//!
//! GUI must never run on the host, so this test refuses unless it can prove
//! it runs inside the headless test VM — the shared gate in
//! [`rustamp_app::window::require_test_vm`]. There is deliberately **no**
//! override. Run it only inside the headless test VM:
//!
//! ```text
//! cargo test -p rustamp-app --test eq_window -- --ignored
//! ```
//!
//! It opens the equalizer window at its docked size and position (S07 + S05
//! §3.1: 275 x 116 at (26,145)), reads back the client size, the outer
//! position and the outer size, and exits. The outer size is the frame check:
//! it equals the client size only when the window carries no native caption
//! (wave122), which is the one half of that defect a desktop can observe. It creates no softbuffer surface and plays nothing;
//! painting is covered by the headless render tests.
//!
//! The loop is built off the main thread on purpose: tests run on harness
//! worker threads, so `EventLoop::new()` would panic. See
//! [`rustamp_app::window::build_event_loop`].

use std::sync::{Arc, Mutex};

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

// The shared test-VM gate lives in `rustamp_app::window::require_test_vm`:
// this file keeps no copy of it (an ignored test that can open a window on
// the host is a loaded gun, so the gate has deliberately **no** override).

/// Opens the real equalizer window and checks its client size.
///
/// The size is the S05 §2.1 rest size (275 x 116) and the position is the S05
/// §3.1 docked default ((26,145), one main-window height below the main
/// window at (26,29)) — the same values the player uses in `window.rs`.
#[test]
#[ignore = "VM-only: opens the equalizer window; run inside the headless test VM"]
fn the_equalizer_window_opens_at_its_docked_size_and_position() {
    // The gate, before any window exists.
    rustamp_app::window::require_test_vm();

    let (width, height) =
        rustamp_ui::docking::window_size(rustamp_ui::WindowKind::Equalizer, false, false);
    assert_eq!((width, height), (275, 116));
    let pos = rustamp_ui::DEFAULT_EQUALIZER_POSITION;
    assert_eq!((pos.x, pos.y), (26, 145));

    struct Probe {
        seen: Arc<Mutex<Option<(u32, u32)>>>,
        seen_pos: Arc<Mutex<Option<(i32, i32)>>>,
        seen_outer: Arc<Mutex<Option<(u32, u32)>>>,
    }

    impl ApplicationHandler for Probe {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            let (w, h) =
                rustamp_ui::docking::window_size(rustamp_ui::WindowKind::Equalizer, false, false);
            let at = rustamp_ui::DEFAULT_EQUALIZER_POSITION;
            // The product's own shell, so this probe sees the frame the
            // player ships with: borderless, because the title bar is part
            // of the skin.
            let window: Window = event_loop
                .create_window(
                    rustamp_ui::skinned_window_attributes(
                        "Rustamp Equalizer (ignored probe)",
                        (w, h),
                        rustamp_ui::shell::TaskbarRole::Secondary,
                    )
                    .with_position(PhysicalPosition::new(at.x, at.y)),
                )
                .expect("create the equalizer window");
            let size = window.inner_size();
            *self.seen.lock().unwrap() = Some((size.width, size.height));
            let pos = window.outer_position().expect("read the window position");
            *self.seen_pos.lock().unwrap() = Some((pos.x, pos.y));
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
    let seen_pos = Arc::new(Mutex::new(None));
    let seen_outer = Arc::new(Mutex::new(None));
    let mut probe = Probe {
        seen: Arc::clone(&seen),
        seen_pos: Arc::clone(&seen_pos),
        seen_outer: Arc::clone(&seen_outer),
    };
    // Off the main thread on purpose: this test runs on a harness worker
    // thread, so `EventLoop::new()` would panic. The product keeps the
    // default; see `rustamp_app::window::build_event_loop`.
    let event_loop = rustamp_app::window::build_event_loop(true).expect("create the event loop");
    event_loop.run_app(&mut probe).expect("run the event loop");
    assert_eq!(*seen.lock().unwrap(), Some((275, 116)));
    assert_eq!(
        *seen_pos.lock().unwrap(),
        Some((26, 145)),
        "the equalizer must open at its S05 §3.1 docked position"
    );
    // The frame check that needs a desktop: a window with a native caption
    // is taller and wider on the outside than its client area. Borderless
    // means the two are the same, which is also what makes the docked
    // neighbour stack client edges gap-free (S05 §3.3).
    assert_eq!(
        *seen_outer.lock().unwrap(),
        Some((275, 116)),
        "the equalizer window must have no native frame: outer size == client size"
    );
}
