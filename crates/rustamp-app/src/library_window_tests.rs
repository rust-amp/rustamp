// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The media library window: the cover column and the status line.
//!
//! Every test is headless: null output, the built-in skin, no OS window and
//! no child process. The covers are built by the test itself - a PNG in an
//! ID3v2 tag in front of a real MP3 - so the suite needs neither a picture
//! library nor a network.
//!
//! The scan's progress is tested with a **fake** scan: a real scan of a
//! directory is fast and hard to steer, and the status line must be proven
//! for every phase and for the cancelled case.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rustamp_core::config::Config;
use rustamp_gfx::{Image, Rect};
use rustamp_ui::LibraryRow;
use rustamp_ui::library_render::{
    LIBRARY_STATUS_TEXT, LIBRARY_WINDOW_BACKGROUND, LibraryChrome, cover_cell,
};
use tempfile::TempDir;

use super::*;
use crate::app::{App, AppOptions};
use crate::cli::OutputChoice;

// ---------------------------------------------------------------------------
// The picture
// ---------------------------------------------------------------------------

/// A red pixel, opaque.
const RED: [u8; 4] = [255, 0, 0, 255];

/// The CRC-32 of a PNG chunk's contents.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// The Adler-32 of the uncompressed bytes of a zlib stream.
fn adler32(bytes: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// A zlib stream of `data`, stored without compression.
///
/// The picture is a few pixels wide, so the stored blocks are smaller than
/// a compressed stream would be - and the test needs no compressor.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut offset = 0;
    loop {
        let len = (data.len() - offset).min(u16::MAX as usize);
        let last = offset + len >= data.len();
        out.push(u8::from(last));
        out.extend_from_slice(&(len as u16).to_le_bytes());
        out.extend_from_slice(&(!(len as u16)).to_le_bytes());
        out.extend_from_slice(&data[offset..offset + len]);
        offset += len;
        if last {
            break;
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// One chunk of a PNG: its length, its kind, its contents and its CRC.
fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::new();
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    out
}

/// A PNG of one colour, `width` x `height` pixels.
///
/// The picture is written by hand: the cover decoder reads PNG, and a test
/// of a few pixels does not need a picture library to make one.
fn png(width: u32, height: u32, colour: [u8; 3]) -> Vec<u8> {
    // One row of the picture: the filter byte and the row's own pixels.
    let pixels: Vec<u8> = std::iter::repeat_n(colour, width as usize)
        .flatten()
        .collect();
    let mut row = vec![0u8]; // the filter: none
    row.extend_from_slice(&pixels);
    let raw: Vec<u8> = std::iter::repeat_n(row, height as usize)
        .flatten()
        .collect();

    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    // 8 bits per channel, truecolour, no compression, no filter, no interlace.
    header.extend_from_slice(&[8, 2, 0, 0, 0]);

    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    out.extend_from_slice(&png_chunk(b"IHDR", &header));
    out.extend_from_slice(&png_chunk(b"IDAT", &zlib_stored(&raw)));
    out.extend_from_slice(&png_chunk(b"IEND", &[]));
    out
}

// ---------------------------------------------------------------------------
// The track
// ---------------------------------------------------------------------------

/// The size of an ID3v2 tag, in synchsafe bytes.
fn synchsafe(size: u32) -> [u8; 4] {
    [
        ((size >> 21) & 0x7F) as u8,
        ((size >> 14) & 0x7F) as u8,
        ((size >> 7) & 0x7F) as u8,
        (size & 0x7F) as u8,
    ]
}

/// One ID3v2.3 frame: its id, its size, its flags and its data.
fn id3v2_frame(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.extend_from_slice(id);
    frame.extend_from_slice(&(data.len() as u32).to_be_bytes());
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(data);
    frame
}

/// An ID3v2.3 tag with the given frames.
fn id3v2_tag(frames: &[Vec<u8>]) -> Vec<u8> {
    let body = frames.concat();
    let mut tag = Vec::new();
    tag.extend_from_slice(b"ID3");
    tag.extend_from_slice(&[0x03, 0x00, 0x00]);
    tag.extend_from_slice(&synchsafe(body.len() as u32));
    tag.extend_from_slice(&body);
    tag
}

/// An `APIC` frame: the front cover of the tag.
fn apic_frame(picture: &[u8]) -> Vec<u8> {
    let mut data = vec![0x00]; // ISO-8859-1
    data.extend_from_slice(b"image/png");
    data.push(0x00);
    data.push(3); // the front cover
    data.extend_from_slice(b"cover");
    data.push(0x00);
    data.extend_from_slice(picture);
    id3v2_frame(b"APIC", &data)
}

/// The media fixture: a real MP3 with tags.
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../rustamp-decoders/tests/fixtures")
        .join("sine.mp3")
}

/// The fixture's audio, without the tag it carries.
///
/// The tag is dropped, so the tag the test writes is the only one the
/// reader sees.
fn fixture_audio() -> Vec<u8> {
    let bytes = std::fs::read(fixture()).expect("read the fixture");
    if !bytes.starts_with(b"ID3") {
        return bytes;
    }
    let size = ((bytes[6] as usize & 0x7F) << 21)
        | ((bytes[7] as usize & 0x7F) << 14)
        | ((bytes[8] as usize & 0x7F) << 7)
        | (bytes[9] as usize & 0x7F);
    bytes[10 + size..].to_vec()
}

/// A track with an embedded front cover.
///
/// The fixture's audio carries an ID3v2 tag with the picture in front of
/// it: the file is a real MP3, and its cover is the picture the test put
/// there.
fn track_with_cover(dir: &Path, name: &str, picture: &[u8]) -> PathBuf {
    let mut bytes = id3v2_tag(&[apic_frame(picture)]);
    bytes.extend_from_slice(&fixture_audio());
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write the track");
    path
}

/// A track without a cover: the fixture, as it is.
fn track_without_cover(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::copy(fixture(), &path).expect("copy the fixture");
    path
}

// ---------------------------------------------------------------------------
// The player
// ---------------------------------------------------------------------------

/// A headless player whose configuration lives in `dir`.
fn library_app(dir: &TempDir) -> App {
    let options = AppOptions {
        config: Config::default(),
        skin: crate::skin::builtin(),
        output: OutputChoice::Null,
        volume: 200,
        file: None,
        seconds: None,
    };
    let mut app = App::new(options).expect("create the player");
    app.set_config_path(Some(dir.path().join("rustamp.toml")));
    app
}

/// Adds `media` to the library and waits for the background scan.
///
/// The scan runs on the app's own thread, so the test pumps the app until
/// the rows are there.
fn scan(app: &mut App, media: &Path) {
    app.library_add_folder(media.to_path_buf())
        .expect("add the folder");
    for _ in 0..600 {
        app.pump().expect("pump");
        if app.library_view().total() > 0 {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the library scan did not finish");
}

/// The cell of the row at `index`, as the app draws the list.
fn row_cell(app: &App, index: usize) -> Rect {
    let (width, height) = app.library_client_size();
    let chrome = LibraryChrome::new(app.library_view(), Rect::new(0, 0, width, height));
    cover_cell(chrome.rows, index)
}

/// The colour of the window's background, opaque.
fn background() -> [u8; 4] {
    let [red, green, blue] = LIBRARY_WINDOW_BACKGROUND;
    [red, green, blue, 255]
}

/// The number of pixels of `colour` in `rect`.
fn count(frame: &Image, rect: Rect, colour: [u8; 4]) -> usize {
    let mut found = 0;
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            if frame.pixel(x, y) == Some(colour) {
                found += 1;
            }
        }
    }
    found
}

/// A row for the track at `path`.
fn row(path: &Path) -> LibraryRow {
    LibraryRow {
        id: 1,
        path: path.to_path_buf(),
        title: "Track".to_string(),
        artist: "Rustamp".to_string(),
        album: "Tests".to_string(),
        duration_ms: None,
    }
}

// ---------------------------------------------------------------------------
// The cache and the renderer
// ---------------------------------------------------------------------------

#[test]
fn a_cached_cover_is_lent_to_the_renderer_with_its_own_size() {
    let dir = TempDir::new().expect("temp dir");
    let path = track_with_cover(dir.path(), "cover.mp3", &png(2, 2, [255, 0, 0]));
    let cache = ArtworkCache::new();
    let rows = vec![row(&path)];
    let mut owns = Vec::new();

    let (covers, failure) = covers_of(&cache, &rows, &mut owns);

    assert!(failure.is_none(), "the cover was read: {failure:?}");
    let cover = covers[0].expect("the row has a cover");
    assert_eq!((cover.width, cover.height), (2, 2), "the cover's own size");
    assert_eq!(cover.pixels.len(), 2 * 2 * 4, "the cover's own pixels");
    assert_eq!(&cover.pixels[..4], &RED, "the cover's own colour");
}

#[test]
fn a_row_without_a_cover_has_no_view() {
    let dir = TempDir::new().expect("temp dir");
    let path = track_without_cover(dir.path(), "plain.mp3");
    let cache = ArtworkCache::new();
    let rows = vec![row(&path)];
    let mut owns = Vec::new();

    let (covers, failure) = covers_of(&cache, &rows, &mut owns);

    assert!(
        failure.is_none(),
        "a track without a cover is not a failure: {failure:?}"
    );
    assert!(covers[0].is_none(), "the row has no cover");
}

#[test]
fn a_cover_that_cannot_be_read_has_no_view_and_is_reported() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("gone.mp3");
    let cache = ArtworkCache::new();
    let rows = vec![row(&path)];
    let mut owns = Vec::new();

    let (covers, failure) = covers_of(&cache, &rows, &mut owns);

    assert!(covers[0].is_none(), "the row has no cover");
    let failure = failure.expect("the failure is reported");
    assert!(failure.contains("gone.mp3"), "{failure}");
}

