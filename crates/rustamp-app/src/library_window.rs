// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The media library: the app's index, its view, the glue between them, the
//! input map and the background scan.
//!
//! # What lives here
//!
//! The media library is a **modern** window: it has a native title bar, a
//! system font and its own chrome, so no skin sheet applies to it (see
//! [`rustamp_ui::library_render`]). The OS window itself belongs to the
//! presenter (`crate::window`); this module is the app side of it:
//!
//! * the **index** ([`rustamp_library::Library`]): the SQLite file the tracks
//!   live in. It is opened when the window opens for the first time and kept
//!   for the rest of the run.
//! * the **view** ([`rustamp_ui::LibraryView`]): the page of rows, the search
//!   text, the sort column, the selection and the scroll. It is headless: it
//!   never opens a database and never runs a query.
//! * the **glue**: the view's request becomes the index's query, and the
//!   index's rows become the view's page ([`LibraryWindow::refresh`]).
//! * the **background scan**: the app owns the thread. The index travels to
//!   the worker and comes back with it ([`LibraryWindow::start_scan`]).
//!
//! # Where the database lives
//!
//! `library.sqlite3` **beside the configuration file** - the same directory
//! as `rustamp.toml`, which is `%APPDATA%\Rustamp` by default. The index is
//! therefore found where the rest of the player's state is, travels with it
//! and is removed with it. A player without a configuration file (no
//! `%APPDATA%`) has nowhere to keep one and uses an **in-memory** index
//! instead, which lives exactly as long as the run.
//!
//! # The input map (own choice)
//!
//! No spec measures the media library (S20 is not started), so the map below
//! is ours. It follows the shape of the window: a search field above a list.
//!
//! | Input | Effect |
//! | --- | --- |
//! | a printable character | appends to the search text, while the search field has the focus |
//! | `Backspace` | deletes the last character of the search text, while the field has the focus |
//! | `Tab` | moves the focus between the search field and the list |
//! | `Up` / `Down` | moves the selection one row |
//! | `PageUp` / `PageDown` | moves the selection one screenful |
//! | `Home` / `End` | moves the selection to the first / last row |
//! | `Enter` | plays the selected row (the existing `AddItems` path) |
//! | `Escape` | stops the running scan |
//! | `F5` | scans the registered roots again |
//! | `Ctrl+F` | focuses the search field |
//! | `Ctrl+O` | asks for a folder to add to the library |
//!
//! The movement keys always move the selection, whether the search field has
//! the focus or not: the field holds one line of text, and the list is what
//! the window is for.
//!
//! # The covers
//!
//! The list has a cover column: it shows the album art of the row. The art
//! comes from the **artwork cache** ([`ArtworkCache`]), which the app owns
//! and which lives as long as the library window does.
//!
//! Every frame asks the cache for the cover of every visible row and lends
//! the answers to the renderer as **borrowed views** ([`CoverRef`]): the
//! renderer reads the thumbnail's own bytes, so nothing is copied per frame.
//!
//! A track without a cover is drawn without one, and a cover that cannot be
//! read is drawn as no cover as well: the failure is reported once, and the
//! frame goes on.
//!
//! The cache notices a changed file by the file's `(mtime, size)`, so a
//! rescan that changed a track invalidates its cover on its own. The app
//! invalidates nothing.
//!
//! The cover of a row is read and decoded the first time the row is drawn,
//! and the frame that draws it waits for it. The frames after that are
//! served from the cache.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use rustamp_core::command::Command;
use rustamp_core::error::{Error, Result};
use rustamp_core::playlist::MediaLocation;
use rustamp_core::state::WindowId;
use rustamp_gfx::{Image, Rect};
use rustamp_library::{
    ArtworkCache, Cancel, CoverImage, Library, Query, Root, ScanPhase, ScanProgress, SortKey,
    TrackRow,
};
use rustamp_ui::library_render::{CoverRef, LIBRARY_CELL_WIDTH, LIBRARY_LINE_HEIGHT};
use rustamp_ui::{
    Key, LibraryChrome, LibraryColumn, LibraryLayout, LibraryRequest, LibraryRow, LibrarySort,
    LibraryView, Modifiers, scroll_for_click,
};

use crate::app::App;

#[cfg(test)]
#[path = "library_window_tests.rs"]
mod library_window_tests;

