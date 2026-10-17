//! Production plugin approval store and cryptographic identity verification.
//!
//! # Architecture and Invariants
//!
//! The plugin host loads only binaries that have been explicitly and independently
//! approved by the user or security administrator. Structural PE preflight
//! ([`crate::pe_preflight`]) validates format, bounds, architecture, and exports,
//! but is NOT a malware scanner or approval mechanism.
//!
//! To eliminate TOCTOU check-load races and path-based identity confusion, this store
//! records and verifies a three-part cryptographic identity tuple for each approved DLL:
//! 1. **Whole-image SHA-256 hash**: full cryptographic binding against any bit changes.
//! 2. **Target machine (`0x014c`)**: explicit i386 COFF machine validation.
//! 3. **Image size (bytes)**: second-preimage defense and instant truncation check.
//!
//! # Storage and Format
//!
//! The store lives in a dedicated file beside the user configuration (e.g. `plugins-approved.toml`).
//! It uses TOML syntax matching Rustamp's configuration standard (`crates/rustamp-core/src/config.rs`).
//! Keeping approvals in a dedicated file separates security trust records from routine
//! mutable UI/playback settings, preventing accidental truncation or corruption during
//! ordinary configuration saves.
//!
//! A malformed store **never silently trusts or falls back to an empty allowlist**:
//! it refuses loudly with [`ApprovalStoreError::Malformed`] or [`ApprovalStoreError::UnsupportedVersion`].
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

use std::collections::HashSet;
use std::fmt;
use std::path::Path;

use crate::pe_preflight::{PePreflightError, check_pe_i386_dll, check_pe_i386_machine};

/// Current schema version of the approval store format.
pub const CURRENT_APPROVAL_STORE_VERSION: u32 = 1;

/// Expected COFF machine architecture for classic Winamp plugins (i386 = `0x014c`).
pub const EXPECTED_PLUGIN_MACHINE: u16 = 0x014c;

/// An approved plugin entry in the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalEntry {
    /// Human-readable file name or relative identifier (e.g. `"in_openmpt.dll"`).
    pub name: String,
    /// Cryptographic SHA-256 digest of the entire image, formatted as 64 lowercase hex characters.
    pub sha256: String,
    /// Total image size in bytes.
    pub size: u64,
    /// COFF machine architecture (must be [`EXPECTED_PLUGIN_MACHINE`]).
    pub machine: u16,
}

impl ApprovalEntry {
    /// Create a new approval entry with validation.
    pub fn new(
        name: impl Into<String>,
        sha256: impl Into<String>,
        size: u64,
        machine: u16,
    ) -> Result<Self, ApprovalStoreError> {
        let name = name.into();
        let sha256 = sha256.into().to_ascii_lowercase();
        if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ApprovalStoreError::Malformed(format!(
                "invalid sha256 hex string for '{name}': expected 64 hex characters, found '{sha256}'"
            )));
        }
        if machine != EXPECTED_PLUGIN_MACHINE {
            return Err(ApprovalStoreError::InvalidMachine {
                actual: machine,
                expected: EXPECTED_PLUGIN_MACHINE,
            });
        }
        Ok(Self {
            name,
            sha256,
            size,
            machine,
        })
    }

    /// Check if raw binary candidate bytes match this approved entry's exact identity.
    pub fn matches_bytes(&self, bytes: &[u8]) -> bool {
        if self.size != bytes.len() as u64 {
            return false;
        }
        let actual_hash = Sha256::digest_hex(bytes);
        if self.sha256 != actual_hash {
            return false;
        }
        match check_pe_i386_machine(bytes) {
            Ok(()) => self.machine == EXPECTED_PLUGIN_MACHINE,
            Err(_) => false,
        }
    }
}

/// A parsed, validated in-memory plugin approval store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalStore {
    version: u32,
    entries: Vec<ApprovalEntry>,
}

