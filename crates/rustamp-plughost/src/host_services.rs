//! The services a Winamp 5.x plug-in expects of its host at start-up (S21).
//!
//! A plug-in written for a 5.x host asks the host window for a *service
//! manager* (IPC 3025), looks the services it needs up by a 16-byte identifier
//! and calls them through one **generic call** (S21 §3). The 2.95 reference
//! answers 3025 with 1, "unsupported" (S10), and most 5.x plug-ins call through
//! that answer unchecked and fault. This module provides the least that lets
//! them start: a service manager that knows the **language service** (strings
//! from the plug-in's own resource table, S21 §5.1), plus the settings paths
//! and random generators the plug-ins ask for through IPC (S21 §4).
//!
//! Everything here is the **answer to a request sent by code inside the
//! plug-in host process**; the 2.95 answers of S10 stay the default for every
//! other sender (see [`crate::host_window`]).
//!
//! The message handling is plain Rust over raw argument arrays, so it is tested
//! headlessly on any target. Only the thin member-call shim that real plug-ins
//! reach it through exists on 32-bit x86 (`extern "thiscall"`).

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

/// The window message plug-ins send the main window (`WM_USER`, S10).
pub const WM_WA_IPC: u32 = 0x400;

/// IPC id: the pointer to the service manager (S21 §4).
pub const IPC_GET_API_SERVICE: u32 = 3025;
/// IPC id: ANSI file name of the plug-in's settings (S21 §4).
pub const IPC_GETINIFILE: u32 = 334;
/// IPC id: ANSI directory the plug-in may create a sub-directory in.
pub const IPC_GETINIDIRECTORY: u32 = 336;
/// IPC id: the settings file name as wide text.
pub const IPC_GETINIFILEW: u32 = 1334;
/// IPC id: the same directory as wide text.
pub const IPC_GETINIDIRECTORYW: u32 = 1335;
/// IPC id: the addresses of two random generators (wparam 0 int, 1 float).
pub const IPC_GET_RANDFUNC: u32 = 3015;
/// IPC id: the plug-ins folder as wide text; `Milkdrop2\data` sits below it
/// (S21 addendum 1, notes 5).
pub const IPC_GETPLUGINDIRECTORYW: u32 = 1336;
/// IPC id: which d3dx9 library the host has loaded (0 = none, the plug-in loads
/// its own by name; S21 addendum 1).
pub const IPC_GET_D3DX9: u32 = 3044;
/// IPC id: the wide title of a playlist position, as a pointer to a wide string
/// that stays valid for the call; asked every frame by MilkDrop.
pub const IPC_GETPLAYLISTTITLEW: u32 = 213;
/// IPC id 505: asks for a function that embeds a window in the host; the answer
/// is called as a function when it is not 0 (see [`crate::embed_frame`]). The 2.95
/// answer of S10 (1) stays for the player's own window.
pub const IPC_GET_EMBEDIF: u32 = 505;
/// IPC id 201: the skin directory (S10 §8.2). The host has no skin of its own,
/// and "1", the answer for an unknown id, would be read as a string at address 1.
/// `vis_avs` does, and tests for null.
pub const IPC_GETSKIN: u32 = 201;
/// IPC ids a plug-in asks of a real player, for which the host has nothing to give
/// or do. "1", the answer for an id nobody knows, would be taken for a handle or a
/// state (`gen_win7shell` asks for ratings, `gen_thinger` for a skin bitmap), so
/// these answer 0. The list is what free plug-ins' sources ask for, by the numbers
/// of their own headers.
pub const IPC_KNOWN_WITHOUT_ANSWER: [u32; 29] = [
    100, 103, 120, 129, 200, 202, 241, 243, 245, 246, 247, 248, 249, 280, 362, 364, 503, 606, 611,
    628, 631, 633, 635, 638, 639, 640, 3002, 3004, 3010,
];
/// IPC ids of the playlist's strings (S10 §8.2): `index` in, a pointer out, null
/// when the host does not know the entry (see [`crate::plugin_commands`]).
pub const IPC_GETPLAYLISTFILE: u32 = 211;
/// See [`IPC_GETPLAYLISTFILE`].
pub const IPC_GETPLAYLISTTITLE: u32 = 212;
/// See [`IPC_GETPLAYLISTFILE`].
pub const IPC_GETPLAYLISTFILEW: u32 = 214;

/// What the generic call is asked about (S21 §3 and §5.1).
const MSG_REGISTER_FACTORY: i32 = 10;
const MSG_DEREGISTER_FACTORY: i32 = 20;
const MSG_LOOKUP_SERVICE: i32 = 50;
const MSG_FACTORY_GET: i32 = 300;
const MSG_FACTORY_RELEASE: i32 = 310;
const MSG_LANGUAGE_START: i32 = 30;
const MSG_LANGUAGE_MENU: i32 = 52;
const MSG_LANGUAGE_ANSI: i32 = 10;
const MSG_LANGUAGE_WIDE: i32 = 11;

/// The identifier of the language service as it sits in memory: the usual
/// 16-byte layout of a GUID, the first three groups little-endian
/// (30AED4E5-EF10-4277-8D49-27AB5570E891).
pub const LANGUAGE_SERVICE_ID: [u8; 16] = [
    0xE5, 0xD4, 0xAE, 0x30, 0x10, 0xEF, 0x77, 0x42, 0x8D, 0x49, 0x27, 0xAB, 0x55, 0x70, 0xE8, 0x91,
];

