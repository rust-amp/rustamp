//! Offline PE import inspection for the shipped executables.
//!
//! `cargo xtask pe-imports <file>...` and `cargo xtask package` parse the
//! import directory (and the delay-load import directory) of each executable
//! and check every imported DLL against an allowlist of Windows system
//! libraries. Nothing is loaded or executed: the file is only read as bytes.
//!
//! The check is a *refusal* check. A file that cannot be read, is not a PE
//! image, has a malformed or empty import table, targets the wrong machine, or
//! imports any module outside the allowlist (`MSVCP140.dll`, `VCRUNTIME140.dll`,
//! `ucrtbase.dll`, `api-ms-win-crt-*` and every other non-system module)
//! is an error; it never silently passes.
//!
//! What it proves: the *static* import tables name only allowlisted modules.
//! What it does not prove: that the executable starts on a clean machine, nor
//! anything about `LoadLibrary` calls made at run time.

use std::fs;
use std::path::Path;

/// `IMAGE_FILE_MACHINE_I386`.
pub const MACHINE_X86: u16 = 0x014c;
/// `IMAGE_FILE_MACHINE_AMD64`.
pub const MACHINE_X64: u16 = 0x8664;

/// Windows system libraries a shipped executable may import (lower case).
///
/// Every entry is a module that ships with Windows 10/11 itself. The C
/// runtime is deliberately absent (`ucrtbase.dll`, `api-ms-win-crt-*`,
/// `vcruntime*`, `msvcp*`, `msvcr*`): the package policy is the static MSVC
/// CRT, so an import of any of them means the policy was broken.
pub const SYSTEM_DLL_ALLOWLIST: &[&str] = &[
    "advapi32.dll",
    "avrt.dll",
    "bcrypt.dll",
    "bcryptprimitives.dll",
    "combase.dll",
    "comctl32.dll",
    "comdlg32.dll",
    "crypt32.dll",
    "d3d11.dll",
    "dwmapi.dll",
    "dxgi.dll",
    "gdi32.dll",
    "imm32.dll",
    "iphlpapi.dll",
    "kernel32.dll",
    "kernelbase.dll",
    "mmdevapi.dll",
    "msimg32.dll",
    "ntdll.dll",
    "ole32.dll",
    "oleaut32.dll",
    "powrprof.dll",
    "propsys.dll",
    "psapi.dll",
    "rpcrt4.dll",
    "secur32.dll",
    "setupapi.dll",
    "shcore.dll",
    "shell32.dll",
    "shlwapi.dll",
    "user32.dll",
    "userenv.dll",
    "uxtheme.dll",
    "version.dll",
    "winmm.dll",
    "ws2_32.dll",
    "wtsapi32.dll",
];

/// Whether `name` is accepted by the built-in policy: an exact allowlist entry
/// or a Windows API-set name (`api-ms-win-*`, `ext-ms-win-*`) other than the
/// UCRT forwarders (`api-ms-win-crt-*`), which would mean a dynamic CRT.
pub fn is_system_dll(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("api-ms-win-crt-") {
        return false;
    }
    if (lower.starts_with("api-ms-win-") || lower.starts_with("ext-ms-win-"))
        && lower.ends_with(".dll")
    {
        return true;
    }
    SYSTEM_DLL_ALLOWLIST.contains(&lower.as_str())
}

/// What the import directories of one PE image name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeImports {
    pub machine: u16,
    pub is_pe32_plus: bool,
    /// Load-time imports, in table order.
    pub imports: Vec<String>,
    /// Delay-load imports, in table order.
    pub delay_imports: Vec<String>,
}

impl PeImports {
    /// Every named module, load-time first.
    pub fn all(&self) -> impl Iterator<Item = &String> {
        self.imports.iter().chain(self.delay_imports.iter())
    }
}

