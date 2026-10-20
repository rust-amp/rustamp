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
//! | `Ctrl+M` | removes the rows whose file is gone |
//! | `Ctrl+O` | asks for a folder to add to the library |
//!
//! The movement keys always move the selection, whether the search field has
//! the focus or not: the field holds one line of text, and the list is what
//! the window is for.
//!
//! # The missing entries
//!
//! The list can show rows whose file is gone: the file was deleted or moved,
//! or the drive it was on is not attached. `Ctrl+M` removes those rows from
//! the index - it drops the **row**, never the file, and it needs no full
//! rescan: the index asks the file system about every row it holds under the
//! registered roots ([`Library::remove_missing`]).
//!
//! The chord is an **own choice** (S20 is not started, so nothing measures
//! the media library): `Ctrl+M`, for "missing". It is in neither the main
//! window's table nor the global table, and it collides with nothing the
//! library window binds already.
//!
//! **A running scan refuses the action.** The worker holds the index while it
//! scans, so the removal cannot run beside it, and stopping the scan would
//! throw away the batches it has not committed yet. The window says so and
//! leaves the scan alone; `Escape` stops the scan, and the action works
//! after it.
//!
//! The action reloads the page afterwards: the rows that are gone are not in
//! the list any more and the total is smaller. It leaves a **note** for the
//! app to show: how many rows it looked at, how many it dropped, and which
//! rows it kept because their file could not be checked.
//!
//! # The status line
//!
//! The bottom of the window is the **status line**. It shows the visible
//! range of the list - the `1-20 of 1234` of the view - and, while a scan
//! runs, the scan itself: its phase and its counters.
//!
//! The rules are:
//!
//! * a scan that runs shows its phase and its counters (`scanning: 120
//!   seen, 30 added, 5 updated, 2 skipped`),
//! * a scan that is over shows the visible range again,
//! * a scan that was **cancelled** says so and shows the **committed**
//!   counters (`cancelled: 30 added, 5 updated, 0 removed, 2 skipped
//!   (committed)`). The live counters of a scan include the batch that is
//!   in flight, so they can be ahead of the index by one batch; the report
//!   the scan returns counts the committed batches only, and that is what a
//!   cancelled scan shows.
//!
//! The scan runs on its own thread and its events travel to the interface
//! thread over a channel: the worker never waits for the interface, and the
//! interface never waits for the worker.
//!
//! The text itself belongs to the **view**: the app puts the scan's text
//! into it ([`LibraryView::set_scan_status`]), and the renderer draws the
//! view's status line into the band. The app paints nothing itself.
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
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;

use rustamp_core::command::Command;
use rustamp_core::error::{Error, Result};
use rustamp_core::playlist::MediaLocation;
use rustamp_core::state::WindowId;
use rustamp_gfx::{Image, Rect};
use rustamp_library::{
    ArtworkCache, Cancel, CoverImage, Library, Query, RemoveReport, Root, ScanPhase, ScanProgress,
    ScanReport, SortKey, TrackRow,
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
    /// The progress events the worker sent.
    ///
    /// The events travel over a channel: the worker sends them and the
    /// interface thread takes them, so neither side ever waits for the
    /// other. Only the newest event is kept - the older ones are worthless.
    events: Receiver<ScanProgress>,
    /// The worker thread; it gives the index back when it is done.
    handle: JoinHandle<ScanOutcome>,
}

/// What the worker gives back: the index and the scan's outcome.
struct ScanOutcome {
    /// The index, with the rows the scan committed.
    index: Library,
    /// The scan's result: the report it committed, or the error that
    /// stopped it.
    ///
    /// A cancelled scan still reports what it committed: the batches it
    /// wrote are in the index, and the batch in flight is not.
    result: Result<ScanReport>,
}

