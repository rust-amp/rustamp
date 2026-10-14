//! # rustamp-plugin-abi
//!
//! `#[repr(C)]` definitions of the classic Winamp 2.x plugin ABI, written
//! from the clean-room specification `specs/S09-plugin-abi.md` only. Section
//! numbers in doc comments refer to S09.
//!
//! Layouts are exact on 32-bit x86 only (S09 section 0: all 2.95 plugins are
//! 32-bit x86 and structures hold raw 32-bit pointers). Compile-time asserts
//! under `#[cfg(target_arch = "x86")]` pin every size and offset. On 64-bit
//! targets the types only have to compile; their 64-bit layout is unused
//! because the plugin host is 32-bit only. All callbacks are cdecl
//! (`extern "C"`, S09 section 1).
//!
//! Unresolved spec ambiguities are listed in `OPEN_QUESTIONS.md`.

pub mod common;
pub mod dsp;
pub mod general;
pub mod input;
pub mod output;
pub mod vis;

use serde::{Deserialize, Serialize};

/// The kind of a classic plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PluginKind {
    /// `in_*.dll` - input (decoder) plugin.
    Input,
    /// `out_*.dll` - output plugin.
    Output,
    /// `vis_*.dll` - visualization plugin.
    Visualization,
    /// `dsp_*.dll` - DSP / effect plugin.
    Dsp,
    /// `gen_*.dll` - general-purpose plugin.
    General,
}

impl PluginKind {
    /// All kinds.
    pub const ALL: [PluginKind; 5] = [
        PluginKind::Input,
        PluginKind::Output,
        PluginKind::Visualization,
        PluginKind::Dsp,
        PluginKind::General,
    ];

    /// Name of the exported entry point for this kind (S09 section 2),
    /// NUL-terminated for `GetProcAddress`.
    pub fn export_name(self) -> &'static [u8] {
        match self {
            PluginKind::Input => common::EXPORT_IN_GET_MODULE,
            PluginKind::Output => common::EXPORT_OUT_GET_MODULE,
            PluginKind::Visualization => common::EXPORT_VIS_GET_HEADER,
            PluginKind::Dsp => common::EXPORT_DSP_GET_HEADER,
            PluginKind::General => common::EXPORT_GEN_GET_PLUGIN,
        }
    }

    /// Kind whose entry point has the given name (with or without trailing NUL).
    pub fn from_export_name(name: &[u8]) -> Option<PluginKind> {
        let name = name.strip_suffix(&[0]).unwrap_or(name);
        Self::ALL
            .into_iter()
            .find(|k| k.export_name().strip_suffix(&[0]) == Some(name))
    }

    /// Header/module version expected by a 2.x host (S09 section 2).
    pub fn expected_version(self) -> i32 {
        match self {
            PluginKind::Input => common::IN_VERSION,
            PluginKind::Output => common::OUT_VERSION,
            PluginKind::Visualization => common::VIS_VERSION,
            PluginKind::Dsp => common::DSP_VERSION,
            PluginKind::General => common::GEN_VERSION,
        }
    }

    /// Conventional file-name prefix of this kind of plugin (e.g. `"vis_"`).
    pub fn file_prefix(self) -> &'static str {
        match self {
            PluginKind::Input => "in_",
            PluginKind::Output => "out_",
            PluginKind::Visualization => "vis_",
            PluginKind::Dsp => "dsp_",
            PluginKind::General => "gen_",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes() {
        assert_eq!(PluginKind::Visualization.file_prefix(), "vis_");
    }

    #[test]
    fn export_names_roundtrip() {
        for k in PluginKind::ALL {
            assert_eq!(PluginKind::from_export_name(k.export_name()), Some(k));
            assert_eq!(k.export_name().last(), Some(&0));
        }
        assert_eq!(PluginKind::from_export_name(b"nope"), None);
    }

    #[test]
    fn read_string() {
        let s = b"hello\0";
        // SAFETY: NUL-terminated literal.
        assert_eq!(
            unsafe { common::read_c_string_lossy(s.as_ptr().cast()) },
            "hello"
        );
        // SAFETY: null is allowed.
        assert_eq!(
            unsafe { common::read_c_string_lossy(core::ptr::null()) },
            ""
        );
    }

    #[test]
    fn constants_and_defaults() {
        assert_eq!(common::WM_WA_MPEG_EOF, 0x402);
        assert!(input::InModule::default().init.is_none());
        assert_eq!(vis::VisModule::default().spectrum_data[1][575], 0);
    }

    #[cfg(target_arch = "x86")]
    #[test]
    fn x86_sizes() {
        assert_eq!(core::mem::size_of::<input::InModule>(), 0x98);
        assert_eq!(core::mem::size_of::<vis::VisModule>(), 0x938);
    }
}