/// The number of the media library's window, in the `Gen` range of
/// [`WindowId`].
///
/// The ASCII of `LIBR`, so it cannot be mistaken for a plugin's own window
/// number and cannot collide with the small numbers a plugin would use.
pub const LIBRARY_WINDOW_NUMBER: u32 = 0x4C49_4252;

/// The window id of the media library.
///
/// **Own choice.** S04 records the `Library` menu row (S04 §2 row 8) and its
/// accelerator hint `Alt+L` (S04 §3.3), but no public command id for it: S03
/// has no row for it. The window therefore takes an id of its own, in the
/// `Gen` range that [`WindowId`] reserves for "generic / plugin window
/// (`gen_*` plugins, library, ...)".
pub const LIBRARY_WINDOW_ID: WindowId = WindowId::Gen(LIBRARY_WINDOW_NUMBER);

/// The client size the library window opens with, in physical pixels.
///
/// **Own choice:** S20 (the media library) is not started, so no spec
/// measures the window's size. The window is resizable, so this is only the
/// size it opens with.
pub const LIBRARY_CLIENT_SIZE: (u32, u32) = (800, 480);

/// The file name of the library database.
///
/// The database lives **beside the configuration file** (see the module
/// docs), so it is found where the rest of the player's state is.
pub const LIBRARY_DATABASE_NAME: &str = "library.sqlite3";

/// A running scan: the index is with the worker until it comes back.
///
/// The worker owns the index while it scans: a scan writes to the database,
/// and the app must not query it while that happens.
struct ScanWorker {
    /// The flag the scan checks between two files.
    cancel: Cancel,
    /// The latest progress event.
    ///
    /// One slot, not a queue: the interface shows the newest value and the
    /// older ones are worthless.
    progress: Arc<Mutex<Option<ScanProgress>>>,
    /// The worker thread; it gives the index back when it is done.
    handle: JoinHandle<ScanOutcome>,
}

/// What the worker gives back: the index and the scan's outcome.
struct ScanOutcome {
    /// The index, with the rows the scan committed.
    index: Library,
    /// The scan's result: the first error that stopped it.
    result: Result<()>,
}

/// The app's media library: the index, the view and the scan.
pub struct LibraryWindow {
    /// The index, while no scan holds it.
    index: Option<Library>,
    /// The running scan (the index is with the worker while it runs).
    worker: Option<ScanWorker>,
    /// The headless view model: the page, the search, the selection.
    view: LibraryView,
    /// The client size, in physical pixels.
    size: (u32, u32),
    /// Whether the search field has the focus.
    search_focused: bool,
    /// Whether the window is open.
    open: bool,
    /// A "pick a folder" request for the presenter.
    folder_request: bool,
    /// The last failure the user should see; it is shown in the caption.
    error: Option<String>,
    /// The registered roots, as the preferences page shows them.
    ///
    /// The roots live in the index. This is the list the app read last, and
    /// it answers while a scan holds the index: a scan does not change the
    /// roots, so the list is still the truth.
    roots: Vec<PathBuf>,
    /// The decoded covers of the rows the window shows.
    ///
    /// The cache is the app's: it is asked once per frame and it keeps the
    /// answers, so a cover is read from the file only once. It notices a
    /// changed file by the file's stat, so a rescan that changed a track
    /// invalidates its cover without any work here.
    artwork: ArtworkCache,
    /// The cover failure of the last frame, if there was one.
    cover_failure: Option<String>,
    /// The cover failure the app was told about already.
    ///
    /// The failure repeats with every frame while it lasts, so it is
    /// reported once.
    noted_cover_failure: Option<String>,
}

impl Default for LibraryWindow {
    fn default() -> Self {
        let mut window = Self {
            index: None,
            worker: None,
            view: LibraryView::new(1),
            size: LIBRARY_CLIENT_SIZE,
            search_focused: true,
            open: false,
            folder_request: false,
            error: None,
            roots: Vec::new(),
            artwork: ArtworkCache::new(),
            cover_failure: None,
            noted_cover_failure: None,
        };
        window.update_viewport();
        window
    }
}

impl Drop for LibraryWindow {
    fn drop(&mut self) {
        // The worker owns the index: it has to be stopped and joined before
        // the app goes away, or the thread would outlive the database.
        if let Some(worker) = self.worker.take() {
            worker.cancel.cancel();
            let _ = worker.handle.join();
        }
    }
}