impl Default for ApprovalStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ApprovalStore {
    /// Create a new, empty approval store at current schema version.
    pub fn new() -> Self {
        Self {
            version: CURRENT_APPROVAL_STORE_VERSION,
            entries: Vec::new(),
        }
    }

    /// Schema version of this store.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Number of approved plugin entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store contains zero approved entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Read-only slice of all approved plugin records.
    pub fn entries(&self) -> &[ApprovalEntry] {
        &self.entries
    }

    /// Find an entry by exact SHA-256 hash.
    pub fn find_by_hash(&self, sha256_hex: &str) -> Option<&ApprovalEntry> {
        let needle = sha256_hex.to_ascii_lowercase();
        self.entries.iter().find(|e| e.sha256 == needle)
    }

    /// Find an entry by human-readable plugin name.
    pub fn find_by_name(&self, name: &str) -> Option<&ApprovalEntry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Test whether candidate image bytes match any approved record in the store.
    pub fn is_approved(&self, bytes: &[u8]) -> bool {
        self.check_approval(bytes).is_ok()
    }

    /// Verify candidate image bytes against the store.
    ///
    /// Validates machine architecture (`0x014c`), computes the full SHA-256 digest,
    /// and checks for an exact matching entry.
    ///
    /// Returns a reference to the matching [`ApprovalEntry`], or [`ApprovalStoreError::NotApproved`]
    /// if no matching approved identity exists.
    pub fn check_approval(&self, bytes: &[u8]) -> Result<&ApprovalEntry, ApprovalStoreError> {
        let size = bytes.len() as u64;
        let machine = match check_pe_i386_machine(bytes) {
            Ok(()) => EXPECTED_PLUGIN_MACHINE,
            Err(PePreflightError::UnsupportedMachine { machine }) => {
                return Err(ApprovalStoreError::InvalidMachine {
                    actual: machine,
                    expected: EXPECTED_PLUGIN_MACHINE,
                });
            }
            Err(_) => return Err(ApprovalStoreError::NotApproved),
        };

        let digest = Sha256::digest_hex(bytes);

        for entry in &self.entries {
            if entry.size == size && entry.machine == machine && entry.sha256 == digest {
                return Ok(entry);
            }
        }
        Err(ApprovalStoreError::NotApproved)
    }

    /// Preflight candidate bytes and record an approval for them.
    ///
    /// Fails with [`ApprovalStoreError::Preflight`] if the bytes are not a structurally
    /// valid i386 PE DLL with supported exports.
    pub fn approve_bytes(
        &mut self,
        name: &str,
        bytes: &[u8],
    ) -> Result<ApprovalEntry, ApprovalStoreError> {
        check_pe_i386_dll(bytes).map_err(ApprovalStoreError::Preflight)?;
        let sha256 = Sha256::digest_hex(bytes);
        let size = bytes.len() as u64;
        let machine = EXPECTED_PLUGIN_MACHINE;

        let entry = ApprovalEntry {
            name: name.to_string(),
            sha256,
            size,
            machine,
        };

        if let Some(pos) = self.entries.iter().position(|e| e.sha256 == entry.sha256) {
            self.entries[pos] = entry.clone();
        } else {
            self.entries.push(entry.clone());
        }
        Ok(entry)
    }

    /// Read a file from `path`, preflight its bytes, and approve it.
    pub fn approve_file(&mut self, path: &Path) -> Result<ApprovalEntry, ApprovalStoreError> {
        let bytes = std::fs::read(path).map_err(ApprovalStoreError::Io)?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("plugin.dll");
        self.approve_bytes(name, &bytes)
    }

    /// Revoke an approved entry by exact SHA-256 hash. Returns true if removed.
    pub fn revoke_by_hash(&mut self, sha256_hex: &str) -> bool {
        let needle = sha256_hex.to_ascii_lowercase();
        if let Some(pos) = self.entries.iter().position(|e| e.sha256 == needle) {
            self.entries.remove(pos);
            true
        } else {
            false
        }
    }

