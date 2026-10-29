// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The clipboard read behind `Ctrl`+`V` in a dialog's entry field.
//!
//! The UI thread never touches the clipboard. A Win32 clipboard can be held
//! open by another process (`OpenClipboard` then fails or waits), and a paste
//! must not freeze the player while that is decided. [`Clipboard::request`]
//! therefore starts a **worker thread** that reads the text, and the presenter
//! picks the result up on its next tick ([`Clipboard::take`]) and types it
//! into the open dialog through [`App::dialog_text`](crate::app::App::dialog_text)
//! — the same path a key press's OS text takes, so paste and typing cannot
//! drift apart.
//!
//! Only the `windows` crate is used (the dependency the control channel and
//! `rustamp-platform` already carry), so no new package enters the lockfile.
//! The read is the standard four steps — `OpenClipboard`,
//! `GetClipboardData(CF_UNICODETEXT)`, `GlobalLock`/`GlobalSize`, then
//! `GlobalUnlock` and `CloseClipboard` on every path — and it runs on the
//! worker thread, which is also the thread that must close the clipboard it
//! opened.
//!
//! An empty clipboard, a clipboard without text and a clipboard that cannot
//! be opened at all read as `None`: a paste with nothing to paste is a
//! **silent no-op**, exactly as typing nothing is. Nothing here raises a note
//! or an error box.
//!
//! Headless the module is inert: a `--null` run never creates a dialog, so
//! nothing calls [`Clipboard::request`], and [`Clipboard::with_reader`] takes
//! a plain closure, so the worker handoff is unit-tested without touching the
//! operating system's clipboard.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// How the text is read: the Win32 clipboard in the player, a plain closure in
/// the tests. `None` means "nothing to paste".
type Reader = dyn Fn() -> Option<String> + Send + Sync;

/// The clipboard worker: one read at a time, the text handed to the UI thread.
///
/// A read is started by [`request`](Self::request) and its result is picked up
/// by [`take`](Self::take). The two never meet on the same thread: `take` runs
/// on the UI thread in the presenter's tick, `request` where the key press
/// arrived.
pub struct Clipboard {
    /// The reader, shared with the worker thread that runs it.
    read: Arc<Reader>,
    /// The text a finished read left for the UI thread. `None` until a read
    /// found something.
    incoming: Arc<Mutex<Option<String>>>,
    /// Whether a read is in flight. A second `Ctrl`+`V` while one runs is
    /// dropped rather than queued, so the clipboard is never read twice for
    /// one field and pressing the chord repeatedly cannot pile threads up.
    reading: Arc<AtomicBool>,
}

impl Clipboard {
    /// The player's clipboard: the Win32 one on Windows, nothing elsewhere.
    pub fn new() -> Self {
        Self::with_reader(platform::read_text)
    }