impl LibraryWindow {
    /// Whether the window is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The view model: the page, the search text, the selection.
    pub fn view(&self) -> &LibraryView {
        &self.view
    }

    /// The client size of the window, in physical pixels.
    pub fn client_size(&self) -> (u32, u32) {
        self.size
    }

    /// The latest scan progress, if a scan is running.
    pub fn progress(&self) -> Option<ScanProgress> {
        self.worker.as_ref()?.progress.lock().ok()?.clone()
    }

    /// The native caption of the window.
    ///
    /// The caption carries what the window itself cannot: the failure that
    /// stopped the last scan, or the progress of the running one.
    pub fn title(&self) -> String {
        if let Some(error) = &self.error {
            return format!("Rustamp Library - {error}");
        }
        if let Some(progress) = self.progress() {
            let phase = match progress.phase {
                ScanPhase::Scanning => "scanning",
                ScanPhase::Removing => "removing",
                ScanPhase::Finished => "finished",
            };
            return format!(
                "Rustamp Library - {phase}: {} files (Esc stops the scan)",
                progress.seen
            );
        }
        "Rustamp Library".to_string()
    }

    /// The viewport of the window: its whole client area.
    fn viewport(&self) -> Rect {
        Rect::new(0, 0, self.size.0, self.size.1)
    }

    /// The chrome of the window: a pure function of the viewport and the view.
    fn chrome(&self) -> LibraryChrome {
        LibraryChrome::new(&self.view, self.viewport())
    }

    /// The column layout, sized to the list band.
    ///
    /// The layout counts its columns in text cells, so it is sized to the
    /// band the list is drawn in, not to the window.
    fn layout(&self) -> LibraryLayout {
        LibraryLayout::default().resized_to(self.chrome().rows.width / LIBRARY_CELL_WIDTH)
    }

    /// Keeps the view's viewport - the number of rows the window shows - in
    /// step with the window's height.
    fn update_viewport(&mut self) {
        let rows = (self.chrome().rows.height / LIBRARY_LINE_HEIGHT) as usize;
        self.view.set_viewport(rows);
    }

    /// Opens or closes the window.
    ///
    /// The index is opened on the first opening: `database` is the path of
    /// the SQLite file, and `None` means an in-memory index (a player
    /// without a configuration file has nowhere to keep one).
    pub fn toggle(&mut self, database: Option<&Path>) -> Result<()> {
        if self.open {
            return self.close();
        }
        self.ensure_index(database)?;
        self.open = true;
        self.refresh()
    }

    /// Opens the index if it is not open yet.
    ///
    /// The library window opens the index when it opens; the preferences
    /// page needs the roots before that, so the app asks for the index here.
    /// A running scan holds the index: it is not opened a second time, and
    /// the scan is left alone.
    pub fn ensure_index(&mut self, database: Option<&Path>) -> Result<()> {
        if self.index.is_none() && self.worker.is_none() {
            self.index = Some(Self::open_index(database)?);
            let roots = self.read_roots(database)?;
            self.roots = roots;
        }
        Ok(())
    }

    /// Closes the window (its own close box).
    ///
    /// The scan the window showed is stopped: the user cannot watch it any
    /// more, and a scan that runs on behind a closed window is a surprise.
    pub fn close(&mut self) -> Result<()> {
        if !self.open {
            return Ok(());
        }
        self.open = false;
        self.folder_request = false;
        self.cancel_scan();
        self.refresh()
    }

    /// Opens the index at `database`, or an in-memory one.
    fn open_index(database: Option<&Path>) -> Result<Library> {
        let Some(database) = database else {
            return Library::in_memory();
        };
        if let Some(parent) = database
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        Library::open(database)
    }

    /// Runs the view's request against the index and hands the rows back.
    ///
    /// The request is scroll-aligned: the view asks for the rows at its
    /// scroll offset, and the app loads exactly those. A match set that
    /// shrank clamps the scroll, and the clamped offset is asked for again -
    /// at most twice, so the loop cannot run away.
    pub fn refresh(&mut self) -> Result<()> {
        let Some(index) = &self.index else {
            // A scan holds the index: the view keeps the page it has.
            return Ok(());
        };
        for _ in 0..2 {
            let request = self.view.request();
            let page = index.query(&query_of(&request))?;
            let rows = page.rows.into_iter().map(library_row).collect();
            self.view
                .apply_page(rows, page.total, request.offset as usize);
            if self.view.scroll() == request.offset as usize {
                break;
            }
        }
        Ok(())
    }

