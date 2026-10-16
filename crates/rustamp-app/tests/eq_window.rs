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
//! §3.1: 275 x 116 at (26,145)), reads back the client size and exits. It
//! creates no softbuffer surface and plays nothing; painting is covered by the
//! headless render tests.

use std::sync::{Arc, Mutex};

use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
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
    }

    impl ApplicationHandler for Probe {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            let (w, h) =
                rustamp_ui::docking::window_size(rustamp_ui::WindowKind::Equalizer, false, false);
            let at = rustamp_ui::DEFAULT_EQUALIZER_POSITION;
            let window: Window = event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Rustamp Equalizer (ignored probe)")
                        .with_inner_size(PhysicalSize::new(w, h))
                        .with_position(PhysicalPosition::new(at.x, at.y))
                        .with_resizable(false),
                )
                .expect("create the equalizer window");
            let size = window.inner_size();
            *self.seen.lock().unwrap() = Some((size.width, size.height));
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
        seen: Arc::clone(&seen),
    };
    let event_loop = EventLoop::new().expect("create the event loop");
    event_loop.run_app(&mut probe).expect("run the event loop");
    assert_eq!(*seen.lock().unwrap(), Some((275, 116)));
}
