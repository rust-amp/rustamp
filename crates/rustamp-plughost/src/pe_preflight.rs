//! Passive PE machine-header preflight, not a DLL loader or an approval gate.
//!
//! S09 section 0 requires a 32-bit x86 plugin/host boundary. The PE format
//! facts used here are also listed in task M3hC finding 11: DOS signature,
//! PE offset at 0x3c, PE signature, and COFF machine 0x014c. This independent
//! implementation copies no GPL application code and adds no dependency.
//!
//! Only supplied bytes are inspected: no filesystem access, OS loader,
//! symbol resolution, callbacks, windows or processes. A matching header
//! does NOT prove a valid DLL, exports, PE32 optional header, loadability,
//! instruction-set requirements, provenance or permission to execute it.
//! A future loader must revalidate the exact approved bytes it loads; this
//! result must never authorize a later load of the unprotected source pathname
//! (TOCTOU). The loader instead seals and loads a byte-identical private copy.
//!
//! `check_pe_i386_dll` additionally checks passive DLL/PE32 structure and a
//! supported, non-forwarded executable export. Neither check detects malicious
//! code or establishes the version returned by a plugin getter.

use std::fmt;

use rustamp_plugin_abi::PluginKind;

const DOS_HEADER_LEN: usize = 64;
const PE_SIGNATURE_AND_COFF_LEN: usize = 24;
const IMAGE_FILE_MACHINE_I386: u16 = 0x014c;

/// Why passive machine-header preflight refused the supplied image bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PePreflightError {
    /// The fixed DOS header is incomplete.
    TruncatedDosHeader,
    /// The DOS header does not start with `MZ`.
    InvalidDosSignature,
    /// The PE offset overlaps the fixed DOS header or cannot be represented.
    InvalidPeOffset,
    /// The PE signature or full COFF header lies outside the supplied bytes.
    TruncatedPeHeader,
    /// The declared PE header does not start with `PE\0\0`.
    InvalidPeSignature,
    /// The COFF machine is not 32-bit x86, irrespective of the file's name.
    UnsupportedMachine {
        /// The actual COFF machine value, for diagnostics.
        machine: u16,
    },
    /// Structural DLL validation failed before any OS mapping.
    InvalidDll(&'static str),
    /// The optional header is not PE32, even if COFF claims i386.
    UnsupportedOptionalHeader { magic: u16 },
    /// Neither supported undecorated plugin getter is present.
    MissingSupportedExport,
}

impl fmt::Display for PePreflightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TruncatedDosHeader => f.write_str("incomplete PE DOS header"),
            Self::InvalidDosSignature => f.write_str("missing PE DOS signature MZ"),
            Self::InvalidPeOffset => f.write_str("invalid PE header offset"),
            Self::TruncatedPeHeader => f.write_str("incomplete PE signature or COFF header"),
            Self::InvalidPeSignature => f.write_str("missing PE signature PE\\0\\0"),
            Self::UnsupportedMachine { machine } => write!(
                f,
                "plugin PE machine {machine:#06x} is not 32-bit x86 (expected 0x014c)"
            ),
            Self::InvalidDll(reason) => write!(f, "invalid plugin DLL: {reason}"),
            Self::UnsupportedOptionalHeader { magic } => {
                write!(
                    f,
                    "plugin optional header {magic:#06x} is not PE32 (expected 0x010b)"
                )
            }
            Self::MissingSupportedExport => f.write_str(
                "plugin DLL exports neither winampGetInModule2, winampDSPGetHeader2, \
                 winampGetGeneralPurposePlugin, winampVisGetHeader nor winampGetOutModule",
            ),
        }
    }
}

impl std::error::Error for PePreflightError {}