/// The application service and the service MilkDrop looks up beside it
/// (23B96771-09D7-46D3-9AE2-20DCEA6C86EA and 57B7A1B6-700E-44FF-9CB0-70B92BAF3959):
/// the host has nothing to offer for them, but the plug-in calls what it gets
/// unchecked from user actions (help, web links), so it gets a service that
/// answers every message with "not understood" (S21 addendum 1, notes 6).
///
/// The Winamp 5.66 input plug-ins look up two more
/// (AEFBF8BE-E0AA-4318-8CC1-4353410B64DC and 000CF46E-4DF6-4A43-BBE7-40E7A3EA02ED),
/// call the first in `play` without a null check, and take "not understood" for
/// an answer.
pub const NULL_SERVICE_IDS: [[u8; 16]; 4] = [
    [
        0x71, 0x67, 0xB9, 0x23, 0xD7, 0x09, 0xD3, 0x46, 0x9A, 0xE2, 0x20, 0xDC, 0xEA, 0x6C, 0x86,
        0xEA,
    ],
    [
        0xB6, 0xA1, 0xB7, 0x57, 0x0E, 0x70, 0xFF, 0x44, 0x9C, 0xB0, 0x70, 0xB9, 0x2B, 0xAF, 0x39,
        0x59,
    ],
    [
        0xBE, 0xF8, 0xFB, 0xAE, 0xAA, 0xE0, 0x18, 0x43, 0x8C, 0xC1, 0x43, 0x53, 0x41, 0x0B, 0x64,
        0xDC,
    ],
    [
        0x6E, 0xF4, 0x0C, 0x00, 0xF6, 0x4D, 0x43, 0x4A, 0xBB, 0xE7, 0x40, 0xE7, 0xA3, 0xEA, 0x02,
        0xED,
    ],
];

/// What a language "handle" is when the plug-in gave no module: any non-zero
/// number (S21 §5.1). With a module, the handle IS the module (addendum 1).
const LANGUAGE_HANDLE: usize = 0x4C41_4E47;

/// Which of the three objects a call is addressed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ObjectKind {
    /// The service manager (IPC 3025 hands out a pointer to it).
    Manager = 1,
    /// The factory of the language service.
    LanguageFactory = 2,
    /// The language service itself.
    Language = 3,
    /// The factory of a service the host does not provide but must hand out.
    NullFactory = 4,
    /// That service: it answers every message with "not understood".
    Null = 5,
}

/// An object in the shape every caller expects: the first word points at a
/// table whose entry 0 is the generic call (S21 §3).
#[repr(C)]
pub struct ServiceObject {
    table: *const *const c_void,
    kind: ObjectKind,
}

// SAFETY: the objects are immutable statics that only hold pointers to other
// immutable statics.
unsafe impl Sync for ServiceObject {}

/// The table: one entry, the generic call.
#[repr(C)]
struct Table([*const c_void; 1]);

// SAFETY: immutable, holds a function address.
unsafe impl Sync for Table {}

#[cfg(target_arch = "x86")]
static TABLE: Table = Table([generic_call_shim as *const c_void]);

/// Off x86 nothing can call the objects as a real plug-in would (the member
/// call convention does not exist there); the table keeps the shape for tests.
#[cfg(not(target_arch = "x86"))]
static TABLE: Table = Table([generic_call_host as *const c_void]);

static MANAGER: ServiceObject = ServiceObject {
    table: TABLE.0.as_ptr(),
    kind: ObjectKind::Manager,
};
static LANGUAGE_FACTORY: ServiceObject = ServiceObject {
    table: TABLE.0.as_ptr(),
    kind: ObjectKind::LanguageFactory,
};
static LANGUAGE: ServiceObject = ServiceObject {
    table: TABLE.0.as_ptr(),
    kind: ObjectKind::Language,
};
static NULL_FACTORY: ServiceObject = ServiceObject {
    table: TABLE.0.as_ptr(),
    kind: ObjectKind::NullFactory,
};
static NULL_SERVICE: ServiceObject = ServiceObject {
    table: TABLE.0.as_ptr(),
    kind: ObjectKind::Null,
};

/// The pointer a plug-in is given for IPC 3025 and in the input module's
/// service-manager member (S21 §1 question 1).
pub fn service_manager() -> *const ServiceObject {
    &MANAGER
}

fn object_of(kind: ObjectKind) -> *const ServiceObject {
    match kind {
        ObjectKind::Manager => &MANAGER,
        ObjectKind::LanguageFactory => &LANGUAGE_FACTORY,
        ObjectKind::Language => &LANGUAGE,
        ObjectKind::NullFactory => &NULL_FACTORY,
        ObjectKind::Null => &NULL_SERVICE,
    }
}

/// Reads argument `index` as a machine word (S21 §3: every argument is passed
/// by address).
///
/// # Safety
///
/// `args` must point to `count` readable pointers, each to a readable word.
unsafe fn word_arg(args: *const *const c_void, count: i32, index: usize) -> Option<usize> {
    if args.is_null() || index >= usize::try_from(count).unwrap_or(0) {
        return None;
    }
    // SAFETY: the caller's contract.
    let at = unsafe { *args.add(index) };
    if at.is_null() {
        return None;
    }
    // SAFETY: the caller's contract; unaligned reads cost nothing here.
    Some(unsafe { at.cast::<usize>().read_unaligned() })
}

