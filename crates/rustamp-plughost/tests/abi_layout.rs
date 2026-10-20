//! The host's side of the classic plugin ABI layout (`specs/S09-plugin-abi.md`).
//!
//! `rustamp-plugin-abi` owns the structs and checks them against the spec's
//! member tables. This file is the **host's** view of the same contract: the
//! members the host writes into - the ones S09's tables mark "host" - must sit
//! at the byte offsets S09 gives, because that is where a 32-bit plugin reads
//! them (S09 §3.1, §4.1, §5.2, §6.2, §7.1).
//!
//! The check runs on the **memory image** of the module, not on the
//! compiler's metadata: the host writes a value into a member and reads the
//! bytes of the module struct back at the offset the plugin would use. On
//! 32-bit x86 - the target the ABI is defined for (S09 §0) - that offset is
//! the spec's offset; on a 64-bit target the same member simply sits further
//! out, because pointers are wider there.
//!
//! Nothing here loads a DLL, opens a window or calls a plugin: the modules are
//! plain Rust values, and no callback is invoked.

use std::ffi::c_void;
use std::mem::size_of;

use rustamp_plugin_abi::PluginKind;
use rustamp_plugin_abi::common;
use rustamp_plugin_abi::dsp::{DspHeader, DspModule};
use rustamp_plugin_abi::general::GeneralPlugin;
use rustamp_plugin_abi::input::InModule;
use rustamp_plugin_abi::output::OutModule;
use rustamp_plugin_abi::vis::{VisHeader, VisModule};

/// The marker the host writes into a handle member, so that the test can find
/// it in the module's bytes.
const MARKER: usize = 0x5566_7788;

/// The bytes of a module struct, as a plugin sees them.
fn image_bytes<T>(module: &T) -> &[u8] {
    // SAFETY: the caller owns a live, initialized module; the returned slice
    // borrows it and is read only.
    unsafe { std::slice::from_raw_parts(module as *const T as *const u8, size_of::<T>()) }
}

/// Checks one member the host writes: it must sit at the spec's offset, and
/// what the host put there must be what a plugin reads at that offset.
macro_rules! host_member {
    // A handle: a raw pointer the host writes.
    ($ty:ty, $field:ident, $spec_offset:expr, handle as $cast:ty) => {{
        let mut module = <$ty>::default();
        module.$field = MARKER as $cast;
        let at = core::mem::offset_of!($ty, $field);
        #[cfg(target_arch = "x86")]
        assert_eq!(
            at,
            $spec_offset,
            "{}::{} must sit at 0x{:02X} on 32-bit x86, but sits at 0x{:02X}",
            stringify!($ty),
            stringify!($field),
            $spec_offset,
            at
        );
        let image = image_bytes(&module);
        let got = usize::from_le_bytes(image[at..at + size_of::<usize>()].try_into().unwrap());
        assert_eq!(
            got,
            MARKER,
            "{}::{} is not what a plugin reads at byte offset {}",
            stringify!($ty),
            stringify!($field),
            at
        );
    }};
    // A callback slot: a null slot must read as a null pointer, because S09
    // §0 allows every callback member to be null.
    ($ty:ty, $field:ident, $spec_offset:expr, callback) => {{
        let mut module = <$ty>::default();
        module.$field = None;
        let at = core::mem::offset_of!($ty, $field);
        #[cfg(target_arch = "x86")]
        assert_eq!(
            at,
            $spec_offset,
            "{}::{} must sit at 0x{:02X} on 32-bit x86, but sits at 0x{:02X}",
            stringify!($ty),
            stringify!($field),
            $spec_offset,
            at
        );
        let image = image_bytes(&module);
        assert!(
            image[at..at + 4].iter().all(|byte| *byte == 0),
            "{}::{} is not a null pointer at byte offset {}",
            stringify!($ty),
            stringify!($field),
            at
        );
    }};
    // A 32-bit integer member.
    ($ty:ty, $field:ident, $spec_offset:expr, int, $value:expr) => {{
        let mut module = <$ty>::default();
        module.$field = $value;
        let at = core::mem::offset_of!($ty, $field);
        #[cfg(target_arch = "x86")]
        assert_eq!(
            at,
            $spec_offset,
            "{}::{} must sit at 0x{:02X} on 32-bit x86, but sits at 0x{:02X}",
            stringify!($ty),
            stringify!($field),
            $spec_offset,
            at
        );
        let image = image_bytes(&module);
        let got = i32::from_le_bytes(image[at..at + 4].try_into().unwrap());
        assert_eq!(
            got,
            $value,
            "{}::{} is not what a plugin reads at byte offset {}",
            stringify!($ty),
            stringify!($field),
            at
        );
    }};
    // A byte array the host fills row by row.
    ($ty:ty, $field:ident, $spec_offset:expr, bytes) => {{
        let mut module = <$ty>::default();
        module.$field[0][0] = 0xA5;
        let at = core::mem::offset_of!($ty, $field);
        #[cfg(target_arch = "x86")]
        assert_eq!(
            at,
            $spec_offset,
            "{}::{} must sit at 0x{:02X} on 32-bit x86, but sits at 0x{:02X}",
            stringify!($ty),
            stringify!($field),
            $spec_offset,
            at
        );
        let image = image_bytes(&module);
        assert_eq!(
            image[at],
            0xA5,
            "{}::{} is not what a plugin reads at byte offset {}",
            stringify!($ty),
            stringify!($field),
            at
        );
    }};
}