/// Require an `I386` COFF machine in a bounded, complete PE machine header.
///
/// `Ok(())` means only that this header check matched. It does not establish
/// that these bytes form a DLL or that loading them is safe or authorized.
/// S09's x86 requirement does not by itself establish an i686 CPU baseline.
pub fn check_pe_i386_machine(image: &[u8]) -> Result<(), PePreflightError> {
    let dos = image
        .get(..DOS_HEADER_LEN)
        .ok_or(PePreflightError::TruncatedDosHeader)?;
    if &dos[..2] != b"MZ" {
        return Err(PePreflightError::InvalidDosSignature);
    }
    let offset = u32::from_le_bytes([dos[0x3c], dos[0x3d], dos[0x3e], dos[0x3f]]);
    let offset = usize::try_from(offset).map_err(|_| PePreflightError::InvalidPeOffset)?;
    if offset < DOS_HEADER_LEN {
        return Err(PePreflightError::InvalidPeOffset);
    }
    let end = offset
        .checked_add(PE_SIGNATURE_AND_COFF_LEN)
        .ok_or(PePreflightError::InvalidPeOffset)?;
    let pe = image
        .get(offset..end)
        .ok_or(PePreflightError::TruncatedPeHeader)?;
    if &pe[..4] != b"PE\0\0" {
        return Err(PePreflightError::InvalidPeSignature);
    }
    let machine = u16::from_le_bytes([pe[4], pe[5]]);
    if machine != IMAGE_FILE_MACHINE_I386 {
        return Err(PePreflightError::UnsupportedMachine { machine });
    }
    Ok(())
}

