// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The plug-in pages' model: which classic plug-in DLLs sit in the plug-in
//! directory, what the approval store says about each, and which of them the
//! user has switched off.
//!
//! Everything here is headless and **executes nothing**. A scan reads each DLL
//! as bytes, runs the passive PE preflight and compares the image against the
//! approval store; no DLL is ever mapped (only the 32-bit plug-in host maps
//! one, and only after the same store approves the exact bytes again).
//!
//! # The approval gate
//!
//! * **There is no auto-approval.** Nothing in this module records an approval:
//!   the store is opened read-only, and a DLL is [`PluginStatus::Approved`] only
//!   when its whole-image SHA-256, size and machine match an entry the operator
//!   put there.
//! * **A plug-in that is not approved cannot be enabled** ([`set_enabled`]
//!   refuses), so the UI cannot lead to a load the host would refuse anyway.
//! * **A broken store approves nothing.** An unreadable or malformed store is
//!   reported in [`PluginScan::store_problem`] and every DLL stays
//!   [`PluginStatus::NotApproved`]; it is never read as an empty allowlist that
//!   quietly "works".
//!
//! # What is and is not known about a plug-in
//!
//! The kind comes from the file-name prefix (`in_`, `out_`, `vis_`, `dsp_`,
//! `gen_`) and is cross-checked against the export the preflight finds. The
//! plug-in's own description and module version are **only** reported by the
//! plug-in once the host has loaded it, so a scan cannot show them.

use std::path::{Path, PathBuf};

use rustamp_core::config::PluginsConfig;
use rustamp_plughost::ApprovalStore;
use rustamp_plughost::pe_preflight::check_pe_i386_dll;
use rustamp_plugin_abi::PluginKind;

/// File name of the approval store, beside the configuration file.
pub const APPROVALS_FILE: &str = "plugins-approved.toml";

/// Largest DLL a scan reads (a plug-in is a few MiB; this only bounds memory).
pub const MAX_PLUGIN_BYTES: u64 = 64 * 1024 * 1024;

/// What the approval gate says about one DLL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginStatus {
    /// The exact image is in the approval store.
    Approved,
    /// A structurally valid plug-in that the store does not approve.
    NotApproved,
    /// The host cannot load this file at all; the reason says why.
    Refused(String),
}

impl PluginStatus {
    /// Short display text.
    pub fn label(&self) -> String {
        match self {
            Self::Approved => "approved".to_string(),
            Self::NotApproved => "not approved".to_string(),
            Self::Refused(reason) => format!("refused: {reason}"),
        }
    }
}

/// One DLL found in the plug-in directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginEntry {
    /// File name, e.g. `in_openmpt.dll`.
    pub file: String,
    /// Full path.
    pub path: PathBuf,
    /// Kind declared by the file-name prefix.
    pub kind: PluginKind,
    /// File size in bytes.
    pub size: u64,
    /// The approval gate's verdict.
    pub status: PluginStatus,
}

impl PluginEntry {
    /// Whether the plug-in is switched on: approved **and** not on the
    /// disabled list. Anything that is not approved is off, whatever the list
    /// says.
    pub fn is_enabled(&self, config: &PluginsConfig) -> bool {
        self.status == PluginStatus::Approved && !is_disabled(config, &self.file)
    }

    /// Whether the user may switch this plug-in on.
    pub fn can_enable(&self) -> bool {
        self.status == PluginStatus::Approved
    }

    /// The kind's display name.
    pub fn kind_name(&self) -> &'static str {
        kind_name(self.kind)
    }

    /// One list row: file, kind, size, approval, and the on/off state.
    pub fn row_text(&self, config: &PluginsConfig) -> String {
        let state = if self.is_enabled(config) { "on" } else { "off" };
        format!(
            "{}   ({}, {}) - {} - {}",
            self.file,
            self.kind_name(),
            size_text(self.size),
            self.status.label(),
            state
        )
    }
}

/// Display name of a plug-in kind.
pub fn kind_name(kind: PluginKind) -> &'static str {
    match kind {
        PluginKind::Input => "input",
        PluginKind::Output => "output",
        PluginKind::Visualization => "visualization",
        PluginKind::Dsp => "DSP/effect",
        PluginKind::General => "general purpose",
    }
}

