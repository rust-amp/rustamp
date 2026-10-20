// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Single instance and command-line forwarding (task card F2).
//!
//! A second `rustamp.exe` must not open a second player. Instead it hands its
//! command line to the running player and exits: the running player loads the
//! file the second launch named, applies the flags that still make sense
//! (`--volume`), and reports the flags that were fixed at the first launch
//! (`--skin`, `--no-audio`, the `--seconds` test aid).
//!
//! # What the check looks for (S13 §2)
//!
//! The spec says the player registers a **window class**, that the class name
//! is what `/CLASS=` overrides, and that a second start looks for a window of
//! that class and hands its arguments over instead of opening a second
//! player. The class name is the identity of the running instance here: the
//! lock and the channel are named after it, so `/CLASS=` reaches the check.
//!
//! The default is the main window's class, `Winamp v1.x` — S09 §1 and S09 §8
//! item 1 publish that name as an ABI fact that plugins and external tools
//! depend on, and S13 §2 points at S09 for it. The class name is
//! **case-sensitive** (S13 §2), so the key does not fold its case.
//!
//! # How the "is one running?" question is answered
//!
//! The lock is a **named mutex** (`Local\Rustamp.SingleInstance.<key>`). Its
//! great property is that it cannot go stale: the kernel object is owned by the
//! process that created it and is released by the kernel when that process
//! dies, whatever kills it (a crash, `TerminateProcess`, a power loss). A dead
//! instance therefore never leaves a lock behind that would block the next
//! start — there is no lock file to clean up and no "held but dead" state to
//! distinguish. The card's stale-instance requirement is met by construction.
//!
//! The mutex only elects the first instance. The command line travels over a
//! **named pipe** (`\\.\pipe\rustamp-single-instance-<key>`) that the first
//! instance listens on for its whole life; a second instance connects, writes
//! one length-framed [`ForwardRequest`] and exits.
//!
//! S13 §2 says the hand-over is a window message but does not name it
//! (S13 §5.8), so the pipe is this implementation's own channel. What the
//! second instance does when the first is busy is open as well (S13 §5.5); it
//! hands over either way.
//!
//! # Instance key
//!
//! The key is derived from the window class and the resolved config path
//! ([`instance_key`]). The class is the half S13 §2 fixes; the configuration
//! path is task card F2's own choice and keeps two normal launches (same
//! configuration) in one instance, while tests that point `RUSTAMP_CONFIG` at
//! their own temp file never collide with each other or with a player the
//! developer is running.
//!
//! # Testing seam
//!
//! The decision points are pure and unit-tested without a second process:
//! [`startup_action`] (free vs. held), [`instance_key`], and the
//! [`ForwardRequest`] wire format ([`ForwardRequest::encode`] /
//! [`ForwardRequest::decode`]). A genuine two-process hand-off and a real
//! crash-kill cannot be exercised in-process and belong to the test VM.
//!
//! # Applying the hand-off
//!
//! The platform crate cannot reach a running [`rustamp_core`]-level `App`, so
//! the listener only queues decoded requests; the app drains them on its own
//! thread with [`take_pending`] and turns each into [`ForwardRequest::commands`].

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rustamp_core::command::Command;
use rustamp_core::playlist::MediaLocation;

pub use imp::{Claim, Instance, claim, forward};

/// Magic marking a hand-off frame (`Rustamp Single-instance Forward`, v1).
pub const MAGIC: [u8; 4] = *b"RSF1";

/// Wire protocol version of [`ForwardRequest`].
///
/// Version 2 added the "added without playing" field (S13 §1: `/BOOKMARK
/// <path>` adds the path as a bookmark rather than playing it), so a frame of
/// version 1 is refused instead of being misread.
pub const PROTOCOL: u16 = 2;

/// Largest accepted hand-off frame, in bytes.
///
/// A frame only carries one path, one volume and a handful of refusal strings,
/// so this is a generous ceiling that still bounds a malicious or corrupt
/// client's allocation.
pub const MAX_FRAME_BYTES: usize = 256 * 1024;