/// Reads argument `index` as the address of a 16-byte identifier.
///
/// # Safety
///
/// As [`word_arg`], and the argument's address must point to 16 bytes.
unsafe fn id_arg(args: *const *const c_void, count: i32, index: usize) -> Option<[u8; 16]> {
    if args.is_null() || index >= usize::try_from(count).unwrap_or(0) {
        return None;
    }
    // SAFETY: the caller's contract.
    let at = unsafe { *args.add(index) };
    if at.is_null() {
        return None;
    }
    let mut id = [0u8; 16];
    // SAFETY: the caller's contract: 16 readable bytes.
    unsafe { std::ptr::copy_nonoverlapping(at.cast::<u8>(), id.as_mut_ptr(), 16) };
    Some(id)
}

/// The generic call, for every object (S21 §3): answers `message` and writes
/// the result into `*result`; returns 1 when it handled the message, 0 when it
/// did not (the result cell is then left alone).
///
/// # Safety
///
/// `this` must point to one of this module's objects; `result` to a writable
/// word; `args` to `count` readable pointers to the arguments.
pub unsafe fn generic_call_host(
    this: *const ServiceObject,
    message: i32,
    result: *mut usize,
    args: *const *const c_void,
    count: i32,
) -> i32 {
    if this.is_null() || result.is_null() {
        return 0;
    }
    // SAFETY: the caller's contract.
    let kind = unsafe { (*this).kind };
    crate::fault::trace_event(&format!(
        "service call: {kind:?} message {message} with {count} argument(s)"
    ));
    // SAFETY: forwarded.
    match kind {
        ObjectKind::Manager => unsafe { manager_call(message, result, args, count) },
        ObjectKind::LanguageFactory => match message {
            MSG_FACTORY_GET => {
                // SAFETY: a writable result cell.
                unsafe { *result = object_of(ObjectKind::Language) as usize };
                1
            }
            MSG_FACTORY_RELEASE => 1,
            _ => 0,
        },
        ObjectKind::Language => unsafe { language_call(message, result, args, count) },
        ObjectKind::NullFactory => match message {
            MSG_FACTORY_GET => {
                // SAFETY: a writable result cell.
                unsafe { *result = object_of(ObjectKind::Null) as usize };
                1
            }
            MSG_FACTORY_RELEASE => 1,
            _ => 0,
        },
        // Every message of this service is "not understood".
        ObjectKind::Null => 0,
    }
}

/// # Safety
///
/// As [`generic_call_host`].
unsafe fn manager_call(
    message: i32,
    result: *mut usize,
    args: *const *const c_void,
    count: i32,
) -> i32 {
    match message {
        // A plug-in registering or deregistering a factory of its own: accepted;
        // nothing at start-up calls into it (S21 §3).
        MSG_REGISTER_FACTORY | MSG_DEREGISTER_FACTORY => 1,
        MSG_LOOKUP_SERVICE => {
            // SAFETY: the caller's contract.
            let Some(id) = (unsafe { id_arg(args, count, 0) }) else {
                return 0;
            };
            if id == LANGUAGE_SERVICE_ID {
                // SAFETY: a writable result cell.
                unsafe { *result = object_of(ObjectKind::LanguageFactory) as usize };
                1
            } else if NULL_SERVICE_IDS.contains(&id) {
                // SAFETY: a writable result cell.
                unsafe { *result = object_of(ObjectKind::NullFactory) as usize };
                1
            } else {
                // The other services are looked up null-checked at start-up (S21 §5).
                crate::fault::trace_event(&format!("service lookup: unknown identifier {id:02X?}"));
                0
            }
        }
        _ => 0,
    }
}

/// # Safety
///
/// As [`generic_call_host`].
unsafe fn language_call(
    message: i32,
    result: *mut usize,
    args: *const *const c_void,
    count: i32,
) -> i32 {
    match message {
        MSG_LANGUAGE_START => {
            // The result is used as a module handle to find the plug-in's own
            // resources: it is the module the plug-in passed (S21 addendum 1,
            // notes 1). Without one, any non-zero number.
            // SAFETY: the caller's contract.
            let module = unsafe { word_arg(args, count, 0) }.unwrap_or(LANGUAGE_HANDLE);
            // SAFETY: a writable result cell.
            unsafe { *result = if module == 0 { LANGUAGE_HANDLE } else { module } };
            1
        }
        MSG_LANGUAGE_MENU => {
            // Arguments: language handle (ignored), module handle, resource id.
            // The menu is the plug-in's own resource; the plug-in destroys the
            // handle, so every call makes a fresh one (S21 addendum 1, notes 2).
            // SAFETY: the caller's contract.
            let (Some(module), Some(id)) = (unsafe { word_arg(args, count, 1) }, unsafe {
                word_arg(args, count, 2)
            }) else {
                return 0;
            };
            let menu = load_menu(module, id as u32);
            if menu == 0 {
                return 0;
            }
            // SAFETY: a writable result cell.
            unsafe { *result = menu };
            1
        }
        MSG_LANGUAGE_ANSI | MSG_LANGUAGE_WIDE => {
            // Arguments: language handle, module handle, string id, destination
            // buffer or null, its length (S21 §5.1).
            // SAFETY: the caller's contract.
            let (Some(module), Some(id), Some(buffer), Some(length)) = (unsafe {
                (
                    word_arg(args, count, 1),
                    word_arg(args, count, 2),
                    word_arg(args, count, 3),
                    word_arg(args, count, 4),
                )
            }) else {
                return 0;
            };
            let wide = message == MSG_LANGUAGE_WIDE;
            // SAFETY: the module handle is the plug-in's own; the buffer is the
            // caller's, `length` units long.
            let text = unsafe { load_string(wide, module, id as u32, buffer, length) };
            // SAFETY: a writable result cell.
            unsafe { *result = text };
            1
        }
        _ => 0,
    }
}