// ---------------------------------------------------------------------------
// The frame
// ---------------------------------------------------------------------------

#[test]
fn the_cover_of_a_row_is_drawn_into_its_cell() {
    let dir = TempDir::new().expect("temp dir");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("media folder");
    track_with_cover(&media, "cover.mp3", &png(2, 2, [255, 0, 0]));
    let mut app = library_app(&dir);
    scan(&mut app, &media);

    let frame = app.render_library();

    let cell = row_cell(&app, 0);
    assert_eq!(
        count(&frame, cell, RED),
        (cell.width * cell.height) as usize,
        "the cover fills the cell of its row"
    );
}

#[test]
fn a_row_without_a_cover_leaves_its_cell_empty() {
    let dir = TempDir::new().expect("temp dir");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("media folder");
    track_without_cover(&media, "plain.mp3");
    let mut app = library_app(&dir);
    scan(&mut app, &media);

    let frame = app.render_library();

    let cell = row_cell(&app, 0);
    assert_eq!(
        count(&frame, cell, background()),
        (cell.width * cell.height) as usize,
        "the cell of a row without a cover keeps the background"
    );
}

#[test]
fn a_cover_that_cannot_be_read_leaves_the_frame_renderable() {
    let dir = TempDir::new().expect("temp dir");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("media folder");
    let track = track_with_cover(&media, "cover.mp3", &png(2, 2, [255, 0, 0]));
    let mut app = library_app(&dir);
    scan(&mut app, &media);

    // The track is gone: the index still lists it, and the cache cannot
    // read it.
    std::fs::remove_file(&track).expect("remove the track");

    let frame = app.render_library();

    assert_eq!(
        (frame.width(), frame.height()),
        LIBRARY_CLIENT_SIZE,
        "the frame is drawn"
    );
    let cell = row_cell(&app, 0);
    assert_eq!(
        count(&frame, cell, background()),
        (cell.width * cell.height) as usize,
        "the cell of a row whose cover cannot be read stays empty"
    );

    // The failure is reported once, not once per frame.
    let notes = app.drain_notes();
    assert!(
        notes.iter().any(|note| note.contains("cover.mp3")),
        "the failure is noted: {notes:?}"
    );
    app.render_library();
    assert!(
        app.drain_notes().is_empty(),
        "the failure is not noted again"
    );
}