    /// Updates the client size and loads the newly visible page.
    pub fn resize(&mut self, size: (u32, u32)) -> Result<()> {
        self.size = size;
        self.update_viewport();
        self.refresh()
    }

    /// Paints the window: the page, the chrome and the status line.
    pub fn render(&mut self) -> Image {
        let mut frame = Image::new(self.size.0, self.size.1);
        // The covers of the visible rows: the cache's answers, lent to the
        // renderer as borrowed views. The answers are kept in `owns`, which
        // outlives the call, so the pixels are read where they already lie.
        let mut owns: Vec<Option<Arc<CoverImage>>> = Vec::new();
        let (covers, failure) = covers_of(&self.artwork, self.view.visible_rows(), &mut owns);
        self.cover_failure = failure;
        rustamp_ui::render_library_window(
            &mut frame,
            &self.view,
            &self.layout(),
            self.viewport(),
            self.search_focused,
            &covers,
        );
        frame
    }

    /// The cover failure of the last frame, if it is a new one.
    ///
    /// The failure repeats with every frame while it lasts, so the app is
    /// told about it once: a note per frame would drown the presenter's
    /// list.
    pub fn take_cover_failure(&mut self) -> Option<String> {
        if self.cover_failure == self.noted_cover_failure {
            return None;
        }
        self.noted_cover_failure = self.cover_failure.clone();
        self.cover_failure.take()
    }

    /// A left-button press at the client coordinates `(x, y)`.
    ///
    /// The press is mapped to the band it lands in: the search field takes
    /// the focus, a column header sorts by that column, the scrollbar
    /// scrolls to the place the press sits at, and a row is selected.
    pub fn click(&mut self, x: i32, y: i32) -> Result<()> {
        let chrome = self.chrome();
        if inside(chrome.search, x, y) {
            self.search_focused = true;
            return Ok(());
        }
        self.search_focused = false;
        if inside(chrome.header, x, y) {
            self.sort_by_column(x);
            return self.refresh();
        }
        if inside(chrome.scrollbar, x, y) {
            self.view
                .scroll_to(scroll_for_click(&self.view, &chrome, y));
            return self.refresh();
        }
        if inside(chrome.rows, x, y) {
            let row = ((y - chrome.rows.y) as u32 / LIBRARY_LINE_HEIGHT) as usize;
            let absolute = self.view.scroll() + row;
            self.select_absolute(absolute)?;
        }
        self.refresh()
    }

    /// Sorts by the column the press at `x` landed in.
    fn sort_by_column(&mut self, x: i32) {
        let layout = self.layout();
        for column in layout.columns() {
            let start = layout.column_x(column) as i32 * LIBRARY_CELL_WIDTH as i32;
            let end = start + layout.column_width(column) as i32 * LIBRARY_CELL_WIDTH as i32;
            if x >= start && x < end {
                self.view.toggle_sort(sort_of(column));
                return;
            }
        }
    }

    /// The text typed into the search field.
    ///
    /// Only the field that has the focus takes the text. Control characters
    /// never reach it: they are the keyboard's business, not the search's.
    pub fn text(&mut self, text: &str) -> Result<()> {
        if !self.search_focused {
            return Ok(());
        }
        let mut search = self.view.text().to_owned();
        search.extend(text.chars().filter(|character| !character.is_control()));
        if search == self.view.text() {
            return Ok(());
        }
        self.view.set_search(&search);
        self.refresh()
    }

