// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The **modern** window path: an OS window that is not skinned.
//!
//! Every classic window of this player is **borderless**: its title bar is
//! drawn from the skin, so a native caption would be a second title bar with
//! the wrong buttons, the wrong height and the wrong colours (see
//! [`rustamp_ui::shell`]). The **media library** is not one of them: it has
//! a native title bar, a system font and its own chrome, so it is an
//! ordinary window of the operating system - decorated and resizable.
//!
//! The attributes still come from the shared shell
//! ([`rustamp_ui::shell::skinned_window_attributes`]): it is the only place
//! in the player that builds attributes from `winit`'s own defaults, and
//! `winit::window::WindowAttributes` has no other constructor. The modern
//! path therefore starts from the shell's attributes and replaces the two
//! decisions that make a window modern - the decoration and the
//! resizability - which is exactly what a window that is not skinned needs.
//! The taskbar role is the shell's: the library is another window of the
//! same application, so it asks for no taskbar entry of its own
//! ([`TaskbarRole::Secondary`]).
//!
//! [`TaskbarRole::Secondary`]: rustamp_ui::shell::TaskbarRole::Secondary

use rustamp_ui::shell::TaskbarRole;
use winit::window::WindowAttributes;

/// Whether a modern window carries the native frame.
///
/// Always `true`: the window is not skinned, so its title bar is the
/// operating system's.
pub const MODERN_DECORATIONS: bool = true;

/// Whether a modern window can be resized by the user.
///
/// Always `true`: the media library is a list of tracks, and the list grows
/// with the window.
pub const MODERN_RESIZABLE: bool = true;

/// The attributes of a modern window with the given title and **client**
/// size, in physical pixels.
///
/// The client size is the size the window's content is drawn in, so the
/// caller may resize the window freely: the content follows the client area
/// ([`crate::library_window`]).
pub fn modern_window_attributes(
    title: &str,
    size: (u32, u32),
    role: TaskbarRole,
) -> WindowAttributes {
    rustamp_ui::shell::skinned_window_attributes(title, size, role)
        .with_decorations(MODERN_DECORATIONS)
        .with_resizable(MODERN_RESIZABLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defect this module exists for: the shared shell is built for the
    /// **classic** windows, which are borderless, so a window that is not
    /// skinned must say so itself.
    #[test]
    fn a_modern_window_has_the_native_frame_and_can_be_resized() {
        let attributes =
            modern_window_attributes("Rustamp Library", (800, 480), TaskbarRole::Secondary);
        assert!(
            attributes.decorations,
            "the media library is not skinned: its title bar is the operating system's"
        );
        assert!(
            attributes.resizable,
            "the media library is a list of tracks: the list grows with the window"
        );
        assert_eq!(
            rustamp_ui::attributes_client_size(&attributes),
            Some((800, 480)),
            "the client area must pass through the shell untouched"
        );
        assert_eq!(attributes.title, "Rustamp Library");
    }

    /// The shell is still the shell: the modern path adds the frame, it does
    /// not fork the attributes.
    #[test]
    fn the_modern_path_starts_from_the_shared_shell() {
        let classic = rustamp_ui::shell::skinned_window_attributes(
            "Rustamp",
            (800, 480),
            TaskbarRole::Secondary,
        );
        assert!(!classic.decorations, "the classic windows stay borderless");
        assert!(!classic.resizable, "the classic windows keep their size");
    }
}
