// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The main-menu popup wiring (task card wave21b).
//!
//! [`rustamp_ui::menu`] owns the table (rows, separators, submenus, checks);
//! this module is the app side: it builds the [`MenuFlags`] from live app
//! state, resolves check marks, and activates menu leaves through the same
//! key-dispatch the shortcuts use. It opens no window: the OS popup that
//! hosts the rendered menu belongs to the presenter (`window.rs`, S04).

use rustamp_core::command::Command;
use rustamp_core::error::Result;
use rustamp_ui::menu::{MenuCheck, MenuFlags, check_state};
use rustamp_ui::{ShortcutAction, ShortcutOutcome};

use crate::app::App;
use crate::routing;

/// What activating one menu leaf did.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuOutcome {
    /// The leaf emitted commands, which were submitted (transport, toggles,
    /// dialog requests — the app's existing arms apply or note them).
    Commands(Vec<Command>),
    /// The leaf was the UI-local time flip (RFC 0005 §9); carries whether
    /// the display now shows remaining (`true`) or elapsed (`false`).
    TimeDisplay(bool),
    /// The leaf has no command yet; the app kept its "not implemented" note
    /// naming the item (see [`routing::unhandled_message`]).
    Noted,
}

impl App {
    /// The check-mark flags from live app state (never hard-coded).
    ///
    /// The main window exists until the app quits; the mode flags are the
    /// app's own; stop-after-current comes from the engine snapshot.
    pub fn menu_flags(&self) -> MenuFlags {
        MenuFlags {
            main_window_open: !self.quit(),
            always_on_top: self.always_on_top(),
            easy_move: self.easy_move(),
            stop_after_current: self.engine().state().stop_after_current,
        }
    }

    /// Whether the `check` mark is on right now.
    pub fn menu_check(&self, check: MenuCheck) -> bool {
        let flags = self.menu_flags();
        check_state(check, self.state(), &flags)
    }

    /// Activates one menu leaf: runs the shortcut dispatch for `action` and
    /// submits what it emits, exactly as if the matching key had been
    /// pressed. `label` is the menu row's label and only names the note when
    /// the action has no command yet.
    pub fn activate_menu_item(
        &mut self,
        label: &str,
        action: ShortcutAction,
    ) -> Result<MenuOutcome> {
        let now = self.now_ms();
        match self.ui_mut().dispatch_shortcut(action, now) {
            ShortcutOutcome::Commands(commands) => {
                let emitted = commands.clone();
                for command in &emitted {
                    // The dispatch has already applied everything the UI owns -
                    // its own comment says so for the window toggles - so only
                    // the commands another layer owns may be submitted here.
                    // Submitting a UI-owned one would apply it twice and toggle
                    // a window back to where it started.
                    if routing::app_owns(command) {
                        self.submit(command.clone())?;
                    }
                }
                Ok(MenuOutcome::Commands(emitted))
            }
            ShortcutOutcome::Handled => Ok(MenuOutcome::TimeDisplay(self.state().time_remaining)),
            ShortcutOutcome::Unhandled(action) => {
                self.note(format!(
                    "main menu \"{label}\": {}",
                    routing::unhandled_message(action)
                ));
                Ok(MenuOutcome::Noted)
            }
        }
    }
}
