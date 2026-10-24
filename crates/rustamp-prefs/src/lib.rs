// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-prefs
//!
//! The preferences dialog's page model: which pages exist, what they are
//! called, and which page each [`Command`] opens.
//!
//! The reference's preferences UI is **not measured** — no spec covers its
//! pages, their controls or their layout (S04 is not started; RFC 0005 §7
//! fixes only the two page-opening command ids). So this crate is
//! deliberately a **frame**, not a copy of the reference:
//!
//! * [`PAGES`] lists exactly the pages the project has evidence for: the two
//!   [`PrefsPage`] values S03 records command ids for, both measured
//!   (2026-10-15): 40191 opens the dialog at Plug-ins > Visualization and
//!   40317 at Options > Visualization (RFC 0005 §7). Growing the list follows
//!   the [`Command`] append rule and needs a measured page first.
//! * [`DEFAULT_PAGE`] is the page a bare [`Command::OpenPrefs`] (Ctrl+P, id
//!   40012) shows. No spec fixes the dialog's initial page, so this is an
//!   **own choice**, pinned by tests.
//! * [`page_name`] returns own-choice display names, not reference text.
//! * A page shows its name and nothing else: the controls, their layout and
//!   their effects are unmeasured, so there is nothing honest to draw yet.
//!
//! The dialog shell (open/close state, the OS window) lives in `rustamp-ui`'s
//! `DialogManager`: the current page is the open `DialogKind::Preferences`
//! payload (`prefs_page`), switching pages is `select_prefs_page` or a new
//! `OpenPrefsPage` command. This crate owns the list, the names, the default
//! and the command mapping those paths share.
//!
//! What a future lane must measure before this can be more than a frame: the
//! full page list, every page's controls and their layout, what each control
//! does (and persists), and the dialog's initial page — all recorded as
//! findings, never guessed.
//!
//! Skeleton (milestone M0). Codes against the frozen contracts in
//! [`rustamp_core`]; see `docs/rfc/0001-core-contracts.md`.

mod plugins;

pub use plugins::{
    APPROVALS_FILE, EnableRefused, MAX_PLUGIN_BYTES, PluginEntry, PluginScan, PluginStatus,
    kind_name, scan, set_enabled,
};

use rustamp_core::command::{Command, PrefsPage};

/// Every page the preferences dialog can show, in stable order.
///
/// Provisional: exactly the two pages the project has evidence for (see the
/// crate docs). A new page needs its own measurement first.
pub const PAGES: &[PrefsPage] = &[PrefsPage::VisPlugin, PrefsPage::VisOptions];

/// The page a bare [`Command::OpenPrefs`] shows.
///
/// Own choice: no spec fixes the dialog's initial page (S04 open). Pinned by
/// tests so a future measurement can move it deliberately.
pub const DEFAULT_PAGE: PrefsPage = PrefsPage::VisPlugin;

/// The page's display name.
///
/// Own choice, not reference text: it paraphrases the measured facts (which
/// command id opens which page, RFC 0005 §7) without claiming the reference's
/// wording.
pub fn page_name(page: PrefsPage) -> &'static str {
    match page {
        PrefsPage::VisPlugin => "Visualization plug-in",
        PrefsPage::VisOptions => "Visualization options",
    }
}

/// Which page `command` opens: [`DEFAULT_PAGE`] for a bare
/// [`Command::OpenPrefs`], the requested page for
/// [`Command::OpenPrefsPage`], and nothing for every other command.
pub fn opening_page(command: &Command) -> Option<PrefsPage> {
    match command {
        Command::OpenPrefs => Some(DEFAULT_PAGE),
        Command::OpenPrefsPage(page) => Some(*page),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_list_is_exactly_the_two_measured_pages() {
        assert_eq!(PAGES, &[PrefsPage::VisPlugin, PrefsPage::VisOptions]);
    }

    #[test]
    fn the_default_page_is_a_listed_page() {
        assert!(
            PAGES.contains(&DEFAULT_PAGE),
            "the default must be openable: {DEFAULT_PAGE:?}"
        );
    }

    #[test]
    fn page_names_are_pinned_own_choices() {
        assert_eq!(page_name(PrefsPage::VisPlugin), "Visualization plug-in");
        assert_eq!(page_name(PrefsPage::VisOptions), "Visualization options");
    }

    #[test]
    fn bare_open_prefs_opens_the_default_page() {
        assert_eq!(opening_page(&Command::OpenPrefs), Some(DEFAULT_PAGE));
    }

    #[test]
    fn open_prefs_page_opens_the_requested_page() {
        for page in [PrefsPage::VisPlugin, PrefsPage::VisOptions] {
            assert_eq!(
                opening_page(&Command::OpenPrefsPage(page)),
                Some(page),
                "{page:?}"
            );
        }
    }

    #[test]
    fn other_commands_open_no_prefs_page() {
        for command in [
            Command::Play,
            Command::OpenAbout,
            Command::OpenMainMenu,
            Command::OpenPlaylist,
        ] {
            assert_eq!(opening_page(&command), None, "{command:?}");
        }
    }
}