    /// Revoke an approved entry by name. Returns true if removed.
    pub fn revoke_by_name(&mut self, name: &str) -> bool {
        if let Some(pos) = self.entries.iter().position(|e| e.name == name) {
            self.entries.remove(pos);
            true
        } else {
            false
        }
    }

    /// Parse an approval store from a TOML string.
    ///
    /// Fails loudly on any syntax error, unknown key, missing required field,
    /// invalid hash/machine value, or unsupported version.
    pub fn parse(input: &str) -> Result<Self, ApprovalStoreError> {
        let mut version: Option<u32> = None;
        let mut entries: Vec<ApprovalEntry> = Vec::new();
        let mut current_entry: Option<PartialEntry> = None;

        for (line_idx, line) in input.lines().enumerate() {
            let line_num = line_idx + 1;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let content = strip_comments(trimmed).trim();
            if content.is_empty() {
                continue;
            }

            if content == "[[plugins]]" || content == "[[plugin]]" {
                if version.is_none() {
                    return Err(ApprovalStoreError::Malformed(format!(
                        "line {line_num}: [[plugins]] table array appears before version header"
                    )));
                }
                if let Some(partial) = current_entry.take() {
                    let entry = partial.finalize(line_num)?;
                    entries.push(entry);
                }
                current_entry = Some(PartialEntry::default());
                continue;
            }

            let Some((key, val)) = content.split_once('=') else {
                return Err(ApprovalStoreError::Malformed(format!(
                    "line {line_num}: syntax error: expected 'key = value', found '{content}'"
                )));
            };

            let key = key.trim();
            let val = val.trim();

            let Some(entry) = current_entry.as_mut() else {
                match key {
                    "version" => {
                        let parsed_ver: u32 = val.parse().map_err(|_| {
                            ApprovalStoreError::Malformed(format!(
                                "line {line_num}: invalid version integer '{val}'"
                            ))
                        })?;
                        if parsed_ver != CURRENT_APPROVAL_STORE_VERSION {
                            return Err(ApprovalStoreError::UnsupportedVersion(parsed_ver));
                        }
                        if version.is_some() {
                            return Err(ApprovalStoreError::Malformed(format!(
                                "line {line_num}: duplicate version header"
                            )));
                        }
                        version = Some(parsed_ver);
                    }
                    other => {
                        return Err(ApprovalStoreError::Malformed(format!(
                            "line {line_num}: unexpected top-level key '{other}'"
                        )));
                    }
                }
                continue;
            };

            match key {
                "name" => {
                    let s = parse_string_literal(val).ok_or_else(|| {
                        ApprovalStoreError::Malformed(format!(
                            "line {line_num}: invalid string literal for name '{val}'"
                        ))
                    })?;
                    if entry.name.is_some() {
                        return Err(ApprovalStoreError::Malformed(format!(
                            "line {line_num}: duplicate name in plugin entry"
                        )));
                    }
                    entry.name = Some(s);
                }
                "sha256" => {
                    let s = parse_string_literal(val).ok_or_else(|| {
                        ApprovalStoreError::Malformed(format!(
                            "line {line_num}: invalid string literal for sha256 '{val}'"
                        ))
                    })?;
                    let lower = s.to_ascii_lowercase();
                    if lower.len() != 64 || !lower.chars().all(|c| c.is_ascii_hexdigit()) {
                        return Err(ApprovalStoreError::Malformed(format!(
                            "line {line_num}: sha256 must be 64 hex characters, found '{s}'"
                        )));
                    }
                    if entry.sha256.is_some() {
                        return Err(ApprovalStoreError::Malformed(format!(
                            "line {line_num}: duplicate sha256 in plugin entry"
                        )));
                    }
                    entry.sha256 = Some(lower);
                }
                "size" => {
                    let s: u64 = val.parse().map_err(|_| {
                        ApprovalStoreError::Malformed(format!(
                            "line {line_num}: invalid integer for size '{val}'"
                        ))
                    })?;
                    if entry.size.is_some() {
                        return Err(ApprovalStoreError::Malformed(format!(
                            "line {line_num}: duplicate size in plugin entry"
                        )));
                    }
                    entry.size = Some(s);
                }
                "machine" => {
                    let m: u16 = parse_u16_dec_or_hex(val).ok_or_else(|| {
                        ApprovalStoreError::Malformed(format!(
                            "line {line_num}: invalid machine value '{val}'"
                        ))
                    })?;
                    if m != EXPECTED_PLUGIN_MACHINE {
                        return Err(ApprovalStoreError::InvalidMachine {
                            actual: m,
                            expected: EXPECTED_PLUGIN_MACHINE,
                        });
                    }
                    if entry.machine.is_some() {
                        return Err(ApprovalStoreError::Malformed(format!(
                            "line {line_num}: duplicate machine in plugin entry"
                        )));
                    }
                    entry.machine = Some(m);
                }
                other => {
                    return Err(ApprovalStoreError::Malformed(format!(
                        "line {line_num}: unknown entry key '{other}'"
                    )));
                }
            }
        }

        if let Some(partial) = current_entry.take() {
            let entry = partial.finalize(input.lines().count())?;
            entries.push(entry);
        }

        let Some(ver) = version else {
            return Err(ApprovalStoreError::Malformed(
                "missing version header in approval store".to_string(),
            ));
        };

        let mut seen = HashSet::new();
        for entry in &entries {
            if !seen.insert(&entry.sha256) {
                return Err(ApprovalStoreError::Malformed(format!(
                    "duplicate entry for sha256 {}",
                    entry.sha256
                )));
            }
        }

        Ok(Self {
            version: ver,
            entries,
        })
    }