// ---------------------------------------------------------------------------
// The scan's progress in the status line
// ---------------------------------------------------------------------------

/// A scan the test drives: it emits the events it was given and then waits
/// until the scan is cancelled.
///
/// A real scan of a directory is fast and hard to steer, and the status line
/// must be proven for every phase and for the cancelled case - so the test
/// runs a scan of its own.
struct FakeScan {
    /// The events the fake emits, in order.
    events: Vec<ScanProgress>,
    /// The report the fake returns when it was cancelled.
    report: ScanReport,
}

impl ScanJob for FakeScan {
    fn run(self, cancel: &Cancel, on_progress: &mut dyn FnMut(ScanProgress)) -> ScanOutcome {
        for event in &self.events {
            on_progress(event.clone());
        }
        // The fake waits for the cancel, like a real scan waits for the
        // next file: the test decides when the window stops it.
        while !cancel.is_cancelled() {
            std::thread::sleep(Duration::from_millis(1));
        }
        ScanOutcome {
            index: Library::in_memory().expect("an in-memory index"),
            result: Ok(self.report),
        }
    }
}

/// A progress event of a running scan.
fn event(
    phase: ScanPhase,
    seen: usize,
    added: usize,
    updated: usize,
    skipped: usize,
) -> ScanProgress {
    ScanProgress {
        phase,
        path: None,
        seen,
        added,
        updated,
        skipped,
        removed: 0,
    }
}

