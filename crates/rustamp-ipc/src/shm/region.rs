//! Mappings that back a ring (RFC 0003 section 3).
//!
//! This is one of the two modules in the crate that touch raw pointers: it owns
//! the lifetime of the memory a ring lives in and hands out the base address.
//! Everything above it (`shm::ring`) works through the safe ring API.
//!
//! Two implementations exist:
//!
//! * [`AnonymousRegion`] - a plain aligned allocation in this process, shared by
//!   cloning the handle. Used by tests and by any single-process use.
//! * [`NamedRegion`] - a Windows named file mapping (`CreateFileMappingW` /
//!   `OpenFileMappingW` + `MapViewOfFile`), i.e. the real cross-process path.

use core::alloc::Layout;
use core::ptr::NonNull;
use std::sync::Arc;

use super::RingError;
use super::header::{RING_MIN_ALIGN, RING_STATE_LIVE};

/// Alignment the ring asks a mapping for; keeps the header and each 64-byte
/// cache line of it on separate lines for the anonymous backing store too.
const REGION_ALIGN: usize = 64;

const _: () = assert!(REGION_ALIGN >= RING_MIN_ALIGN);

/// A byte region that a ring header and its slots live in.
///
/// # Safety
///
/// The ring relies on the following contract, which an implementation must
/// uphold for as long as the value is alive:
///
/// * [`MappedRegion::base`] returns the same address on every call.
/// * That address is valid and writable for [`MappedRegion::len`] bytes.
/// * It is aligned to [`RING_MIN_ALIGN`] and the memory stays mapped until the
///   value is dropped.
/// * The memory is not aliased by a Rust `&mut` for the lifetime of the region.
pub trait MappedRegion: Send {
    /// Base address of the mapping.
    ///
    /// # Safety
    ///
    /// The caller must uphold the invariants of [`MappedRegion`]: the pointer is
    /// only used with the ring's atomic publication protocol (header fields
    /// through whole-atomic accesses, slot data through whole-slot copies) and
    /// must not be used after `self` is dropped.
    unsafe fn base(&self) -> *mut u8;

    /// Size of the mapping in bytes.
    fn len(&self) -> usize;

    /// Whether the mapping is empty; always `false` for a valid ring region.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The allocation behind one or more [`AnonymousRegion`] handles.
struct AnonymousAlloc {
    ptr: NonNull<u8>,
    layout: Layout,
}

impl Drop for AnonymousAlloc {
    fn drop(&mut self) {
        // SAFETY: `ptr` was returned by `alloc_zeroed` with exactly this
        // `layout`, this is the last handle (the `Arc` count reached zero), and
        // the memory has not been freed or reallocated since.
        unsafe { std::alloc::dealloc(self.ptr.as_ptr(), self.layout) }
    }
}

// SAFETY: the allocation is plain memory with no thread affinity; the ring's
// atomic protocol, not Rust aliasing, is what keeps concurrent access sound.
unsafe impl Send for AnonymousAlloc {}
// SAFETY: see above; every access goes through the atomics in `RingHeader`.
unsafe impl Sync for AnonymousAlloc {}

/// An in-process region backed by one aligned heap allocation.
///
/// Cloning gives another handle to the *same* memory, which is how a test (or a
/// single-process user) puts a producer and a consumer on one ring.
#[derive(Clone)]
pub struct AnonymousRegion {
    inner: Arc<AnonymousAlloc>,
}

impl AnonymousRegion {
    /// Allocate `size` zeroed bytes, aligned to 64 bytes.
    pub fn new(size: usize) -> Result<AnonymousRegion, RingError> {
        let layout =
            Layout::from_size_align(size, REGION_ALIGN).map_err(|_| RingError::InvalidLayout)?;
        if layout.size() == 0 {
            return Err(RingError::InvalidLayout);
        }
        // SAFETY: `layout` has a non-zero size, so `alloc_zeroed` is called with
        // a valid layout. The result is checked for null before use.
        let raw = unsafe { std::alloc::alloc_zeroed(layout) };
        let ptr = NonNull::new(raw).ok_or(RingError::OutOfMemory)?;
        Ok(AnonymousRegion {
            inner: Arc::new(AnonymousAlloc { ptr, layout }),
        })
    }
}

impl MappedRegion for AnonymousRegion {
    unsafe fn base(&self) -> *mut u8 {
        self.inner.ptr.as_ptr()
    }

    fn len(&self) -> usize {
        self.inner.layout.size()
    }
}

/// A Windows named file mapping.
///
/// `create` makes the section (or attaches to an existing one with the same
/// name, which is what makes the generation rule in RFC 0003 section 8
/// necessary); `open` attaches to a section that already exists.
#[cfg(windows)]
pub struct NamedRegion {
    handle: windows::Win32::Foundation::HANDLE,
    base: *mut u8,
    len: usize,
}

// SAFETY: the section handle is process-local but not thread-affine, and the
// mapped view is shared memory that the ring accesses only through atomics.
#[cfg(windows)]
unsafe impl Send for NamedRegion {}

#[cfg(windows)]
impl NamedRegion {
    /// Create the section `name` of `size` bytes and map it.
    ///
    /// If a section with that name already exists, Windows returns a handle to
    /// the existing one; the caller (the ring creator) is responsible for
    /// bumping the generation in that case.
    pub fn create(name: &str, size: usize) -> Result<NamedRegion, RingError> {
        use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows::Win32::System::Memory::{CreateFileMappingW, PAGE_READWRITE};

        let wide = wide_name(name);
        let size = u32::try_from(size).map_err(|_| RingError::InvalidLayout)?;
        // SAFETY: `INVALID_HANDLE_VALUE` selects the paging file as the backing
        // store, a null security descriptor means the default DACL, and `wide`
        // is a NUL-terminated UTF-16 string that outlives the call.
        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                size,
                windows::core::PCWSTR(wide.as_ptr()),
            )
        }
        .map_err(|e| RingError::MappingFailed {
            op: "CreateFileMappingW",
            code: e.code().0,
        })?;

