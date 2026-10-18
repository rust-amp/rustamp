// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The window shell: the one place that decides what an OS window of this
//! player looks like **outside** its client area.
//!
//! Every classic window is **borderless**. Spec S01 measures the three classic
//! windows in a coordinate space whose "origin [is] at the window's client
//! top-left" and puts a skinned title bar at (0,0,275,14) inside that client
//! area — [`crate::layout::MainWindowLayout::title_bar`],
//! [`crate::equalizer::EqualizerLayout::title_bar`] and
//! [`crate::playlist_window::TITLE_BAR_HEIGHT`] all draw it from the skin,
//! with the skin's own close, shade and (where the skin has one) menu
//! buttons. A native caption on top of that is a second title bar: it shows
//! the OS's buttons instead of the skin's, it is the wrong height and colour
//! for every skin, and it is what the user would drag instead of the skinned
//! bar.
//!
//! `winit` does not give that for free: [`winit::window::WindowAttributes`]
//! defaults `decorations` to `true` (a `WS_CAPTION | WS_BORDER | WS_SYSMENU`
//! window with minimise, maximise and close boxes on Windows), so a window
//! built from `Window::default_attributes()` without saying otherwise is
//! decorated. Every window this player opens therefore goes through
//! [`skinned_window_attributes`] — it is the only place in `rustamp-ui` and
//! `rustamp-app` that calls `Window::default_attributes()`, and
//! `crates/rustamp-app/tests/window_shell.rs` fails if a second one appears.
//!
//! # One application, one taskbar button
//!
//! The classic windows are one application. The taskbar showed one button per
//! window, because every `winit` window is an ordinary top-level window and
//! the shell gives each of those a button. [`TaskbarRole`] is the decision
//! per window: exactly one window - the main window - represents the player
//! in the taskbar; the equalizer, the playlist, the dialogs and the popups
//! are [`TaskbarRole::Secondary`] and ask the shell not to list them
//! (`skip_taskbar`, i.e. `ITaskbarList::DeleteTab` on Windows). Closing the
//! main window quits, so the player never loses its only button while it is
//! still running.
//!
//! # Why the client geometry is unaffected
//!
//! The frame lives outside the client area. [`skinned_window_attributes`]
//! names the **client** size (`with_inner_size`), which is what every geometry
//! test measures (`window.inner_size()`), and dropping the frame does not
//! touch it: the client area stays exactly the 275 x 116 (and 550 x 232) that
//! S01 and S05 pin. What does change is the *outer* size,
//! which now equals the client size, so an outer-frame-to-outer-frame docking
//! placement finally stacks client edges gap-free (S05 §3.3) instead of being
//! a caption's height too low.

use winit::dpi::PhysicalSize;
use winit::window::{Window, WindowAttributes};

/// Whether a window represents the whole player in the taskbar.
///
/// The taskbar lists top-level windows, so a player with the main window, the
/// equalizer and the playlist open appeared three times. Winamp's windows are
/// one application and the taskbar must say so, so exactly one window carries
/// the entry and the rest opt out. See the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskbarRole {
    /// The one window that stands for the player: it keeps the taskbar entry.
    ///
    /// This is the main window. It exists for the whole run (closing it
    /// quits), so the entry never disappears under the user.
    Application,
    /// Another window of the same application: no taskbar entry of its own.
    ///
    /// The equalizer, the playlist editor, the dialogs and the popups. They
    /// are still ordinary focusable windows; they are simply not separate
    /// applications.
    Secondary,
}

/// Whether a window of this player carries the native window frame.
///
/// Always `false`: the title bar is part of the skin (see the module docs).
/// Named so a test can state the rule rather than a bare `false`.
pub const SKINNED_DECORATIONS: bool = false;

/// Whether a window of this player can be resized by the user.
///
/// Always `false`: the classic windows have exactly the sizes S01/S05 measure,
/// and the player sets them itself (double-size, windowshade).
pub const SKINNED_RESIZABLE: bool = false;