/// Strings handed out for a null destination live in this ring: a caller prints
/// or formats them at once, and 256 entries are long enough for that.
static HANDED_OUT: Mutex<Vec<Box<[u8]>>> = Mutex::new(Vec::new());
const HANDED_OUT_LIMIT: usize = 256;

fn keep(text: Vec<u8>) -> usize {
    let boxed = text.into_boxed_slice();
    let at = boxed.as_ptr() as usize;
    if let Ok(mut ring) = HANDED_OUT.lock() {
        if ring.len() >= HANDED_OUT_LIMIT {
            ring.remove(0);
        }
        ring.push(boxed);
    }
    at
}

/// Loads string `id` of the string table of `module` (ANSI or wide). With a
/// buffer, copies into it (at most `length` units, terminated) and returns the
/// buffer; with none, returns a pointer to a private copy. A missing id gives a
/// valid empty string, never null (S21 §5.1).
///
/// # Safety
///
/// `buffer` is null or points to `length` writable units of the right width.
unsafe fn load_string(wide: bool, module: usize, id: u32, buffer: usize, length: usize) -> usize {
    let units = if wide {
        load_units_wide(module, id)
    } else {
        load_units_ansi(module, id)
    };
    let width = if wide { 2 } else { 1 };
    if buffer == 0 || length == 0 {
        let mut bytes = Vec::with_capacity((units.len() + 1) * width);
        for unit in &units {
            bytes.extend_from_slice(&unit.to_le_bytes()[..width]);
        }
        bytes.extend(std::iter::repeat_n(0u8, width));
        return keep(bytes);
    }
    let copy = units.len().min(length - 1);
    for (index, unit) in units.iter().take(copy).enumerate() {
        // SAFETY: within the caller's buffer.
        unsafe {
            if wide {
                (buffer as *mut u16).add(index).write_unaligned(*unit);
            } else {
                (buffer as *mut u8).add(index).write(*unit as u8);
            }
        }
    }
    // SAFETY: `copy < length`.
    unsafe {
        if wide {
            (buffer as *mut u16).add(copy).write_unaligned(0);
        } else {
            (buffer as *mut u8).add(copy).write(0);
        }
    }
    buffer
}

#[cfg(windows)]
fn load_units_wide(module: usize, id: u32) -> Vec<u16> {
    use windows::Win32::Foundation::HINSTANCE;
    use windows::Win32::UI::WindowsAndMessaging::LoadStringW;
    let mut buffer = [0u16; 1024];
    // SAFETY: a valid buffer of the length passed; a bad module handle makes
    // the call return 0.
    let len = unsafe {
        LoadStringW(
            Some(HINSTANCE(module as *mut c_void)),
            id,
            windows::core::PWSTR(buffer.as_mut_ptr()),
            buffer.len() as i32,
        )
    };
    buffer[..usize::try_from(len).unwrap_or(0)].to_vec()
}

#[cfg(windows)]
fn load_units_ansi(module: usize, id: u32) -> Vec<u16> {
    use windows::Win32::Foundation::HINSTANCE;
    use windows::Win32::UI::WindowsAndMessaging::LoadStringA;
    let mut buffer = [0u8; 1024];
    // SAFETY: as above.
    let len = unsafe {
        LoadStringA(
            Some(HINSTANCE(module as *mut c_void)),
            id,
            windows::core::PSTR(buffer.as_mut_ptr()),
            buffer.len() as i32,
        )
    };
    buffer[..usize::try_from(len).unwrap_or(0)]
        .iter()
        .map(|b| u16::from(*b))
        .collect()
}

/// Loads menu resource `id` of `module`; 0 when there is none.
#[cfg(windows)]
fn load_menu(module: usize, id: u32) -> usize {
    use windows::Win32::Foundation::HINSTANCE;
    use windows::Win32::UI::WindowsAndMessaging::LoadMenuW;
    // A resource id is passed in the low word of the name pointer.
    // SAFETY: a bad module handle or id makes the call fail, not fault.
    unsafe {
        LoadMenuW(
            Some(HINSTANCE(module as *mut c_void)),
            windows::core::PCWSTR(id as usize as *const u16),
        )
    }
    .map_or(0, |menu| menu.0 as usize)
}

#[cfg(not(windows))]
fn load_menu(_module: usize, _id: u32) -> usize {
    0
}

#[cfg(not(windows))]
fn load_units_wide(_module: usize, _id: u32) -> Vec<u16> {
    Vec::new()
}

#[cfg(not(windows))]
fn load_units_ansi(_module: usize, _id: u32) -> Vec<u16> {
    Vec::new()
}

