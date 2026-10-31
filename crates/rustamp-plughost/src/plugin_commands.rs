//! What plug-ins ask of the player, and what the host tells them about it.
//!
//! A classic plug-in talks to the main window: it posts `WM_COMMAND`s (S10, the
//! ids of [`crate::commands`]), sends IPCs that change something, and forwards
//! the keys its own window does not use (`vis_avs` does). The window procedure
//! notes each here, from whichever thread the plug-in called on, and the UI loop
//! hands them to the player, which owns the transport.
//!
//! The other way round, plug-ins read the current track from the window's title
//! (`WM_GETTEXT`, `vis_avs`'s Text effect) and from the playlist IPCs 211-214, so
//! the player tells the host what plays ([`set_now_playing`]).

use std::path::PathBuf;
use std::sync::Mutex;

use rustamp_proto::command::Command;
use rustamp_proto::playlist::MediaLocation;

use crate::commands::command_for_id;

static PENDING: Mutex<Vec<Command>> = Mutex::new(Vec::new());

fn push(command: Command) {
    if let Ok(mut pending) = PENDING.lock() {
        pending.push(command);
    }
}

/// A `WM_COMMAND` a plug-in sent the main window. The player gets what the table
/// forwards; the rest is dropped with its reason in the table.
pub fn note_window_command(id: u32) {
    if let Some(command) = command_for_id(id) {
        push(command);
    } else {
        crate::fault::trace_event(&format!("WM_COMMAND {id} is not forwarded"));
    }
}

/// A key a plug-in's window passed on: the five transport letters of the main
/// window (S03 §1). The other keys of that table need a state the host lacks.
pub fn note_key(virtual_key: u32) {
    let id = match u8::try_from(virtual_key) {
        Ok(b'Z') => 40044,
        Ok(b'X') => 40045,
        Ok(b'C') => 40046,
        Ok(b'V') => 40047,
        Ok(b'B') => 40048,
        _ => return,
    };
    note_window_command(id);
}

/// An IPC that changes the player: the rows of S10 with an effect the player can
/// carry out. `wparam` is the raw message parameter.
pub fn note_ipc(ipc_id: u32, wparam: isize) {
    match ipc_id {
        // A file for the playlist, as a string of the plug-in's.
        100 => {
            if let Some(path) = file_argument(wparam) {
                push(Command::AddItems {
                    items: vec![MediaLocation::File(PathBuf::from(path))],
                    at: None,
                    play: false,
                });
            }
        }
        102 => push(Command::Play),
        106 => {
            if let Ok(to_ms) = u64::try_from(wparam) {
                push(Command::Seek { to_ms });
            }
        }
        // -666 asks for the volume instead of setting it (S10).
        122 => {
            if let Ok(volume) = u8::try_from(wparam) {
                push(Command::SetVolume(volume));
            }
        }
        _ => {}
    }
}

/// The string a plug-in passed as a message parameter, if it is one. Only a
/// plug-in in this process can hand a pointer that means anything, and a message
/// from elsewhere must not take the host down.
#[cfg(windows)]
fn file_argument(wparam: isize) -> Option<String> {
    unsafe extern "system" {
        fn IsBadStringPtrA(text: *const u8, max: usize) -> i32;
    }
    const MAX_PATH_BYTES: usize = 4096;
    let text = wparam as *const u8;
    // SAFETY: the call only probes the memory.
    if text.is_null() || unsafe { IsBadStringPtrA(text, MAX_PATH_BYTES) } != 0 {
        return None;
    }
    // SAFETY: probed above, up to its terminator.
    let bytes = unsafe { std::ffi::CStr::from_ptr(text.cast()) }.to_bytes();
    (!bytes.is_empty()).then(|| crate::input::from_ansi(bytes))
}

#[cfg(not(windows))]
fn file_argument(_wparam: isize) -> Option<String> {
    None
}

/// What the plug-ins asked for since the last call.
pub fn take() -> Vec<Command> {
    PENDING
        .lock()
        .map(|mut pending| std::mem::take(&mut *pending))
        .unwrap_or_default()
}

/// The track as plug-ins read it. The strings stay valid for good: a plug-in
/// keeps a pointer it was handed, and titles are few and small.
#[derive(Clone, Copy)]
struct NowPlaying {
    position: u32,
    title_ansi: &'static [u8],
    title_wide: &'static [u16],
    file_ansi: &'static [u8],
    file_wide: &'static [u16],
}

static NOW_PLAYING: Mutex<Option<NowPlaying>> = Mutex::new(None);

fn wide(text: &str) -> &'static [u16] {
    Box::leak(
        text.encode_utf16()
            .chain(Some(0))
            .collect::<Vec<u16>>()
            .into_boxed_slice(),
    )
}

