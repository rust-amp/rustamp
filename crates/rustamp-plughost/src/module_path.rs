// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The path of the module a plug-in asks about.
//!
//! The host maps an approved image itself instead of letting the loader do it,
//! so the image is **not** in the process's module list and the system's
//! `GetModuleFileName` cannot answer for it: it fails with `ERROR_MOD_NOT_FOUND`
//! or names nothing at all. A plug-in that derives its own folder that way then
//! looks in the wrong place and behaves as if it had no data - Cockos Happy AVS
//! reads `<its folder>\vis_avs.dat` in `init` and draws a black frame when the
//! read fails, while the older AVS 2.82 happens not to ask.
//!
//! So a plug-in's imports of `GetModuleFileNameA`/`GetModuleFileNameW` are
//! pointed at this module's answers: the plug-in's own image gets its path, and
//! every other handle is forwarded to the system unchanged.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The images this host mapped, and the path each came from. The lock is taken
/// for a read on every call a plug-in makes, so it is kept short.
static MAPPED: Mutex<Vec<(usize, PathBuf)>> = Mutex::new(Vec::new());

/// Remembers the path of a mapped image. A base that is noted twice keeps the
/// last path, which is the image that is mapped there now.
pub fn note(base: usize, path: &Path) {
    if let Ok(mut mapped) = MAPPED.lock() {
        mapped.retain(|(known, _)| *known != base);
        mapped.push((base, path.to_path_buf()));
    }
}

/// Forgets a mapped image, so a later map at the same base cannot inherit it.
pub fn forget(base: usize) {
    if let Ok(mut mapped) = MAPPED.lock() {
        mapped.retain(|(known, _)| *known != base);
    }
}

/// The path noted for `module`, if this host mapped it.
pub fn path_for(module: *mut c_void) -> Option<PathBuf> {
    let base = module as usize;
    MAPPED
        .lock()
        .ok()
        .and_then(|mapped| mapped.iter().find(|(known, _)| *known == base).map(|(_, path)| path.clone()))
}

/// Writes `text` and a terminator into `buffer`, as the wide API does, and
/// answers the number of units written without the terminator. `None` when the
/// buffer is too small for the whole path: the caller then reports
/// `ERROR_INSUFFICIENT_BUFFER`, the answer every caller that asks for the size
/// first relies on.
fn write_wide(text: &[u16], buffer: &mut [u16]) -> Option<u32> {
    if buffer.len() <= text.len() {
        if let Some(first) = buffer.first_mut() {
            *first = 0;
        }
        return None;
    }
    buffer[..text.len()].copy_from_slice(text);
    buffer[text.len()] = 0;
    Some(text.len() as u32)
}

/// What the wide answer turned into a narrow one for the same path. The plug-in's
/// paths are the ones the host was given, and the host's own names are ASCII, so
/// a byte per character is the honest narrow form.
fn narrow(text: &[u16]) -> Vec<u8> {
    text.iter().map(|unit| *unit as u8).collect()
}

fn write_narrow(text: &[u8], buffer: &mut [u8]) -> Option<u32> {
    if buffer.len() <= text.len() {
        if let Some(first) = buffer.first_mut() {
            *first = 0;
        }
        return None;
    }
    buffer[..text.len()].copy_from_slice(text);
    buffer[text.len()] = 0;
    Some(text.len() as u32)
}

/// The buffer's writable slice, from the caller's own length.
///
/// # Safety
///
/// `buffer` is writable for `size` units, as the API's contract says.
unsafe fn buffer_wide<'a>(buffer: *mut u16, size: u32) -> Option<&'a mut [u16]> {
    if buffer.is_null() || size == 0 {
        return None;
    }
    // SAFETY: the caller states the buffer's length.
    Some(unsafe { std::slice::from_raw_parts_mut(buffer, size as usize) })
}

/// # Safety
///
/// See [`buffer_wide`], with bytes.
unsafe fn buffer_narrow<'a>(buffer: *mut u8, size: u32) -> Option<&'a mut [u8]> {
    if buffer.is_null() || size == 0 {
        return None;
    }
    // SAFETY: the caller states the buffer's length.
    Some(unsafe { std::slice::from_raw_parts_mut(buffer, size as usize) })
}

/// The answer for a path, or `None` when the buffer is too small.
fn answer_wide(path: &Path, buffer: &mut [u16]) -> Option<u32> {
    let text: Vec<u16> = path.to_string_lossy().encode_utf16().collect();
    write_wide(&text, buffer)
}

fn answer_narrow(path: &Path, buffer: &mut [u8]) -> Option<u32> {
    let text = narrow(&path.to_string_lossy().encode_utf16().collect::<Vec<u16>>());
    write_narrow(&text, buffer)
}

