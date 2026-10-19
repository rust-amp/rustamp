// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Skin cursors: a skin's cursor file turned into an operating-system cursor.
//!
//! A skin ships its cursors as ordinary Windows files: a `.cur` is an icon
//! directory, an `.ani` a RIFF container whose frames are icon directories
//! again. The operating system loads such a file itself, so this module
//! never decodes a cursor into pixels: it hands the file's bytes to the
//! operating system's own cursor loader (`LoadCursorFromFile`) and keeps
//! the handle the OS returns.
//!
//! # What is measured and what is a choice
//!
//! Measured (S17 §8): the base skin's cursors are ordinary `.cur` files.
//! Measured (Windows 11, 2026-10-19): `CreateIconFromResourceEx` - the
//! entry point that takes the bytes of an icon *resource* in memory -
//! refuses those very bytes and returns a null handle without an error,
//! while the operating system's own file loader accepts the same file. The
//! bytes therefore travel through a temporary file, which is deleted as
//! soon as the cursor exists.
//!
//! Chosen here: an animated cursor is shown through its **first frame** (the
//! `fram` chunk, or the first `icon` chunk of the frame sequence), because
//! the operating system's animated-cursor path (`AniCreateIconIndirect`)
//! wants a parsed `ANICURSOR` structure, and this player shows one still
//! frame of a skin's animation.
//!
//! A file the operating system refuses leaves the system's own cursor in
//! place - the same outcome as a skin that carries no cursor for the slot
//! at all. That is this player's answer to S17 §8's open question ("whether
//! a cursor file of the wrong size is rejected"): **a file the OS cannot
//! read is treated as if it were not there**, and the system's cursor is
//! shown instead.
//!
//! # The headless seam
//!
//! [`cursor_payload`] is pure: it finds the bytes the OS should read and
//! answers with a plain error when the file cannot hold a cursor. It is
//! unit-tested without an operating system. The Win32 half ([`OsCursor`],
//! [`set_window_cursor`], [`restore_cursor`]) needs the OS, and showing a
//! cursor needs a desktop.

/// The container a cursor file uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorFormat {
    /// A `.cur` file: an icon directory holding one or more images.
    Cur,
    /// An `.ani` file: a RIFF container whose frames are icon directories.
    Ani,
}

/// Why a cursor file could not be turned into an operating-system cursor.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CursorError {
    /// The file is too short to hold a cursor directory.
    #[error("the cursor file is too short to hold a cursor")]
    TooShort,
    /// The file's directory says it holds no image.
    #[error("the cursor file holds no image")]
    NoImage,
    /// The animated cursor has no frame to show.
    #[error("the animated cursor holds no frame")]
    NoFrame,
    /// The operating system refused the cursor.
    #[error("the operating system refused the cursor")]
    Refused,
    /// There is no skin cursor on this platform yet.
    #[error("skinned cursors are only implemented on Windows")]
    Unsupported,
}

/// The bytes the operating system should read the cursor from.
///
/// A `.cur` file *is* the icon directory the OS wants, so its bytes are
/// returned unchanged (after a shape check). An `.ani` file is a RIFF
/// container: its `fram` chunk carries the first frame as a cursor
/// directory, and its `LIST fram` sequence carries one `icon` chunk per
/// frame; the first of those is the fallback.
pub fn cursor_payload(bytes: &[u8], format: CursorFormat) -> Result<&[u8], CursorError> {
    match format {
        CursorFormat::Cur => {
            // ICONDIR: idReserved, idType, idCount. The OS reads the rest.
            if bytes.len() < 6 {
                return Err(CursorError::TooShort);
            }
            if count_of(bytes) == 0 {
                return Err(CursorError::NoImage);
            }
            Ok(bytes)
        }
        CursorFormat::Ani => ani_frame(bytes).ok_or(CursorError::NoFrame),
    }
}

/// The number of images the icon directory at the start of `bytes` holds.
fn count_of(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[4], bytes[5]])
}

/// The first frame of an animated cursor.
///
/// The RIFF container is `"RIFF" <size> "ANIM"` followed by chunks, each a
/// four-byte id, a 32-bit size and the data, padded to an even length.
fn ani_frame(bytes: &[u8]) -> Option<&[u8]> {
    let mut rest = bytes.strip_prefix(b"RIFF")?;
    // The RIFF size covers everything after the size field itself.
    read_u32(rest)?;
    rest = rest.get(4..)?;
    if !rest.starts_with(b"ANIM") {
        return None;
    }
    rest = rest.get(4..)?;
    while let Some((id, data, tail)) = next_chunk(rest) {
        match id {
            // The first frame, in cursor-directory form.
            b"fram" => return Some(data),
            // The frame sequence: one "icon" chunk per frame.
            b"LIST" => {
                if let Some(mut inner) = data.strip_prefix(b"fram") {
                    while let Some((id, icon, tail)) = next_chunk(inner) {
                        if id == b"icon" {
                            return Some(icon);
                        }
                        inner = tail;
                    }
                }
            }
            _ => {}
        }
        rest = tail;
    }
    None
}