    /// The library's own key map.
    ///
    /// Returns the commands the key produced: `Enter` plays the selected row
    /// through the existing `AddItems` path, so the track is queued and
    /// started like any other.
    pub fn key(&mut self, key: Key, modifiers: Modifiers) -> Result<Vec<Command>> {
        if modifiers == Modifiers::CTRL {
            match key {
                Key::Letter(b'F') => self.search_focused = true,
                Key::Letter(b'O') => self.folder_request = true,
                _ => {}
            }
            return Ok(Vec::new());
        }
        if modifiers != Modifiers::NONE {
            // Alt and Shift belong to the player's own shortcuts.
            return Ok(Vec::new());
        }
        match key {
            Key::Tab => self.search_focused = !self.search_focused,
            Key::Escape => self.cancel_scan(),
            Key::Backspace if self.search_focused => {
                let mut search = self.view.text().to_owned();
                search.pop();
                self.view.set_search(&search);
                self.refresh()?;
            }
            Key::Enter => {
                if let Some(row) = self.view.selected_row() {
                    return Ok(vec![Command::AddItems {
                        items: vec![MediaLocation::File(row.path.clone())],
                        at: None,
                        play: true,
                    }]);
                }
            }
            Key::Up | Key::Down | Key::PageUp | Key::PageDown | Key::Home | Key::End => {
                if let Some(target) = self.selection_target(key) {
                    self.select_absolute(target)?;
                    // The selection scrolls the row into sight, so the page
                    // is loaded again at the offset the view ended up at.
                    self.refresh()?;
                }
            }
            Key::F(5) => self.rescan()?,
            _ => {}
        }
        Ok(Vec::new())
    }

    /// The absolute row index a movement key asks for.
    ///
    /// `None` when there is nothing to move to: an empty match set has no
    /// rows at all. With no selection the movement starts at the visible
    /// rows, like the playlist editor does: `Down` takes the first visible
    /// row and `Up` the last one.
    fn selection_target(&self, key: Key) -> Option<usize> {
        let total = self.view.total();
        if total == 0 {
            return None;
        }
        let selected = self
            .view
            .selected_index()
            .map(|index| self.view.offset() + index);
        let page = self.view.viewport().max(1);
        let last_visible = (self.view.scroll() + page).min(total) - 1;
        let target = match key {
            Key::Up => selected.map_or(last_visible, |row| row.saturating_sub(1)),
            Key::Down => selected.map_or(self.view.scroll(), |row| (row + 1).min(total - 1)),
            Key::PageUp => selected.map_or(self.view.scroll(), |row| row.saturating_sub(page)),
            Key::PageDown => selected.map_or(self.view.scroll(), |row| (row + page).min(total - 1)),
            Key::Home => 0,
            Key::End => total - 1,
            _ => return None,
        };
        Some(target)
    }

    /// Selects the row at the absolute index `absolute` of the match set.
    ///
    /// The view knows only the rows it has loaded, so a row outside the
    /// loaded page is reached by scrolling to it and loading the page it is
    /// on: the app loads more rows and the movement continues.
    ///
    /// The caller loads the page again afterwards: the selection scrolls the
    /// row into sight, and the page follows the scroll.
    fn select_absolute(&mut self, absolute: usize) -> Result<()> {
        if absolute < self.view.offset() {
            return Ok(());
        }
        let index = absolute - self.view.offset();
        if !self.view.select_index(index) {
            // The row is not on the loaded page: scroll to it and load it.
            self.view.scroll_to(absolute);
            self.refresh()?;
            let index = absolute.saturating_sub(self.view.offset());
            self.view.select_index(index);
        }
        Ok(())
    }

    /// The mouse wheel: a movement of `rows` rows.
    pub fn scroll(&mut self, rows: isize) -> Result<()> {
        self.view.scroll_by(rows);
        self.refresh()
    }

    /// Registers `path` as a root and scans it.
    pub fn scan_folder(&mut self, path: PathBuf) -> Result<()> {
        // A running scan holds the index: it is stopped and taken back
        // first, so the root can be registered.
        self.stop_scan();
        let Some(index) = self.index.as_mut() else {
            return Err(Error::NotFound("the library index is not open".into()));
        };
        index.add_root(&path)?;
        // The new root is in the index now: the list the app keeps says so
        // before the scan takes the index away.
        let roots = root_paths(index.roots()?);
        self.roots = roots;
        self.start_scan(vec![path])
    }

    /// The registered roots, oldest first.
    ///
    /// The index that is open answers. Without one - the library window was
    /// never opened, or a scan holds the index - the list the app read last
    /// answers: a scan does not change the roots, so the list is still the
    /// truth.
    pub fn roots(&self) -> Result<Vec<PathBuf>> {
        match self.index.as_ref() {
            Some(index) => Ok(root_paths(index.roots()?)),
            None => Ok(self.roots.clone()),
        }
    }

    /// Reads the registered roots into the list the app keeps.
    ///
    /// The index that is open answers; without one a short connection to
    /// the same database file answers. A player whose library has no
    /// database file yet has no roots, and the file is not created just to
    /// read them.
    pub fn refresh_roots(&mut self, database: Option<&Path>) -> Result<()> {
        let roots = self.read_roots(database)?;
        self.roots = roots;
        Ok(())
    }