/// Whether the single-instance lock was free or already held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockState {
    /// No other instance held the lock.
    Free,
    /// Another instance already held the lock.
    Held,
}

/// What a launch should do, given the lock state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupAction {
    /// Nothing holds the lock: become the first instance and listen.
    BecomeFirstInstance,
    /// Another instance holds the lock: forward the command line and exit.
    ForwardAndExit,
}

/// Maps a [`LockState`] to the launch's [`StartupAction`].
///
/// Pure, so the free/held decision is unit-tested without any OS object.
pub fn startup_action(state: LockState) -> StartupAction {
    match state {
        LockState::Free => StartupAction::BecomeFirstInstance,
        LockState::Held => StartupAction::ForwardAndExit,
    }
}

/// The main window's class, and the class the single-instance check looks for
/// by default (S13 §2).
///
/// S09 §1 and S09 §8 item 1 publish the name as an ABI fact that plugins and
/// external tools depend on; S13 §2 names it as the default of the check.
/// `/CLASS=` overrides it (S13 §1).
pub const DEFAULT_CLASS: &str = "Winamp v1.x";

/// Derives the instance key from the window class and the resolved config
/// path.
///
/// The class is the half S13 §2 fixes: it is the class the single-instance
/// check looks for, it is **case-sensitive** ("The class name is a
/// case-sensitive string"), and `/CLASS=` changes it. The configuration path
/// is task card F2's own choice: the same path yields the same key (one
/// instance per configuration), a `None` path yields a fixed fallback, and
/// case is folded because Windows paths are case-insensitive.
///
/// The result is a 64-bit FNV-1a hash in hex, so the key is a safe
/// object-name component.
pub fn instance_key(class: &str, config_path: Option<&Path>) -> String {
    let mut source = String::with_capacity(class.len() + 32);
    source.push_str(class);
    source.push('\0');
    match config_path {
        Some(path) => source.push_str(&path.to_string_lossy().to_lowercase()),
        None => source.push_str("<no-config-path>"),
    }
    format!("{:016x}", fnv1a_64(source.as_bytes()))
}

/// 64-bit FNV-1a, used only to turn a path into a stable object-name fragment.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The named-mutex name for a key (`Local\` = the logon session).
fn mutex_name(key: &str) -> String {
    format!("Local\\Rustamp.SingleInstance.{key}")
}

/// The named-pipe name for a key.
fn pipe_name(key: &str) -> String {
    format!(r"\\.\pipe\rustamp-single-instance-{key}")
}

/// The command line a second launch hands to the running instance.
///
/// Only the parts of the command line that can still take effect are carried:
/// a file to play, a path to add without playing it, and an initial volume.
/// Flags that were fixed when the first instance started (output mode, skin,
/// the `--seconds` test aid) are listed in [`ForwardRequest::refusals`] and
/// reported to the user by the sender rather than silently dropped.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ForwardRequest {
    /// The file to play, if any.
    pub file: Option<PathBuf>,
    /// A path to add to the playlist **without** playing it, if any.
    ///
    /// `/BOOKMARK <path>` (S13 §1) and `--enqueue` both land here: the
    /// reference adds a bookmark, and Rustamp has no bookmark store yet
    /// (RFC 0005 O12), so the path is added to the playlist instead — added
    /// rather than played, which is what S13 §1 settles.
    pub added: Option<PathBuf>,
    /// The volume to apply (`0..=255`), if any.
    pub volume: Option<u8>,
    /// Human-readable notes for flags the running instance cannot apply.
    pub refusals: Vec<String>,
}

/// Why a hand-off frame could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// The frame exceeds [`MAX_FRAME_BYTES`].
    #[error("hand-off frame is too large ({0} bytes)")]
    TooLarge(usize),
    /// The magic marker is missing or wrong.
    #[error("hand-off frame has a bad magic marker")]
    BadMagic,
    /// The protocol version is not [`PROTOCOL`].
    #[error("unsupported hand-off protocol version {0}")]
    Protocol(u16),
    /// The frame ended before a field was complete.
    #[error("hand-off frame is truncated")]
    Truncated,
    /// A path or refusal was not valid UTF-8.
    #[error("hand-off frame contains invalid UTF-8")]
    Utf8,
}