/// What the worker runs.
///
/// The app scans the registered roots ([`RootScan`]); the trait is the seam
/// the tests use, because a real scan of a directory is fast and hard to
/// steer, and the window's status line must be proven for every phase and
/// for the cancelled case.
trait ScanJob: Send + 'static {
    /// Runs the scan, reporting what it does through `on_progress`.
    ///
    /// The report describes the **committed** state: a cancelled scan
    /// reports the batches it wrote, not the files it saw.
    fn run(self, cancel: &Cancel, on_progress: &mut dyn FnMut(ScanProgress)) -> ScanOutcome;
}

/// The scan of the registered roots.
///
/// The index travels to the worker and comes back with it: a scan writes to
/// the database, and the app must not query it while that happens.
struct RootScan {
    /// The index, with the rows the scan will commit.
    index: Library,
    /// The roots to scan, in order.
    roots: Vec<PathBuf>,
}

impl ScanJob for RootScan {
    fn run(self, cancel: &Cancel, on_progress: &mut dyn FnMut(ScanProgress)) -> ScanOutcome {
        let RootScan { mut index, roots } = self;
        let mut report = ScanReport::default();
        for root in roots {
            if cancel.is_cancelled() {
                // The scan was stopped before this root: what the earlier
                // roots committed is what the index holds.
                report.cancelled = true;
                break;
            }
            match index.scan_with_progress(&root, cancel, &mut *on_progress) {
                Ok(scanned) => merge_report(&mut report, scanned),
                Err(error) => {
                    return ScanOutcome {
                        index,
                        result: Err(error),
                    };
                }
            }
        }
        ScanOutcome {
            index,
            result: Ok(report),
        }
    }
}