/// Splits a RIFF chunk into its id, its data and the rest of the stream.
fn next_chunk(bytes: &[u8]) -> Option<(&[u8; 4], &[u8], &[u8])> {
    let id: &[u8; 4] = bytes.get(..4)?.try_into().ok()?;
    let size = read_u32(bytes.get(4..8)?)? as usize;
    let data = bytes.get(8..8 + size)?;
    // Chunks are padded to an even length.
    let padded = size + (size & 1);
    let tail = bytes.get(8 + padded..).unwrap_or(&[]);
    Some((id, data, tail))
}

/// Reads a little-endian 32-bit number.
fn read_u32(bytes: &[u8]) -> Option<u32> {
    let raw: [u8; 4] = bytes.get(..4)?.try_into().ok()?;
    Some(u32::from_le_bytes(raw))
}

/// A cursor the operating system can show.
///
/// The handle is owned: dropping the cursor destroys it, so a cache of
/// these may not outlive the skin it was built from.
pub struct OsCursor {
    /// The Win32 handle. Absent off Windows, where no skin cursor exists.
    #[cfg(windows)]
    handle: windows::Win32::UI::WindowsAndMessaging::HCURSOR,
}

impl OsCursor {
    /// Builds the operating system's cursor for a skin's cursor file.
    ///
    /// A file the operating system refuses is an error, not a panic: the
    /// caller keeps the system's own cursor.
    pub fn build(bytes: &[u8], format: CursorFormat) -> Result<Self, CursorError> {
        let payload = cursor_payload(bytes, format)?;
        build_cursor(payload)
    }
}

#[cfg(windows)]
fn build_cursor(payload: &[u8]) -> Result<OsCursor, CursorError> {
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicU32, Ordering};

    use windows::Win32::UI::WindowsAndMessaging::{HCURSOR, LoadCursorFromFileW};
    use windows::core::PCWSTR;

    /// Numbers the temporary files, so two cursors never share a name.
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    // The operating system loads a cursor from a *file*
    // (`LoadCursorFromFile`), and its own loader is the one that
    // understands every cursor format a skin may ship. Its
    // `CreateIconFromResourceEx` does not: measured on Windows 11
    // (2026-10-19), it refuses the very bytes of the built-in skin's
    // `normal.cur` and returns a null handle without an error, while
    // `LoadCursorFromFile` accepts the same file. So the bytes go through
    // a temporary file, which is deleted as soon as the cursor exists.
    let path = std::env::temp_dir().join(format!(
        "rustamp-cursor-{}-{}.cur",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, payload).map_err(|_| CursorError::Refused)?;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: the name is NUL-terminated and outlives the call; the OS
    // reads the file and keeps its own copy of the cursor.
    let loaded = unsafe { LoadCursorFromFileW(PCWSTR(wide.as_ptr())) };
    // The cursor is the OS's own now; the file is not needed any more.
    let _ = std::fs::remove_file(&path);
    let handle = loaded.map_err(|_| CursorError::Refused)?;
    Ok(OsCursor {
        handle: HCURSOR(handle.0),
    })
}

#[cfg(not(windows))]
fn build_cursor(_payload: &[u8]) -> Result<OsCursor, CursorError> {
    Err(CursorError::Unsupported)
}

#[cfg(windows)]
impl Drop for OsCursor {
    fn drop(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::DestroyCursor;

        // SAFETY: the handle was built by `LoadCursorFromFile` and is
        // owned here alone, so it is destroyed exactly once.
        let _ = unsafe { DestroyCursor(self.handle) };
    }
}

/// The window under the pointer, or `0` when the pointer is over none.
///
/// The handle is the one [`set_window_cursor`] takes, so a caller can
/// compare it with the windows it knows.
#[cfg(windows)]
pub fn pointer_window() -> isize {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, WindowFromPoint};

    let mut point = POINT::default();
    // SAFETY: `point` is a valid out-parameter.
    if unsafe { GetCursorPos(&mut point) }.is_err() {
        return 0;
    }
    // SAFETY: the point is in screen coordinates.
    unsafe { WindowFromPoint(point) }.0 as isize
}

/// Stub: there is no skin cursor off Windows yet.
#[cfg(not(windows))]
pub fn pointer_window() -> isize {
    0
}

/// Shows `cursor` while the pointer is over the window `raw_window`.
///
/// The operating system keeps a cursor that was set for a window only while
/// the pointer is over that window, so the caller restores the system's own
/// cursor when the pointer leaves (see [`restore_cursor`]).
///
/// Returns whether the cursor was applied. A window that is not the one
/// under the pointer is refused: the pointer may have moved on before the
/// event that asked for the cursor was handled, and the window it is over
/// now has a cursor of its own.
#[cfg(windows)]
pub fn set_window_cursor(raw_window: isize, cursor: &OsCursor) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::SetCursor;

    if raw_window == 0 || pointer_window() != raw_window {
        return false;
    }
    // SAFETY: the cursor is live and owned by the caller; the OS copies
    // the handle, it does not take ownership.
    unsafe { SetCursor(Some(cursor.handle)) };
    true
}