/// A forward cursor over a frame, with checked reads.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(len).ok_or(DecodeError::Truncated)?;
        if end > self.bytes.len() {
            return Err(DecodeError::Truncated);
        }
        let out = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
}

/// Appends a length-prefixed UTF-8 path to a frame.
fn push_path(out: &mut Vec<u8>, path: &Path) {
    let text = path.to_string_lossy();
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
}

impl ForwardRequest {
    /// Encodes the request into the hand-off wire format.
    ///
    /// Layout (all integers little-endian): `MAGIC` (4), protocol (u16), a
    /// file present flag (u8) and, if present, its length (u32) and UTF-8
    /// bytes, an added present flag (u8) and, if present, its length (u32)
    /// and UTF-8 bytes, a volume present flag (u8) and, if present, the byte,
    /// a refusal count (u16) and that many length-prefixed UTF-8 strings.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&PROTOCOL.to_le_bytes());
        match &self.file {
            Some(path) => {
                out.push(1);
                push_path(&mut out, path);
            }
            None => out.push(0),
        }
        match &self.added {
            Some(path) => {
                out.push(1);
                push_path(&mut out, path);
            }
            None => out.push(0),
        }
        match self.volume {
            Some(volume) => {
                out.push(1);
                out.push(volume);
            }
            None => out.push(0),
        }
        out.extend_from_slice(&(self.refusals.len() as u16).to_le_bytes());
        for refusal in &self.refusals {
            out.extend_from_slice(&(refusal.len() as u32).to_le_bytes());
            out.extend_from_slice(refusal.as_bytes());
        }
        out
    }

    /// Decodes a hand-off frame produced by [`ForwardRequest::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(DecodeError::TooLarge(bytes.len()));
        }
        let mut reader = Reader { bytes, pos: 0 };
        if reader.take(4)? != MAGIC {
            return Err(DecodeError::BadMagic);
        }
        let protocol = reader.u16()?;
        if protocol != PROTOCOL {
            return Err(DecodeError::Protocol(protocol));
        }
        let file = if reader.u8()? == 1 {
            let len = reader.u32()? as usize;
            let text = std::str::from_utf8(reader.take(len)?).map_err(|_| DecodeError::Utf8)?;
            Some(PathBuf::from(text))
        } else {
            None
        };
        let added = if reader.u8()? == 1 {
            let len = reader.u32()? as usize;
            let text = std::str::from_utf8(reader.take(len)?).map_err(|_| DecodeError::Utf8)?;
            Some(PathBuf::from(text))
        } else {
            None
        };
        let volume = if reader.u8()? == 1 {
            Some(reader.u8()?)
        } else {
            None
        };
        let count = reader.u16()? as usize;
        let mut refusals = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            let len = reader.u32()? as usize;
            let text = std::str::from_utf8(reader.take(len)?).map_err(|_| DecodeError::Utf8)?;
            refusals.push(text.to_owned());
        }
        Ok(ForwardRequest {
            file,
            added,
            volume,
            refusals,
        })
    }

    /// The commands the running first instance should apply, in order.
    ///
    /// The volume lands before the file, so a forwarded `--volume` is in force
    /// when playback starts. An added path is **added** and not played (S13
    /// §1: `/BOOKMARK <path>` adds the path as a bookmark rather than playing
    /// it).
    pub fn commands(&self) -> Vec<Command> {
        let mut commands = Vec::new();
        if let Some(volume) = self.volume {
            commands.push(Command::SetVolume(volume));
        }
        if let Some(file) = &self.file {
            commands.push(Command::AddItems {
                items: vec![MediaLocation::File(file.clone())],
                at: None,
                play: true,
            });
        }
        if let Some(added) = &self.added {
            commands.push(Command::AddItems {
                items: vec![MediaLocation::File(added.clone())],
                at: None,
                play: false,
            });
        }
        commands
    }
}

