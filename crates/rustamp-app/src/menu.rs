// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The main-menu popup wiring (task card wave21b, popup wave26).
//!
//! [`rustamp_ui::menu`] owns the table (rows, separators, submenus, checks),
//! the renderer and the pure popup geometry (placement anchor, hit-testing);
//! this module is the app side: it owns the popup's open state, builds the
//! [`MenuFlags`] from live app state, resolves check marks, and activates
//! menu leaves through the same key-dispatch the shortcuts use.
//!
//! **The live popup is the operating system's own menu** (wave122). The
//! presenter asks for [`App::native_main_menu`], shows it with
//! `TrackPopupMenu`, and routes the chosen entry back through
//! [`App::activate_menu_choice`]. The self-drawn 323 x 569 surface
//! ([`render_menu`](App::render_menu)) and the click hit-testing
//! ([`App::menu_click`]) are the retired path: they keep their S04 receipts
//! and their tests, and nothing in the player uses them.

use rustamp_core::command::Command;
use rustamp_core::error::Result;
use rustamp_gfx::Image;
use rustamp_ui::menu::{MenuAction, MenuCheck, MenuFlags, check_state, menu_item_at};
use rustamp_ui::{ShortcutAction, ShortcutOutcome};

use crate::app::App;
use crate::routing;

/// Which clutterbar popup is waiting to be shown (RFC 0016).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClutterMenu {
    /// The Options popup (letter O, S04 §4).
    Options,
    /// The visualization menu (letter V, S04 §5).
    Visualization,
}

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