/// Stub: there is no skin cursor off Windows yet.
#[cfg(not(windows))]
pub fn set_window_cursor(_raw_window: isize, _cursor: &OsCursor) -> bool {
    false
}

/// Restores the operating system's own pointer.
///
/// The system resets a cursor that was set for a window when the pointer
/// leaves it, but the player restores it itself as well: the pointer can
/// leave a window without the system resetting the cursor, and the player
/// owes the user the system's arrow in that case.
#[cfg(windows)]
pub fn restore_cursor() {
    use windows::Win32::UI::WindowsAndMessaging::{IDC_ARROW, LoadCursorW, SetCursor};

    // The standard arrow is a shared system cursor: it must not be
    // destroyed, and asking for it is cheap.
    // SAFETY: loading a stock cursor is a plain system call.
    let Ok(arrow) = (unsafe { LoadCursorW(None, IDC_ARROW) }) else {
        return;
    };
    // SAFETY: the arrow is live and shared; `SetCursor` only reads it.
    unsafe { SetCursor(Some(arrow)) };
}

/// Stub: there is no skin cursor off Windows yet.
#[cfg(not(windows))]
pub fn restore_cursor() {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal icon directory: one 1x1 image with no pixels behind it.
    fn tiny_cur() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0u16.to_le_bytes()); // idReserved
        bytes.extend_from_slice(&2u16.to_le_bytes()); // idType: a cursor
        bytes.extend_from_slice(&1u16.to_le_bytes()); // idCount
        bytes.extend_from_slice(&[1, 1, 0, 0]); // size, colours, reserved
        bytes.extend_from_slice(&0u16.to_le_bytes()); // hotspot x
        bytes.extend_from_slice(&0u16.to_le_bytes()); // hotspot y
        bytes.extend_from_slice(&0u32.to_le_bytes()); // bytes in the image
        bytes.extend_from_slice(&22u32.to_le_bytes()); // offset of the image
        bytes
    }

    /// A RIFF container with the given chunks.
    fn riff(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (id, data) in chunks {
            body.extend_from_slice(*id);
            body.extend_from_slice(&(data.len() as u32).to_le_bytes());
            body.extend_from_slice(data);
            if data.len() % 2 == 1 {
                body.push(0);
            }
        }
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
        bytes.extend_from_slice(b"ANIM");
        bytes.extend_from_slice(&body);
        bytes
    }

    #[test]
    fn a_cur_file_is_its_own_payload() {
        let cur = tiny_cur();
        assert_eq!(cursor_payload(&cur, CursorFormat::Cur), Ok(&cur[..]));
    }

    #[test]
    fn a_cur_file_too_short_to_be_a_cursor_is_refused() {
        assert_eq!(
            cursor_payload(b"nope", CursorFormat::Cur),
            Err(CursorError::TooShort)
        );
    }

    #[test]
    fn a_cur_file_without_an_image_is_refused() {
        let mut cur = tiny_cur();
        cur[4] = 0;
        cur[5] = 0;
        assert_eq!(
            cursor_payload(&cur, CursorFormat::Cur),
            Err(CursorError::NoImage)
        );
    }

    #[test]
    fn an_animated_cursor_shows_its_first_frame() {
        let cur = tiny_cur();
        let ani = riff(&[(b"fram", cur.clone())]);
        assert_eq!(cursor_payload(&ani, CursorFormat::Ani), Ok(&cur[..]));
    }

    #[test]
    fn an_animated_cursor_falls_back_to_the_first_icon_of_its_sequence() {
        let cur = tiny_cur();
        // A frame list: the "fram" type followed by one "icon" chunk per
        // frame.
        let mut list = Vec::new();
        list.extend_from_slice(b"fram");
        list.extend_from_slice(b"icon");
        list.extend_from_slice(&(cur.len() as u32).to_le_bytes());
        list.extend_from_slice(&cur);
        let ani = riff(&[(b"LIST", list)]);
        assert_eq!(cursor_payload(&ani, CursorFormat::Ani), Ok(&cur[..]));
    }

    #[test]
    fn an_animated_cursor_without_a_frame_is_refused() {
        let ani = riff(&[(b"anih", vec![0; 36])]);
        assert_eq!(
            cursor_payload(&ani, CursorFormat::Ani),
            Err(CursorError::NoFrame)
        );
        // Not a RIFF container at all.
        assert_eq!(
            cursor_payload(b"not an ani file", CursorFormat::Ani),
            Err(CursorError::NoFrame)
        );
    }
}