    /// Load and validate an approval store from a file on disk.
    pub fn load_from_file(path: &Path) -> Result<Self, ApprovalStoreError> {
        let content = std::fs::read_to_string(path).map_err(ApprovalStoreError::Io)?;
        Self::parse(&content)
    }

    /// Open an approval store from disk, or return an empty store if the file does not exist.
    ///
    /// If the file exists but contains invalid data or syntax, this returns [`ApprovalStoreError::Malformed`]
    /// loudly and will NOT fall back to an empty store.
    pub fn open_or_empty(path: &Path) -> Result<Self, ApprovalStoreError> {
        match std::fs::read_to_string(path) {
            Ok(content) => Self::parse(&content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(ApprovalStoreError::Io(e)),
        }
    }

    /// Serialize this approval store into standard TOML text.
    pub fn to_toml_string(&self) -> String {
        let mut out = String::new();
        out.push_str("# Rustamp plugin approval store v1\n");
        out.push_str(&format!("version = {}\n\n", self.version));
        for entry in &self.entries {
            out.push_str("[[plugins]]\n");
            out.push_str(&format!("name = \"{}\"\n", escape_string(&entry.name)));
            out.push_str(&format!("sha256 = \"{}\"\n", entry.sha256));
            out.push_str(&format!("size = {}\n", entry.size));
            out.push_str(&format!("machine = {}\n\n", entry.machine));
        }
        out
    }

    /// Atomically write the approval store to `path`.
    pub fn save_to_file(&self, path: &Path) -> Result<(), ApprovalStoreError> {
        let toml = self.to_toml_string();
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let temp_path = parent.join(format!(
            ".tmp-approvals-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));

        {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .map_err(ApprovalStoreError::Io)?;
            file.write_all(toml.as_bytes())
                .map_err(ApprovalStoreError::Io)?;
            file.sync_all().map_err(ApprovalStoreError::Io)?;
        }

        std::fs::rename(&temp_path, path).map_err(|e| {
            let _ = std::fs::remove_file(&temp_path);
            ApprovalStoreError::Io(e)
        })?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers for Parsing
// ---------------------------------------------------------------------------

#[derive(Default)]
struct PartialEntry {
    name: Option<String>,
    sha256: Option<String>,
    size: Option<u64>,
    machine: Option<u16>,
}

impl PartialEntry {
    fn finalize(self, line_num: usize) -> Result<ApprovalEntry, ApprovalStoreError> {
        let name = self
            .name
            .unwrap_or_else(|| "unnamed_plugin.dll".to_string());
        let Some(sha256) = self.sha256 else {
            return Err(ApprovalStoreError::Malformed(format!(
                "line {line_num}: plugin entry missing required field 'sha256'"
            )));
        };
        let Some(size) = self.size else {
            return Err(ApprovalStoreError::Malformed(format!(
                "line {line_num}: plugin entry missing required field 'size'"
            )));
        };
        let Some(machine) = self.machine else {
            return Err(ApprovalStoreError::Malformed(format!(
                "line {line_num}: plugin entry missing required field 'machine'"
            )));
        };
        Ok(ApprovalEntry {
            name,
            sha256,
            size,
            machine,
        })
    }
}

fn strip_comments(s: &str) -> &str {
    let mut in_quotes = false;
    for (i, c) in s.char_indices() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if c == '#' && !in_quotes {
            return &s[..i];
        }
    }
    s
}

fn parse_string_literal(s: &str) -> Option<String> {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        let inner = &s[1..s.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => return None,
                }
            } else {
                out.push(c);
            }
        }
        Some(out)
    } else {
        None
    }
}

fn parse_u16_dec_or_hex(s: &str) -> Option<u16> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16).ok()
    } else {
        s.parse::<u16>().ok()
    }
}

fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Error Types
// ---------------------------------------------------------------------------

/// Why an approval store operation failed.
#[derive(Debug)]
pub enum ApprovalStoreError {
    /// File I/O error reading or writing the store.
    Io(std::io::Error),
    /// Syntax or structural error in the approval store file.
    Malformed(String),
    /// Unsupported approval store format version.
    UnsupportedVersion(u32),
    /// An approval record or candidate image declared an invalid machine architecture.
    InvalidMachine { actual: u16, expected: u16 },
    /// The plugin image failed passive PE preflight prior to approval.
    Preflight(PePreflightError),
    /// The candidate image is not approved in this store.
    NotApproved,
}

impl fmt::Display for ApprovalStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "approval store I/O error: {e}"),
            Self::Malformed(msg) => write!(f, "malformed approval store: {msg}"),
            Self::UnsupportedVersion(v) => write!(
                f,
                "unsupported approval store version {v} (expected {CURRENT_APPROVAL_STORE_VERSION})"
            ),
            Self::InvalidMachine { actual, expected } => write!(
                f,
                "invalid plugin machine architecture {actual:#06x} (expected {expected:#06x})"
            ),
            Self::Preflight(e) => write!(f, "plugin PE preflight failed before approval: {e}"),
            Self::NotApproved => f.write_str("plugin image is not approved by the approval store"),
        }
    }
}

impl std::error::Error for ApprovalStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Preflight(e) => Some(e),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Pure-Rust Clean-Room SHA-256 (FIPS 180-4)
// ---------------------------------------------------------------------------