/// The member-call shim: object pointer in ECX, four stack arguments that the
/// callee removes (S21 §3).
#[cfg(target_arch = "x86")]
unsafe extern "thiscall" fn generic_call_shim(
    this: *const ServiceObject,
    message: i32,
    result: *mut usize,
    args: *const *const c_void,
    count: i32,
) -> i32 {
    // SAFETY: the plug-in calls with the arguments of the generic call.
    unsafe { generic_call_host(this, message, result, args, count) }
}

/// The settings paths of S21 §4: where the host keeps what plug-ins save.
struct Paths {
    file_ansi: Vec<u8>,
    directory_ansi: Vec<u8>,
    file_wide: Vec<u16>,
    /// The settings folder: it already holds a `Plugins` folder (IPC 1335).
    base_wide: Vec<u16>,
    /// The plug-ins folder (IPC 1336).
    directory_wide: Vec<u16>,
}

/// Builds the answers for a settings folder `base` and a plug-ins folder
/// `directory` (a sub-folder of it, in the classic layout).
fn build_paths(base: &std::path::Path, directory: &std::path::Path) -> Paths {
    // Created so a plug-in that makes a sub-directory below it can (S21 §4).
    let _ = std::fs::create_dir_all(directory);
    let file = base.join("plugin-settings.ini");
    let lossy = |path: &std::path::Path| path.to_string_lossy().into_owned();
    let ansi = |text: String| {
        let mut bytes = text.into_bytes();
        bytes.push(0);
        bytes
    };
    let wide = |text: String| {
        let mut units: Vec<u16> = text.encode_utf16().collect();
        units.push(0);
        units
    };
    Paths {
        file_ansi: ansi(lossy(&file)),
        directory_ansi: ansi(lossy(directory)),
        file_wide: wide(lossy(&file)),
        base_wide: wide(lossy(base)),
        directory_wide: wide(lossy(directory)),
    }
}

/// The folder of the plug-in loaded last, as the host knows it.
static PLUGIN_FOLDER: Mutex<Option<std::path::PathBuf>> = Mutex::new(None);

/// Records the folder a plug-in is being loaded from (the loader calls this).
///
/// In a classic layout that folder is named `Plugins`; the plug-ins ask the host
/// for it (IPC 336, 1336) and for the folder above it (1335) and look for their
/// own data below (`Milkdrop2\data` for MilkDrop), so the answers follow the
/// plug-in: a player installed as `<root>\Plugins\vis_milk2.dll` with
/// `<root>\Plugins\Milkdrop2\data` beside it needs no other setup. A plug-in
/// in a folder with another name, or `RUSTAMP_PLUGIN_SETTINGS` set (the test
/// seam), keeps the default settings folder of the host.
pub fn note_plugin_folder(folder: &std::path::Path) {
    // The canonical path carries the `\\?\` prefix; the plug-ins get plain paths.
    let text = folder.to_string_lossy();
    let plain = text.strip_prefix(r"\\?\").unwrap_or(&text);
    if let Ok(mut slot) = PLUGIN_FOLDER.lock() {
        *slot = Some(std::path::PathBuf::from(plain));
    }
}

/// The answer set to use now: the host's default, or the one that follows the
/// folder of the plug-in loaded last (see [`note_plugin_folder`]).
///
/// Every set is kept for the rest of the process: a pointer handed to a plug-in
/// must stay valid.
fn paths() -> &'static Paths {
    use std::collections::HashMap;
    static DEFAULT: OnceLock<Paths> = OnceLock::new();
    static BY_FOLDER: Mutex<Option<HashMap<std::path::PathBuf, &'static Paths>>> = Mutex::new(None);

    let override_dir = std::env::var_os("RUSTAMP_PLUGIN_SETTINGS").map(std::path::PathBuf::from);
    let folder = PLUGIN_FOLDER.lock().ok().and_then(|slot| slot.clone());
    let classic = folder.filter(|folder| {
        override_dir.is_none()
            && folder
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("plugins"))
            && folder.parent().is_some()
    });
    if let Some(folder) = classic
        && let Ok(mut map) = BY_FOLDER.lock()
    {
        let map = map.get_or_insert_with(HashMap::new);
        let base = folder.parent().expect("checked above").to_path_buf();
        let paths: &'static Paths = map
            .entry(folder.clone())
            .or_insert_with(|| Box::leak(Box::new(build_paths(&base, &folder))));
        return paths;
    }
    DEFAULT.get_or_init(|| {
        let base = override_dir
            .or_else(|| {
                std::env::var_os("APPDATA").map(|dir| std::path::Path::new(&dir).join("Rustamp"))
            })
            .unwrap_or_else(|| std::env::temp_dir().join("Rustamp"));
        build_paths(&base, &base.join("plugins"))
    })
}

/// An empty wide string, for answers that must be a valid pointer but have no
/// text to give (the title of a playlist position the host does not hold).
struct EmptyWide([u16; 1]);

// SAFETY: immutable.
unsafe impl Sync for EmptyWide {}

static EMPTY_WIDE: EmptyWide = EmptyWide([0]);

/// A random non-negative int, 15 bits (S21 §4: what the plug-ins index with).
extern "C" fn random_int() -> i32 {
    ((next_random() >> 8) & 0x7FFF) as i32
}

/// A random float in `[0, 1)`.
extern "C" fn random_float() -> f32 {
    (next_random() >> 8) as f32 / 16_777_216.0
}

fn next_random() -> u32 {
    static STATE: AtomicU32 = AtomicU32::new(0x2545_F491);
    // xorshift32; a lost update between threads only costs randomness.
    let mut x = STATE.load(Ordering::Relaxed);
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    STATE.store(x, Ordering::Relaxed);
    x
}