/// The attributes of a skinned window with the given title and **client**
/// size, in physical pixels.
///
/// This is the shared shell: borderless ([`SKINNED_DECORATIONS`]),
/// non-resizable ([`SKINNED_RESIZABLE`]), and sized by its client area. The
/// caller adds whatever else is its own business — a position, a window
/// level — on the returned value; those builders do not touch the frame.
///
/// The title is still set: it is what the taskbar and the window list show,
/// and the alt-tab entry would otherwise read `winit window`.
pub fn skinned_window_attributes(
    title: &str,
    size: (u32, u32),
    role: TaskbarRole,
) -> WindowAttributes {
    let attributes = Window::default_attributes()
        .with_title(title)
        .with_inner_size(PhysicalSize::new(size.0, size.1))
        .with_resizable(SKINNED_RESIZABLE)
        .with_decorations(SKINNED_DECORATIONS);
    #[cfg(windows)]
    let attributes = {
        use winit::platform::windows::WindowAttributesExtWindows;
        attributes.with_skip_taskbar(role == TaskbarRole::Secondary)
    };
    #[cfg(not(windows))]
    let _ = role;
    attributes
}

/// The client size `attributes` asks for, in physical pixels, if it asks for
/// one.
///
/// A test-facing reader: the attributes store a `Size`, and every caller here
/// sets a physical one, so the scale factor cannot matter (`1.0` is exact for
/// a `PhysicalSize`).
pub fn attributes_client_size(attributes: &WindowAttributes) -> Option<(u32, u32)> {
    attributes
        .inner_size
        .map(|size| size.to_physical::<u32>(1.0))
        .map(|size| (size.width, size.height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dialogs::ABOUT_CLIENT_SIZE;
    use crate::equalizer::EQUALIZER_WINDOW_SIZE;
    use crate::layout::{DOUBLE_HEIGHT, DOUBLE_WIDTH, WINDOW_HEIGHT, WINDOW_WIDTH};
    use crate::playlist::PLAYLIST_WINDOW_SIZE;

    /// The defect this module exists for: a window built from `winit`'s
    /// defaults is decorated, and the shell must say otherwise.
    #[test]
    fn the_skinned_shell_has_no_native_frame() {
        let attributes = skinned_window_attributes(
            "Rustamp",
            (WINDOW_WIDTH, WINDOW_HEIGHT),
            TaskbarRole::Application,
        );
        assert!(
            !attributes.decorations,
            "a classic window is borderless: the title bar is part of the skin"
        );
        assert!(!attributes.resizable, "a classic window has a fixed size");
        // The guard: `winit`'s own default is the defect, so the test is only
        // worth anything while that default is still `true`.
        assert!(
            Window::default_attributes().decorations,
            "winit still defaults to a decorated window, so saying so is required"
        );
    }

    #[test]
    fn the_shell_keeps_the_exact_client_size_of_every_window() {
        for (title, size) in [
            ("Rustamp", (WINDOW_WIDTH, WINDOW_HEIGHT)),
            ("Rustamp", (DOUBLE_WIDTH, DOUBLE_HEIGHT)),
            ("Rustamp Equalizer", EQUALIZER_WINDOW_SIZE),
            ("Rustamp Playlist", PLAYLIST_WINDOW_SIZE),
            ("About Rustamp", ABOUT_CLIENT_SIZE),
        ] {
            let attributes = skinned_window_attributes(title, size, TaskbarRole::Secondary);
            assert_eq!(
                attributes_client_size(&attributes),
                Some(size),
                "{title}: the shell must ask for the measured client area, not a frame-adjusted one"
            );
            assert_eq!(attributes.title, title);
        }
    }

    /// The frame must survive the builders the callers chain onto the shell:
    /// a saved position or an always-on-top level must not bring the caption
    /// back.
    #[test]
    fn the_callers_builders_do_not_bring_the_frame_back() {
        let attributes = skinned_window_attributes(
            "Rustamp",
            (WINDOW_WIDTH, WINDOW_HEIGHT),
            TaskbarRole::Application,
        )
        .with_position(winit::dpi::PhysicalPosition::new(26, 29))
        .with_window_level(winit::window::WindowLevel::AlwaysOnTop);
        assert!(!attributes.decorations);
        assert_eq!(
            attributes_client_size(&attributes),
            Some((WINDOW_WIDTH, WINDOW_HEIGHT))
        );
    }
}
