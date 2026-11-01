// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The player's own MilkDrop window ([`rustamp_milkdrop`]), which needs no
//! plug-in. The app owns this model and the session; the presenter owns the OS
//! window (`window/milkdrop.rs`).
//!
//! **Own choices**, since this window has no reference to follow:
//!
//! * `Alt+D` and a row of the Visualization menu open and close it (S03 binds no
//!   other key to `Alt+D`).
//! * Its keys: `Space` the next preset, `Backspace` the one before, `R` a random
//!   one, `F11` or `Alt+Enter` fullscreen, `Escape` out of fullscreen. Every other
//!   key is the player's own shortcut.
//! * The title bar is the skinned strip of the media library
//!   ([`rustamp_ui::chrome`]); fullscreen has none.
//! * The settings are read from the configuration file when the window opens, so
//!   an edit takes effect on the next opening:
//!
//! ```toml
//! [milkdrop]
//! preset_folder = "D:/presets"   # relative to the file; default: the built-in presets
//! preset_seconds = 30            # 0 turns the timed change off
//! random_order = true            # false: the timed change takes the next in the list
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustamp_core::state::WindowId;
use rustamp_core::vis::VisFrame;
use rustamp_gfx::{Image, Rect};
use rustamp_milkdrop::{RenderedFrame, Request, Session, Settings, Status};
use rustamp_skin::Skin;
use rustamp_ui::chrome::TITLE_BAR_HEIGHT;
use rustamp_ui::{Key, Modifiers};

/// The window this module draws.
pub const MILKDROP_WINDOW_ID: WindowId = WindowId::Vis;

/// The whole window's first size: a 4:3 picture under the title bar.
pub const MILKDROP_WINDOW_SIZE: (u32, u32) = (640, 480 + TITLE_BAR_HEIGHT);

/// The name of the settings' table in the configuration file.
const CONFIG_TABLE: &str = "milkdrop";

/// What a key does in this window. A key that is not here is the player's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyAction {
    /// Ask for another preset.
    Preset(Request),
    /// In or out of fullscreen.
    ToggleFullscreen,
    /// Out of fullscreen.
    LeaveFullscreen,
}

/// The meaning of a key in this window.
pub fn key_action(key: Key, modifiers: Modifiers, fullscreen: bool) -> Option<KeyAction> {
    match (key, modifiers) {
        (Key::Space, Modifiers::NONE) => Some(KeyAction::Preset(Request::Next)),
        (Key::Backspace, Modifiers::NONE) => Some(KeyAction::Preset(Request::Previous)),
        (Key::Letter(b'R'), Modifiers::NONE) => Some(KeyAction::Preset(Request::Random)),
        (Key::F(11), Modifiers::NONE) | (Key::Enter, Modifiers::ALT) => {
            Some(KeyAction::ToggleFullscreen)
        }
        (Key::Escape, Modifiers::NONE) if fullscreen => Some(KeyAction::LeaveFullscreen),
        _ => None,
    }
}

/// The settings of the configuration file at `path`, and what was wrong with them.
///
/// The app has already refused a file it cannot parse when it started, so a file
/// that went bad since then only costs the settings, with a note.
pub fn read_settings(path: Option<&Path>) -> (Settings, Vec<String>) {
    let mut settings = Settings::default();
    let mut notes = Vec::new();
    let Some(path) = path else {
        return (settings, notes);
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (settings, notes),
        Err(error) => {
            notes.push(format!("{} cannot be read: {error}", path.display()));
            return (settings, notes);
        }
    };
    let table = match text.parse::<toml::Table>() {
        Ok(table) => table,
        Err(error) => {
            notes.push(format!("{} cannot be parsed: {error}", path.display()));
            return (settings, notes);
        }
    };
    let Some(milkdrop) = table.get(CONFIG_TABLE).and_then(toml::Value::as_table) else {
        return (settings, notes);
    };
    for (key, value) in milkdrop {
        let understood = match key.as_str() {
            "preset_folder" => value.as_str().is_some_and(|folder| {
                let folder = PathBuf::from(folder);
                settings.preset_folder = Some(if folder.is_absolute() {
                    folder
                } else {
                    path.parent()
                        .map_or(folder.clone(), |parent| parent.join(&folder))
                });
                true
            }),
            "preset_seconds" => value
                .as_float()
                .or_else(|| value.as_integer().map(|seconds| seconds as f64))
                .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
                .is_some_and(|seconds| {
                    settings.preset_seconds = seconds;
                    true
                }),
            "random_order" => value.as_bool().is_some_and(|random| {
                settings.random_order = random;
                true
            }),
            _ => false,
        };
        if !understood {
            notes.push(format!(
                "setting '{CONFIG_TABLE}.{key}' is not applied: the value {value} is not understood"
            ));
        }
    }
    (settings, notes)
}