fn u16_at(b: &[u8], off: usize) -> Result<u16, String> {
    let s = b
        .get(off..off.checked_add(2).ok_or("offset overflow")?)
        .ok_or_else(|| format!("truncated image: u16 at {off:#x}"))?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], off: usize) -> Result<u32, String> {
    let s = b
        .get(off..off.checked_add(4).ok_or("offset overflow")?)
        .ok_or_else(|| format!("truncated image: u32 at {off:#x}"))?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

struct Section {
    va: u32,
    span: u32,
    raw_ptr: u32,
    raw_size: u32,
}

fn rva_to_offset(sections: &[Section], rva: u32) -> Result<usize, String> {
    for s in sections {
        if rva >= s.va && rva - s.va < s.span {
            let delta = rva - s.va;
            if delta >= s.raw_size {
                return Err(format!("RVA {rva:#x} lies in uninitialised section data"));
            }
            return Ok(s.raw_ptr as usize + delta as usize);
        }
    }
    Err(format!("RVA {rva:#x} is not inside any section"))
}

fn read_cstr(bytes: &[u8], off: usize) -> Result<String, String> {
    let tail = bytes
        .get(off..)
        .ok_or_else(|| format!("DLL name offset {off:#x} is outside the file"))?;
    let end =
        tail.iter().take(260).position(|&c| c == 0).ok_or_else(|| {
            format!("DLL name at {off:#x} is not NUL-terminated within 260 bytes")
        })?;
    if end == 0 {
        return Err(format!("empty DLL name at {off:#x}"));
    }
    let raw = &tail[..end];
    if !raw.iter().all(|&c| (0x21..0x7f).contains(&c)) {
        return Err(format!("DLL name at {off:#x} is not printable ASCII"));
    }
    Ok(String::from_utf8_lossy(raw).into_owned())
}

/// Upper bound on descriptors per table; a real table has a few dozen.
const MAX_DESCRIPTORS: usize = 4096;

/// Parse the import tables of a PE image held in memory.
pub fn parse_imports(bytes: &[u8]) -> Result<PeImports, String> {
    if bytes.get(0..2) != Some(b"MZ") {
        return Err("not a PE image: missing MZ header".to_string());
    }
    let pe = u32_at(bytes, 0x3c)? as usize;
    if bytes.get(pe..pe.saturating_add(4)) != Some(b"PE\0\0") {
        return Err("not a PE image: missing PE signature".to_string());
    }
    let coff = pe + 4;
    let machine = u16_at(bytes, coff)?;
    let section_count = u16_at(bytes, coff + 2)? as usize;
    let opt_size = u16_at(bytes, coff + 16)? as usize;
    let opt = coff + 20;
    let is_pe32_plus = match u16_at(bytes, opt)? {
        0x10b => false,
        0x20b => true,
        other => return Err(format!("unknown optional header magic {other:#x}")),
    };
    let (count_off, dirs_off) = if is_pe32_plus {
        (opt + 108, opt + 112)
    } else {
        (opt + 92, opt + 96)
    };
    if dirs_off > opt + opt_size {
        return Err("optional header too small for data directories".to_string());
    }
    let dir_count = u32_at(bytes, count_off)? as usize;

    let mut sections = Vec::new();
    let table = opt + opt_size;
    for i in 0..section_count {
        let at = table + i * 40;
        let vsize = u32_at(bytes, at + 8)?;
        let va = u32_at(bytes, at + 12)?;
        let raw_size = u32_at(bytes, at + 16)?;
        let raw_ptr = u32_at(bytes, at + 20)?;
        sections.push(Section {
            va,
            span: vsize.max(raw_size),
            raw_ptr,
            raw_size,
        });
    }

    let directory = |index: usize| -> Result<(u32, u32), String> {
        if index >= dir_count {
            return Ok((0, 0));
        }
        let at = dirs_off + index * 8;
        Ok((u32_at(bytes, at)?, u32_at(bytes, at + 4)?))
    };

    let (imp_rva, imp_size) = directory(1)?;
    if imp_rva == 0 || imp_size == 0 {
        return Err("empty import directory: nothing to verify, refusing".to_string());
    }
    let mut imports = Vec::new();
    let mut at = rva_to_offset(&sections, imp_rva)?;
    for _ in 0..MAX_DESCRIPTORS {
        let name_rva = u32_at(bytes, at + 12)?;
        let first_thunk = u32_at(bytes, at + 16)?;
        let original = u32_at(bytes, at)?;
        if name_rva == 0 && first_thunk == 0 && original == 0 {
            break;
        }
        imports.push(read_cstr(bytes, rva_to_offset(&sections, name_rva)?)?);
        at += 20;
        if imports.len() == MAX_DESCRIPTORS {
            return Err("import table has no terminator".to_string());
        }
    }

    let mut delay_imports = Vec::new();
    let (delay_rva, delay_size) = directory(13)?;
    if delay_rva != 0 && delay_size != 0 {
        let mut at = rva_to_offset(&sections, delay_rva)?;
        loop {
            let attributes = u32_at(bytes, at)?;
            let name_rva = u32_at(bytes, at + 4)?;
            if name_rva == 0 {
                break;
            }
            if attributes & 1 == 0 {
                return Err("delay-load descriptor uses absolute addresses; refusing".to_string());
            }
            delay_imports.push(read_cstr(bytes, rva_to_offset(&sections, name_rva)?)?);
            at += 32;
            if delay_imports.len() >= MAX_DESCRIPTORS {
                return Err("delay-load table has no terminator".to_string());
            }
        }
    }

    Ok(PeImports {
        machine,
        is_pe32_plus,
        imports,
        delay_imports,
    })
}

/// Check one image's modules against `allow` (case-insensitive names) or, when
/// `allow` is `None`, against the built-in Windows system policy.
pub fn check_modules(imports: &PeImports, allow: Option<&[String]>) -> Result<(), String> {
    let mut bad: Vec<&str> = Vec::new();
    for name in imports.all() {
        let ok = match allow {
            Some(list) => list.iter().any(|a| a.eq_ignore_ascii_case(name)),
            None => is_system_dll(name),
        };
        if !ok && !bad.iter().any(|b| b.eq_ignore_ascii_case(name)) {
            bad.push(name);
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("imports non-system module(s): {}", bad.join(", ")))
    }
}

/// Read, parse and check one file. Returns the parsed imports on success.
///
/// `machine` (when given) must equal the image's machine type, so a 32-bit
/// host cannot be shipped where a 64-bit binary is expected or vice versa.
pub fn verify_file(
    path: &Path,
    machine: Option<u16>,
    allow: Option<&[String]>,
) -> Result<PeImports, String> {
    let bytes = fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let imports = parse_imports(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(expected) = machine
        && imports.machine != expected
    {
        return Err(format!(
            "{}: machine type {:#06x}, expected {:#06x}",
            path.display(),
            imports.machine,
            expected
        ));
    }
    check_modules(&imports, allow).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(imports)
}

fn machine_name(machine: u16) -> &'static str {
    match machine {
        MACHINE_X86 => "x86 (i686)",
        MACHINE_X64 => "x86_64",
        _ => "other",
    }
}

/// One-line summary of a successful inspection.
pub fn summary(path: &Path, imports: &PeImports) -> String {
    let mut names: Vec<String> = imports
        .imports
        .iter()
        .map(|n| n.to_ascii_lowercase())
        .collect();
    names.extend(
        imports
            .delay_imports
            .iter()
            .map(|n| format!("{} (delay-load)", n.to_ascii_lowercase())),
    );
    format!(
        "{} [{}]: {} module(s): {}",
        path.display(),
        machine_name(imports.machine),
        names.len(),
        names.join(", ")
    )
}

/// `cargo xtask pe-imports [--allow a.dll,b.dll] [--machine x64|x86] <file>...`
pub fn run(args: &[String]) -> Result<(), String> {
    let mut allow: Option<Vec<String>> = None;
    let mut machine: Option<u16> = None;
    let mut files = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--allow" => {
                let v = it
                    .next()
                    .ok_or("--allow needs a comma-separated DLL list")?;
                allow = Some(
                    v.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect(),
                );
            }
            "--machine" => {
                machine = Some(match it.next().map(String::as_str) {
                    Some("x64") => MACHINE_X64,
                    Some("x86") => MACHINE_X86,
                    _ => return Err("--machine needs `x64` or `x86`".to_string()),
                });
            }
            "-h" | "--help" => {
                println!(
                    "usage: cargo xtask pe-imports [--allow a.dll,b.dll] [--machine x64|x86] <file>...\n\n\
                     Parse the import tables of each PE file (read-only, nothing is executed)\n\
                     and fail unless every imported DLL is a Windows system library.\n\
                     --allow replaces the built-in allowlist with the given names."
                );
                return Ok(());
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown pe-imports option `{other}`"));
            }
            file => files.push(file.to_string()),
        }
    }
    if files.is_empty() {
        return Err("pe-imports needs at least one file".to_string());
    }
    let mut failures = Vec::new();
    for file in &files {
        let path = Path::new(file);
        match verify_file(path, machine, allow.as_deref()) {
            Ok(imports) => println!("OK   {}", summary(path, &imports)),
            Err(e) => {
                println!("FAIL {e}");
                failures.push(e);
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} of {} file(s) failed the import inspection",
            failures.len(),
            files.len()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal PE32+ image with one `.idata` section naming `dlls`.
    fn image(machine: u16, dlls: &[&str], delay: &[&str]) -> Vec<u8> {
        let pe_off = 0x40usize;
        let opt_size = 112 + 16 * 8;
        let sect_table = pe_off + 4 + 20 + opt_size;
        let raw_ptr = 0x200usize;
        let va = 0x1000u32;
        // idata layout: descriptors, delay descriptors, then names.
        let imp_len = (dlls.len() + 1) * 20;
        let delay_len = if delay.is_empty() {
            0
        } else {
            (delay.len() + 1) * 32
        };
        let mut names = Vec::new();
        let mut imp_name_rvas = Vec::new();
        let mut delay_name_rvas = Vec::new();
        let names_base = va as usize + imp_len + delay_len;
        for d in dlls {
            imp_name_rvas.push((names_base + names.len()) as u32);
            names.extend_from_slice(d.as_bytes());
            names.push(0);
        }
        for d in delay {
            delay_name_rvas.push((names_base + names.len()) as u32);
            names.extend_from_slice(d.as_bytes());
            names.push(0);
        }
        let total = imp_len + delay_len + names.len();
        let mut b = vec![0u8; raw_ptr + total];
        b[0..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&(pe_off as u32).to_le_bytes());
        b[pe_off..pe_off + 4].copy_from_slice(b"PE\0\0");
        let coff = pe_off + 4;
        b[coff..coff + 2].copy_from_slice(&machine.to_le_bytes());
        b[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
        b[coff + 16..coff + 18].copy_from_slice(&(opt_size as u16).to_le_bytes());
        let opt = coff + 20;
        b[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        b[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
        let dirs = opt + 112;
        if !dlls.is_empty() {
            b[dirs + 8..dirs + 12].copy_from_slice(&va.to_le_bytes());
            b[dirs + 12..dirs + 16].copy_from_slice(&(imp_len as u32).to_le_bytes());
        }
        if !delay.is_empty() {
            let d = dirs + 13 * 8;
            b[d..d + 4].copy_from_slice(&(va + imp_len as u32).to_le_bytes());
            b[d + 4..d + 8].copy_from_slice(&(delay_len as u32).to_le_bytes());
        }
        let s = sect_table;
        b[s + 8..s + 12].copy_from_slice(&(total as u32).to_le_bytes());
        b[s + 12..s + 16].copy_from_slice(&va.to_le_bytes());
        b[s + 16..s + 20].copy_from_slice(&(total as u32).to_le_bytes());
        b[s + 20..s + 24].copy_from_slice(&(raw_ptr as u32).to_le_bytes());
        for (i, rva) in imp_name_rvas.iter().enumerate() {
            let at = raw_ptr + i * 20;
            b[at..at + 4].copy_from_slice(&0x2000u32.to_le_bytes());
            b[at + 12..at + 16].copy_from_slice(&rva.to_le_bytes());
            b[at + 16..at + 20].copy_from_slice(&0x3000u32.to_le_bytes());
        }
        for (i, rva) in delay_name_rvas.iter().enumerate() {
            let at = raw_ptr + imp_len + i * 32;
            b[at..at + 4].copy_from_slice(&1u32.to_le_bytes());
            b[at + 4..at + 8].copy_from_slice(&rva.to_le_bytes());
        }
        let n = raw_ptr + imp_len + delay_len;
        b[n..n + names.len()].copy_from_slice(&names);
        b
    }

    #[test]
    fn parses_load_time_and_delay_imports() {
        let img = image(
            MACHINE_X64,
            &["KERNEL32.dll", "user32.dll"],
            &["SHCORE.dll"],
        );
        let p = parse_imports(&img).unwrap();
        assert_eq!(p.machine, MACHINE_X64);
        assert!(p.is_pe32_plus);
        assert_eq!(p.imports, ["KERNEL32.dll", "user32.dll"]);
        assert_eq!(p.delay_imports, ["SHCORE.dll"]);
        assert!(check_modules(&p, None).is_ok());
    }

    #[test]
    fn rejects_runtime_dlls_even_when_system_ones_are_present() {
        for bad in [
            "MSVCP140.dll",
            "VCRUNTIME140.dll",
            "ucrtbase.dll",
            "api-ms-win-crt-runtime-l1-1-0.dll",
            "evil.dll",
        ] {
            let img = image(MACHINE_X64, &["KERNEL32.dll", bad], &[]);
            let p = parse_imports(&img).unwrap();
            let err = check_modules(&p, None).unwrap_err();
            assert!(err.contains(bad), "{err}");
        }
        let delayed = image(MACHINE_X64, &["KERNEL32.dll"], &["VCRUNTIME140.dll"]);
        assert!(check_modules(&parse_imports(&delayed).unwrap(), None).is_err());
    }

    #[test]
    fn api_sets_pass_but_the_crt_forwarders_do_not() {
        assert!(is_system_dll("api-ms-win-core-synch-l1-2-0.dll"));
        assert!(is_system_dll("NTDLL.DLL"));
        assert!(!is_system_dll("api-ms-win-crt-stdio-l1-1-0.dll"));
        assert!(!is_system_dll("msvcp140.dll"));
    }

    #[test]
    fn an_explicit_allowlist_replaces_the_builtin_one() {
        let img = image(MACHINE_X64, &["KERNEL32.dll"], &[]);
        let p = parse_imports(&img).unwrap();
        assert!(check_modules(&p, Some(&["kernel32.dll".to_string()])).is_ok());
        assert!(check_modules(&p, Some(&["user32.dll".to_string()])).is_err());
    }

    #[test]
    fn refuses_unreadable_malformed_and_empty_images() {
        assert!(parse_imports(b"").is_err());
        assert!(parse_imports(b"not a pe file at all").is_err());
        let img = image(MACHINE_X64, &["KERNEL32.dll"], &[]);
        assert!(parse_imports(&img[..0x150]).is_err());
        let empty = image(MACHINE_X64, &[], &[]);
        assert!(parse_imports(&empty).unwrap_err().contains("empty import"));
        assert!(
            verify_file(Path::new("definitely/not/here.exe"), None, None)
                .unwrap_err()
                .contains("cannot read")
        );
    }

    #[test]
    fn the_machine_type_is_enforced() {
        let dir = std::env::temp_dir().join(format!("rustamp-pe-imports-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("x.exe");
        fs::write(&file, image(MACHINE_X86, &["KERNEL32.dll"], &[])).unwrap();
        assert!(verify_file(&file, Some(MACHINE_X86), None).is_ok());
        assert!(
            verify_file(&file, Some(MACHINE_X64), None)
                .unwrap_err()
                .contains("expected")
        );
        let _ = fs::remove_dir_all(dir);
    }
}