/// Polls the window until the scan reached the phase `phase`.
fn wait_for_phase(window: &mut LibraryWindow, phase: ScanPhase) {
    for _ in 0..2000 {
        window.poll().expect("poll the window");
        if window.progress().is_some_and(|event| event.phase == phase) {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("the scan never reached the phase {phase:?}");
}

/// Polls the window until the status line holds `wanted`.
fn wait_for_status(window: &mut LibraryWindow, wanted: &str) -> String {
    for _ in 0..2000 {
        window.poll().expect("poll the window");
        let status = window.status_line();
        if status.contains(wanted) {
            return status;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!(
        "the status line never held {wanted:?}: {}",
        window.status_line()
    );
}

/// The status band of the window, as the app draws it.
fn status_band(window: &LibraryWindow) -> Rect {
    let (width, height) = window.client_size();
    LibraryChrome::new(window.view(), Rect::new(0, 0, width, height)).status
}

/// The colour of the status line's text.
fn status_ink() -> [u8; 4] {
    let [red, green, blue] = LIBRARY_STATUS_TEXT;
    [red, green, blue, 255]
}

/// The x of the leftmost pixel of the given colour in the band.
fn first_ink_x(frame: &Image, band: Rect, colour: [u8; 4]) -> Option<i32> {
    for x in band.x..band.right() {
        for y in band.y..band.bottom() {
            if frame.pixel(x, y) == Some(colour) {
                return Some(x);
            }
        }
    }
    None
}

#[test]
fn a_running_scan_shows_its_phase_and_its_counters() {
    let mut window = LibraryWindow::default();
    window.start_job(FakeScan {
        events: vec![event(ScanPhase::Scanning, 120, 30, 5, 2)],
        report: ScanReport::default(),
    });
    wait_for_phase(&mut window, ScanPhase::Scanning);

    let status = window.status_line();
    assert!(status.contains("scanning"), "{status}");
    assert!(status.contains("120 seen"), "{status}");
    assert!(status.contains("30 added"), "{status}");
    assert!(status.contains("5 updated"), "{status}");
    assert!(status.contains("2 skipped"), "{status}");
}

#[test]
fn the_scan_status_lives_in_the_view() {
    let mut window = LibraryWindow::default();
    window.start_job(FakeScan {
        events: vec![event(ScanPhase::Scanning, 120, 30, 5, 2)],
        report: ScanReport::default(),
    });
    wait_for_phase(&mut window, ScanPhase::Scanning);

    let status = window
        .view()
        .scan_status()
        .expect("the view carries the scan's text")
        .to_string();
    assert!(status.contains("scanning"), "{status}");
    assert_eq!(
        window.status_line(),
        status,
        "the window's status line is the view's"
    );
}

#[test]
fn the_finished_phase_restores_the_normal_status_line() {
    let mut window = LibraryWindow::default();
    let normal = window.view().status_line();
    window.start_job(FakeScan {
        events: vec![
            event(ScanPhase::Scanning, 10, 4, 0, 0),
            event(ScanPhase::Finished, 10, 4, 0, 0),
        ],
        report: ScanReport::default(),
    });
    wait_for_phase(&mut window, ScanPhase::Finished);

    assert_eq!(
        window.status_line(),
        normal,
        "the visible range is the status line again"
    );
    assert!(
        window.view().scan_status().is_none(),
        "the view's scan status is cleared"
    );
}

#[test]
fn a_cancelled_scan_shows_the_committed_numbers() {
    let mut window = LibraryWindow::default();
    window.start_job(FakeScan {
        events: vec![event(ScanPhase::Scanning, 120, 30, 5, 2)],
        report: ScanReport {
            scanned: 40,
            added: 12,
            updated: 3,
            removed: 1,
            skipped: 2,
            cancelled: true,
        },
    });
    wait_for_phase(&mut window, ScanPhase::Scanning);

    // Escape stops the scan.
    window
        .key(Key::Escape, Modifiers::NONE)
        .expect("escape stops the scan");

    // The worker comes back with the report it committed.
    let status = wait_for_status(&mut window, "cancelled");
    assert!(status.contains("12 added"), "{status}");
    assert!(status.contains("3 updated"), "{status}");
    assert!(status.contains("1 removed"), "{status}");
    assert!(status.contains("2 skipped"), "{status}");
    assert!(
        !status.contains("30 added") && !status.contains("120 seen"),
        "the committed numbers are shown, not the live ones: {status}"
    );
    assert_eq!(
        window.view().scan_status(),
        Some(status.as_str()),
        "the view carries the cancelled text"
    );
}

#[test]
fn without_a_scan_the_status_line_is_the_visible_range() {
    let window = LibraryWindow::default();
    assert_eq!(
        window.status_line(),
        window.view().status_line(),
        "no scan leaves the status line alone"
    );
}

#[test]
fn the_status_band_shows_the_scan_progress() {
    let mut window = LibraryWindow::default();
    let band = status_band(&window);
    let quiet = count(&window.render(), band, status_ink());

    window.start_job(FakeScan {
        events: vec![event(ScanPhase::Scanning, 120, 30, 5, 2)],
        report: ScanReport::default(),
    });
    wait_for_phase(&mut window, ScanPhase::Scanning);
    let busy = count(&window.render(), band, status_ink());

    assert!(
        busy > quiet,
        "the progress is painted into the band: {quiet} ink pixels without the \
         scan, {busy} with it"
    );

    // The text stands where the visible range stands: at the left edge of
    // the band.
    let frame = window.render();
    let first = first_ink_x(&frame, band, status_ink()).expect("the band holds ink");
    assert!(
        first <= band.x + 3,
        "the text starts at the left edge of the band: {first} against {}",
        band.x
    );
}

#[test]
fn the_caption_carries_the_scan_progress() {
    let mut window = LibraryWindow::default();
    window.start_job(FakeScan {
        events: vec![event(ScanPhase::Scanning, 120, 30, 5, 2)],
        report: ScanReport::default(),
    });
    wait_for_phase(&mut window, ScanPhase::Scanning);

    let title = window.title();
    assert!(title.starts_with("Rustamp Library - scanning:"), "{title}");
    assert!(title.contains("120 seen"), "{title}");
}

// ---------------------------------------------------------------------------
// The recovery of a damaged database
// ---------------------------------------------------------------------------

/// A database file that is not a database.
fn damaged_database(dir: &TempDir) -> PathBuf {
    let path = dir.path().join(LIBRARY_DATABASE_NAME);
    std::fs::write(&path, b"this is not a database, not even a little one")
        .expect("write the damaged file");
    path
}

/// The files of `dir` that are a backup of a damaged database.
fn backups(dir: &TempDir) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir.path())
        .expect("read the folder")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(".broken-") && !name.ends_with(".txt"))
        })
        .collect();
    found.sort();
    found
}