/// What a click on the open popup did.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuClickOutcome {
    /// A leaf row was chosen and the popup closed: `Some` carries the
    /// activation outcome for a `Shortcut` leaf (through
    /// [`App::activate_menu_item`), or the quit for `Exit`; `None` for a
    /// `NoAction` row (the title row, `Library`, `Minibrowser`), which keeps its draw-only
    /// treatment — S04 §9 explicitly does not cover click-through, so no
    /// behaviour is invented for these rows.
    Activated(Option<MenuOutcome>),
    /// A submenu parent was clicked: nothing opens (S04 §9 — submenu
    /// contents were never measured), so the popup stays open and a note
    /// names the submenu instead of inventing its entries.
    SubmenuDeferred(rustamp_ui::menu::SubmenuId),
    /// A separator or an out-of-bounds point: ignored, the popup stays open.
    /// A separator click is not a selection, and S04 §9 leaves its effect
    /// unmeasured. Dismissal on a click *outside* the popup is the
    /// presenter's job — only it sees presses on other windows — so
    /// out-of-bounds points are likewise ignored here.
    Ignored,
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

    /// Opens the main-menu popup (`Command::OpenMainMenu`, S03 §1 id 40043:
    /// Alt+F and the window-menu button). The presenter creates the OS window
    /// for this state; opening twice keeps it open.
    pub fn open_main_menu(&mut self) {
        self.main_menu_open = true;
    }

    /// Asks for a clutterbar popup (`Command::OpenOptionsMenu` /
    /// `Command::OpenVisualizationMenu`). The presenter takes it from the
    /// tick ([`App::take_clutter_menu`]); a second request replaces the first.
    pub fn open_clutter_menu(&mut self, menu: ClutterMenu) {
        self.pending_clutter_menu = Some(menu);
    }

    /// Takes the pending clutterbar popup request, if any.
    pub fn take_clutter_menu(&mut self) -> Option<ClutterMenu> {
        self.pending_clutter_menu.take()
    }

    /// The native popup tree for `menu`, with live check marks.
    pub fn native_clutter_menu(&self, menu: ClutterMenu) -> rustamp_ui::menu::NativeMainMenu {
        match menu {
            ClutterMenu::Options => {
                rustamp_ui::menu::native_options_menu(self.state(), &self.menu_flags())
            }
            ClutterMenu::Visualization => rustamp_ui::menu::native_visualization_menu(),
        }
    }

    /// Closes the main-menu popup, if open. The presenter drops the OS window
    /// for this state.
    pub fn close_main_menu(&mut self) {
        self.main_menu_open = false;
    }

    /// Whether the main-menu popup is open.
    pub fn main_menu_is_open(&self) -> bool {
        self.main_menu_open
    }

    /// The main menu as a native popup tree, with live check marks.
    ///
    /// This is the **live** menu (wave122): the operating system draws it, so
    /// the separators, the submenu arrows, the right-aligned accelerators and
    /// the tick marks are the system menu's own, and the text is in the
    /// system font. The rows and their order are the S04 §2 model
    /// unchanged; the check marks resolve against live state exactly as
    /// before ([`App::menu_flags`]).
    ///
    /// The presenter shows it and routes the chosen id back through
    /// [`App::activate_menu_choice`].
    pub fn native_main_menu(&self) -> rustamp_ui::menu::NativeMainMenu {
        rustamp_ui::menu::native_main_menu(self.state(), &self.menu_flags())
    }

    /// Runs what one chosen native-menu entry means.
    ///
    /// A leaf goes through the existing activation path
    /// ([`App::activate_menu_item`]) - the same dispatch the matching key
    /// press runs - and a measured row with no command keeps its draw-only
    /// treatment (S04 §9 covers no click-through for the title row,
    /// `Library` and `Minibrowser`), so nothing is invented for it. `Exit`
    /// submits `Command::Quit`.
    ///
    /// The popup itself is already closed by the time this runs: the system
    /// menu returns only once the user has chosen or dismissed.
    pub fn activate_menu_choice(
        &mut self,
        choice: rustamp_ui::menu::MenuChoice,
    ) -> Result<Option<MenuOutcome>> {
        match choice {
            rustamp_ui::menu::MenuChoice::Action { label, action } => {
                self.activate_menu_item(label, action).map(Some)
            }
            rustamp_ui::menu::MenuChoice::Exit => self.quit_from_menu().map(Some),
            rustamp_ui::menu::MenuChoice::NoAction { .. } => Ok(None),
            rustamp_ui::menu::MenuChoice::SetTimeRemaining(remaining) => {
                // Set, don't toggle: the shortcut is a flip, so only flip when
                // the reading differs.
                if self.state().time_remaining == remaining {
                    return Ok(Some(MenuOutcome::TimeDisplay(remaining)));
                }
                self.activate_menu_item(choice.label(), ShortcutAction::ToggleTimeRemaining)
                    .map(Some)
            }
        }
    }

    /// Renders one frame of the **retired** self-drawn popup at the measured
    /// client size (323 x 569, S04 §§1,8) with live check marks.
    ///
    /// Nothing in the player paints this any more: the live menu is the
    /// system popup ([`App::native_main_menu`]). It is kept for the S04
    /// geometry receipts and the golden tests that pin them.
    pub fn render_menu(&self) -> Image {
        rustamp_ui::menu::render_main_menu(self.skin.skin(), self.state(), &self.menu_flags())
    }

    /// A left-button press at popup-client `(x, y)` on the open popup.
    ///
    /// Leaf rows go through the existing activation path
    /// ([`App::activate_menu_item`]) — no second path is invented — and close
    /// the popup; submenu parents open nothing (S04 §9) and keep it open with
    /// a note; separators and out-of-bounds points are ignored.
    pub fn menu_click(&mut self, x: i32, y: i32) -> Result<MenuClickOutcome> {
        let Some(item) = menu_item_at(x, y) else {
            return Ok(MenuClickOutcome::Ignored);
        };
        match item.action {
            MenuAction::Shortcut(action) => {
                let outcome = self.activate_menu_item(item.label, action)?;
                self.close_main_menu();
                Ok(MenuClickOutcome::Activated(Some(outcome)))
            }
            MenuAction::Exit => {
                let outcome = self.quit_from_menu()?;
                self.close_main_menu();
                Ok(MenuClickOutcome::Activated(Some(outcome)))
            }
            MenuAction::NoAction => {
                self.close_main_menu();
                Ok(MenuClickOutcome::Activated(None))
            }
            MenuAction::Submenu(id) => {
                self.note(format!(
                    "main menu \"{}\": no submenu opens \
                     (S04 §9 leaves submenu contents unmeasured)",
                    item.label
                ));
                Ok(MenuClickOutcome::SubmenuDeferred(id))
            }
        }
    }

    /// The `Exit` row: submits `Command::Quit`, the same command the
    /// tray's Exit entry sends, so the app shuts down through its usual path.
    fn quit_from_menu(&mut self) -> Result<MenuOutcome> {
        let commands = vec![Command::Quit];
        self.forward(&commands)?;
        Ok(MenuOutcome::Commands(commands))
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
                // Forward exactly as the key path does: `forward` hands each
                // command to the layer that owns it (the UI machine for its own,
                // the engine or the app for the rest). Submitting here instead
                // sent every command down the app channel, so a UI-owned toggle
                // never reached the UI and its check mark never moved.
                self.forward(&emitted)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_core::audio::DecoderRegistry;
    use rustamp_core::config::Config;
    use rustamp_ui::menu::{MAIN_MENU, MainMenuId, MenuRow, row_bands};

    use crate::app::AppOptions;
    use crate::cli::OutputChoice;

    fn test_app() -> App {
        let options = AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            file: None,
            seconds: None,
        };
        App::with_registry(options, DecoderRegistry::new()).unwrap()
    }

    /// The popup-client middle of the band carrying `id`.
    fn row_middle(id: MainMenuId) -> (i32, i32) {
        let index = MAIN_MENU
            .iter()
            .position(|row| matches!(row, MenuRow::Item(item) if item.id == id))
            .unwrap();
        let (top, bottom) = row_bands()[index];
        (10, ((top + bottom) / 2) as i32)
    }

    #[test]
    fn opening_and_closing_tracks_the_popup_state() {
        let mut app = test_app();
        assert!(!app.main_menu_is_open());
        app.open_main_menu();
        assert!(app.main_menu_is_open());
        // Opening twice keeps it open.
        app.open_main_menu();
        assert!(app.main_menu_is_open());
        app.close_main_menu();
        assert!(!app.main_menu_is_open());
        // Closing twice stays closed.
        app.close_main_menu();
        assert!(!app.main_menu_is_open());
    }

    #[test]
    fn the_open_command_opens_the_popup_without_a_note() {
        let mut app = test_app();
        app.submit(Command::OpenMainMenu).unwrap();
        assert!(app.main_menu_is_open());
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn clicking_a_leaf_activates_through_the_key_path_and_closes() {
        // `View file info` (S03 §1 id 40188) emits `OpenFileInfo`, which the
        // app's dialog arm opens — the same effect as pressing Alt+3.
        let mut app = test_app();
        app.open_main_menu();
        let (x, y) = row_middle(MainMenuId::ViewFileInfo);
        let outcome = app.menu_click(x, y).unwrap();
        assert!(matches!(
            outcome,
            MenuClickOutcome::Activated(Some(MenuOutcome::Commands(_)))
        ));
        assert!(!app.main_menu_is_open());
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::FileInfo));
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn clicking_exit_quits_the_player() {
        let mut app = test_app();
        app.open_main_menu();
        let (x, y) = row_middle(MainMenuId::Exit);
        let outcome = app.menu_click(x, y).unwrap();
        assert_eq!(
            outcome,
            MenuClickOutcome::Activated(Some(MenuOutcome::Commands(vec![Command::Quit])))
        );
        assert!(!app.main_menu_is_open());
        assert!(app.drain_notes().is_empty());
        assert!(app.quit(), "Exit must quit the player");
    }

    #[test]
    fn clicking_a_no_action_row_closes_without_dispatching() {
        // `Library` has no S03 row (S04 §2): the popup closes, nothing is
        // submitted, no note is recorded.
        let mut app = test_app();
        app.open_main_menu();
        let (x, y) = row_middle(MainMenuId::Library);
        let outcome = app.menu_click(x, y).unwrap();
        assert_eq!(outcome, MenuClickOutcome::Activated(None));
        assert!(!app.main_menu_is_open());
        assert!(app.drain_notes().is_empty());
        assert!(!app.quit());
    }

    #[test]
    fn clicking_a_submenu_parent_opens_nothing_and_keeps_the_popup() {
        // S04 §9: submenu contents were never measured, so `Play` opens
        // nothing and says so; the popup stays open.
        let mut app = test_app();
        app.open_main_menu();
        let (x, y) = row_middle(MainMenuId::Play);
        let outcome = app.menu_click(x, y).unwrap();
        assert_eq!(
            outcome,
            MenuClickOutcome::SubmenuDeferred(rustamp_ui::menu::SubmenuId::Play)
        );
        assert!(app.main_menu_is_open());
        let notes = app.drain_notes();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("\"Play\""), "{notes:?}");
    }

    #[test]
    fn clicking_a_separator_or_outside_point_is_ignored() {
        let mut app = test_app();
        app.open_main_menu();
        // The first separator band (S04 §2, rule at y=40).
        let (top, bottom) = row_bands()[1];
        let middle = ((top + bottom) / 2) as i32;
        assert_eq!(
            app.menu_click(10, middle).unwrap(),
            MenuClickOutcome::Ignored
        );
        assert!(app.main_menu_is_open());
        // Out-of-bounds points are ignored here; dismissal outside the popup
        // is the presenter's path (it alone sees other windows).
        for (x, y) in [(-1, 10), (10, -1), (323, 10), (10, 569)] {
            assert_eq!(app.menu_click(x, y).unwrap(), MenuClickOutcome::Ignored);
        }
        assert!(app.main_menu_is_open());
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn the_menu_frame_renders_at_the_measured_size() {
        let app = test_app();
        let image = app.render_menu();
        assert_eq!(
            (image.width(), image.height()),
            rustamp_ui::menu::MAIN_MENU_CLIENT_SIZE
        );
    }
}

