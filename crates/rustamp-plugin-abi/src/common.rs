//! Constants and helpers shared by all plugin kinds (S09 sections 1, 2, 3.6,
//! 5.3, 8 and 10).

use core::ffi::{c_char, c_void};

/// Window handle (`HWND`). A 32-bit pointer-sized value on x86 (S09 section 0).
pub type Hwnd = *mut c_void;
/// Module handle (`HINSTANCE`) of a plugin DLL (S09 section 1, "DLL instance").
pub type Hinstance = *mut c_void;

/// Row length in bytes of one spectrum or waveform row (S09 sections 5.2, 9).
pub const VIS_ROW_LEN: usize = 576;
/// Maximum number of spectrum or waveform rows a visualization can request
/// (S09 section 5.2).
pub const VIS_MAX_ROWS: usize = 2;

/// Version of an input module in 2.95 (`version` member, S09 section 3.2).
pub const IN_VERSION: i32 = 0x100;
/// 5.64+ input version allowing a wide-character description (S09 section 3.2). 5.x-only.
pub const IN_VERSION_WIDE_DESC: i32 = 0x101;
/// Flag group OR-ed into the input version: wide-character file names
/// (S09 section 3.2). 5.x-only.
pub const IN_VERSION_FLAG_WIDE_FILENAMES: u32 = 0x0F00_0000;
/// Flag group OR-ed into the input version: `init` returns a status and the
/// 39th member exists (S09 section 3.2). 5.x-only.
pub const IN_VERSION_FLAG_INIT_STATUS: u32 = 0xF000_0000;

/// Input `flags` bit value 1: audio goes through a Winamp output plugin (S09 section 3.3).
pub const IN_FLAG_USES_OUTPUT: i32 = 1;
/// Input `flags` bit value 2: plugin does its own equalization (S09 section 3.3). 5.x-only.
pub const IN_FLAG_OWN_EQ: i32 = 2;
/// Input `flags` bit value 8: replay gain already applied (S09 section 3.3). 5.x-only.
pub const IN_FLAG_REPLAYGAIN_APPLIED: i32 = 8;
/// Input `flags` bit value 16: replay-gain preamp already applied (S09 section 3.3). 5.x-only.
pub const IN_FLAG_REPLAYGAIN_PREAMP_APPLIED: i32 = 16;

/// Output module version in 2.95 (S09 section 4.1).
pub const OUT_VERSION: i32 = 0x10;
/// Output module version with wide description (S09 section 4.1). 5.x-only.
pub const OUT_VERSION_WIDE_DESC: i32 = 0x11;

/// Visualization header version in 2.x (S09 section 5.1).
pub const VIS_VERSION: i32 = 0x101;
/// Visualization header version used with the main-window-taking entry point
/// (S09 section 5.1). 5.x-only.
pub const VIS_VERSION_5X: i32 = 0x102;

/// DSP header version in 2.x (S09 section 6.1).
pub const DSP_VERSION: i32 = 0x20;
/// DSP header version 0x21 (S09 section 6.1). 5.x-only.
pub const DSP_VERSION_21: i32 = 0x21;
/// DSP header version 0x22 (S09 section 6.1). 5.x-only.
pub const DSP_VERSION_22: i32 = 0x22;

/// General purpose plugin version in 2.95 (S09 section 7.1).
pub const GEN_VERSION: i32 = 0x10;
/// General purpose plugin version with wide description (S09 section 7.1). 5.x-only.
pub const GEN_VERSION_WIDE_DESC: i32 = 0x11;

/// `WM_USER` base message number (S09 section 3.6).
pub const WM_USER: u32 = 0x0400;
/// IPC channel message number, `WM_WA_IPC` (S09 sections 3.6, 8).
pub const WM_WA_IPC: u32 = WM_USER;
/// End-of-track notification `WM_WA_MPEG_EOF` (`WM_USER + 2`), posted by an
/// input plugin to the main window with both parameters zero (S09 section 3.6).
pub const WM_WA_MPEG_EOF: u32 = WM_USER + 2;

/// IPC number 0, version query (S09 section 8, item 3).
pub const IPC_GETVERSION: u32 = 0;
/// Value returned by the version query in Winamp 2.95 (S09 section 8, item 3).
/// Measured 0x2905 in two independent S10 sandbox runs; previously 0x2095 (SDK-derived, never measured).
/// The old value sits below the 0x2900 plugin threshold, causing plugins to take the wrong branch.
/// RFC 0009 corrects this.
pub const WINAMP_VERSION_295: u32 = 0x2905;
/// Threshold plugins compare the version against to detect 2.9+ IPCs
/// (S09 section 8, item 3).
pub const WINAMP_VERSION_2_9X_THRESHOLD: u32 = 0x2900;
/// IPC 611, register a vis command window (S09 section 5.3). 5.x-only.
pub const IPC_SET_VIS_COMMAND_WINDOW: u32 = 611;
/// First vis command id (S09 section 5.3). 5.x-only.
pub const VIS_COMMAND_ID_FIRST: u32 = 40382;
/// Last vis command id (S09 section 5.3). 5.x-only.
pub const VIS_COMMAND_ID_LAST: u32 = 40391;