/// Whether a hand-off reached the running instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandOff {
    /// The frame was written to the running instance's pipe.
    Delivered,
    /// No instance answered within the retry window.
    Unreachable,
}

/// Requests the listener has decoded and that wait to be applied.
///
/// Process-global: the listener runs on its own thread and the app thread
/// drains it with [`take_pending`].
static PENDING: Mutex<Vec<ForwardRequest>> = Mutex::new(Vec::new());

/// Queues a decoded request for the app thread.
fn push_pending(request: ForwardRequest) {
    let mut guard = PENDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.push(request);
}

/// Drains every hand-off request decoded so far.
///
/// Called on the app's own thread (the window loop drains it every frame). The
/// lock recovers from poisoning, so a panic while queueing never wedges the
/// player.
pub fn take_pending() -> Vec<ForwardRequest> {
    let mut guard = PENDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::mem::take(&mut *guard)
}

#[cfg(windows)]
mod imp {
    //! Win32 implementation: a named mutex for the lock and a named pipe for
    //! the channel.

    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::windows::io::FromRawHandle;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, ERROR_PIPE_CONNECTED, GetLastError, HANDLE,
    };
    use windows::Win32::Storage::FileSystem::PIPE_ACCESS_INBOUND;
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::PCWSTR;

    use super::{
        ForwardRequest, HandOff, LockState, MAX_FRAME_BYTES, StartupAction, mutex_name, pipe_name,
        push_pending, startup_action,
    };

    /// How long a forward waits for the running instance's pipe to answer.
    const FORWARD_TIMEOUT: Duration = Duration::from_millis(1000);

    /// How long a forward waits between connection attempts.
    const FORWARD_RETRY: Duration = Duration::from_millis(50);

    /// How long [`Instance::drop`] waits for the listener thread to stop.
    const STOP_TIMEOUT: Duration = Duration::from_millis(500);

    /// The outcome of claiming the single-instance lock.
    pub enum Claim {
        /// This process became the first instance; keep the guard alive for
        /// the whole run.
        First(Instance),
        /// Another instance owns the lock.
        AlreadyRunning,
    }

    impl std::fmt::Debug for Claim {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Claim::First(_) => f.write_str("Claim::First(..)"),
                Claim::AlreadyRunning => f.write_str("Claim::AlreadyRunning"),
            }
        }
    }

    /// The first instance's guard: the mutex handle and the listener thread.
    ///
    /// Dropping it stops the listener, joins it and releases the mutex, so the
    /// next launch becomes the first instance.
    pub struct Instance {
        /// The held mutex; invalid only in the degraded "no mutex" case.
        mutex: HANDLE,
        /// Set to ask the listener thread to stop.
        stop: Arc<AtomicBool>,
        /// The listener thread (`None` if the thread could not be spawned).
        thread: Option<std::thread::JoinHandle<()>>,
        /// The instance key, so the drop can wake the listener.
        key: String,
    }

    impl std::fmt::Debug for Instance {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Instance")
                .field("key", &self.key)
                .finish_non_exhaustive()
        }
    }

    /// Claims the single-instance lock for `key`.
    ///
    /// Returns [`Claim::First`] with the guard when the lock was free,
    /// [`Claim::AlreadyRunning`] when another instance holds it. If the mutex
    /// cannot be created at all (a rare access error) this degrades to
    /// [`Claim::First`] with no mutex: the player still starts, it just cannot
    /// participate in the protocol.
    pub fn claim(key: &str) -> Claim {
        let name = mutex_name(key);
        let wide = to_wide(&name);
        // SAFETY: `wide` is a NUL-terminated UTF-16 string that outlives the
        // call; no security attributes are wanted (None), and the initial
        // owner is `false` so we hold the object without taking the mutex's
        // own lock. `CreateMutexW` returns an owned handle on success.
        match unsafe { CreateMutexW(None, false, PCWSTR(wide.as_ptr())) } {
            Ok(handle) => {
                // SAFETY: reading the last-error value is always sound; the
                // value is meaningful here because `CreateMutexW` set it.
                let state = if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                    LockState::Held
                } else {
                    LockState::Free
                };
                match startup_action(state) {
                    StartupAction::BecomeFirstInstance => {
                        Claim::First(Instance::start(handle, key))
                    }
                    StartupAction::ForwardAndExit => {
                        // Not ours: close the handle we just opened so we hold
                        // nothing, then let the caller forward and exit.
                        // SAFETY: `handle` is a valid owned handle from
                        // `CreateMutexW` and is closed exactly once.
                        let _ = unsafe { CloseHandle(handle) };
                        Claim::AlreadyRunning
                    }
                }
            }
            Err(_) => Claim::First(Instance::start(HANDLE(std::ptr::null_mut()), key)),
        }
    }

    impl Instance {
        /// Holds the mutex and starts the listener thread.
        fn start(mutex: HANDLE, key: &str) -> Self {
            let stop = Arc::new(AtomicBool::new(false));
            let listener_stop = Arc::clone(&stop);
            let pipe = pipe_name(key);
            // A failed spawn is not fatal: forwarding simply never reaches
            // this process (the caller treats it as Unreachable).
            let thread = std::thread::Builder::new()
                .name("rustamp-single-instance".to_owned())
                .spawn(move || listen(&pipe, &listener_stop))
                .ok();
            Self {
                mutex,
                stop,
                thread,
                key: key.to_owned(),
            }
        }
    }

    impl Drop for Instance {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            // Wake a listener blocked in `ConnectNamedPipe` by connecting to
            // its pipe ourselves; the connection is empty, so the listener
            // reads EOF, sees the stop flag and exits. Retry until the thread
            // finishes or the deadline passes.
            let pipe = pipe_name(&self.key);
            let deadline = Instant::now() + STOP_TIMEOUT;
            while !self
                .thread
                .as_ref()
                .is_some_and(|thread| thread.is_finished())
            {
                if Instant::now() >= deadline {
                    break;
                }
                let _ = std::fs::OpenOptions::new().write(true).open(&pipe);
                std::thread::sleep(Duration::from_millis(10));
            }
            if let Some(thread) = self.thread.take() {
                // Only join a thread that actually stopped; otherwise drop the
                // handle (detach) rather than block the shutdown forever.
                if thread.is_finished() {
                    let _ = thread.join();
                }
            }
            if !self.mutex.is_invalid() {
                // SAFETY: `self.mutex` is a valid handle from `CreateMutexW`
                // and is closed exactly once, here.
                let _ = unsafe { CloseHandle(self.mutex) };
            }
        }
    }

    /// Sends `request` to the instance that owns `key`.
    pub fn forward(key: &str, request: &ForwardRequest) -> HandOff {
        let pipe = pipe_name(key);
        let frame = request.encode();
        let mut body = Vec::with_capacity(4 + frame.len());
        body.extend_from_slice(&(frame.len() as u32).to_le_bytes());
        body.extend_from_slice(&frame);

        let deadline = Instant::now() + FORWARD_TIMEOUT;
        loop {
            // A failed open means no listener yet (the first instance is still
            // starting) or the pipe is busy: retry until the deadline.
            if let Ok(mut file) = std::fs::OpenOptions::new().write(true).open(&pipe)
                && file.write_all(&body).is_ok()
                && file.flush().is_ok()
            {
                return HandOff::Delivered;
            }
            if Instant::now() >= deadline {
                return HandOff::Unreachable;
            }
            std::thread::sleep(FORWARD_RETRY);
        }
    }

    /// The listener loop: one pipe instance per connection.
    fn listen(pipe: &str, stop: &AtomicBool) {
        while !stop.load(Ordering::SeqCst) {
            let Some(handle) = create_pipe(pipe) else {
                break;
            };
            if !connect(handle) {
                // SAFETY: `handle` is a valid owned pipe handle from
                // `create_pipe`; it is closed exactly once.
                let _ = unsafe { CloseHandle(handle) };
                continue;
            }
            if stop.load(Ordering::SeqCst) {
                // Our own shutdown connection: close and leave.
                // SAFETY: `handle` is a valid owned pipe handle; closed once.
                let _ = unsafe { DisconnectNamedPipe(handle) };
                let _ = unsafe { CloseHandle(handle) };
                break;
            }
            // Read exactly one length-framed request. `File` takes ownership
            // of the handle and closes (and so disconnects) it on drop.
            // SAFETY: `handle` is a valid owned pipe handle; ownership is
            // transferred to the `File` exactly once.
            let mut file = unsafe { File::from_raw_handle(handle.0) };
            if let Some(request) = read_frame(&mut file) {
                push_pending(request);
            }
            drop(file);
        }
        // The pipe name is freed when the last instance handle closes.
    }

    /// Creates one listening pipe instance, or `None` on failure.
    fn create_pipe(pipe: &str) -> Option<HANDLE> {
        let wide = to_wide(pipe);
        let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT;
        // SAFETY: `wide` is a NUL-terminated UTF-16 name that outlives the
        // call; the buffer sizes are constants; no security attributes are
        // wanted (None). The returned handle is owned by us.
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide.as_ptr()),
                PIPE_ACCESS_INBOUND,
                mode,
                1,
                MAX_FRAME_BYTES as u32,
                MAX_FRAME_BYTES as u32,
                0,
                None,
            )
        };
        (!handle.is_invalid()).then_some(handle)
    }

    /// Blocks until a client connects; `false` on a real failure.
    fn connect(handle: HANDLE) -> bool {
        // SAFETY: `handle` is a valid pipe handle; `None` requests a blocking
        // call on the calling thread (no OVERLAPPED).
        match unsafe { ConnectNamedPipe(handle, None) } {
            Ok(()) => true,
            Err(error) => {
                // A client that connected between create and connect makes
                // ConnectNamedPipe report ERROR_PIPE_CONNECTED, which the
                // windows crate surfaces as an error; that is success here.
                (error.code().0 as u32) & 0xFFFF == ERROR_PIPE_CONNECTED.0
            }
        }
    }

    /// Reads one `[u32 length][frame]` from `file`; `None` on any short or
    /// malformed read (the connection is simply dropped).
    fn read_frame(file: &mut File) -> Option<ForwardRequest> {
        let mut length_bytes = [0u8; 4];
        file.read_exact(&mut length_bytes).ok()?;
        let length = u32::from_le_bytes(length_bytes) as usize;
        if length == 0 || length > MAX_FRAME_BYTES {
            return None;
        }
        let mut frame = vec![0u8; length];
        file.read_exact(&mut frame).ok()?;
        ForwardRequest::decode(&frame).ok()
    }

    /// UTF-16, NUL-terminated, for the `*W` Win32 calls.
    fn to_wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(not(windows))]
