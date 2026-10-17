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
//! result must never authorize a later path-based load (TOCTOU).

use std::fmt;

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

#[cfg(test)]
mod tests {
    use super::*;

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