/// The 5.x answer to window message [`WM_WA_IPC`] `id`, or `None` when the id is
/// not one of the start-up requests of S21 §4 (the surface then answers as the
/// 2.95 reference does).
///
/// Only for requests sent by code inside this process: the caller decides.
pub fn ipc_answer(id: u32, wparam: usize) -> Option<usize> {
    if IPC_KNOWN_WITHOUT_ANSWER.contains(&id) {
        return Some(0);
    }
    Some(match id {
        IPC_GETSKIN => 0,
        IPC_GETPLAYLISTFILE => crate::plugin_commands::file_ansi(wparam),
        IPC_GETPLAYLISTTITLE => crate::plugin_commands::title_ansi(wparam),
        IPC_GETPLAYLISTFILEW => crate::plugin_commands::file_wide(wparam),
        IPC_GET_API_SERVICE => service_manager() as usize,
        // With a state block as its parameter the request is the call itself
        // (S10 §13).
        IPC_GET_EMBEDIF if wparam != 0 => crate::embed_frame::embed_now(wparam),
        IPC_GET_EMBEDIF => crate::embed_frame::embed_function(),
        IPC_GETINIFILE => paths().file_ansi.as_ptr() as usize,
        IPC_GETINIDIRECTORY => paths().directory_ansi.as_ptr() as usize,
        IPC_GETINIFILEW => paths().file_wide.as_ptr() as usize,
        IPC_GETINIDIRECTORYW => paths().base_wide.as_ptr() as usize,
        IPC_GETPLUGINDIRECTORYW => paths().directory_wide.as_ptr() as usize,
        IPC_GET_D3DX9 => 0,
        IPC_GETPLAYLISTTITLEW => {
            crate::plugin_commands::title_wide(wparam).unwrap_or(EMPTY_WIDE.0.as_ptr() as usize)
        }
        IPC_GET_RANDFUNC => {
            if wparam == 1 {
                random_float as extern "C" fn() -> f32 as usize
            } else {
                random_int as extern "C" fn() -> i32 as usize
            }
        }
        _ => return None,
    })
}

/// The offset of the service-manager member in a 5.x input module (S21 §6).
pub const INPUT_SERVICE_MANAGER_OFFSET: usize = 0x98;

/// The input version value that marks modules which carry that member.
pub const INPUT_VERSION_WITH_SERVICES: u32 = 0xFF00_0101;

#[cfg(test)]
mod tests {
    use super::*;

    /// The answers follow a process-wide "plug-in folder"; tests that read the
    /// paths or change that folder take turns.
    static PATHS_LOCK: Mutex<()> = Mutex::new(());

    fn paths_turn() -> std::sync::MutexGuard<'static, ()> {
        PATHS_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Calls the generic call the way a plug-in's argument array looks: every
    /// argument passed by its address.
    fn call(object: *const ServiceObject, message: i32, args: &[*const c_void]) -> (i32, usize) {
        let mut result = 0usize;
        // SAFETY: live objects, a writable cell, live argument addresses.
        let handled = unsafe {
            generic_call_host(
                object,
                message,
                &mut result,
                args.as_ptr(),
                args.len() as i32,
            )
        };
        (handled, result)
    }

    fn object_at(address: usize) -> *const ServiceObject {
        address as *const ServiceObject
    }

    #[test]
    fn the_manager_knows_the_language_service_and_nothing_else() {
        let (handled, factory) = call(
            service_manager(),
            MSG_LOOKUP_SERVICE,
            &[LANGUAGE_SERVICE_ID.as_ptr().cast()],
        );
        assert_eq!(handled, 1);
        assert_ne!(factory, 0);

        let other = [7u8; 16];
        let (handled, _) = call(
            service_manager(),
            MSG_LOOKUP_SERVICE,
            &[other.as_ptr().cast()],
        );
        assert_eq!(handled, 0, "an unknown service is not understood");
    }

    #[test]
    fn registering_a_factory_is_accepted_and_unknown_messages_are_not() {
        let factory = 0usize;
        assert_eq!(
            call(
                service_manager(),
                MSG_REGISTER_FACTORY,
                &[(&factory as *const usize).cast()]
            )
            .0,
            1
        );
        assert_eq!(
            call(
                service_manager(),
                MSG_DEREGISTER_FACTORY,
                &[(&factory as *const usize).cast()]
            )
            .0,
            1
        );
        assert_eq!(call(service_manager(), 9999, &[]).0, 0);
    }

    #[test]
    fn the_factory_hands_out_the_language_service() {
        let (_, factory) = call(
            service_manager(),
            MSG_LOOKUP_SERVICE,
            &[LANGUAGE_SERVICE_ID.as_ptr().cast()],
        );
        let one = 1usize;
        let (handled, language) = call(
            object_at(factory),
            MSG_FACTORY_GET,
            &[(&one as *const usize).cast()],
        );
        assert_eq!(handled, 1);
        assert_eq!(language, object_of(ObjectKind::Language) as usize);
        assert_eq!(call(object_at(factory), MSG_FACTORY_RELEASE, &[]).0, 1);
        assert_eq!(call(object_at(factory), 5, &[]).0, 0);
    }

