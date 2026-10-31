//! The system's version as an old plug-in needs to hear it.
//!
//! `vis_nsfs` answers its entry point with null on any system from major version
//! 6 on, and its modules check again before they start. Everything else the
//! library needs from the host is there. So a library that refuses and asks
//! `GetVersionExA` gets its import slot pointed at an answer of Windows XP SP3,
//! the last system it runs on by its own check, and is asked once more.
//!
//! Only that library hears the old version, and only after it refused: a
//! library that loads as it is keeps the truth.

/// The slot's offset in a mapped image for an imported function.
///
/// `image` is the image as it is mapped: offsets in it are relative addresses.
/// Everything is read through checks, since the image is the plug-in's.
fn import_slot(image: &[u8], library: &str, function: &str) -> Option<usize> {
    let u16_at = |at: usize| Some(u16::from_le_bytes(image.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |at: usize| Some(u32::from_le_bytes(image.get(at..at + 4)?.try_into().ok()?));
    let text_at = |at: usize, wanted: &str| {
        let bytes = wanted.as_bytes();
        image.get(at..at + bytes.len() + 1).is_some_and(|text| {
            text[..bytes.len()].eq_ignore_ascii_case(bytes) && text[bytes.len()] == 0
        })
    };
    let headers = u32_at(0x3C)? as usize;
    let optional = headers + 24;
    // The optional header's own mark says how wide the import table's slots are.
    let (slot_size, directories) = match u16_at(optional)? {
        0x10B => (4usize, optional + 96),
        0x20B => (8usize, optional + 112),
        _ => return None,
    };
    let mut descriptor = u32_at(directories + 8)? as usize;
    if descriptor == 0 {
        return None;
    }
    loop {
        let names = u32_at(descriptor)? as usize;
        let name = u32_at(descriptor + 12)? as usize;
        let slots = u32_at(descriptor + 16)? as usize;
        if name == 0 {
            return None;
        }
        if text_at(name, library) {
            // A library linked without the name table keeps the names in the
            // slots until it is bound; the loader has replaced them by now.
            let names = if names == 0 { return None } else { names };
            for index in 0.. {
                let at = names + index * slot_size;
                let entry = u32_at(at)?;
                let high = if slot_size == 8 { u32_at(at + 4)? } else { 0 };
                if entry == 0 && high == 0 {
                    break;
                }
                let by_ordinal = if slot_size == 8 {
                    high & 0x8000_0000 != 0
                } else {
                    entry & 0x8000_0000 != 0
                };
                if !by_ordinal && text_at(entry as usize + 2, function) {
                    return Some(slots + index * slot_size);
                }
            }
        }
        descriptor += 20;
    }
}

/// What `GetVersionExA` writes for the plug-in: the size asked for stays, and
/// the rest says Windows XP SP3. Bytes past the old block stay zero.
fn write_old_version(block: &mut [u8]) {
    let size = block.get(..4).map(|size| size.to_vec());
    block.fill(0);
    if let Some(size) = size {
        block[..4].copy_from_slice(&size);
    }
    let fields: [(usize, u32); 4] = [(4, 5), (8, 1), (12, 2600), (16, 2)];
    for (at, value) in fields {
        if let Some(slot) = block.get_mut(at..at + 4) {
            slot.copy_from_slice(&value.to_le_bytes());
        }
    }
    let service_pack = b"Service Pack 3";
    if let Some(slot) = block.get_mut(20..20 + service_pack.len()) {
        slot.copy_from_slice(service_pack);
    }
}

/// The two sizes the block has in the system's definition.
const OLD_BLOCK: u32 = 148;
const NEW_BLOCK: u32 = 156;

/// The plug-in's `GetVersionExA`.
#[cfg(windows)]
unsafe extern "system" fn old_get_version_ex(block: *mut u8) -> i32 {
    if block.is_null() {
        return 0;
    }
    // SAFETY: the caller states its block's size in the block's first member,
    // which the function's contract makes readable.
    let size = unsafe { block.cast::<u32>().read_unaligned() };
    if size != OLD_BLOCK && size != NEW_BLOCK {
        return 0;
    }
    // SAFETY: writable for the size it states, as above.
    let block = unsafe { std::slice::from_raw_parts_mut(block, size as usize) };
    write_old_version(block);
    1
}

/// Points the mapped library's `GetVersionExA` at the old answer; whether it
/// imports the function, and so whether asking again can change anything.
///
/// # Safety
///
/// `module` is the base of a library mapped in this process, on the thread that
/// loads it, before any of its threads runs.
#[cfg(windows)]
pub unsafe fn report_old_system(module: *mut core::ffi::c_void) -> bool {
    unsafe extern "system" {
        fn VirtualProtect(address: *mut u8, size: usize, new: u32, old: *mut u32) -> i32;
    }
    const PAGE_READWRITE: u32 = 0x04;
    let base = module.cast::<u8>();
    // SAFETY: the headers of a mapped image are readable; the size comes from
    // its optional header, past the file header's fixed place.
    let image = unsafe {
        let headers = base.add(0x3C).cast::<u32>().read_unaligned() as usize;
        let size = base.add(headers + 24 + 56).cast::<u32>().read_unaligned() as usize;
        std::slice::from_raw_parts(base, size)
    };
    let Some(slot) = import_slot(image, "KERNEL32.dll", "GetVersionExA") else {
        return false;
    };
    let width = std::mem::size_of::<usize>();
    let replacement = old_get_version_ex as unsafe extern "system" fn(*mut u8) -> i32 as usize;
    // SAFETY: the slot lies in the mapped image, found through its own tables;
    // its page is made writable for the one write and restored afterwards.
    unsafe {
        let target = base.add(slot);
        let mut previous = 0;
        if VirtualProtect(target, width, PAGE_READWRITE, &mut previous) == 0 {
            return false;
        }
        target.cast::<usize>().write_unaligned(replacement);
        VirtualProtect(target, width, previous, &mut previous);
    }
    crate::fault::trace_event("the plug-in refused to load: it now hears Windows XP");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 32-bit image with one import table: `KERNEL32.dll`, importing `Sleep`,
    /// an ordinal and `GetVersionExA`.
    fn image() -> Vec<u8> {
        let mut image = vec![0u8; 0x1000];
        let put32 = |image: &mut Vec<u8>, at: usize, value: u32| {
            image[at..at + 4].copy_from_slice(&value.to_le_bytes());
        };
        put32(&mut image, 0x3C, 0x80);
        image[0x80 + 24..0x80 + 26].copy_from_slice(&0x10Bu16.to_le_bytes());
        // The import directory.
        put32(&mut image, 0x80 + 24 + 96 + 8, 0x200);
        // The one descriptor: names, library's name, slots; then the end.
        put32(&mut image, 0x200, 0x300);
        put32(&mut image, 0x200 + 12, 0x400);
        put32(&mut image, 0x200 + 16, 0x500);
        image[0x400..0x400 + 13].copy_from_slice(b"KERNEL32.dll\0");
        put32(&mut image, 0x300, 0x600);
        put32(&mut image, 0x304, 0x8000_0005);
        put32(&mut image, 0x308, 0x620);
        image[0x602..0x608].copy_from_slice(b"Sleep\0");
        image[0x622..0x630].copy_from_slice(b"GetVersionExA\0");
        image
    }

    #[test]
    fn the_slot_of_the_import_is_found_by_its_name() {
        let image = image();
        assert_eq!(
            import_slot(&image, "kernel32.DLL", "GetVersionExA"),
            Some(0x508)
        );
        assert_eq!(import_slot(&image, "KERNEL32.dll", "Sleep"), Some(0x500));
        assert_eq!(import_slot(&image, "KERNEL32.dll", "Sleepy"), None);
        assert_eq!(import_slot(&image, "USER32.dll", "Sleep"), None);
    }

    #[test]
    fn a_broken_image_finds_nothing_and_does_not_panic() {
        assert_eq!(import_slot(&[], "KERNEL32.dll", "Sleep"), None);
        assert_eq!(import_slot(&[0; 0x40], "KERNEL32.dll", "Sleep"), None);
        let mut image = image();
        // The descriptor points past the image.
        image[0x80 + 24 + 96 + 8..0x80 + 24 + 96 + 12]
            .copy_from_slice(&0xFFFF_FF00u32.to_le_bytes());
        assert_eq!(import_slot(&image, "KERNEL32.dll", "Sleep"), None);
    }

    #[test]
    fn the_answer_says_xp_and_keeps_the_size() {
        let mut block = vec![0xAA; NEW_BLOCK as usize];
        block[..4].copy_from_slice(&NEW_BLOCK.to_le_bytes());
        write_old_version(&mut block);
        let word = |at: usize| u32::from_le_bytes(block[at..at + 4].try_into().unwrap());
        assert_eq!(word(0), NEW_BLOCK);
        assert_eq!(word(4), 5);
        assert_eq!(word(8), 1);
        assert_eq!(word(16), 2);
        assert_eq!(&block[20..34], b"Service Pack 3");
        assert_eq!(block[OLD_BLOCK as usize], 0);
    }

    #[cfg(windows)]
    #[test]
    fn a_block_of_a_size_nobody_knows_is_refused() {
        let mut block = [0u8; 64];
        block[..4].copy_from_slice(&64u32.to_le_bytes());
        // SAFETY: a live block of the size it states.
        assert_eq!(unsafe { old_get_version_ex(block.as_mut_ptr()) }, 0);
        // SAFETY: the null case is the function's own to answer.
        assert_eq!(unsafe { old_get_version_ex(std::ptr::null_mut()) }, 0);
    }
}