/// Check a bounded PE32 DLL layout and select a supported executable getter.
///
/// This is a conservative loading policy, not a complete Windows PE validator,
/// signature/trust check or proof that a getter/callback is safe. It examines
/// no instructions and calls no library code. Module versions are dynamic ABI
/// results and cannot be established here (S09 sections 2, 3.5 and 7.2).
pub fn check_pe_i386_dll(image: &[u8]) -> Result<PluginKind, PePreflightError> {
    check_pe_i386_machine(image)?;
    let invalid = PePreflightError::InvalidDll;
    let pe = read_u32(image, 0x3c)? as usize;
    let characteristics = read_u16(image, pe + 22)?;
    if characteristics & 0x2002 != 0x2002 {
        return Err(invalid("COFF must mark an executable DLL"));
    }
    let optional = pe + PE_SIGNATURE_AND_COFF_LEN;
    let optional_len = usize::from(read_u16(image, pe + 20)?);
    let optional_bytes = range(image, optional, optional_len)?;
    let magic = read_u16(optional_bytes, 0)?;
    if magic != 0x010b {
        return Err(PePreflightError::UnsupportedOptionalHeader { magic });
    }
    if optional_len < 104 || read_u32(optional_bytes, 92)? == 0 {
        return Err(invalid("missing PE32 export data directory"));
    }
    let count = usize::from(read_u16(image, pe + 6)?);
    if !(1..=96).contains(&count) {
        return Err(invalid("section count must be between 1 and 96"));
    }
    let table = optional + optional_len;
    range(image, table, count * 40)?;
    let headers = read_u32(optional_bytes, 60)? as usize;
    let image_size = read_u32(optional_bytes, 56)? as usize;
    if headers < table + count * 40 || headers > image.len() || image_size < headers {
        return Err(invalid("invalid image/header size"));
    }
    let mut sections = Vec::with_capacity(count);
    for index in 0..count {
        let row = table + index * 40;
        let virtual_size = read_u32(image, row + 8)? as usize;
        let rva = read_u32(image, row + 12)? as usize;
        let raw_size = read_u32(image, row + 16)? as usize;
        let raw = read_u32(image, row + 20)? as usize;
        let span = virtual_size.max(raw_size);
        let end = rva
            .checked_add(span)
            .ok_or(invalid("section RVA overflow"))?;
        if rva < headers || end > image_size {
            return Err(invalid("section lies outside image"));
        }
        if raw_size != 0 {
            if raw < headers {
                return Err(invalid("section overlaps headers"));
            }
            range(image, raw, raw_size)?;
        }
        let section = Section {
            rva,
            end,
            raw,
            raw_size,
            executable: read_u32(image, row + 36)? & 0x2000_0000 != 0,
        };
        for other in &sections {
            let other: &Section = other;
            if rva < other.end && other.rva < end {
                return Err(invalid("overlapping virtual sections"));
            }
            if raw_size != 0
                && other.raw_size != 0
                && raw < other.raw + other.raw_size
                && other.raw < raw + raw_size
            {
                return Err(invalid("overlapping file sections"));
            }
        }
        sections.push(section);
    }
    let entry = read_u32(optional_bytes, 16)? as usize;
    if entry != 0 {
        executable_rva(image, &sections, entry)?;
    }
    let export = read_u32(optional_bytes, 96)? as usize;
    let export_size = read_u32(optional_bytes, 100)? as usize;
    if export == 0 || export_size < 40 {
        return Err(PePreflightError::MissingSupportedExport);
    }
    let directory = rva_range(image, &sections, export, export_size)?;
    let functions = read_u32(directory, 20)? as usize;
    let names = read_u32(directory, 24)? as usize;
    let function_table = rva_range(
        image,
        &sections,
        read_u32(directory, 28)? as usize,
        functions
            .checked_mul(4)
            .ok_or(invalid("export count overflow"))?,
    )?;
    let name_table = rva_range(
        image,
        &sections,
        read_u32(directory, 32)? as usize,
        names
            .checked_mul(4)
            .ok_or(invalid("export count overflow"))?,
    )?;
    let ordinal_table = rva_range(
        image,
        &sections,
        read_u32(directory, 36)? as usize,
        names
            .checked_mul(2)
            .ok_or(invalid("export count overflow"))?,
    )?;
    let mut previous = None;
    let mut selected: Option<(u8, PluginKind)> = None;
    for index in 0..names {
        let name_rva = read_u32(name_table, index * 4)? as usize;
        let name = export_name(image, &sections, name_rva)?;
        // Windows searches the name table in lexical order. Merely scanning an
        // unsorted table could "find" a getter that GetProcAddress cannot find.
        if previous.is_some_and(|previous: &[u8]| previous >= name) {
            return Err(invalid("export names must be unique and sorted"));
        }
        previous = Some(name);
        let ordinal = usize::from(read_u16(ordinal_table, index * 2)?);
        if ordinal >= functions {
            return Err(invalid("plugin export ordinal is out of bounds"));
        }
        let Some((rank, kind)) = supported_getter(name) else {
            continue;
        };
        let function = read_u32(function_table, ordinal * 4)? as usize;
        if (export..export + export_size).contains(&function) {
            return Err(invalid("forwarded plugin getters are not supported"));
        }
        executable_rva(image, &sections, function)?;
        // The kind that comes first in the table wins. A DLL exporting several
        // getters is pathological, but the choice has to be stable.
        if selected.is_none_or(|(current, _)| rank < current) {
            selected = Some((rank, kind));
        }
    }
    selected
        .map(|(_, kind)| kind)
        .ok_or(PePreflightError::MissingSupportedExport)
}

/// The getters the passive preflight recognises, in the order they win when a
/// DLL exports more than one of them.
///
/// Own policy, not S09: a plugin DLL exports exactly one getter (S09 §2), so
/// the order only has to be stable and documented. Input keeps the precedence
/// it always had, the output getter comes last — an `out_` DLL is never
/// mistaken for another kind — and the relative order of the kinds that were
/// already supported is unchanged. The visualization getter sits between the
/// general and the output getter: like the output getter it is recognized by
/// its export alone, because 2.95 probes any DLL for it and no `vis_` file
/// name prefix is part of the contract (S09 §2, U-5).
const SUPPORTED_GETTERS: [(&[u8], PluginKind); 5] = [
    (b"winampGetInModule2", PluginKind::Input),
    (b"winampDSPGetHeader2", PluginKind::Dsp),
    (b"winampGetGeneralPurposePlugin", PluginKind::General),
    (b"winampVisGetHeader", PluginKind::Visualization),
    (b"winampGetOutModule", PluginKind::Output),
];