/// The files of `dir` that hold the reason of a recovery.
fn reasons(dir: &TempDir) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir.path())
        .expect("read the folder")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(".broken-") && name.ends_with(".txt"))
        })
        .collect();
    found.sort();
    found
}

/// Runs one damage class through the window's branch.
///
/// A damage that is about the file sets the file aside and tells the user;
/// a damage that is about the machine leaves everything alone.
fn branch(make_error: impl Fn(PathBuf) -> OpenError, recovered: bool) {
    let dir = TempDir::new().expect("temp dir");
    let database = damaged_database(&dir);
    let bytes = std::fs::read(&database).expect("read the damaged file");
    let mut window = LibraryWindow::default();

    let outcome = window.open_fresh_index(&database, make_error(database.clone()));

    if recovered {
        assert!(outcome.is_ok(), "the damaged file is recovered");
        let backup = backups(&dir);
        assert_eq!(backup.len(), 1, "the file was set aside: {backup:?}");
        assert_eq!(
            std::fs::read(&backup[0]).expect("read the backup"),
            bytes,
            "the backup keeps the bytes of the damaged file"
        );
        // A fresh index takes the place of the damaged file.
        assert!(
            Library::open(&database).is_ok(),
            "a fresh index took the place of the damaged file"
        );
        assert_ne!(
            std::fs::read(&database).expect("read the fresh index"),
            bytes,
            "the file at the old name is the fresh index, not the damaged file"
        );
        let note = window.action_note.clone().expect("the user is told");
        assert!(note.contains("set aside"), "{note}");
        assert!(note.contains("starts empty"), "{note}");
    } else {
        assert!(outcome.is_err(), "the damage is refused");
        assert_eq!(
            std::fs::read(&database).expect("read the file"),
            bytes,
            "the file is left exactly as it was"
        );
        assert!(backups(&dir).is_empty(), "nothing was renamed");
        assert!(window.action_note.is_none(), "nothing is told");
    }
}