/// Clean-room SHA-256 implementation per FIPS 180-4 standard specification.
#[derive(Debug, Clone)]
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buf_len: usize,
    total_len: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    /// Initialized hasher with FIPS 180-4 initial hash values H^(0).
    pub fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: [0u8; 64],
            buf_len: 0,
            total_len: 0,
        }
    }

    /// Feed input bytes into the cryptographic digest.
    pub fn update(&mut self, mut data: &[u8]) {
        self.total_len = self
            .total_len
            .checked_add(data.len() as u64)
            .expect("image size must fit in u64");

        if self.buf_len > 0 {
            let needed = 64 - self.buf_len;
            if data.len() >= needed {
                self.buffer[self.buf_len..64].copy_from_slice(&data[..needed]);
                let chunk = self.buffer;
                Self::process_block(&mut self.state, &chunk);
                self.buf_len = 0;
                data = &data[needed..];
            } else {
                self.buffer[self.buf_len..self.buf_len + data.len()].copy_from_slice(data);
                self.buf_len += data.len();
                return;
            }
        }

        while data.len() >= 64 {
            let chunk: &[u8; 64] = data[..64].try_into().unwrap();
            Self::process_block(&mut self.state, chunk);
            data = &data[64..];
        }

        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    /// Finalize and compute the 32-byte SHA-256 digest.
    pub fn finalize(mut self) -> [u8; 32] {
        let bit_len = self
            .total_len
            .checked_mul(8)
            .expect("image bit length must not overflow u64");
        self.buffer[self.buf_len] = 0x80;
        self.buf_len += 1;

        if self.buf_len > 56 {
            self.buffer[self.buf_len..64].fill(0);
            let chunk = self.buffer;
            Self::process_block(&mut self.state, &chunk);
            self.buf_len = 0;
        }

        self.buffer[self.buf_len..56].fill(0);
        self.buffer[56..64].copy_from_slice(&bit_len.to_be_bytes());
        let chunk = self.buffer;
        Self::process_block(&mut self.state, &chunk);

        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    /// Compute 32-byte digest directly from a slice.
    pub fn digest(data: &[u8]) -> [u8; 32] {
        let mut hasher = Self::new();
        hasher.update(data);
        hasher.finalize()
    }

    /// Compute 64-character lowercase hex digest directly from a slice.
    pub fn digest_hex(data: &[u8]) -> String {
        let d = Self::digest(data);
        let mut s = String::with_capacity(64);
        for byte in d {
            use std::fmt::Write;
            let _ = write!(&mut s, "{:02x}", byte);
        }
        s
    }

    #[inline]
    fn process_block(state: &mut [u32; 8], block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];

        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..(i + 1) * 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pe_preflight::test_dll;

    #[test]
    fn sha256_nist_standard_vectors() {
        assert_eq!(
            Sha256::digest_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            Sha256::digest_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            Sha256::digest_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn empty_store_approves_nothing() {
        let store = ApprovalStore::new();
        assert!(store.is_empty());
        assert_eq!(store.len(), 0);

        let valid_bytes = test_dll(0x014c);
        let result = store.check_approval(&valid_bytes);
        assert!(matches!(result, Err(ApprovalStoreError::NotApproved)));
        assert!(!store.is_approved(&valid_bytes));

        let parsed_empty = ApprovalStore::parse("version = 1\n").unwrap();
        assert!(parsed_empty.is_empty());
        assert!(!parsed_empty.is_approved(&valid_bytes));
    }

    #[test]
    fn approved_image_is_accepted_and_matches_exact_identity() {
        let mut store = ApprovalStore::new();
        let bytes = test_dll(0x014c);
        let entry = store
            .approve_bytes("in_test.dll", &bytes)
            .expect("valid i386 DLL must be approved");

        assert_eq!(entry.name, "in_test.dll");
        assert_eq!(entry.size, bytes.len() as u64);
        assert_eq!(entry.machine, EXPECTED_PLUGIN_MACHINE);
        assert_eq!(entry.sha256, Sha256::digest_hex(&bytes));

        let approved = store
            .check_approval(&bytes)
            .expect("image must be approved");
        assert_eq!(approved, &entry);
        assert!(store.is_approved(&bytes));
        assert!(entry.matches_bytes(&bytes));
    }

    #[test]
    fn changed_image_is_refused_even_with_identical_size_and_headers() {
        let mut store = ApprovalStore::new();
        let mut bytes = test_dll(0x014c);
        store.approve_bytes("in_test.dll", &bytes).unwrap();

        // Mutate one byte in the PE image (e.g. at offset 0x300 in code section).
        // File length, PE header, machine, and exports remain identical.
        bytes[0x300] ^= 0x55;

        let result = store.check_approval(&bytes);
        assert!(matches!(result, Err(ApprovalStoreError::NotApproved)));
        assert!(!store.is_approved(&bytes));
    }

    #[test]
    fn malformed_store_is_refused_loudly() {
        // Missing version
        assert!(matches!(
            ApprovalStore::parse(""),
            Err(ApprovalStoreError::Malformed(_))
        ));

        // Unsupported version
        assert!(matches!(
            ApprovalStore::parse("version = 2\n"),
            Err(ApprovalStoreError::UnsupportedVersion(2))
        ));

        // Missing required field 'sha256'
        let missing_sha = "version = 1\n[[plugins]]\nname = \"p.dll\"\nsize = 100\nmachine = 332\n";
        assert!(matches!(
            ApprovalStore::parse(missing_sha),
            Err(ApprovalStoreError::Malformed(_))
        ));

        // Missing required field 'size'
        let missing_size = "version = 1\n[[plugins]]\nname = \"p.dll\"\nsha256 = \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\"\nmachine = 332\n";
        assert!(matches!(
            ApprovalStore::parse(missing_size),
            Err(ApprovalStoreError::Malformed(_))
        ));

        // Missing required field 'machine'
        let missing_machine = "version = 1\n[[plugins]]\nname = \"p.dll\"\nsha256 = \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\"\nsize = 100\n";
        assert!(matches!(
            ApprovalStore::parse(missing_machine),
            Err(ApprovalStoreError::Malformed(_))
        ));

        // Invalid sha256 hex length (63 chars)
        let bad_sha_len = "version = 1\n[[plugins]]\nname = \"p.dll\"\nsha256 = \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b85\"\nsize = 100\nmachine = 332\n";
        assert!(matches!(
            ApprovalStore::parse(bad_sha_len),
            Err(ApprovalStoreError::Malformed(_))
        ));

        // Invalid machine architecture (x86_64 = 0x8664 = 34404)
        let bad_machine = "version = 1\n[[plugins]]\nname = \"p.dll\"\nsha256 = \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\"\nsize = 100\nmachine = 0x8664\n";
        assert!(matches!(
            ApprovalStore::parse(bad_machine),
            Err(ApprovalStoreError::InvalidMachine {
                actual: 0x8664,
                expected: 0x014c
            })
        ));

        // Duplicate entries for the same sha256
        let duplicate = "version = 1\n\
            [[plugins]]\nname = \"p1.dll\"\nsha256 = \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\"\nsize = 100\nmachine = 332\n\
            [[plugins]]\nname = \"p2.dll\"\nsha256 = \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\"\nsize = 100\nmachine = 332\n";
        assert!(matches!(
            ApprovalStore::parse(duplicate),
            Err(ApprovalStoreError::Malformed(_))
        ));
    }

    #[test]
    fn approval_store_roundtrip_and_atomic_file_io() {
        let mut store = ApprovalStore::new();
        let bytes1 = test_dll(0x014c);
        let mut bytes2 = test_dll(0x014c);
        bytes2[0x350] ^= 0x11;

        store.approve_bytes("plugin1.dll", &bytes1).unwrap();
        store.approve_bytes("plugin2.dll", &bytes2).unwrap();
        assert_eq!(store.len(), 2);

        let toml = store.to_toml_string();
        let reloaded = ApprovalStore::parse(&toml).expect("store must round-trip");
        assert_eq!(store, reloaded);

        // Test atomic file write and read
        let temp_dir = std::env::temp_dir();
        let store_file = temp_dir.join(format!(
            "rustamp-test-approvals-{}.toml",
            std::process::id()
        ));
        store.save_to_file(&store_file).unwrap();

        let from_file = ApprovalStore::load_from_file(&store_file).unwrap();
        assert_eq!(store, from_file);

        let _ = std::fs::remove_file(&store_file);
    }
}