/// The rank and kind of a recognised getter, `None` for any other export.
fn supported_getter(name: &[u8]) -> Option<(u8, PluginKind)> {
    SUPPORTED_GETTERS
        .iter()
        .enumerate()
        .find(|(_, (candidate, _))| *candidate == name)
        .map(|(rank, (_, kind))| (rank as u8, *kind))
}

struct Section {
    rva: usize,
    end: usize,
    raw: usize,
    raw_size: usize,
    executable: bool,
}

fn range(image: &[u8], start: usize, len: usize) -> Result<&[u8], PePreflightError> {
    let end = start
        .checked_add(len)
        .ok_or(PePreflightError::InvalidDll("range overflow"))?;
    image
        .get(start..end)
        .ok_or(PePreflightError::InvalidDll("truncated file range"))
}

fn read_u16(image: &[u8], start: usize) -> Result<u16, PePreflightError> {
    let bytes = range(image, start, 2)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(image: &[u8], start: usize) -> Result<u32, PePreflightError> {
    let bytes = range(image, start, 4)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn rva_range<'a>(
    image: &'a [u8],
    sections: &[Section],
    rva: usize,
    len: usize,
) -> Result<&'a [u8], PePreflightError> {
    for section in sections {
        if let Some(delta) = rva.checked_sub(section.rva)
            && delta < section.raw_size
            && len <= section.raw_size - delta
        {
            return range(image, section.raw + delta, len);
        }
    }
    Err(PePreflightError::InvalidDll(
        "RVA is not backed by section file bytes",
    ))
}

fn executable_rva(image: &[u8], sections: &[Section], rva: usize) -> Result<(), PePreflightError> {
    rva_range(image, sections, rva, 1)?;
    if !sections
        .iter()
        .any(|section| section.executable && (section.rva..section.end).contains(&rva))
    {
        return Err(PePreflightError::InvalidDll(
            "entry/getter RVA is not executable",
        ));
    }
    Ok(())
}

fn export_name<'a>(
    image: &'a [u8],
    sections: &[Section],
    rva: usize,
) -> Result<&'a [u8], PePreflightError> {
    for section in sections {
        if let Some(delta) = rva.checked_sub(section.rva)
            && delta < section.raw_size
        {
            // Own conservative policy: bound work per name, not an ABI limit.
            let tail = range(
                image,
                section.raw + delta,
                (section.raw_size - delta).min(4096),
            )?;
            let end =
                tail.iter()
                    .position(|byte| *byte == 0)
                    .ok_or(PePreflightError::InvalidDll(
                        "export name is unterminated or exceeds 4095 bytes",
                    ))?;
            return Ok(&tail[..end]);
        }
    }
    Err(PePreflightError::InvalidDll(
        "export name RVA is not file-backed",
    ))
}