    /// A clipboard whose text comes from `read` instead of the operating
    /// system. The player itself uses [`new`](Self::new); this exists so the
    /// worker handoff is tested headless.
    pub fn with_reader(read: impl Fn() -> Option<String> + Send + Sync + 'static) -> Self {
        Self {
            read: Arc::new(read),
            incoming: Arc::new(Mutex::new(None)),
            reading: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Starts a read on a worker thread and returns immediately.
    ///
    /// Returns `false` when a read was already in flight (or the thread could
    /// not be started, which is reported nowhere: the paste is then simply a
    /// no-op, like an empty clipboard).
    pub fn request(&self) -> bool {
        if self.reading.swap(true, Ordering::AcqRel) {
            return false;
        }
        let read = Arc::clone(&self.read);
        let incoming = Arc::clone(&self.incoming);
        let reading = Arc::clone(&self.reading);
        let spawned = std::thread::Builder::new()
            .name("clipboard-read".to_owned())
            .spawn(move || {
                if let Some(text) = read()
                    && let Ok(mut slot) = incoming.lock()
                {
                    *slot = Some(text);
                }
                reading.store(false, Ordering::Release);
            });
        if spawned.is_err() {
            self.reading.store(false, Ordering::Release);
            return false;
        }
        true
    }

    /// Takes the text a finished read left, if any.
    ///
    /// A taken text is gone: one paste delivers one text, and the next `tick`
    /// does not type it again.
    pub fn take(&self) -> Option<String> {
        self.incoming.lock().ok().and_then(|mut slot| slot.take())
    }
}

impl Default for Clipboard {
    fn default() -> Self {
        Self::new()
    }
}

/// The operating system's half: the Win32 clipboard on Windows, nothing
/// anywhere else (the presenter's windows are Win32-only anyway).
#[cfg(windows)]
#[allow(unsafe_code)] // the clipboard FFI boundary only; the rest of the crate denies unsafe.
mod platform {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

    /// `CF_UNICODETEXT` (13, `winuser.h`): the clipboard's UTF-16 text.
    ///
    /// Spelled out because the `windows` crate keeps the clipboard-format
    /// constants in its OLE module, whose feature this crate does not enable;
    /// the value is a public Win32 constant and has not changed since
    /// Windows NT.
    const CF_UNICODETEXT: u32 = 13;

    /// The most UTF-16 units a paste reads (1 MiB of text).
    ///
    /// A bound and not a policy: the field clamps the text it takes far below
    /// this ([`MAX_MEDIA_LEN`](crate::dialogs::MAX_MEDIA_LEN) is 512
    /// characters), so this only keeps a corrupted `GlobalSize` from walking
    /// off the block.
    const MAX_UNITS: usize = 1 << 19;

    /// Reads the clipboard's text, or `None` when it has none.
    pub(super) fn read_text() -> Option<String> {
        // SAFETY: `OpenClipboard` opens the per-thread clipboard and is closed
        // again below on this same thread, whatever the read does.
        unsafe { OpenClipboard(None) }.ok()?;
        // SAFETY: the clipboard is open on this thread; every early return is
        // followed by the close below.
        let text = unsafe { read_open_clipboard() };
        // SAFETY: the clipboard was opened on this thread above.
        let _ = unsafe { CloseClipboard() };
        text
    }

    /// Reads `CF_UNICODETEXT` from the clipboard this thread has open.
    ///
    /// # Safety
    ///
    /// The caller must have opened the clipboard on this thread with
    /// `OpenClipboard`.
    unsafe fn read_open_clipboard() -> Option<String> {
        // SAFETY: the clipboard is open on this thread (the caller's duty).
        let handle = unsafe { GetClipboardData(CF_UNICODETEXT) }.ok()?;
        let memory = HGLOBAL(handle.0);
        // SAFETY: `memory` is the block `GetClipboardData` just handed us, and
        // a locked clipboard block stays valid until `GlobalUnlock`.
        let pointer = unsafe { GlobalLock(memory) } as *const u16;
        if pointer.is_null() {
            return None;
        }
        // SAFETY: the handle came from the clipboard, so `GlobalSize` reports
        // its real length; the bound keeps a lying size harmless.
        let units = (unsafe { GlobalSize(memory) } / size_of::<u16>()).min(MAX_UNITS);
        // SAFETY: the block holds `GlobalSize` bytes of UTF-16 and the slice
        // stops at that many units; a clipboard text is NUL-terminated, so the
        // slice below ends at the terminator.
        let all = unsafe { std::slice::from_raw_parts(pointer, units) };
        let len = all.iter().position(|&unit| unit == 0).unwrap_or(all.len());
        let text = String::from_utf16_lossy(&all[..len]);
        // SAFETY: `memory` was locked above, on this thread.
        let _ = unsafe { GlobalUnlock(memory) };
        Some(text)
    }
}

#[cfg(not(windows))]
mod platform {
    /// No Win32 clipboard: nothing to paste (the presenter has no window here
    /// either).
    pub(super) fn read_text() -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    /// Waits for the worker to leave its text, the way the presenter's tick
    /// waits for it: poll, up to a second, then give up.
    fn wait_take(clipboard: &Clipboard) -> Option<String> {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if let Some(text) = clipboard.take() {
                return Some(text);
            }
            assert!(Instant::now() < deadline, "the worker never delivered");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn the_text_of_a_read_arrives_once() {
        // The worker's handoff, without the operating system's clipboard.
        let clipboard = Clipboard::with_reader(|| Some("http://example.com/song.mp3".to_owned()));
        assert!(clipboard.request(), "the first read starts");
        assert_eq!(
            wait_take(&clipboard).as_deref(),
            Some("http://example.com/song.mp3")
        );
        // A taken text is gone: the next tick does not type it again.
        assert_eq!(clipboard.take(), None);
    }

    #[test]
    fn an_empty_clipboard_leaves_nothing_to_take() {
        // An unreadable or empty clipboard is `None`, never an error: the
        // paste is a silent no-op.
        let reads = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&reads);
        let clipboard = Clipboard::with_reader(move || {
            counted.fetch_add(1, Ordering::Relaxed);
            None
        });
        assert!(clipboard.request());
        // The worker runs the reader; a later tick finds nothing.
        let deadline = Instant::now() + Duration::from_secs(1);
        while reads.load(Ordering::Relaxed) == 0 {
            assert!(Instant::now() < deadline, "the worker never read");
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(clipboard.take(), None);
    }

    #[test]
    fn a_second_press_while_a_read_runs_is_dropped() {
        // The gate is `reading`: while one read is in flight the second press
        // starts no thread and returns `false`. Held open with a barrier so
        // the test does not race the worker.
        let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
        let release = Arc::new(AtomicBool::new(false));
        let gate = Arc::clone(&release);
        let entered = Mutex::new(Some(entered_tx));
        let clipboard = Clipboard::with_reader(move || {
            if let Ok(mut slot) = entered.lock()
                && let Some(entered_tx) = slot.take()
            {
                let _ = entered_tx.send(());
            }
            while !gate.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(1));
            }
            Some("late".to_owned())
        });
        assert!(clipboard.request());
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the first read entered the reader");
        assert!(!clipboard.request(), "the second press is dropped");
        release.store(true, Ordering::Release);
        assert_eq!(wait_take(&clipboard).as_deref(), Some("late"));
        // The gate opened again: the next press reads again. The worker clears
        // its flag just after it stores the text, so the press may have to
        // wait one moment for the *previous* read to finish.
        let deadline = Instant::now() + Duration::from_secs(1);
        while !clipboard.request() {
            assert!(Instant::now() < deadline, "the worker never finished");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(wait_take(&clipboard).as_deref(), Some("late"));
    }
}