    /// Forgets the root `path` and the tracks indexed under it.
    ///
    /// A running scan is stopped first: a scan of the removed root would
    /// index its tracks again, and the root would leave tracks behind that
    /// no later scan removes (only the registered roots are scanned).
    ///
    /// The index that is open answers; without one a short connection to
    /// the same database file answers.
    pub fn remove_root(&mut self, database: Option<&Path>, path: &Path) -> Result<()> {
        self.stop_scan();
        if let Some(index) = self.index.as_mut() {
            index.remove_root(path)?;
        } else {
            let Some(database) = database else {
                return Err(Error::NotFound(
                    "the media library has no index to remove a root from".into(),
                ));
            };
            let mut index = Library::open(database)?;
            index.remove_root(path)?;
        }
        self.refresh_roots(database)
    }

    /// Asks the presenter for a folder to add to the library.
    ///
    /// The request is drained from the tick, never from a window procedure:
    /// the folder dialog runs its own modal message loop.
    pub fn request_folder(&mut self) {
        self.folder_request = true;
    }

    /// The roots of the index, read from the index that is open or from a
    /// short connection to the database file.
    fn read_roots(&self, database: Option<&Path>) -> Result<Vec<PathBuf>> {
        if let Some(index) = self.index.as_ref() {
            return Ok(root_paths(index.roots()?));
        }
        let Some(database) = database else {
            // Without a database file the roots live in the in-memory index,
            // which is only there while the library window holds it.
            return Ok(Vec::new());
        };
        if !database.is_file() {
            // The library was never used: it has no roots, and the database
            // file is not created just to read them.
            return Ok(Vec::new());
        }
        let index = Library::open(database)?;
        Ok(root_paths(index.roots()?))
    }

    /// Scans the registered roots again.
    pub fn rescan(&mut self) -> Result<()> {
        let Some(index) = &self.index else {
            return Ok(());
        };
        let roots = root_paths(index.roots()?);
        self.start_scan(roots)
    }

    /// Takes the "pick a folder" request, if the library made one.
    pub fn take_folder_request(&mut self) -> bool {
        std::mem::take(&mut self.folder_request)
    }

    /// Starts a scan of `roots` on the app's own worker thread.
    ///
    /// The index travels to the worker and comes back with it. While the
    /// scan runs the app cannot query the index, so the view keeps the page
    /// it has and is refreshed when the worker returns.
    fn start_scan(&mut self, roots: Vec<PathBuf>) -> Result<()> {
        if roots.is_empty() {
            return Ok(());
        }
        // A running scan is stopped first: the index is needed here.
        self.stop_scan();
        let Some(index) = self.index.take() else {
            return Ok(());
        };
        let cancel = Cancel::new();
        let progress = Arc::new(Mutex::new(None));
        let worker_cancel = cancel.clone();
        let worker_progress = Arc::clone(&progress);
        let handle = std::thread::spawn(move || {
            let mut outcome = ScanOutcome {
                index,
                result: Ok(()),
            };
            for root in roots {
                if worker_cancel.is_cancelled() {
                    break;
                }
                let scanned = outcome
                    .index
                    .scan_with_progress(&root, &worker_cancel, |event| {
                        if let Ok(mut slot) = worker_progress.lock() {
                            *slot = Some(event);
                        }
                    });
                if let Err(error) = scanned {
                    outcome.result = Err(error);
                    break;
                }
            }
            outcome
        });
        self.error = None;
        self.worker = Some(ScanWorker {
            cancel,
            progress,
            handle,
        });
        Ok(())
    }

    /// Asks the running scan to stop.
    ///
    /// The scan stops at the next file boundary, so the batches that were
    /// committed stay in the index and the rest is picked up by the next
    /// scan.
    fn cancel_scan(&self) {
        if let Some(worker) = &self.worker {
            worker.cancel.cancel();
        }
    }

    /// Stops the running scan and takes the index back.
    fn stop_scan(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        worker.cancel.cancel();
        self.take_back(worker);
    }