#[test]
fn a_damaged_file_is_set_aside_and_a_fresh_index_takes_its_place() {
    branch(
        |path| OpenError::NotADatabase {
            path,
            detail: "the file does not start with the SQLite header".to_owned(),
        },
        true,
    );
    branch(
        |path| OpenError::Truncated {
            path,
            detail: "the header is cut short".to_owned(),
        },
        true,
    );
    branch(
        |path| OpenError::Corrupt {
            path,
            detail: "database disk image is malformed".to_owned(),
        },
        true,
    );
    branch(
        |path| OpenError::ForeignSchema {
            path,
            detail: "the table tracks is missing".to_owned(),
        },
        true,
    );
    branch(
        |path| OpenError::FutureVersion {
            path,
            found: 2,
            supported: 1,
        },
        true,
    );
}

#[test]
fn the_damage_of_the_machine_is_refused_and_nothing_is_renamed() {
    branch(|path| OpenError::NotAFile { path }, false);
    branch(|path| OpenError::ReadOnly { path }, false);
    branch(
        |path| OpenError::Locked {
            path,
            detail: "database is locked".to_owned(),
        },
        false,
    );
    branch(
        |path| OpenError::Io {
            path,
            detail: "access is denied".to_owned(),
        },
        false,
    );
    branch(
        |path| OpenError::Failed {
            path,
            detail: "no such table: tracks".to_owned(),
        },
        false,
    );
}