fn size_text(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

/// The result of one scan of the plug-in directory.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginScan {
    /// The directory that was scanned.
    pub dir: PathBuf,
    /// Plug-in DLLs found, sorted by file name.
    pub entries: Vec<PluginEntry>,
    /// Why the directory could not be listed, if it could not.
    pub dir_problem: Option<String>,
    /// Why the approval store could not be used, if it could not.
    pub store_problem: Option<String>,
}

/// Scans `dir` for plug-in DLLs and classifies each against the approval store
/// at `approvals`.
///
/// A missing store file is an empty allowlist (nothing approved, no problem to
/// report: the operator simply has not approved anything). A store that exists
/// but cannot be read approves nothing and is reported.
pub fn scan(dir: &Path, approvals: &Path) -> PluginScan {
    let mut result = PluginScan {
        dir: dir.to_path_buf(),
        ..PluginScan::default()
    };
    let store = match ApprovalStore::open_or_empty(approvals) {
        Ok(store) => Some(store),
        Err(error) => {
            result.store_problem = Some(format!("{}: {error}", approvals.display()));
            None
        }
    };
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(error) => {
            // A directory that does not exist is the normal "no plug-ins yet".
            if error.kind() != std::io::ErrorKind::NotFound {
                result.dir_problem = Some(format!("{}: {error}", dir.display()));
            }
            return result;
        }
    };
    for item in listing.flatten() {
        let path = item.path();
        let is_dll = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("dll"));
        if !is_dll || !item.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let Some(kind) = PluginKind::from_path(&path) else {
            continue;
        };
        let Some(file) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
        else {
            continue;
        };
        let (size, status) = classify(&path, kind, store.as_ref());
        result.entries.push(PluginEntry {
            file,
            path,
            kind,
            size,
            status,
        });
    }
    result
        .entries
        .sort_by_cached_key(|entry| entry.file.to_ascii_lowercase());
    result
}

fn classify(
    path: &Path,
    declared: PluginKind,
    store: Option<&ApprovalStore>,
) -> (u64, PluginStatus) {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if size > MAX_PLUGIN_BYTES {
        return (
            size,
            PluginStatus::Refused(format!("larger than {} MiB", MAX_PLUGIN_BYTES >> 20)),
        );
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => return (size, PluginStatus::Refused(format!("unreadable: {error}"))),
    };
    let size = bytes.len() as u64;
    match check_pe_i386_dll(&bytes) {
        Err(error) => return (size, PluginStatus::Refused(error.to_string())),
        Ok(found) if found != declared => {
            return (
                size,
                PluginStatus::Refused(format!(
                    "the file name says {}, its export says {}",
                    kind_name(declared),
                    kind_name(found)
                )),
            );
        }
        Ok(_) => {}
    }
    let approved = store.is_some_and(|store| store.is_approved(&bytes));
    (
        size,
        if approved {
            PluginStatus::Approved
        } else {
            PluginStatus::NotApproved
        },
    )
}

/// Why a plug-in could not be switched on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnableRefused {
    /// The plug-in's file name.
    pub file: String,
    /// The gate's verdict that blocked it.
    pub status: PluginStatus,
}

impl std::fmt::Display for EnableRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} cannot be enabled: {} (only plug-ins listed in {APPROVALS_FILE} may run)",
            self.file,
            self.status.label()
        )
    }
}

impl std::error::Error for EnableRefused {}

fn is_disabled(config: &PluginsConfig, file: &str) -> bool {
    config.disabled.iter().any(|f| f.eq_ignore_ascii_case(file))
}