mod imp {
    //! Non-Windows stub: single instance is a Win32 feature for now.

    use super::{ForwardRequest, HandOff};

    /// The outcome of claiming the single-instance lock.
    #[derive(Debug)]
    pub enum Claim {
        /// This process is the first instance.
        First(Instance),
        /// Another instance owns the lock (never on a non-Windows target).
        AlreadyRunning,
    }

    /// The first instance's guard (a no-op off Windows).
    #[derive(Debug)]
    pub struct Instance {
        _priv: (),
    }

    /// Off Windows every launch is the first instance.
    pub fn claim(_key: &str) -> Claim {
        Claim::First(Instance { _priv: () })
    }

    /// Off Windows forwarding is unimplemented.
    pub fn forward(_key: &str, _request: &ForwardRequest) -> HandOff {
        HandOff::Unreachable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_free_lock_becomes_the_first_instance() {
        assert_eq!(
            startup_action(LockState::Free),
            StartupAction::BecomeFirstInstance
        );
    }

    #[test]
    fn a_held_lock_forwards_and_exits() {
        assert_eq!(
            startup_action(LockState::Held),
            StartupAction::ForwardAndExit
        );
    }

    #[test]
    fn the_instance_key_is_stable_and_path_derived() {
        let a = instance_key(
            DEFAULT_CLASS,
            Some(Path::new("C:/Users/x/Rustamp/rustamp.toml")),
        );
        let same = instance_key(
            DEFAULT_CLASS,
            Some(Path::new("C:/Users/x/Rustamp/rustamp.toml")),
        );
        assert_eq!(a, same, "the same path always yields the same key");
        assert_eq!(a.len(), 16, "the key is 64 bits of hex");

        let other = instance_key(DEFAULT_CLASS, Some(Path::new("C:/tmp/other.toml")));
        assert_ne!(a, other, "different configs must not share an instance");

        let none = instance_key(DEFAULT_CLASS, None);
        assert_ne!(none, a);
        assert_ne!(none, other);
    }

    #[test]
    fn the_instance_key_folds_case() {
        assert_eq!(
            instance_key(DEFAULT_CLASS, Some(Path::new("C:/Users/X/Rustamp.toml"))),
            instance_key(DEFAULT_CLASS, Some(Path::new("c:/users/x/rustamp.toml"))),
            "Windows paths are case-insensitive, so the key must be too"
        );
    }

    #[test]
    fn the_class_reaches_the_instance_check() {
        // S13 §2: the class name is what the single-instance check looks for,
        // so `/CLASS=` must change the key.
        let config = Some(Path::new("C:/Users/x/Rustamp/rustamp.toml"));
        let default = instance_key(DEFAULT_CLASS, config);
        let other = instance_key("Rustamp second", config);
        assert_ne!(
            default, other,
            "a different class must be a different instance"
        );
    }

    #[test]
    fn the_class_name_is_case_sensitive() {
        // S13 §2: "The class name is a case-sensitive string."
        let config = Some(Path::new("C:/Users/x/Rustamp/rustamp.toml"));
        assert_ne!(
            instance_key("Winamp v1.x", config),
            instance_key("winamp v1.x", config),
            "the class name must not be case-folded"
        );
    }

    #[test]
    fn a_full_request_round_trips() {
        let request = ForwardRequest {
            file: Some(PathBuf::from(r"C:\music\a b.mp3")),
            added: Some(PathBuf::from(r"C:\music\bookmark.mp3")),
            volume: Some(200),
            refusals: vec![
                "--skin is ignored by the running instance".to_owned(),
                "another note".to_owned(),
            ],
        };
        assert_eq!(ForwardRequest::decode(&request.encode()), Ok(request));
    }

    #[test]
    fn an_empty_request_round_trips() {
        let request = ForwardRequest::default();
        assert_eq!(ForwardRequest::decode(&request.encode()), Ok(request));
    }

    #[test]
    fn a_request_with_only_a_file_round_trips() {
        let request = ForwardRequest {
            file: Some(PathBuf::from("song.ogg")),
            ..ForwardRequest::default()
        };
        assert_eq!(ForwardRequest::decode(&request.encode()), Ok(request));
    }

    #[test]
    fn a_bad_magic_is_rejected() {
        let mut frame = ForwardRequest::default().encode();
        frame[0] = b'X';
        assert_eq!(ForwardRequest::decode(&frame), Err(DecodeError::BadMagic));
    }

    #[test]
    fn a_wrong_protocol_is_reported() {
        let mut frame = ForwardRequest::default().encode();
        frame[4] = 99;
        assert_eq!(
            ForwardRequest::decode(&frame),
            Err(DecodeError::Protocol(99))
        );
    }

    #[test]
    fn a_truncated_frame_is_rejected() {
        let frame = ForwardRequest::default().encode();
        for cut in 0..frame.len() {
            let result = ForwardRequest::decode(&frame[..cut]);
            assert!(result.is_err(), "a frame cut at {cut} must not decode");
        }
    }

    #[test]
    fn an_oversized_frame_is_rejected() {
        let frame = vec![0u8; MAX_FRAME_BYTES + 1];
        assert_eq!(
            ForwardRequest::decode(&frame),
            Err(DecodeError::TooLarge(MAX_FRAME_BYTES + 1))
        );
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let mut frame = Vec::new();
        frame.extend_from_slice(&MAGIC);
        frame.extend_from_slice(&PROTOCOL.to_le_bytes());
        frame.push(1); // a file is present
        frame.extend_from_slice(&2u32.to_le_bytes());
        frame.extend_from_slice(&[0xff, 0xfe]); // not UTF-8
        frame.push(0); // nothing is added
        frame.push(0); // no volume
        frame.extend_from_slice(&0u16.to_le_bytes()); // no refusals
        assert_eq!(ForwardRequest::decode(&frame), Err(DecodeError::Utf8));
    }

    #[test]
    fn an_added_path_is_added_and_not_played() {
        // S13 §1: `/BOOKMARK <path>` adds the path as a bookmark rather than
        // playing it.
        let request = ForwardRequest {
            added: Some(PathBuf::from("bookmark.mp3")),
            ..ForwardRequest::default()
        };
        assert_eq!(
            request.commands(),
            vec![Command::AddItems {
                items: vec![MediaLocation::File(PathBuf::from("bookmark.mp3"))],
                at: None,
                play: false,
            }]
        );
    }

    #[test]
    fn commands_put_the_volume_before_the_file() {
        let request = ForwardRequest {
            file: Some(PathBuf::from("a.mp3")),
            volume: Some(42),
            refusals: vec!["nope".to_owned()],
            ..ForwardRequest::default()
        };
        assert_eq!(
            request.commands(),
            vec![
                Command::SetVolume(42),
                Command::AddItems {
                    items: vec![MediaLocation::File(PathBuf::from("a.mp3"))],
                    at: None,
                    play: true,
                },
            ]
        );
    }

    #[test]
    fn commands_are_empty_without_a_file_or_volume() {
        let request = ForwardRequest {
            refusals: vec!["--skin is ignored".to_owned()],
            ..ForwardRequest::default()
        };
        assert!(request.commands().is_empty());
    }

    #[test]
    fn a_volume_only_request_sets_the_volume() {
        let request = ForwardRequest {
            volume: Some(7),
            ..ForwardRequest::default()
        };
        assert_eq!(request.commands(), vec![Command::SetVolume(7)]);
    }

    /// A key that no other test (or process) can share.
    #[cfg(windows)]
    fn unique_key(label: &str) -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("test-{label}-{}-{n}", std::process::id())
    }