        match Self::map(handle, size as usize) {
            Ok(region) => Ok(region),
            Err(e) => {
                // SAFETY: `handle` is a valid section handle returned above and
                // is closed exactly once here.
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(handle);
                }
                Err(e)
            }
        }
    }

    /// Open an existing section `name` and map `size` bytes of it.
    pub fn open(name: &str, size: usize) -> Result<NamedRegion, RingError> {
        use windows::Win32::System::Memory::{FILE_MAP_ALL_ACCESS, OpenFileMappingW};

        let wide = wide_name(name);
        let size = u32::try_from(size).map_err(|_| RingError::InvalidLayout)?;
        // SAFETY: `wide` is a NUL-terminated UTF-16 name that outlives the call.
        let handle = unsafe {
            OpenFileMappingW(
                FILE_MAP_ALL_ACCESS.0,
                false,
                windows::core::PCWSTR(wide.as_ptr()),
            )
        }
        .map_err(|_| RingError::MappingUnavailable)?;

        match Self::map(handle, size as usize) {
            Ok(region) => Ok(region),
            Err(e) => {
                // SAFETY: `handle` is a valid section handle returned above and
                // is closed exactly once here.
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(handle);
                }
                Err(e)
            }
        }
    }

    fn map(
        handle: windows::Win32::Foundation::HANDLE,
        size: usize,
    ) -> Result<NamedRegion, RingError> {
        use windows::Win32::System::Memory::{FILE_MAP_ALL_ACCESS, MapViewOfFile};

        // SAFETY: `handle` is a live section handle, the offset is zero, and
        // `size` was validated to fit a `u32`.
        let view = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, size) };
        if view.Value.is_null() {
            return Err(RingError::MappingFailed {
                op: "MapViewOfFile",
                code: std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
            });
        }
        Ok(NamedRegion {
            handle,
            base: view.Value.cast::<u8>(),
            len: size,
        })
    }
}

#[cfg(windows)]
impl MappedRegion for NamedRegion {
    unsafe fn base(&self) -> *mut u8 {
        self.base
    }

    fn len(&self) -> usize {
        self.len
    }
}

#[cfg(windows)]
impl Drop for NamedRegion {
    fn drop(&mut self) {
        use windows::Win32::System::Memory::{MEMORY_MAPPED_VIEW_ADDRESS, UnmapViewOfFile};

        // SAFETY: `base` came from a successful `MapViewOfFile` on `handle` and
        // is unmapped exactly once here, before the handle is closed.
        unsafe {
            let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.base.cast(),
            });
            let _ = windows::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

/// Encode a section name as NUL-terminated UTF-16.
#[cfg(windows)]
fn wide_name(name: &str) -> Vec<u16> {
    name.encode_utf16().chain(core::iter::once(0)).collect()
}

/// The recommended section name for a ring (RFC 0003 section 3).
///
/// The result contains no `\`, so the section lives in the caller's session
/// namespace. Allocating here is fine: names are built off the ring hot path.
pub fn ring_name(kind: super::RingKind, generation: u64) -> String {
    format!(
        "rustamp-{}-{}-{:016x}",
        kind.name(),
        std::process::id(),
        generation
    )
}

/// A fresh, non-zero generation value (RFC 0003 section 8).
///
/// Derived from the process id and the wall clock. The ring only compares
/// generations for equality, so uniqueness is what matters, not ordering.
pub(crate) fn fresh_generation() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seed = (u64::from(std::process::id()) << 32) ^ nanos ^ (RING_STATE_LIVE as u64);
    if seed == 0 { 1 } else { seed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shm::header::{RING_HEADER_SIZE, RingKind};

    #[test]
    fn anonymous_region_is_zeroed_and_aligned() {
        let region = AnonymousRegion::new(RING_HEADER_SIZE + 64).unwrap();
        assert_eq!(region.len(), RING_HEADER_SIZE + 64);
        assert!(!region.is_empty());
        // SAFETY: the region is alive, so the base pointer is valid for `len`.
        let base = unsafe { region.base() };
        assert_eq!(base as usize % REGION_ALIGN, 0);
        // SAFETY: reading `len()` zeroed bytes that the region owns.
        let bytes = unsafe { core::slice::from_raw_parts(base, region.len()) };
        assert!(bytes.iter().all(|b| *b == 0));
    }

    #[test]
    fn anonymous_region_clone_shares_memory() {
        let a = AnonymousRegion::new(RING_HEADER_SIZE).unwrap();
        let b = a.clone();
        // SAFETY: both handles are alive and point at the same allocation.
        unsafe {
            *a.base() = 0xAB;
            assert_eq!(*b.base(), 0xAB);
            assert_eq!(a.base(), b.base());
        }
    }

    #[test]
    fn anonymous_region_rejects_empty() {
        assert!(matches!(
            AnonymousRegion::new(0),
            Err(RingError::InvalidLayout)
        ));
    }

    #[test]
    fn generations_are_non_zero() {
        assert_ne!(fresh_generation(), 0);
        assert!(ring_name(RingKind::Pcm, 1).starts_with("rustamp-pcm-"));
        assert!(!ring_name(RingKind::Vis, 1).contains('\\'));
    }
}