const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    #[link_name = "GetModuleFileNameW"]
    fn system_get_module_file_name_w(module: *mut c_void, buffer: *mut u16, size: u32) -> u32;
    #[link_name = "GetModuleFileNameA"]
    fn system_get_module_file_name_a(module: *mut c_void, buffer: *mut u8, size: u32) -> u32;
    fn SetLastError(error: u32);
}

/// The plug-in's `GetModuleFileNameW`.
#[cfg(windows)]
unsafe extern "system" fn own_path_w(module: *mut c_void, buffer: *mut u16, size: u32) -> u32 {
    let Some(path) = path_for(module) else {
        // SAFETY: the caller's own contract, passed on unchanged.
        return unsafe { system_get_module_file_name_w(module, buffer, size) };
    };
    let Some(buffer) = (unsafe { buffer_wide(buffer, size) }) else {
        return 0;
    };
    match answer_wide(&path, buffer) {
        Some(written) => written,
        None => {
            // SAFETY: no pointer arguments.
            unsafe { SetLastError(ERROR_INSUFFICIENT_BUFFER) };
            0
        }
    }
}

/// The plug-in's `GetModuleFileNameA`.
#[cfg(windows)]
unsafe extern "system" fn own_path_a(module: *mut c_void, buffer: *mut u8, size: u32) -> u32 {
    let Some(path) = path_for(module) else {
        // SAFETY: the caller's own contract, passed on unchanged.
        return unsafe { system_get_module_file_name_a(module, buffer, size) };
    };
    let Some(buffer) = (unsafe { buffer_narrow(buffer, size) }) else {
        return 0;
    };
    match answer_narrow(&path, buffer) {
        Some(written) => written,
        None => {
            // SAFETY: no pointer arguments.
            unsafe { SetLastError(ERROR_INSUFFICIENT_BUFFER) };
            0
        }
    }
}

/// Points a mapped image's `GetModuleFileNameA`/`W` imports at the host's own
/// answer and notes the image's path. Answers whether at least one slot was
/// taken, which is what a plug-in that derives its folder needs.
///
/// # Safety
///
/// `module` is the base of a library mapped in this process, and the caller runs
/// on the thread that loads it, before the plug-in's `init` can ask.
#[cfg(windows)]
pub unsafe fn answer_own_path(module: *mut c_void, path: &Path) -> bool {
    note(module as usize, path);
    let mut taken = false;
    for (function, replacement) in [
        (
            "GetModuleFileNameA",
            own_path_a as unsafe extern "system" fn(*mut c_void, *mut u8, u32) -> u32 as usize,
        ),
        (
            "GetModuleFileNameW",
            own_path_w as unsafe extern "system" fn(*mut c_void, *mut u16, u32) -> u32 as usize,
        ),
    ] {
        if unsafe { crate::os_version::point_import(module, "KERNEL32.dll", function, replacement) }
        {
            taken = true;
            crate::fault::trace_event(&format!(
                "{function} of the plug-in now answers its own folder; it is not in the loader's list"
            ));
        }
    }
    taken
}

/// Without an image mapper there is nothing to answer for.
#[cfg(not(windows))]
pub fn answer_own_path(_module: *mut c_void, _path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_noted_image_answers_with_its_path_and_others_do_not() {
        let base = 0x1000_0000usize;
        note(base, Path::new(r"C:\plugins\vis_chavs.dll"));
        assert_eq!(
            path_for(base as *mut c_void),
            Some(PathBuf::from(r"C:\plugins\vis_chavs.dll"))
        );
        assert_eq!(path_for(0x2000_0000usize as *mut c_void), None);
        forget(base);
        assert_eq!(path_for(base as *mut c_void), None);
    }

    #[test]
    fn a_second_note_at_the_same_base_wins() {
        let base = 0x3000_0000usize;
        note(base, Path::new(r"C:\a\one.dll"));
        note(base, Path::new(r"C:\b\two.dll"));
        assert_eq!(
            path_for(base as *mut c_void),
            Some(PathBuf::from(r"C:\b\two.dll"))
        );
        forget(base);
    }

    #[test]
    fn a_buffer_that_is_too_small_is_reported_as_the_api_reports_it() {
        let path = Path::new(r"C:\plugins\vis_chavs.dll");
        let mut small = [0u16; 4];
        assert_eq!(answer_wide(path, &mut small), None);
        assert_eq!(small[0], 0, "the truncated answer is an empty string");

        let mut roomy = [0u16; 64];
        let written = answer_wide(path, &mut roomy).expect("the whole path fits");
        assert_eq!(written as usize, path.to_string_lossy().len());
        assert_eq!(roomy[written as usize], 0);

        let mut narrow_room = [0u8; 64];
        let written = answer_narrow(path, &mut narrow_room).expect("the whole path fits");
        assert_eq!(&narrow_room[..written as usize], path.to_string_lossy().as_bytes());
    }
}