    #[cfg(windows)]
    #[test]
    fn a_second_claim_reports_held_and_drop_releases_the_lock() {
        let key = unique_key("claim");
        let first = claim(&key);
        assert!(matches!(first, Claim::First(_)), "the first claim must win");
        assert!(
            matches!(claim(&key), Claim::AlreadyRunning),
            "a second claim while the first lives must report Held"
        );
        drop(first);
        let third = claim(&key);
        assert!(
            matches!(third, Claim::First(_)),
            "dropping the guard must release the lock (no stale state)"
        );
        drop(third);
    }

    #[cfg(windows)]
    #[test]
    fn a_forwarded_request_reaches_the_pending_queue() {
        let key = unique_key("forward");
        let Claim::First(instance) = claim(&key) else {
            panic!("the unique key must claim as the first instance");
        };
        // Drain anything a parallel test might have queued (keys are unique,
        // so this is only defensive).
        let _ = take_pending();

        let request = ForwardRequest {
            file: Some(PathBuf::from("forwarded.mp3")),
            volume: Some(128),
            refusals: vec!["--skin is ignored".to_owned()],
            ..ForwardRequest::default()
        };
        assert_eq!(forward(&key, &request), HandOff::Delivered);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let pending = take_pending();
            if !pending.is_empty() {
                assert_eq!(pending, vec![request]);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the listener did not queue the forwarded request in time"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        drop(instance);
    }

    #[cfg(windows)]
    #[test]
    fn forwarding_to_a_key_nobody_owns_is_unreachable() {
        let key = unique_key("nobody");
        let request = ForwardRequest {
            file: Some(PathBuf::from("x.mp3")),
            ..ForwardRequest::default()
        };
        assert_eq!(forward(&key, &request), HandOff::Unreachable);
    }
}
