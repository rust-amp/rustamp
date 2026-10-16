// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The equalizer window, opened for real: **sandbox-only, `#[ignore]`d**.
//!
//! GUI must never run on the host (the reference Winamp is driven inside a
//! Windows Sandbox on this machine), so this test refuses unless it can prove
//! it runs as the sandbox user `WDAGUtilityAccount` — the same gate as
//! `tests/headless.rs`. There is deliberately **no** override. Run it only
//! inside the Windows Sandbox:
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

/// The Windows Sandbox account. Everything inside the sandbox runs as this
/// user, which is the marker that proves a run is not on the host.
const SANDBOX_USER: &str = "WDAGUtilityAccount";

/// Refuses unless this process provably runs inside the Windows Sandbox.
///
/// There is deliberately **no** override: the test below opens the app's
/// equalizer window, and an ignored test that can do that on the host is a
/// loaded gun.
fn require_windows_sandbox() {
    let user = std::env::var("USERNAME").unwrap_or_default();
    assert!(
        user.eq_ignore_ascii_case(SANDBOX_USER),
        "refusing to open a GUI window: this test is sandbox-only, but the user is {user:?} \
         rather than {SANDBOX_USER:?}. GUI must never run on the host; run it inside the \
         Windows Sandbox (see crates/rustamp-app/README.md)."
    );
}

/// Opens the real equalizer window and checks its client size.
///
/// The size is the S05 §2.1 rest size (275 x 116) and the position is the S05
/// §3.1 docked default ((26,145), one main-window height below the main
/// window at (26,29)) — the same values the player uses in `window.rs`.
#[test]
#[ignore = "sandbox-only: opens the equalizer window; run inside the Windows Sandbox"]
fn the_equalizer_window_opens_at_its_docked_size_and_position() {
    // The gate, before any window exists.
    require_windows_sandbox();

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