/// Remembers what plays, and returns the main window's title in the classic
/// shape: `3. Artist - Title - Winamp`.
pub fn set_now_playing(position: u32, title: &str, file: &str) -> String {
    let now = NowPlaying {
        position,
        title_ansi: Box::leak(crate::input::ansi_text(title).into_boxed_slice()),
        title_wide: wide(title),
        file_ansi: Box::leak(crate::input::ansi_text(file).into_boxed_slice()),
        file_wide: wide(file),
    };
    if let Ok(mut slot) = NOW_PLAYING.lock() {
        *slot = Some(now);
    }
    format!("{}. {title} - Winamp", position + 1)
}

/// IPC id of the event that names the file that starts to play.
#[cfg(windows)]
const IPC_PLAYING_FILE: u32 = 3003;
/// IPC id of the event that says something about the player changed.
#[cfg(windows)]
pub const IPC_CB_MISC: u32 = 603;
/// What [`IPC_CB_MISC`] says when the title changed.
#[cfg(windows)]
const IPC_CB_MISC_TITLE: usize = 0;

/// Makes `title` and `file` the current track of this process: plug-ins read it
/// through the playlist IPCs and the window's title, and those that watch the
/// player hear of it. Runs on the thread that owns `window`.
#[cfg(windows)]
pub fn announce_track(window: &crate::HostWindow, position: u32, title: &str, file: &str) {
    let text = set_now_playing(position, title, file);
    crate::fault::trace_event(&format!("now playing: {text}"));
    window.set_title(&text);
    window.notify_plugins(current_file_ansi(), IPC_PLAYING_FILE);
    window.notify_plugins(IPC_CB_MISC_TITLE, IPC_CB_MISC);
}

fn current(index: usize) -> Option<NowPlaying> {
    let now = NOW_PLAYING.lock().ok().and_then(|slot| *slot)?;
    (now.position as usize == index).then_some(now)
}

/// The current path in the form IPC 3003 hands it to plug-ins.
pub fn current_file_ansi() -> usize {
    NOW_PLAYING
        .lock()
        .ok()
        .and_then(|slot| *slot)
        .map_or(0, |now| now.file_ansi.as_ptr() as usize)
}

/// IPC 211, the path at `index`, or 0 when it is not the current track: the
/// player keeps the playlist.
pub fn file_ansi(index: usize) -> usize {
    current(index).map_or(0, |now| now.file_ansi.as_ptr() as usize)
}

/// IPC 212, the title at `index`.
pub fn title_ansi(index: usize) -> usize {
    current(index).map_or(0, |now| now.title_ansi.as_ptr() as usize)
}

/// IPC 213, the wide title at `index`, when there is one.
pub fn title_wide(index: usize) -> Option<usize> {
    current(index).map(|now| now.title_wide.as_ptr() as usize)
}

/// IPC 214, the wide path at `index`.
pub fn file_wide(index: usize) -> usize {
    current(index).map_or(0, |now| now.file_wide.as_ptr() as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The state is process-wide, so the tests take turns.
    static TURN: Mutex<()> = Mutex::new(());

    #[test]
    fn what_a_plugin_asks_reaches_the_player_once_and_in_order() {
        let _turn = TURN.lock().unwrap_or_else(|poison| poison.into_inner());
        take();
        note_window_command(40045);
        note_key(u32::from(b'V'));
        note_key(u32::from(b'Q'));
        note_ipc(106, 1500);
        note_ipc(122, 200);
        note_ipc(122, -666);
        note_ipc(104, 0);
        assert_eq!(
            take(),
            [
                Command::Play,
                Command::Stop,
                Command::Seek { to_ms: 1500 },
                Command::SetVolume(200),
            ]
        );
        assert!(take().is_empty());
    }

    #[test]
    fn the_current_track_is_answered_at_its_position_only() {
        let _turn = TURN.lock().unwrap_or_else(|poison| poison.into_inner());
        let title = set_now_playing(2, "Artist - Song", r"C:\a.mp3");
        assert_eq!(title, "3. Artist - Song - Winamp");
        assert_eq!(file_ansi(0), 0);
        assert_eq!(title_wide(1), None);
        let text = title_ansi(2) as *const u8;
        assert!(!text.is_null());
        // SAFETY: the string above, NUL-terminated and kept for good.
        let text = unsafe { std::ffi::CStr::from_ptr(text.cast()) };
        assert_eq!(text.to_str(), Ok("Artist - Song"));
        let path = file_wide(2) as *const u16;
        // SAFETY: a terminated wide string, kept for good.
        assert_eq!(unsafe { *path }, u16::from(b'C'));
    }
}