    #[test]
    fn starting_the_language_service_yields_a_non_zero_handle() {
        let module = 0x1000usize;
        let id = [1u8; 16];
        let (handled, handle) = call(
            object_of(ObjectKind::Language),
            MSG_LANGUAGE_START,
            &[(&module as *const usize).cast(), id.as_ptr().cast()],
        );
        assert_eq!(handled, 1);
        assert_ne!(handle, 0);
    }

    #[test]
    fn a_missing_string_is_a_valid_empty_string_never_null() {
        // Module 0 and an id no table has: the load finds nothing.
        let (handle, module, id, buffer, length) =
            (LANGUAGE_HANDLE, 0usize, 4242usize, 0usize, 0usize);
        let args: [*const c_void; 5] = [
            (&handle as *const usize).cast(),
            (&module as *const usize).cast(),
            (&id as *const usize).cast(),
            (&buffer as *const usize).cast(),
            (&length as *const usize).cast(),
        ];
        for message in [MSG_LANGUAGE_ANSI, MSG_LANGUAGE_WIDE] {
            let (handled, text) = call(object_of(ObjectKind::Language), message, &args);
            assert_eq!(handled, 1);
            assert_ne!(text, 0, "never null");
            // SAFETY: the pointer is the module's own terminated copy.
            let first = unsafe { *(text as *const u8) };
            assert_eq!(first, 0, "an empty string");
        }
    }

    #[test]
    fn a_given_buffer_is_filled_with_a_terminated_copy() {
        // Without a string table the text is empty, so the buffer must come
        // back terminated at once and be the returned pointer.
        let mut buffer = [0xAAu8; 8];
        let (handle, module, id, length) = (LANGUAGE_HANDLE, 0usize, 1usize, buffer.len());
        let address = buffer.as_mut_ptr() as usize;
        let args: [*const c_void; 5] = [
            (&handle as *const usize).cast(),
            (&module as *const usize).cast(),
            (&id as *const usize).cast(),
            (&address as *const usize).cast(),
            (&length as *const usize).cast(),
        ];
        let (handled, text) = call(object_of(ObjectKind::Language), MSG_LANGUAGE_ANSI, &args);
        assert_eq!(handled, 1);
        assert_eq!(text, address);
        assert_eq!(buffer[0], 0);
    }

    #[test]
    fn the_settings_paths_are_terminated_strings_that_stay_valid() {
        let _turn = paths_turn();
        let file = ipc_answer(IPC_GETINIFILE, 0).unwrap() as *const u8;
        // SAFETY: the answer is a pointer into the process-lifetime storage.
        let text = unsafe { std::ffi::CStr::from_ptr(file.cast()) }.to_string_lossy();
        assert!(text.ends_with("plugin-settings.ini"), "{text}");
        let dir = ipc_answer(IPC_GETINIDIRECTORY, 0).unwrap() as *const u8;
        // SAFETY: as above.
        let dir = unsafe { std::ffi::CStr::from_ptr(dir.cast()) }.to_string_lossy();
        assert!(std::path::Path::new(dir.as_ref()).is_dir(), "{dir}");
        // The same answer every time: the pointer must stay valid.
        assert_eq!(ipc_answer(IPC_GETINIFILE, 0), ipc_answer(IPC_GETINIFILE, 0));
    }

    fn wide_text(id: u32) -> String {
        let wide = ipc_answer(id, 0).unwrap() as *const u16;
        let mut units = Vec::new();
        for i in 0.. {
            // SAFETY: a terminated string in process-lifetime storage.
            let unit = unsafe { wide.add(i).read() };
            if unit == 0 {
                break;
            }
            units.push(unit);
        }
        String::from_utf16_lossy(&units)
    }