    /// Takes the index back from a finished worker.
    fn take_back(&mut self, worker: ScanWorker) {
        match worker.handle.join() {
            Ok(outcome) => {
                self.index = Some(outcome.index);
                if let Err(error) = outcome.result {
                    self.error = Some(error.to_string());
                }
            }
            Err(_) => {
                // The worker took the index with it, and a pan left it in an
                // unknown state: the index is gone until the window is
                // reopened.
                self.error = Some("the library scan worker panicked; reopen the library".into());
            }
        }
    }

    /// Takes the index back when the worker is done.
    ///
    /// The app calls this from its pump: the scan runs on its own thread,
    /// and the interface only learns about it when the worker is finished.
    pub fn poll(&mut self) -> Result<()> {
        if !self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.handle.is_finished())
        {
            return Ok(());
        }
        let worker = self.worker.take().expect("the finished worker");
        self.take_back(worker);
        self.refresh()
    }
}

/// The paths of the registered roots.
fn root_paths(roots: Vec<Root>) -> Vec<PathBuf> {
    roots.into_iter().map(|root| root.path).collect()
}

/// The covers of the rows, as borrowed views of the cache's answers.
///
/// The answers are kept in `owns`, which has to outlive the returned
/// views: the renderer borrows the pixels, and the cache owns them.
/// Nothing is copied - a view points at the thumbnail's own bytes.
///
/// A row without a cover has no view, and neither has a row whose cover
/// could not be read: the failure is returned next to the views, and the
/// frame is drawn without that cover.
fn covers_of<'a>(
    cache: &ArtworkCache,
    rows: &[LibraryRow],
    owns: &'a mut Vec<Option<Arc<CoverImage>>>,
) -> (Vec<Option<CoverRef<'a>>>, Option<String>) {
    owns.clear();
    let mut failure = None;
    for row in rows {
        match cache.get(&row.path) {
            Ok(cover) => owns.push(cover),
            Err(error) => {
                owns.push(None);
                if failure.is_none() {
                    failure = Some(format!(
                        "the cover of {} could not be read: {error}",
                        row.path.display()
                    ));
                }
            }
        }
    }
    let owns: &'a Vec<Option<Arc<CoverImage>>> = &*owns;
    let covers = owns
        .iter()
        .map(|answer| {
            answer.as_ref().map(|cover| CoverRef {
                width: cover.width,
                height: cover.height,
                pixels: &cover.pixels,
            })
        })
        .collect();
    (covers, failure)
}

/// The index query the view's request asks for.
///
/// The view owns the search term, the sort column and the window of rows;
/// the app adds nothing to it, so the query is the request and nothing else.
fn query_of(request: &LibraryRequest) -> Query {
    Query {
        text: request.text.clone(),
        sort: match request.sort {
            LibrarySort::Id => SortKey::Id,
            LibrarySort::Title => SortKey::Title,
            LibrarySort::Artist => SortKey::Artist,
            LibrarySort::Album => SortKey::Album,
            LibrarySort::Duration => SortKey::Duration,
        },
        descending: request.descending,
        limit: Some(request.limit),
        offset: request.offset,
        ..Query::default()
    }
}

/// The view's row for an index row.
///
/// The view shows four columns; a tag the file does not carry is an empty
/// string, and a length it does not know stays empty.
fn library_row(row: TrackRow) -> LibraryRow {
    LibraryRow {
        id: row.id,
        path: row.path,
        title: row.metadata.title.unwrap_or_default(),
        artist: row.metadata.artist.unwrap_or_default(),
        album: row.metadata.album.unwrap_or_default(),
        duration_ms: row.metadata.duration_ms,
    }
}

/// The sort key a column stands for.
fn sort_of(column: LibraryColumn) -> LibrarySort {
    match column {
        LibraryColumn::Title => LibrarySort::Title,
        LibraryColumn::Artist => LibrarySort::Artist,
        LibraryColumn::Album => LibrarySort::Album,
        LibraryColumn::Duration => LibrarySort::Duration,
    }
}

/// Whether the point `(x, y)` lies inside `rect`.
fn inside(rect: Rect, x: i32, y: i32) -> bool {
    x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom()
}

impl App {
    /// Whether the media library window is open.
    pub fn library_open(&self) -> bool {
        self.library_window.is_open()
    }

    /// The library's view model: the page, the search text, the selection.
    pub fn library_view(&self) -> &LibraryView {
        self.library_window.view()
    }

    /// The client size of the library window, in physical pixels.
    pub fn library_client_size(&self) -> (u32, u32) {
        self.library_window.client_size()
    }