/// Window class name of the main window (S09 sections 1, 8).
pub const CLASS_MAIN: &[u8] = b"Winamp v1.x\0";
/// Window class name of the equalizer window (S09 section 8).
pub const CLASS_EQ: &[u8] = b"Winamp EQ\0";
/// Window class name of the playlist editor (S09 section 8).
pub const CLASS_PLAYLIST: &[u8] = b"Winamp PE\0";
/// Window class name of the minibrowser (S09 section 8).
pub const CLASS_MINIBROWSER: &[u8] = b"Winamp MB\0";
/// Window class name of the video window (S09 section 8).
pub const CLASS_VIDEO: &[u8] = b"Winamp Video\0";
/// Window class name of frames for embedded windows (S09 section 8).
pub const CLASS_GEN: &[u8] = b"Winamp Gen\0";

/// Name of the plugin sub-folder of the Winamp folder (S09 section 1).
pub const PLUGINS_DIR_NAME: &str = "Plugins";

/// Input entry point `winampGetInModule2` (S09 section 2). NUL-terminated for `GetProcAddress`.
pub const EXPORT_IN_GET_MODULE: &[u8] = b"winampGetInModule2\0";
/// Output entry point `winampGetOutModule` (S09 section 2).
pub const EXPORT_OUT_GET_MODULE: &[u8] = b"winampGetOutModule\0";
/// Visualization entry point `winampVisGetHeader` (S09 section 2).
pub const EXPORT_VIS_GET_HEADER: &[u8] = b"winampVisGetHeader\0";
/// DSP entry point `winampDSPGetHeader2` (S09 section 2).
pub const EXPORT_DSP_GET_HEADER: &[u8] = b"winampDSPGetHeader2\0";
/// General purpose entry point `winampGetGeneralPurposePlugin` (S09 section 2).
pub const EXPORT_GEN_GET_PLUGIN: &[u8] = b"winampGetGeneralPurposePlugin\0";

/// Optional input export `winampGetExtendedFileInfo` (S09 section 3.9). Signature unverified (U-13).
pub const EXPORT_IN_GET_EXTENDED_FILE_INFO: &[u8] = b"winampGetExtendedFileInfo\0";
/// Optional input export `winampSetExtendedFileInfo` (S09 section 3.9).
pub const EXPORT_IN_SET_EXTENDED_FILE_INFO: &[u8] = b"winampSetExtendedFileInfo\0";
/// Optional input export `winampWriteExtendedFileInfo` (S09 section 3.9).
pub const EXPORT_IN_WRITE_EXTENDED_FILE_INFO: &[u8] = b"winampWriteExtendedFileInfo\0";
/// Optional input export `winampGetExtendedRead_open` (S09 section 3.9).
pub const EXPORT_IN_EXTENDED_READ_OPEN: &[u8] = b"winampGetExtendedRead_open\0";
/// Optional input export `winampGetExtendedRead_getData` (S09 section 3.9).
pub const EXPORT_IN_EXTENDED_READ_GETDATA: &[u8] = b"winampGetExtendedRead_getData\0";
/// Optional input export `winampGetExtendedRead_close` (S09 section 3.9).
pub const EXPORT_IN_EXTENDED_READ_CLOSE: &[u8] = b"winampGetExtendedRead_close\0";
/// Optional export `winampUninstallPlugin`, all kinds (S09 section 1). 5.x-only.
pub const EXPORT_UNINSTALL_PLUGIN: &[u8] = b"winampUninstallPlugin\0";
/// Optional output export `winampGetOutModeChange` (S09 section 4.2). 5.x-only.
pub const EXPORT_OUT_MODE_CHANGE: &[u8] = b"winampGetOutModeChange\0";

/// Reads a NUL-terminated ANSI description string into a `String`, lossily
/// (non-UTF-8 bytes become U+FFFD). A null pointer yields an empty string.
/// S09 section 1 (strings are ANSI code page).
///
/// # Safety
///
/// `ptr` must be null, or point to a readable NUL-terminated byte string that
/// stays valid and unmodified for the duration of the call.
pub unsafe fn read_c_string_lossy(ptr: *const c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    // SAFETY: caller guarantees `ptr` is a valid NUL-terminated string.
    let cstr = unsafe { core::ffi::CStr::from_ptr(ptr) };
    cstr.to_string_lossy().into_owned()
}

/// Asserts struct size and field offsets on 32-bit x86 only (S09 section 0:
/// layouts are exact on x86; the plugin host is 32-bit only, so the 64-bit
/// layout is unused and only has to compile).
macro_rules! layout_asserts {
    ($t:ty, size = $size:expr, { $($field:ident = $off:expr),* $(,)? }) => {
        #[cfg(target_arch = "x86")]
        const _: () = {
            assert!(core::mem::size_of::<$t>() == $size);
            $(assert!(core::mem::offset_of!($t, $field) == $off);)*
        };
    };
}
pub(crate) use layout_asserts;