/// The window's model: open or closed, the session, fullscreen.
#[derive(Default)]
pub struct MilkdropWindow {
    session: Option<Session>,
    fullscreen: bool,
    bar: Option<TitleBar>,
}

/// The title bar as last drawn. Text is by far the dearest part of a frame that
/// would otherwise only copy pixels, and it hardly ever changes.
struct TitleBar {
    key: (String, bool, u32, u64),
    image: Image,
}

impl MilkdropWindow {
    /// Whether the window is open.
    pub fn is_open(&self) -> bool {
        self.session.is_some()
    }

    /// Starts a session with `settings` at the window's first size.
    pub fn open(&mut self, settings: Settings) {
        self.session = Some(Session::start(Settings {
            size: body_size(MILKDROP_WINDOW_SIZE, false),
            ..settings
        }));
    }

    /// Stops the session.
    pub fn close(&mut self) {
        self.session = None;
        self.fullscreen = false;
    }

    /// Whether the window should fill the screen.
    pub fn fullscreen(&self) -> bool {
        self.fullscreen
    }

    /// In or out of fullscreen.
    pub fn set_fullscreen(&mut self, fullscreen: bool) {
        self.fullscreen = fullscreen && self.is_open();
    }

    /// Hands audio to the session, if there is one.
    pub fn feed(&self, frame: &VisFrame, source_rate_hz: u32) {
        if let Some(session) = &self.session {
            session.feed(frame, source_rate_hz);
        }
    }

    /// Asks the session for another preset.
    pub fn request(&self, request: Request) {
        if let Some(session) = &self.session {
            session.request(request);
        }
    }

    /// The window's client area changed.
    pub fn resized(&self, size: (u32, u32)) {
        if let Some(session) = &self.session {
            let (width, height) = body_size(size, self.fullscreen);
            session.resize(width, height);
        }
    }

    /// What the session has to say since the last call.
    pub fn take_notes(&self) -> Vec<String> {
        self.session
            .as_ref()
            .map(Session::take_notes)
            .unwrap_or_default()
    }

    /// The newest frame, while frames are what the session has to show.
    fn frame(&self) -> Option<Arc<RenderedFrame>> {
        let session = self.session.as_ref()?;
        match session.status() {
            Status::Running { .. } => session.frame(),
            _ => None,
        }
    }

    /// The newest frame's size and generation, for the diagnostics of a window
    /// that stays black (`RUSTAMP_MILKDROP_DEBUG=1`).
    pub fn frame_shape(&self) -> Option<(u32, u32, u64)> {
        self.frame()
            .map(|frame| (frame.width, frame.height, frame.generation))
    }

    /// Identifies what [`MilkdropWindow::render`] would draw: a change means the
    /// window has to be painted.
    pub fn content_key(&self) -> (u64, Option<Status>) {
        (
            self.frame().map_or(0, |frame| frame.generation),
            self.session.as_ref().map(Session::status),
        )
    }

    /// The title of the bar.
    pub fn title(&self) -> String {
        match self.session.as_ref().map(Session::status) {
            Some(Status::Running { preset }) => format!("MilkDrop - {preset}"),
            _ => "MilkDrop".to_owned(),
        }
    }

    /// The window as `size` pixels: the picture, or the text that says why
    /// there is none, under the title bar.
    pub fn render(
        &mut self,
        size: (u32, u32),
        focused: bool,
        skin: &Skin,
        skin_generation: u64,
    ) -> Image {
        let mut canvas = Image::new(size.0, size.1);
        let top = if self.fullscreen { 0 } else { TITLE_BAR_HEIGHT };
        canvas.fill_rect(Rect::new(0, 0, size.0, size.1), [0, 0, 0, 255]);
        if let Some(frame) = self.frame() {
            blit_scaled(&mut canvas, &frame, top);
        } else if let Some(session) = &self.session {
            let message = match session.status() {
                Status::NoGpu(message) | Status::NoPresets(message) | Status::Failed(message) => {
                    message
                }
                Status::Starting | Status::Running { .. } => "Starting MilkDrop...".to_owned(),
            };
            rustamp_ui::chrome::draw_message(&mut canvas, skin, &message, top as i32 + 12);
        }
        if !self.fullscreen {
            let key = (self.title(), focused, size.0, skin_generation);
            if self.bar.as_ref().is_none_or(|bar| bar.key != key) {
                let mut image = Image::new(size.0, TITLE_BAR_HEIGHT);
                rustamp_ui::chrome::draw_title_bar(&mut image, skin, &key.0, focused);
                self.bar = Some(TitleBar { key, image });
            }
            if let Some(bar) = &self.bar {
                canvas.blit_rect(&bar.image, bar.image.bounds(), 0, 0);
            }
        }
        canvas
    }
}