/// Switches `entry` on or off in `config`.
///
/// Turning **off** always works. Turning **on** is refused unless the approval
/// gate approved the image: the list cannot be used to talk the host into a
/// load it would refuse.
pub fn set_enabled(
    config: &mut PluginsConfig,
    entry: &PluginEntry,
    on: bool,
) -> Result<(), EnableRefused> {
    if on {
        if !entry.can_enable() {
            return Err(EnableRefused {
                file: entry.file.clone(),
                status: entry.status.clone(),
            });
        }
        config
            .disabled
            .retain(|f| !f.eq_ignore_ascii_case(&entry.file));
    } else if !is_disabled(config, &entry.file) {
        config.disabled.push(entry.file.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A synthetic PE32 DLL exporting `export` (never given to an OS loader).
    fn fake_dll(export: &[u8], tweak: u8) -> Vec<u8> {
        let mut bytes = vec![0; 0x600];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        for (offset, value) in [
            (0x84, 0x014c_u16),
            (0x86, 1),
            (0x94, 224),
            (0x96, 0x2102),
            (0x98, 0x10b),
        ] {
            bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        }
        for (offset, value) in [
            (0xa8, 0x1100_u32),
            (0xd0, 0x2000),
            (0xd4, 0x200),
            (0xf4, 16),
            (0xf8, 0x1000),
            (0xfc, 0x100),
            (0x180, 0x400),
            (0x184, 0x1000),
            (0x188, 0x400),
            (0x18c, 0x200),
            (0x19c, 0x6000_0020),
            (0x214, 1),
            (0x218, 1),
            (0x21c, 0x1040),
            (0x220, 0x1044),
            (0x224, 0x1048),
            (0x240, 0x1100),
            (0x244, 0x1060),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[0x260..0x260 + export.len()].copy_from_slice(export);
        bytes[0x300] = tweak;
        bytes
    }

    const INPUT_EXPORT: &[u8] = b"winampGetInModule2\0";
    const GENERAL_EXPORT: &[u8] = b"winampGetGeneralPurposePlugin\0";

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "rustamp-prefs-plugins-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn put(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }
        fn approvals(&self) -> PathBuf {
            self.0.join(APPROVALS_FILE)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn by_name<'a>(scan: &'a PluginScan, file: &str) -> &'a PluginEntry {
        scan.entries
            .iter()
            .find(|e| e.file == file)
            .unwrap_or_else(|| panic!("{file} not found in {:?}", scan.entries))
    }

    #[test]
    fn an_unapproved_dll_is_listed_but_cannot_be_enabled() {
        let scratch = Scratch::new();
        scratch.put("in_one.dll", &fake_dll(INPUT_EXPORT, 1));
        let scan = scan(&scratch.0, &scratch.approvals());
        assert_eq!(scan.store_problem, None, "a missing store is not a problem");
        let entry = by_name(&scan, "in_one.dll");
        assert_eq!(entry.kind, PluginKind::Input);
        assert_eq!(entry.status, PluginStatus::NotApproved);

        let mut config = PluginsConfig::default();
        let refused = set_enabled(&mut config, entry, true).unwrap_err();
        assert_eq!(refused.status, PluginStatus::NotApproved);
        assert!(!entry.is_enabled(&config));
        assert!(config.disabled.is_empty(), "a refusal changes nothing");
    }

    #[test]
    fn only_the_exact_approved_image_is_approved() {
        let scratch = Scratch::new();
        let approved = fake_dll(INPUT_EXPORT, 1);
        scratch.put("in_one.dll", &approved);
        // Same size, same headers, one byte different: not the approved image.
        scratch.put("in_two.dll", &fake_dll(INPUT_EXPORT, 2));
        let mut store = ApprovalStore::new();
        store.approve_bytes("in_one.dll", &approved).unwrap();
        store.save_to_file(&scratch.approvals()).unwrap();

        let scan = scan(&scratch.0, &scratch.approvals());
        assert_eq!(by_name(&scan, "in_one.dll").status, PluginStatus::Approved);
        assert_eq!(
            by_name(&scan, "in_two.dll").status,
            PluginStatus::NotApproved
        );
    }

    #[test]
    fn an_approved_plugin_toggles_through_the_disabled_list() {
        let scratch = Scratch::new();
        let image = fake_dll(GENERAL_EXPORT, 1);
        scratch.put("gen_one.dll", &image);
        let mut store = ApprovalStore::new();
        store.approve_bytes("gen_one.dll", &image).unwrap();
        store.save_to_file(&scratch.approvals()).unwrap();
        let scan = scan(&scratch.0, &scratch.approvals());
        let entry = by_name(&scan, "gen_one.dll");
        assert_eq!(entry.kind, PluginKind::General);

        let mut config = PluginsConfig::default();
        assert!(entry.is_enabled(&config), "approved plug-ins start on");
        set_enabled(&mut config, entry, false).unwrap();
        assert_eq!(config.disabled, vec!["gen_one.dll".to_string()]);
        assert!(!entry.is_enabled(&config));
        // Switching off twice does not duplicate the entry.
        set_enabled(&mut config, entry, false).unwrap();
        assert_eq!(config.disabled.len(), 1);
        set_enabled(&mut config, entry, true).unwrap();
        assert!(config.disabled.is_empty());
        assert!(entry.is_enabled(&config));
    }

    #[test]
    fn a_malformed_store_approves_nothing_and_says_why() {
        let scratch = Scratch::new();
        let image = fake_dll(INPUT_EXPORT, 1);
        scratch.put("in_one.dll", &image);
        std::fs::write(scratch.approvals(), "this is not a store").unwrap();
        let scan = scan(&scratch.0, &scratch.approvals());
        let problem = scan
            .store_problem
            .as_deref()
            .expect("the problem is reported");
        assert!(problem.contains(APPROVALS_FILE), "{problem}");
        assert_eq!(
            by_name(&scan, "in_one.dll").status,
            PluginStatus::NotApproved
        );
    }

    #[test]
    fn files_the_host_cannot_load_are_refused_with_a_reason() {
        let scratch = Scratch::new();
        scratch.put("in_junk.dll", b"not a pe file");
        // The file name says output, the export says input.
        scratch.put("out_wrong.dll", &fake_dll(INPUT_EXPORT, 1));
        scratch.put("readme.txt", b"ignored");
        scratch.put("random.dll", &fake_dll(INPUT_EXPORT, 1));
        let scan = scan(&scratch.0, &scratch.approvals());
        assert_eq!(
            scan.entries
                .iter()
                .map(|e| e.file.as_str())
                .collect::<Vec<_>>(),
            ["in_junk.dll", "out_wrong.dll"],
            "only prefixed DLLs are plug-ins"
        );
        for file in ["in_junk.dll", "out_wrong.dll"] {
            let entry = by_name(&scan, file);
            assert!(matches!(entry.status, PluginStatus::Refused(_)), "{file}");
            assert!(!entry.can_enable());
        }
        let PluginStatus::Refused(reason) = &by_name(&scan, "out_wrong.dll").status else {
            unreachable!()
        };
        assert!(reason.contains("file name says output"), "{reason}");
    }

    #[test]
    fn a_missing_directory_is_an_empty_scan() {
        let scratch = Scratch::new();
        let scan = scan(&scratch.0.join("nope"), &scratch.approvals());
        assert!(scan.entries.is_empty());
        assert_eq!(scan.dir_problem, None);
    }

    #[test]
    fn a_not_approved_entry_is_always_off_even_when_not_listed_as_disabled() {
        let entry = PluginEntry {
            file: "dsp_x.dll".into(),
            path: PathBuf::from("dsp_x.dll"),
            kind: PluginKind::Dsp,
            size: 2048,
            status: PluginStatus::NotApproved,
        };
        let config = PluginsConfig::default();
        assert!(!entry.is_enabled(&config));
        let text = entry.row_text(&config);
        assert!(
            text.contains("dsp_x.dll") && text.contains("DSP/effect"),
            "{text}"
        );
        assert!(
            text.contains("not approved") && text.ends_with("off"),
            "{text}"
        );
    }

    #[test]
    fn disabled_names_match_regardless_of_case() {
        let entry = PluginEntry {
            file: "In_Two.DLL".into(),
            path: PathBuf::from("In_Two.DLL"),
            kind: PluginKind::Input,
            size: 1,
            status: PluginStatus::Approved,
        };
        let config = PluginsConfig {
            disabled: vec!["in_two.dll".into()],
            ..PluginsConfig::default()
        };
        assert!(!entry.is_enabled(&config));
    }
}