#[cfg(test)]
mod clutter_tests {
    use super::*;
    use rustamp_core::audio::DecoderRegistry;
    use rustamp_core::config::Config;

    use crate::app::AppOptions;
    use crate::cli::OutputChoice;

    fn test_app() -> App {
        let options = AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            file: None,
            seconds: None,
        };
        App::with_registry(options, DecoderRegistry::new()).unwrap()
    }

    #[test]
    fn the_clutter_commands_queue_their_popup_once() {
        let mut app = test_app();
        assert_eq!(app.take_clutter_menu(), None);
        app.submit(Command::OpenOptionsMenu).unwrap();
        assert_eq!(app.take_clutter_menu(), Some(ClutterMenu::Options));
        assert_eq!(app.take_clutter_menu(), None);
        app.submit(Command::OpenVisualizationMenu).unwrap();
        assert_eq!(app.take_clutter_menu(), Some(ClutterMenu::Visualization));
    }

    #[test]
    fn the_time_rows_set_the_reading_and_never_clear_it() {
        use rustamp_ui::menu::MenuChoice;
        let mut app = test_app();
        assert!(!app.state().time_remaining);
        // Choosing the row that is already on keeps it on.
        app.activate_menu_choice(MenuChoice::SetTimeRemaining(false))
            .unwrap();
        assert!(!app.state().time_remaining);
        app.activate_menu_choice(MenuChoice::SetTimeRemaining(true))
            .unwrap();
        assert!(app.state().time_remaining);
        app.activate_menu_choice(MenuChoice::SetTimeRemaining(true))
            .unwrap();
        assert!(app.state().time_remaining);
        let menu = app.native_clutter_menu(ClutterMenu::Options);
        assert!(menu.rows.iter().any(|row| matches!(
            row,
            rustamp_platform::PopupRow::Entry { label, checked: true, .. }
                if label == "Time remaining"
        )));
        app.activate_menu_choice(MenuChoice::SetTimeRemaining(false))
            .unwrap();
        assert!(!app.state().time_remaining);
    }
}