/// The part of a client area of `size` the picture fills.
pub fn body_size(size: (u32, u32), fullscreen: bool) -> (u32, u32) {
    let bar = if fullscreen { 0 } else { TITLE_BAR_HEIGHT };
    (size.0.max(1), size.1.saturating_sub(bar).max(1))
}

/// Draws `frame` over the canvas below `top`, stretched to fill it. Nearest
/// neighbour: the frame is either the window's own size, or a smaller one from
/// the software adapter, where the cost of anything better would be frames.
fn blit_scaled(canvas: &mut Image, frame: &RenderedFrame, top: u32) {
    let (width, height) = (canvas.width(), canvas.height().saturating_sub(top));
    if width == 0 || height == 0 || frame.width == 0 || frame.height == 0 {
        return;
    }
    let Ok(source) = Image::from_rgba(frame.width, frame.height, frame.rgba.clone()) else {
        return;
    };
    if (frame.width, frame.height) == (width, height) {
        canvas.blit_rect(&source, source.bounds(), 0, top as i32);
        return;
    }
    let row_bytes = width as usize * 4;
    let mut scaled = vec![0u8; row_bytes * height as usize];
    let columns: Vec<usize> = (0..u64::from(width))
        .map(|x| (x * u64::from(frame.width) / u64::from(width)) as usize * 4)
        .collect();
    for (y, row) in scaled.chunks_exact_mut(row_bytes).enumerate() {
        let source_y = y as u64 * u64::from(frame.height) / u64::from(height);
        let start = source_y as usize * frame.width as usize * 4;
        let source_row = &frame.rgba[start..start + frame.width as usize * 4];
        for (pixel, column) in row.as_chunks_mut::<4>().0.iter_mut().zip(&columns) {
            pixel.copy_from_slice(&source_row[*column..*column + 4]);
        }
    }
    if let Ok(image) = Image::from_rgba(width, height, scaled) {
        canvas.blit_rect(&image, image.bounds(), 0, top as i32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32, rgba: Vec<u8>) -> RenderedFrame {
        RenderedFrame {
            generation: 1,
            width,
            height,
            rgba,
        }
    }

    fn settings(text: &str) -> (Settings, Vec<String>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rustamp.toml");
        std::fs::write(&path, text).unwrap();
        read_settings(Some(&path))
    }

    #[test]
    fn the_keys_of_the_window() {
        let action = |key, modifiers| key_action(key, modifiers, false);
        assert_eq!(
            action(Key::Space, Modifiers::NONE),
            Some(KeyAction::Preset(Request::Next))
        );
        assert_eq!(
            action(Key::Backspace, Modifiers::NONE),
            Some(KeyAction::Preset(Request::Previous))
        );
        assert_eq!(
            action(Key::Letter(b'R'), Modifiers::NONE),
            Some(KeyAction::Preset(Request::Random))
        );
        assert_eq!(
            action(Key::F(11), Modifiers::NONE),
            Some(KeyAction::ToggleFullscreen)
        );
        assert_eq!(
            action(Key::Enter, Modifiers::ALT),
            Some(KeyAction::ToggleFullscreen)
        );
    }

    #[test]
    fn every_other_key_stays_the_players() {
        let action = |key, modifiers| key_action(key, modifiers, false);
        assert_eq!(action(Key::Letter(b'X'), Modifiers::NONE), None);
        assert_eq!(action(Key::Letter(b'R'), Modifiers::ALT), None, "repeat");
        assert_eq!(action(Key::Enter, Modifiers::NONE), None);
        assert_eq!(
            action(Key::Escape, Modifiers::NONE),
            None,
            "Escape means something here only in fullscreen"
        );
        assert_eq!(
            key_action(Key::Escape, Modifiers::NONE, true),
            Some(KeyAction::LeaveFullscreen)
        );
    }

    #[test]
    fn no_file_and_no_table_are_the_defaults() {
        let (read, notes) = read_settings(None);
        assert_eq!(read.preset_folder, None);
        assert!(notes.is_empty());
        let (read, notes) = settings("[ui]\ndouble_size = true\n");
        assert_eq!(read.preset_folder, None);
        assert_eq!(read.preset_seconds, Settings::default().preset_seconds);
        assert!(notes.is_empty());
    }

    #[test]
    fn the_settings_are_read_and_a_relative_folder_is_the_files_neighbour() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rustamp.toml");
        std::fs::write(
            &path,
            "[milkdrop]\npreset_folder = 'packs'\npreset_seconds = 12\nrandom_order = false\n",
        )
        .unwrap();
        let (read, notes) = read_settings(Some(&path));
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(read.preset_folder, Some(dir.path().join("packs")));
        assert_eq!(read.preset_seconds, 12.0);
        assert!(!read.random_order);
    }

    #[test]
    fn a_value_that_makes_no_sense_is_reported_and_not_applied() {
        let (read, notes) = settings(
            "[milkdrop]\npreset_seconds = -3\nrandom_order = 'yes'\npreset_folder = 7\nother = 1\n",
        );
        assert_eq!(read.preset_seconds, Settings::default().preset_seconds);
        assert!(read.random_order);
        assert_eq!(read.preset_folder, None);
        assert_eq!(notes.len(), 4, "{notes:?}");
        assert!(notes.iter().any(|note| note.contains("milkdrop.other")));
    }

    #[test]
    fn a_file_that_went_bad_costs_the_settings_with_a_note() {
        let (read, notes) = settings("[milkdrop\n");
        assert_eq!(read.preset_folder, None);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("cannot be parsed"), "{notes:?}");
    }

    #[test]
    fn the_picture_fills_all_but_the_title_bar() {
        assert_eq!(body_size((640, 500), false), (640, 480));
        assert_eq!(body_size((640, 500), true), (640, 500));
        assert_eq!(body_size((0, 3), false), (1, 1), "never empty");
    }

    #[test]
    fn a_frame_of_the_windows_size_is_copied_under_the_bar() {
        let mut canvas = Image::new(2, 3);
        let rgba = vec![
            1, 2, 3, 255, 4, 5, 6, 255, //
            7, 8, 9, 255, 10, 11, 12, 255,
        ];
        blit_scaled(&mut canvas, &frame(2, 2, rgba.clone()), 1);
        assert_eq!(&canvas.as_rgba()[8..], &rgba[..], "row 0 is the bar's");
        assert_eq!(&canvas.as_rgba()[..8], &[0; 8]);
    }

    #[test]
    fn a_smaller_frame_is_stretched_with_its_corners_in_the_corners() {
        let mut canvas = Image::new(4, 4);
        let rgba = vec![
            10, 0, 0, 255, 20, 0, 0, 255, //
            30, 0, 0, 255, 40, 0, 0, 255,
        ];
        blit_scaled(&mut canvas, &frame(2, 2, rgba), 0);
        let reds: Vec<u8> = canvas
            .as_rgba()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[0])
            .collect();
        assert_eq!(
            reds,
            [
                10, 10, 20, 20, 10, 10, 20, 20, 30, 30, 40, 40, 30, 30, 40, 40
            ]
        );
    }

    #[test]
    fn closed_a_window_does_nothing_and_says_nothing() {
        let mut window = MilkdropWindow::default();
        assert!(!window.is_open());
        window.feed(&VisFrame::default(), 44_100);
        window.request(Request::Next);
        window.resized((10, 10));
        assert!(window.take_notes().is_empty());
        assert_eq!(window.title(), "MilkDrop");
        window.set_fullscreen(true);
        assert!(
            !window.fullscreen(),
            "only an open window can fill the screen"
        );
    }

    #[test]
    fn no_graphics_adapter_is_a_message_in_an_open_window() {
        let mut window = MilkdropWindow::default();
        window.open(Settings {
            backends: rustamp_milkdrop::session::NO_BACKENDS,
            ..Settings::default()
        });
        assert!(window.is_open());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !matches!(window.content_key().1, Some(Status::NoGpu(_))) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(window.title(), "MilkDrop");
        assert_eq!(window.content_key().0, 0, "no picture");
        window.close();
        assert!(!window.is_open());
    }
}