/// The input module's host-filled members (S09 §3.1): the two handles and the
/// analyzer, visualization and DSP callback groups, `set_info` and the output
/// module the hand-over fills.
#[test]
fn the_input_modules_host_filled_members_sit_where_a_plugin_reads_them() {
    host_member!(InModule, main_window, 0x08, handle as *mut c_void);
    host_member!(InModule, dll_instance, 0x0C, handle as *mut c_void);
    host_member!(InModule, analyzer_init, 0x60, callback);
    host_member!(InModule, analyzer_deinit, 0x64, callback);
    host_member!(InModule, analyzer_feed_pcm, 0x68, callback);
    host_member!(InModule, analyzer_get_mode, 0x6C, callback);
    host_member!(InModule, analyzer_feed_rows, 0x70, callback);
    host_member!(InModule, vis_feed_pcm, 0x74, callback);
    host_member!(InModule, vis_get_mode, 0x78, callback);
    host_member!(InModule, vis_feed_frame, 0x7C, callback);
    host_member!(InModule, vis_set_format, 0x80, callback);
    host_member!(InModule, dsp_is_active, 0x84, callback);
    host_member!(InModule, dsp_process, 0x88, callback);
    host_member!(InModule, set_info, 0x90, callback);
    host_member!(
        InModule,
        out_module,
        0x94,
        handle as *mut rustamp_plugin_abi::output::OutModule
    );
}

/// The output module's two host-filled members (S09 §4.1).
#[test]
fn the_output_modules_host_filled_members_sit_where_a_plugin_reads_them() {
    host_member!(OutModule, main_window, 0x0C, handle as *mut c_void);
    host_member!(OutModule, dll_instance, 0x10, handle as *mut c_void);
}

/// The visualization module's host-filled members (S09 §5.2): the two handles,
/// the stream format the host announces and the two row arrays the host
/// fills.
#[test]
fn the_visualization_modules_host_filled_members_sit_where_a_plugin_reads_them() {
    host_member!(VisModule, parent_window, 0x004, handle as *mut c_void);
    host_member!(VisModule, dll_instance, 0x008, handle as *mut c_void);
    host_member!(VisModule, sample_rate, 0x00C, int, 44_100);
    host_member!(VisModule, channels, 0x010, int, 2);
    host_member!(VisModule, spectrum_data, 0x024, bytes);
    host_member!(VisModule, waveform_data, 0x4A4, bytes);
}

/// The DSP module's two host-filled members (S09 §6.2).
#[test]
fn the_dsp_modules_host_filled_members_sit_where_a_plugin_reads_them() {
    host_member!(DspModule, parent_window, 0x04, handle as *mut c_void);
    host_member!(DspModule, dll_instance, 0x08, handle as *mut c_void);
}

/// The general purpose plugin's two host-filled members (S09 §7.1).
#[test]
fn the_general_plugins_host_filled_members_sit_where_a_plugin_reads_them() {
    host_member!(GeneralPlugin, parent_window, 0x14, handle as *mut c_void);
    host_member!(GeneralPlugin, dll_instance, 0x18, handle as *mut c_void);
}

/// The version is the first member of every header and module, so the host
/// reads it at byte offset 0 (S09 §1: "The first member of every
/// header/module is an integer version"), and the value the host expects is
/// the one S09 gives.
#[test]
fn the_version_is_the_first_member_of_every_kind() {
    assert_eq!(core::mem::offset_of!(InModule, version), 0);
    assert_eq!(core::mem::offset_of!(OutModule, version), 0);
    assert_eq!(core::mem::offset_of!(VisHeader, version), 0);
    assert_eq!(core::mem::offset_of!(DspHeader, version), 0);
    assert_eq!(core::mem::offset_of!(GeneralPlugin, version), 0);

    // S09 §2, §3.2, §4.1, §5.1, §6.1, §7.1.
    assert_eq!(PluginKind::Input.expected_version(), 0x100);
    assert_eq!(PluginKind::Output.expected_version(), 0x10);
    assert_eq!(PluginKind::Visualization.expected_version(), 0x101);
    assert_eq!(PluginKind::Dsp.expected_version(), 0x20);
    assert_eq!(PluginKind::General.expected_version(), 0x10);
}

/// The host accepts an input module whose version carries the 5.x flag groups
/// (S09 §3.2): the two groups are cleared and the remainder must be 0x100.
#[test]
fn the_input_version_flags_are_masked_off_before_the_version_is_checked() {
    let mask = (common::IN_VERSION_FLAG_WIDE_FILENAMES as i32)
        | (common::IN_VERSION_FLAG_INIT_STATUS as i32);
    let base = |version: i32| version & !mask;
    assert_eq!(base(common::IN_VERSION), common::IN_VERSION);
    assert_eq!(
        base(common::IN_VERSION | common::IN_VERSION_FLAG_WIDE_FILENAMES as i32),
        common::IN_VERSION
    );
    assert_eq!(
        base(common::IN_VERSION | common::IN_VERSION_FLAG_INIT_STATUS as i32),
        common::IN_VERSION
    );
    // A version the host must refuse stays refused after the masking.
    assert_ne!(base(0x101), common::IN_VERSION);
}
