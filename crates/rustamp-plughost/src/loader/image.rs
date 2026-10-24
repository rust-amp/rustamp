//! Verified-byte snapshots. No function in this module loads or executes code.

use std::path::{Path, PathBuf};

use super::PluginLoadError;
use crate::pe_preflight::check_pe_i386_dll;
use rustamp_plugin_abi::PluginKind;

pub(super) struct PreparedImage {
    pub path: PathBuf,
    pub kind: PluginKind,
    bytes: Vec<u8>,
}

impl PreparedImage {
    pub fn read(path: &Path) -> Result<Self, PluginLoadError> {
        // Never fall back to a relative/unresolved loader path.
        let path = std::fs::canonicalize(path).map_err(PluginLoadError::Io)?;
        let bytes = std::fs::read(&path).map_err(PluginLoadError::Io)?;
        let kind = check_pe_i386_dll(&bytes).map_err(PluginLoadError::Preflight)?;
        Ok(Self { path, kind, bytes })
    }

    pub fn require_approval(&self, approved_image: &[u8]) -> Result<(), PluginLoadError> {
        if self.bytes != approved_image {
            return Err(PluginLoadError::NotApproved);
        }
        Ok(())
    }

    pub fn require_store_approval(
        &self,
        store: &crate::approval::ApprovalStore,
    ) -> Result<(), PluginLoadError> {
        store
            .check_approval(&self.bytes)
            .map(|_| ())
            .map_err(|err| match err {
                crate::approval::ApprovalStoreError::NotApproved => PluginLoadError::NotApproved,
                other => PluginLoadError::ApprovalStore(other),
            })
    }

    #[cfg(windows)]
    pub fn seal(&self) -> Result<SealedImage, PluginLoadError> {
        use std::io::{Read, Write};
        use std::os::windows::fs::OpenOptionsExt;

        // Re-open without write/delete sharing, then compare complete bytes, not
        // timestamps, size or pathname identity. Keep this handle across loading.
        let mut source = locked_reader(&self.path).map_err(PluginLoadError::Io)?;
        let mut current = Vec::new();
        source
            .read_to_end(&mut current)
            .map_err(PluginLoadError::Io)?;
        if current != self.bytes {
            return Err(PluginLoadError::ImageChanged(
                "source changed after preflight",
            ));
        }

        let mut sealed = SealedImage {
            directory: fresh_directory().map_err(PluginLoadError::Io)?,
            file: None,
            directories: Vec::new(),
            source,
        };
        // Pin every canonical ancestor against rename/reparse replacement, not
        // just the final file. This also fails closed on inaccessible ancestors.
        let canonical = std::fs::canonicalize(&sealed.directory).map_err(PluginLoadError::Io)?;
        let mut ancestors: Vec<_> = canonical.ancestors().collect();
        ancestors.reverse();
        for ancestor in ancestors {
            let directory = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(1) // FILE_SHARE_READ only: no write/delete sharing.
                .custom_flags(0x0200_0000) // FILE_FLAG_BACKUP_SEMANTICS: directory handle.
                .open(ancestor)
                .map_err(PluginLoadError::Io)?;
            sealed.directories.push(directory);
        }
        sealed.directory = canonical;
        let path = sealed.path();
        {
            let mut writer = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .share_mode(0)
                .open(&path)
                .map_err(PluginLoadError::Io)?;
            writer.write_all(&self.bytes).map_err(PluginLoadError::Io)?;
        }
        let mut file = locked_reader(&path).map_err(PluginLoadError::Io)?;
        current.clear();
        file.read_to_end(&mut current)
            .map_err(PluginLoadError::Io)?;
        if current != self.bytes {
            return Err(PluginLoadError::ImageChanged(
                "private copy differs from preflight bytes",
            ));
        }
        sealed.file = Some(file);
        Ok(sealed)
    }
}

#[cfg(windows)]
fn locked_reader(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
}

#[cfg(windows)]
fn fresh_directory() -> std::io::Result<PathBuf> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    // Creation is exclusive; guessing a name can cause only a retry/refusal.
    for _ in 0..128 {
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("rustamp-plugin-{}-{serial}", std::process::id()));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no unused plugin snapshot directory",
    ))
}

#[cfg(windows)]
pub(super) struct SealedImage {
    directory: PathBuf,
    file: Option<std::fs::File>,
    directories: Vec<std::fs::File>,
    source: std::fs::File,
}

#[cfg(windows)]
impl SealedImage {
    pub fn path(&self) -> PathBuf {
        self.directory.join("plugin.dll")
    }

    pub fn retain_for_process(self) {
        // S09 retains input/general mappings until process exit. Retain their
        // backing file and namespace locks too, including after an ABI failure.
        // A later load gets a new directory, never Windows' old path-cache entry.
        std::mem::forget(self);
    }
}

#[cfg(windows)]
impl Drop for SealedImage {
    fn drop(&mut self) {
        // Only unmapped snapshots reach Drop. Never FreeLibrary merely to clean
        // up an ABI refusal: plugin code may already have installed subclasses.
        self.file.take();
        let _ = std::fs::remove_file(self.path());
        self.directories.clear();
        let _ = std::fs::remove_dir(&self.directory);
        // Keep source alive through the final loader call (including failures).
        let _ = &self.source;
    }
}