/// Synthetic structure only; never passed to a real OS loader.
#[cfg(test)]
pub(crate) fn test_dll(machine: u16) -> Vec<u8> {
    let mut bytes = vec![0; 0x600];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    for (offset, value) in [
        (0x84, machine),
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
    let name = b"winampGetInModule2\0";
    bytes[0x260..0x260 + name.len()].copy_from_slice(name);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_structural_input_and_general_fixtures_without_loading_them() {
        let mut image = test_dll(0x014c);
        assert_eq!(check_pe_i386_dll(&image), Ok(PluginKind::Input));
        let name = b"winampGetGeneralPurposePlugin\0";
        image[0x260..0x260 + name.len()].copy_from_slice(name);
        assert_eq!(check_pe_i386_dll(&image), Ok(PluginKind::General));
        let name = b"winampDSPGetHeader2\0";
        image[0x260..0x260 + name.len()].copy_from_slice(name);
        assert_eq!(check_pe_i386_dll(&image), Ok(PluginKind::Dsp));
    }

    #[test]
    fn recognises_an_output_plugin_by_its_export() {
        let mut image = test_dll(0x014c);
        let name = b"winampGetOutModule\0";
        image[0x260..0x260 + name.len()].copy_from_slice(name);
        assert_eq!(
            check_pe_i386_dll(&image),
            Ok(PluginKind::Output),
            "the output getter must select the output kind (S09 §2)"
        );
    }

    #[test]
    fn recognises_a_visualization_plugin_by_its_export() {
        let mut image = test_dll(0x014c);
        let name = b"winampVisGetHeader\0";
        image[0x260..0x260 + name.len()].copy_from_slice(name);
        assert_eq!(
            check_pe_i386_dll(&image),
            Ok(PluginKind::Visualization),
            "the visualization getter must select the visualization kind (S09 §2)"
        );
    }

    #[test]
    fn the_getter_table_matches_the_abi_exports() {
        for (name, kind) in SUPPORTED_GETTERS {
            assert_eq!(
                rustamp_plugin_abi::PluginKind::from_export_name(name),
                Some(kind),
                "{name:?} must be the export the ABI assigns to {kind:?}"
            );
        }
        // The order is the documented policy: input first, output last.
        assert_eq!(
            supported_getter(b"winampGetInModule2"),
            Some((0, PluginKind::Input))
        );
        assert_eq!(
            supported_getter(b"winampVisGetHeader"),
            Some((3, PluginKind::Visualization))
        );
        assert_eq!(
            supported_getter(b"winampGetOutModule"),
            Some((4, PluginKind::Output))
        );
        assert_eq!(supported_getter(b"something_else"), None);
    }

    #[test]
    fn dll_check_refuses_wrong_machine_and_pe32_plus_claiming_i386() {
        assert_eq!(
            check_pe_i386_dll(&test_dll(0x8664)),
            Err(PePreflightError::UnsupportedMachine { machine: 0x8664 })
        );
        let mut image = test_dll(0x014c);
        image[0x98..0x9a].copy_from_slice(&0x020b_u16.to_le_bytes());
        let error = check_pe_i386_dll(&image).unwrap_err();
        assert_eq!(
            error,
            PePreflightError::UnsupportedOptionalHeader { magic: 0x020b }
        );
        assert!(error.to_string().contains("not PE32"));
    }

    #[test]
    fn every_truncated_dll_prefix_is_refused() {
        let image = test_dll(0x014c);
        for end in 0..image.len() {
            assert!(
                check_pe_i386_dll(&image[..end]).is_err(),
                "accepted prefix {end}"
            );
        }
    }

    #[test]
    fn refuses_unsorted_names_instead_of_predicting_a_false_get_proc_address_success() {
        let mut image = test_dll(0x014c);
        for (offset, value) in [
            (0x218, 2_u32),
            (0x220, 0x10a0),
            (0x224, 0x10b0),
            (0x2a0, 0x1060),
            (0x2a4, 0x1080),
        ] {
            image[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        image[0x280..0x284].copy_from_slice(b"AAA\0");
        let error = check_pe_i386_dll(&image).unwrap_err();
        assert!(error.to_string().contains("sorted"));
    }

    #[test]
    fn refuses_hostile_structure_exports_and_non_executable_getters_passively() {
        // Each change leaves the original shallow i386 check passing.
        for (offset, bytes, diagnostic) in [
            (0x96, 0x0102_u16.to_le_bytes().to_vec(), "executable DLL"),
            (0x94, 2_u16.to_le_bytes().to_vec(), "export data directory"),
            (0x184, u32::MAX.to_le_bytes().to_vec(), "section"),
            (0x18c, 0_u32.to_le_bytes().to_vec(), "overlaps headers"),
            (0x214, 0x1000_u32.to_le_bytes().to_vec(), "RVA"),
            (0x248, 1_u16.to_le_bytes().to_vec(), "ordinal"),
            (0x240, 0x1060_u32.to_le_bytes().to_vec(), "forwarded"),
            (0x240, 0_u32.to_le_bytes().to_vec(), "RVA"),
            (
                0x19c,
                0x4000_0040_u32.to_le_bytes().to_vec(),
                "not executable",
            ),
            (0x260, b"_winampGetInModule2\0".to_vec(), "exports neither"),
        ] {
            let mut image = test_dll(0x014c);
            image[offset..offset + bytes.len()].copy_from_slice(&bytes);
            assert_eq!(check_pe_i386_machine(&image), Ok(()));
            let error = check_pe_i386_dll(&image).unwrap_err();
            assert!(
                error.to_string().contains(diagnostic),
                "{offset:#x}: {error}"
            );
        }
    }

    // Synthetic header bytes only: no sections, optional header, DLL code or
    // reference binary. These intentionally cannot demonstrate DLL loading.
    fn header(machine: u16) -> Vec<u8> {
        let offset = 0x80;
        let mut image = vec![0; offset + PE_SIGNATURE_AND_COFF_LEN];
        image[..2].copy_from_slice(b"MZ");
        image[0x3c..0x40].copy_from_slice(&(offset as u32).to_le_bytes());
        image[offset..offset + 4].copy_from_slice(b"PE\0\0");
        image[offset + 4..offset + 6].copy_from_slice(&machine.to_le_bytes());
        image
    }

    #[test]
    fn accepts_header_only_fixture_not_loadability_evidence() {
        assert_eq!(check_pe_i386_machine(&header(0x014c)), Ok(()));
    }

    #[test]
    fn rejects_x64_arm_and_unknown_machines_with_actual_value() {
        for machine in [0x8664, 0xaa64, 0x01c4, 0] {
            assert_eq!(
                check_pe_i386_machine(&header(machine)),
                Err(PePreflightError::UnsupportedMachine { machine })
            );
        }
    }

    #[test]
    fn every_truncated_prefix_is_refused_without_panicking() {
        let image = header(0x014c);
        for end in 0..image.len() {
            assert!(check_pe_i386_machine(&image[..end]).is_err());
        }
        assert_eq!(
            check_pe_i386_machine(&image[..63]),
            Err(PePreflightError::TruncatedDosHeader)
        );
        assert_eq!(
            check_pe_i386_machine(&image[..image.len() - 1]),
            Err(PePreflightError::TruncatedPeHeader)
        );
    }

    #[test]
    fn checks_both_signatures() {
        let mut image = header(0x014c);
        image[0] = 0;
        assert_eq!(
            check_pe_i386_machine(&image),
            Err(PePreflightError::InvalidDosSignature)
        );
        image[0] = b'M';
        image[0x83] = 1;
        assert_eq!(
            check_pe_i386_machine(&image),
            Err(PePreflightError::InvalidPeSignature)
        );
    }

    #[test]
    fn refuses_overlapping_and_out_of_bounds_offsets() {
        for offset in [0_u32, 1, 0x3f] {
            let mut image = header(0x014c);
            image[0x3c..0x40].copy_from_slice(&offset.to_le_bytes());
            assert_eq!(
                check_pe_i386_machine(&image),
                Err(PePreflightError::InvalidPeOffset)
            );
        }
        let mut image = header(0x014c);
        image[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            check_pe_i386_machine(&image),
            Err(PePreflightError::TruncatedPeHeader | PePreflightError::InvalidPeOffset)
        ));
    }

    #[test]
    fn follows_declared_offset_instead_of_scanning_for_a_signature() {
        let mut image = header(0x014c);
        image[0x3c..0x40].copy_from_slice(&0x40_u32.to_le_bytes());
        assert_eq!(
            check_pe_i386_machine(&image),
            Err(PePreflightError::InvalidPeSignature)
        );
    }

    #[test]
    fn diagnostic_identifies_wrong_architecture() {
        let error = check_pe_i386_machine(&header(0x8664)).unwrap_err();
        assert_eq!(
            error.to_string(),
            "plugin PE machine 0x8664 is not 32-bit x86 (expected 0x014c)"
        );
    }
}