#[test]
fn a_damaged_database_is_recovered_when_the_library_opens() {
    let dir = TempDir::new().expect("temp dir");
    let database = damaged_database(&dir);
    let bytes = std::fs::read(&database).expect("read the damaged file");

    let mut window = LibraryWindow::default();
    window
        .ensure_index(Some(&database))
        .expect("the library opens");

    let backup = backups(&dir);
    assert_eq!(backup.len(), 1, "the damaged file was set aside");
    assert_eq!(
        std::fs::read(&backup[0]).expect("read the backup"),
        bytes,
        "the backup keeps the bytes of the damaged file"
    );
    let reason = reasons(&dir);
    assert_eq!(reason.len(), 1, "the reason was written next to it");
    let text = std::fs::read_to_string(&reason[0]).expect("read the reason");
    assert!(
        text.contains("not a database"),
        "the reason is typed: {text}"
    );
    assert!(
        text.contains(&database.display().to_string()),
        "the reason names the file: {text}"
    );

    // The fresh index is a good database, and it knows no root: the library
    // starts empty.
    let index = Library::open(&database).expect("the fresh index opens");
    assert!(
        index.roots().expect("the roots").is_empty(),
        "the library starts empty"
    );
    assert!(
        window.roots().expect("the roots").is_empty(),
        "the window knows no root"
    );
}

#[test]
fn a_read_only_database_is_refused_and_left_alone() {
    let dir = TempDir::new().expect("temp dir");
    let database = damaged_database(&dir);
    let bytes = std::fs::read(&database).expect("read the damaged file");
    // The file's own permissions are kept, so that the folder can be
    // cleaned up after the test.
    let original = std::fs::metadata(&database)
        .expect("stat the file")
        .permissions();
    let mut read_only = original.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(&database, read_only).expect("make the file read-only");

    let mut window = LibraryWindow::default();
    let outcome = window.ensure_index(Some(&database));

    assert!(outcome.is_err(), "the read-only database is refused");
    assert_eq!(
        std::fs::read(&database).expect("read the file"),
        bytes,
        "the file is left exactly as it was"
    );
    assert!(backups(&dir).is_empty(), "nothing was renamed");
    assert!(window.action_note.is_none(), "nothing is told");

    std::fs::set_permissions(&database, original).expect("give the file its permissions back");
}

#[test]
fn a_database_that_is_not_a_file_is_refused_and_left_alone() {
    let dir = TempDir::new().expect("temp dir");
    let database = dir.path().join(LIBRARY_DATABASE_NAME);
    std::fs::create_dir(&database).expect("make a folder where the database should be");

    let mut window = LibraryWindow::default();
    let outcome = window.ensure_index(Some(&database));

    assert!(outcome.is_err(), "the folder is refused");
    assert!(database.is_dir(), "the folder is left alone");
    assert!(backups(&dir).is_empty(), "nothing was renamed");
    assert!(window.action_note.is_none(), "nothing is told");
}

#[test]
fn the_recovery_is_told_to_the_user_once() {
    let dir = TempDir::new().expect("temp dir");
    damaged_database(&dir);
    let mut app = library_app(&dir);

    app.toggle_library();

    let notes = app.drain_notes();
    let recovery: Vec<&String> = notes
        .iter()
        .filter(|note| note.contains("set aside"))
        .collect();
    assert_eq!(recovery.len(), 1, "the recovery is told once: {notes:?}");
    let note = recovery[0];
    assert!(note.starts_with("Library: "), "{note}");
    assert!(note.contains("starts empty"), "{note}");

    // The window is closed and opened again: the index is open, so there is
    // nothing to recover and nothing to tell.
    app.toggle_library();
    app.toggle_library();
    let notes = app.drain_notes();
    assert!(
        !notes.iter().any(|note| note.contains("set aside")),
        "the recovery is not told again: {notes:?}"
    );
}