    /// The native caption of the library window.
    pub fn library_title(&self) -> String {
        self.library_window.title()
    }

    /// Renders one frame of the library window.
    ///
    /// A cover that could not be read is noted once: the frame itself is
    /// drawn without that cover, so a broken file never breaks the list.
    pub fn render_library(&mut self) -> Image {
        let image = self.library_window.render();
        if let Some(failure) = self.library_window.take_cover_failure() {
            self.note(format!("Library: {failure}"));
        }
        image
    }

    /// A left-button press in the library window, in client coordinates.
    pub fn library_click(&mut self, x: i32, y: i32) -> Result<()> {
        self.library_window.click(x, y)
    }

    /// The mouse wheel in the library window: a movement of `rows` rows.
    pub fn library_scroll(&mut self, rows: isize) -> Result<()> {
        self.library_window.scroll(rows)
    }

    /// The text typed into the library's search field.
    pub fn library_text(&mut self, text: &str) -> Result<()> {
        self.library_window.text(text)
    }

    /// A key press in the library window.
    ///
    /// The commands the key produced are submitted through the app's own
    /// path, so `Enter` plays the selected row exactly like the playlist
    /// editor plays an entry.
    pub fn library_key_down(&mut self, key: Key, modifiers: Modifiers) -> Result<Vec<Command>> {
        let commands = self.library_window.key(key, modifiers)?;
        self.forward(&commands)?;
        Ok(commands)
    }

    /// The library window was resized to `size`, in physical pixels.
    pub fn library_resize(&mut self, size: (u32, u32)) -> Result<()> {
        self.library_window.resize(size)
    }

    /// Takes the "pick a folder" request, if the library made one.
    pub fn take_library_folder_request(&mut self) -> bool {
        self.library_window.take_folder_request()
    }

    /// Adds a folder to the library and scans it.
    pub fn library_add_folder(&mut self, path: PathBuf) -> Result<()> {
        // The index is opened here when it is not open yet: the folder can
        // be added before the library window was ever opened.
        let database = self.library_database_path();
        self.library_window.ensure_index(database.as_deref())?;
        self.library_window.scan_folder(path)
    }

    /// The media library's roots, as the preferences page shows them.
    ///
    /// The roots live in the media library's own database, not in the
    /// configuration file: the page reads them from the index. A database
    /// that cannot be read leaves the page empty; the failure is reported
    /// when the roots are read for the page (see
    /// [`App::refresh_library_roots`]).
    pub(crate) fn library_roots(&self) -> Vec<String> {
        match self.library_window.roots() {
            Ok(roots) => roots
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Reads the media library's roots for the preferences page.
    ///
    /// The index answers when it is open; without one a short connection to
    /// the database file answers. A player whose library was never used has
    /// no database file, and the file is not created just because the page
    /// was looked at.
    pub fn refresh_library_roots(&mut self) -> Result<()> {
        let database = self.library_database_path();
        self.library_window.refresh_roots(database.as_deref())
    }

    /// Forgets the media library's root `path` and the tracks under it.
    pub(crate) fn remove_library_root(&mut self, path: &Path) -> Result<()> {
        let database = self.library_database_path();
        self.library_window.remove_root(database.as_deref(), path)?;
        // The library window must not show the tracks that are gone now.
        self.library_window.refresh()?;
        Ok(())
    }

    /// Closes the library window (its own close box).
    pub fn close_library_window(&mut self) -> Result<()> {
        self.library_window.close()
    }

    /// The path of the library database: `library.sqlite3` beside the
    /// configuration file.
    ///
    /// `None` when the player has no configuration file: there is then
    /// nowhere to keep the index, and the library uses an in-memory one.
    pub fn library_database_path(&self) -> Option<PathBuf> {
        self.config_path()
            .map(|path| path.with_file_name(LIBRARY_DATABASE_NAME))
    }

    /// Opens or closes the library window.
    pub(crate) fn toggle_library(&mut self) {
        let database = self.library_database_path();
        if let Err(error) = self.library_window.toggle(database.as_deref()) {
            self.note(format!("Library: {error}"));
        }
    }

    /// Takes the index back when the scan worker is done.
    pub(crate) fn poll_library(&mut self) {
        if let Err(error) = self.library_window.poll() {
            self.note(format!("Library: {error}"));
        }
    }
}