/// Adds the counters of one root's report to the report of the whole scan.
///
/// The report of a scan describes the **committed** state, so the counters
/// of the roots are added up: they are the changes the index really holds.
fn merge_report(report: &mut ScanReport, scanned: ScanReport) {
    report.scanned += scanned.scanned;
    report.added += scanned.added;
    report.updated += scanned.updated;
    report.removed += scanned.removed;
    report.skipped += scanned.skipped;
    report.cancelled |= scanned.cancelled;
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
    /// The note of the last action, for the app to show.
    ///
    /// The window does not show notes itself: the app collects them and the
    /// presenter prints them. An action leaves its message here, and the app
    /// takes it.
    action_note: Option<String>,
    /// The latest progress event of the scan.
    ///
    /// The event arrives on the interface thread, over the channel the
    /// worker sends it on. Only the newest one is kept: the older ones are
    /// worthless.
    scan_progress: Option<ScanProgress>,
    /// The committed report of the scan that was cancelled.
    ///
    /// A cancelled scan keeps the batches it committed and drops the batch
    /// in flight, so the numbers it shows are the ones the report carries -
    /// the changes the index really holds.
    cancelled_scan: Option<ScanReport>,
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
            action_note: None,
            scan_progress: None,
            cancelled_scan: None,
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

    /// The latest scan progress, if a scan has reported one.
    pub fn progress(&self) -> Option<ScanProgress> {
        self.scan_progress.clone()
    }

    /// The text the window's status line shows.
    ///
    /// The rules are the ones of the module docs: a running scan shows its
    /// phase and its counters, a scan that is over shows the visible range
    /// of the list again, and a cancelled scan says so and shows the
    /// committed counters.
    ///
    /// The text is the **view's** ([`LibraryView::status_line`]): the app
    /// puts the scan's text into the view, and the renderer draws it.
    pub fn status_line(&self) -> String {
        self.view.status_line()
    }

    /// What the scan has to say, if anything.
    ///
    /// `None` when the scan has nothing to say: the status line is then the
    /// visible range of the list.
    ///
    /// The progress belongs to a scan that **runs**: the worker is alive and
    /// the scan did not reach its end yet. A scan that was cancelled says so
    /// instead, and it says what it committed.
    fn scan_text(&self) -> Option<String> {
        if self.worker.is_some()
            && let Some(progress) = &self.scan_progress
            && progress.phase != ScanPhase::Finished
        {
            return Some(progress_line(progress));
        }
        self.cancelled_scan.as_ref().map(cancelled_line)
    }

    /// Puts the scan's text into the view.
    ///
    /// The view's status line carries the scan's text while the scan has
    /// something to say, and the visible range of the list otherwise. The
    /// renderer draws the view's status line into the status band, so the
    /// app paints nothing itself.
    fn update_scan_status(&mut self) {
        match self.scan_text() {
            Some(text) => self.view.set_scan_status(&text),
            None => self.view.clear_scan_status(),
        }
    }

    /// The native caption of the window.
    ///
    /// The caption carries what the window itself cannot: the failure that
    /// stopped the last scan, or the progress of the running one.
    pub fn title(&self) -> String {
        if let Some(error) = &self.error {
            return format!("Rustamp Library - {error}");
        }
        match self.view.scan_status() {
            Some(text) => format!("Rustamp Library - {text}"),
            None => "Rustamp Library".to_string(),
        }
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
        // The status line is the view's: the renderer draws it into the
        // status band, whether it is the visible range of the list or the
        // scan's own text.
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
                Key::Letter(b'M') => self.remove_missing_entries(),
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

    /// The action of `Ctrl+M`: removes the rows whose file is gone.
    ///
    /// A running scan refuses the action: the worker holds the index while
    /// it scans, and the removal cannot run beside it. The scan is left
    /// alone - stopping it would throw away the batches it has not
    /// committed yet - and the window says so.
    fn remove_missing_entries(&mut self) {
        if self.worker.is_some() {
            self.action_note = Some(
                "a scan is still running: stop it (Escape) before the missing entries are removed"
                    .to_string(),
            );
            return;
        }
        self.action_note = Some(match self.remove_missing() {
            Ok(report) => note_of(&report),
            Err(error) => format!("the missing entries were not removed: {error}"),
        });
    }

    /// Removes the rows whose file is gone and reloads the page.
    ///
    /// The rows are dropped from the index; the files are not touched. The
    /// scope is every registered root ([`Library::remove_missing`] with
    /// `None`).
    fn remove_missing(&mut self) -> Result<RemoveReport> {
        let report = {
            let Some(index) = self.index.as_mut() else {
                return Err(Error::NotFound("the library index is not open".into()));
            };
            index.remove_missing(None)?
        };
        // The page is reloaded: the rows that are gone are not in the list
        // any more, and the total is smaller.
        self.refresh()?;
        Ok(report)
    }

    /// Takes the note of the last action, if there is one.
    fn take_action_note(&mut self) -> Option<String> {
        self.action_note.take()
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
        self.start_job(RootScan { index, roots });
        Ok(())
    }

    /// Starts a job on the app's own worker thread.
    ///
    /// The job runs on the worker; the progress events travel to the
    /// interface thread over a channel, and the index comes back with the
    /// worker. Neither side waits for the other: the worker never blocks on
    /// the interface, and the interface never blocks on the worker.
    fn start_job(&mut self, job: impl ScanJob) {
        let cancel = Cancel::new();
        let (sender, events) = mpsc::channel();
        let worker_cancel = cancel.clone();
        let handle = std::thread::spawn(move || {
            job.run(&worker_cancel, &mut |event| {
                // A send that fails means that nobody listens any more: the
                // window is gone, and the scan stops then.
                if sender.send(event).is_err() {
                    worker_cancel.cancel();
                }
            })
        });
        self.error = None;
        // The new scan has its own story: the last one is over.
        self.scan_progress = None;
        self.cancelled_scan = None;
        self.view.clear_scan_status();
        self.worker = Some(ScanWorker {
            cancel,
            events,
            handle,
        });
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
                let ScanOutcome { index, result } = outcome;
                self.index = Some(index);
                match result {
                    Ok(report) => {
                        if report.cancelled {
                            // The scan was stopped: what it committed is
                            // what the index holds, and that is what the
                            // status line says.
                            self.cancelled_scan = Some(report);
                        }
                    }
                    Err(error) => {
                        self.error = Some(error.to_string());
                    }
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
    /// The progress events are taken on the way, so the status line shows
    /// what the scan does while it runs.
    pub fn poll(&mut self) -> Result<()> {
        self.drain_progress();
        let finished = self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.handle.is_finished());
        if finished {
            let worker = self.worker.take().expect("the finished worker");
            self.take_back(worker);
        }
        // The view carries the scan's text: the renderer draws the status
        // line into the band, and the app paints nothing itself.
        self.update_scan_status();
        if finished {
            self.refresh()?;
        }
        Ok(())
    }

    /// Takes the progress events the worker sent.
    ///
    /// The events arrive on the interface thread: the worker sends them and
    /// this takes them, so neither side ever waits for the other. Only the
    /// newest event is kept - the older ones are worthless.
    fn drain_progress(&mut self) {
        let Some(worker) = &self.worker else {
            return;
        };
        while let Ok(event) = worker.events.try_recv() {
            self.scan_progress = Some(event);
        }
    }
}

/// The status line of a running scan: its phase and its counters.
///
/// The counters are the **live** ones: they include the batch that is in
/// flight, so they can be ahead of the index by one batch. They are the
/// truth about the scan; the truth about the index is the report the scan
/// returns.
fn progress_line(progress: &ScanProgress) -> String {
    let phase = match progress.phase {
        ScanPhase::Scanning => "scanning",
        ScanPhase::Removing => "removing",
        ScanPhase::Finished => "finished",
    };
    format!(
        "{phase}: {seen} seen, {added} added, {updated} updated, {skipped} skipped \
         (Esc stops the scan)",
        seen = progress.seen,
        added = progress.added,
        updated = progress.updated,
        skipped = progress.skipped,
    )
}

/// The status line of a scan that was cancelled.
///
/// The numbers are the **committed** ones - the changes the index really
/// holds. The live counters of a scan include the batch that was in
/// flight, and that batch was never written.
fn cancelled_line(report: &ScanReport) -> String {
    format!(
        "cancelled: {added} added, {updated} updated, {removed} removed, \
         {skipped} skipped (committed)",
        added = report.added,
        updated = report.updated,
        removed = report.removed,
        skipped = report.skipped,
    )
}

/// The paths of the registered roots.
fn root_paths(roots: Vec<Root>) -> Vec<PathBuf> {
    roots.into_iter().map(|root| root.path).collect()
}

/// The note a removal report becomes.
///
/// The report says what the action did: how many rows it looked at, how
/// many it dropped, and which rows it kept because their file could not be
/// checked. A report without a checked row has nothing to say about a
/// removal, so it says that there was nothing to remove.
fn note_of(report: &RemoveReport) -> String {
    let RemoveReport {
        checked,
        removed,
        kept_unreadable,
    } = report;
    if *checked == 0 {
        return "nothing to remove: the index holds no rows".to_string();
    }
    let mut note = format!("removed {removed} of {checked} entries");
    if !kept_unreadable.is_empty() {
        let paths = kept_unreadable
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        note.push_str(&format!(
            "; {} kept, the file could not be checked: {paths}",
            kept_unreadable.len()
        ));
    }
    note
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

    /// The text the library window's status line shows.
    ///
    /// It is the visible range of the list, or what the running scan has to
    /// say.
    pub fn library_status_line(&self) -> String {
        self.library_window.status_line()
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
    ///
    /// An action that has something to say (the removal of the missing
    /// entries) leaves a note, which the app shows.
    pub fn library_key_down(&mut self, key: Key, modifiers: Modifiers) -> Result<Vec<Command>> {
        let commands = self.library_window.key(key, modifiers)?;
        if let Some(note) = self.library_window.take_action_note() {
            self.note(format!("Library: {note}"));
        }
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