    fn ansi_text(id: u32) -> String {
        let ansi = ipc_answer(id, 0).unwrap() as *const u8;
        // SAFETY: a terminated string in process-lifetime storage.
        unsafe { std::ffi::CStr::from_ptr(ansi.cast()) }
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn the_wide_paths_are_the_same_paths() {
        let _turn = paths_turn();
        assert_eq!(wide_text(IPC_GETINIFILEW), ansi_text(IPC_GETINIFILE));
        // 1336 is the plug-ins folder (what 336 gives as ANSI); 1335 is the
        // folder that contains it, the way a plug-in expects its `Plugins` parent.
        assert_eq!(
            wide_text(IPC_GETPLUGINDIRECTORYW),
            ansi_text(IPC_GETINIDIRECTORY)
        );
        let plugins = std::path::PathBuf::from(wide_text(IPC_GETPLUGINDIRECTORYW));
        let base = std::path::PathBuf::from(wide_text(IPC_GETINIDIRECTORYW));
        assert_eq!(plugins.parent(), Some(base.as_path()));
        assert!(
            !wide_text(IPC_GETPLUGINDIRECTORYW).ends_with('\\'),
            "no trailing backslash"
        );
        assert!(wide_text(IPC_GETPLUGINDIRECTORYW).len() < 240);
    }

    #[test]
    fn a_plugin_in_a_plugins_folder_gets_that_folder_and_its_parent() {
        let _turn = paths_turn();
        // The test seam must not be set for this (it overrides the layout).
        if std::env::var_os("RUSTAMP_PLUGIN_SETTINGS").is_some() {
            return;
        }
        let root = std::env::temp_dir().join(format!("rustamp-layout-{}", std::process::id()));
        let plugins = root.join("Plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        // The canonical path of a loaded plug-in carries the extended prefix.
        note_plugin_folder(std::path::Path::new(&format!(r"\\?\{}", plugins.display())));

        assert_eq!(
            wide_text(IPC_GETPLUGINDIRECTORYW),
            plugins.to_string_lossy()
        );
        assert_eq!(wide_text(IPC_GETINIDIRECTORYW), root.to_string_lossy());
        assert_eq!(ansi_text(IPC_GETINIDIRECTORY), plugins.to_string_lossy());
        // The same answers every time: the pointers must stay valid.
        assert_eq!(
            ipc_answer(IPC_GETPLUGINDIRECTORYW, 0),
            ipc_answer(IPC_GETPLUGINDIRECTORYW, 0)
        );
        // Leave the process the way the other tests expect it.
        *PLUGIN_FOLDER.lock().unwrap() = None;
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_playlist_title_is_a_valid_empty_wide_string_and_no_d3dx9_is_loaded() {
        assert_eq!(wide_text(IPC_GETPLAYLISTTITLEW), "");
        assert_eq!(ipc_answer(IPC_GET_D3DX9, 0), Some(0));
    }

    #[test]
    fn the_services_milkdrop_calls_unchecked_answer_every_message_with_no() {
        for id in NULL_SERVICE_IDS {
            let (handled, factory) =
                call(service_manager(), MSG_LOOKUP_SERVICE, &[id.as_ptr().cast()]);
            assert_eq!(handled, 1);
            let one = 1usize;
            let (handled, service) = call(
                object_at(factory),
                MSG_FACTORY_GET,
                &[(&one as *const usize).cast()],
            );
            assert_eq!(handled, 1);
            for message in [0, 10, 30, 365, 9999] {
                assert_eq!(
                    call(object_at(service), message, &[]).0,
                    0,
                    "message {message}"
                );
            }
        }
    }

    #[test]
    fn language_start_hands_back_the_module_the_plugin_passed() {
        let module = 0x0040_0000usize;
        let id = [1u8; 16];
        let (handled, handle) = call(
            object_of(ObjectKind::Language),
            MSG_LANGUAGE_START,
            &[(&module as *const usize).cast(), id.as_ptr().cast()],
        );
        assert_eq!(handled, 1);
        assert_eq!(handle, module);
    }

    #[test]
    fn a_menu_that_does_not_exist_is_not_understood() {
        // Module 0 (this test process) has no menu 110.
        let (handle, module, id) = (LANGUAGE_HANDLE, 0usize, 110usize);
        let args: [*const c_void; 3] = [
            (&handle as *const usize).cast(),
            (&module as *const usize).cast(),
            (&id as *const usize).cast(),
        ];
        assert_eq!(
            call(object_of(ObjectKind::Language), MSG_LANGUAGE_MENU, &args).0,
            0
        );
        // A call with too few arguments is refused too.
        assert_eq!(
            call(
                object_of(ObjectKind::Language),
                MSG_LANGUAGE_MENU,
                &args[..1]
            )
            .0,
            0
        );
    }

    #[test]
    fn the_random_generators_are_in_range() {
        let int = ipc_answer(IPC_GET_RANDFUNC, 0).unwrap();
        let float = ipc_answer(IPC_GET_RANDFUNC, 1).unwrap();
        // SAFETY: the answers are addresses of the functions above.
        let (int, float): (extern "C" fn() -> i32, extern "C" fn() -> f32) = unsafe {
            (
                std::mem::transmute::<usize, extern "C" fn() -> i32>(int),
                std::mem::transmute::<usize, extern "C" fn() -> f32>(float),
            )
        };
        for _ in 0..10_000 {
            let n = int();
            assert!((0..=0x7FFF).contains(&n), "{n}");
            let f = float();
            assert!((0.0..1.0).contains(&f), "{f}");
        }
    }

    #[test]
    fn the_ids_outside_the_start_up_set_are_left_to_the_surface() {
        for id in [0, 104, 105, 3014, 3016, 3026, 335, 65536] {
            assert_eq!(ipc_answer(id, 0), None, "{id}");
        }
        assert_ne!(ipc_answer(IPC_GET_API_SERVICE, 0), None);
    }

    #[test]
    fn an_id_a_real_player_knows_is_never_taken_for_unknown() {
        for id in IPC_KNOWN_WITHOUT_ANSWER {
            assert_eq!(ipc_answer(id, 0), Some(0), "{id}");
        }
    }

    #[test]
    fn a_string_the_host_does_not_have_is_null_and_never_one() {
        // Index past any real playlist: no test sets a track there.
        for id in [
            IPC_GETSKIN,
            IPC_GETPLAYLISTFILE,
            IPC_GETPLAYLISTTITLE,
            IPC_GETPLAYLISTFILEW,
        ] {
            assert_eq!(ipc_answer(id, 999_999), Some(0), "{id}");
        }
    }

    #[test]
    fn the_embedding_request_is_answered_with_the_function() {
        // A plug-in that gets 1 calls address 1.
        assert_eq!(
            ipc_answer(IPC_GET_EMBEDIF, 0),
            Some(crate::embed_frame::embed_function())
        );
    }

    #[test]
    fn the_ipc_answer_for_3025_is_the_service_manager() {
        assert_eq!(
            ipc_answer(IPC_GET_API_SERVICE, 0),
            Some(service_manager() as usize)
        );
    }
}
