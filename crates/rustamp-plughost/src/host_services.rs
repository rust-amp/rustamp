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
/// IPC id 505: asks for a function that embeds a window in the host (the
/// answer is called as a function when it is not 0). **Not in S21**: found when
/// MilkDrop ran past its start-up checks (it called the answer 1 of the 2.95
/// surface as a function and faulted at address 1). The host has no embedding
/// service, so the answer is **0**, "none": own choice, a reading of what a
/// plug-in that tests the answer does. The 2.95 answer of S10 (1) stays for the
/// player's own window.
pub const IPC_GET_EMBEDIF: u32 = 505;

/// What the generic call is asked about (S21 §3 and §5.1).
const MSG_REGISTER_FACTORY: i32 = 10;
const MSG_DEREGISTER_FACTORY: i32 = 20;
const MSG_LOOKUP_SERVICE: i32 = 50;
const MSG_FACTORY_GET: i32 = 300;
const MSG_FACTORY_RELEASE: i32 = 310;
const MSG_LANGUAGE_START: i32 = 30;
const MSG_LANGUAGE_ANSI: i32 = 10;
const MSG_LANGUAGE_WIDE: i32 = 11;

/// The identifier of the language service as it sits in memory: the usual
/// 16-byte layout of a GUID, the first three groups little-endian
/// (30AED4E5-EF10-4277-8D49-27AB5570E891).
pub const LANGUAGE_SERVICE_ID: [u8; 16] = [
    0xE5, 0xD4, 0xAE, 0x30, 0x10, 0xEF, 0x77, 0x42, 0x8D, 0x49, 0x27, 0xAB, 0x55, 0x70, 0xE8, 0x91,
];

/// What a language "handle" is: any non-zero number works (S21 §5.1).
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
            // SAFETY: a writable result cell.
            unsafe { *result = LANGUAGE_HANDLE };
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
    directory_wide: Vec<u16>,
}

fn paths() -> &'static Paths {
    static PATHS: OnceLock<Paths> = OnceLock::new();
    PATHS.get_or_init(|| {
        let base = std::env::var_os("RUSTAMP_PLUGIN_SETTINGS")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("APPDATA").map(|dir| std::path::Path::new(&dir).join("Rustamp"))
            })
            .unwrap_or_else(|| std::env::temp_dir().join("Rustamp"));
        let directory = base.join("plugins");
        // Created so a plug-in that makes a sub-directory below it can (S21 §4).
        let _ = std::fs::create_dir_all(&directory);
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
            directory_ansi: ansi(lossy(&directory)),
            file_wide: wide(lossy(&file)),
            directory_wide: wide(lossy(&directory)),
        }
    })
}

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
    Some(match id {
        IPC_GET_API_SERVICE => service_manager() as usize,
        IPC_GET_EMBEDIF => 0,
        IPC_GETINIFILE => paths().file_ansi.as_ptr() as usize,
        IPC_GETINIDIRECTORY => paths().directory_ansi.as_ptr() as usize,
        IPC_GETINIFILEW => paths().file_wide.as_ptr() as usize,
        IPC_GETINIDIRECTORYW => paths().directory_wide.as_ptr() as usize,
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

    #[test]
    fn the_wide_paths_are_the_same_paths() {
        let wide = ipc_answer(IPC_GETINIFILEW, 0).unwrap() as *const u16;
        let mut units = Vec::new();
        for i in 0.. {
            // SAFETY: a terminated string in process-lifetime storage.
            let unit = unsafe { wide.add(i).read() };
            if unit == 0 {
                break;
            }
            units.push(unit);
        }
        let wide = String::from_utf16_lossy(&units);
        let ansi = ipc_answer(IPC_GETINIFILE, 0).unwrap() as *const u8;
        // SAFETY: as above.
        let ansi = unsafe { std::ffi::CStr::from_ptr(ansi.cast()) }.to_string_lossy();
        assert_eq!(wide, ansi);
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
    fn the_embedding_request_is_answered_with_none_not_with_one() {
        // A plug-in that gets 1 calls address 1; "no embedding" is 0.
        assert_eq!(ipc_answer(IPC_GET_EMBEDIF, 0), Some(0));
    }

    #[test]
    fn the_ipc_answer_for_3025_is_the_service_manager() {
        assert_eq!(
            ipc_answer(IPC_GET_API_SERVICE, 0),
            Some(service_manager() as usize)
        );
    }
}
