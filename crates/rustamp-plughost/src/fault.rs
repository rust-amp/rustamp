//! Plugin-fault isolation inside the plugin host (PLAN M3: "Crash isolation:
//! restart the host on plugin crash").
//!
//! # What a plugin fault is
//!
//! A classic plugin runs **in this process**: the host calls its `init`,
//! `config`, `quit`, `render` and `process` callbacks itself (S09 §3.5, §5.3,
//! §6.3). A plugin that dereferences a null pointer or writes through a bad
//! pointer therefore raises a Windows exception *inside the host process*.
//! The player is a different process (RFC 0006 §8), so **the player keeps
//! playing**; what dies is the host — and with it every plugin the host had
//! loaded.
//!
//! # What this module does
//!
//! 1. **The host survives long enough to report.** A process-wide
//!    [`SetUnhandledExceptionFilter`] catches the exception, writes one
//!    diagnostic line to stderr and ends the process with
//!    [`PLUGIN_FAULT_EXIT_CODE`] — a code the supervisor recognises (see
//!    [`crate::restart`]). Without the filter the process would die with the
//!    raw NTSTATUS, which says *that* it crashed but never *what* crashed.
//! 2. **The report names the plugin.** Every plugin call the host makes runs
//!    inside [`plugin_call_scope`], which records the plugin and the call in a
//!    thread-local. The filter reads that record and names the plugin in the
//!    line it writes — the only thing a crash report can still say reliably.
//! 3. **The filter stays cheap and allocation-free.** It runs on a possibly
//!    damaged stack: it formats into a fixed buffer, writes it with
//!    `WriteFile` and ends the process with `TerminateProcess`. It never
//!    allocates, never takes a lock and never unwinds.
//!
//! # What this module does not do
//!
//! * It does **not** resume the host after a fault. Continuing past an access
//!   violation is undefined behaviour, so the host ends and the supervisor
//!   starts a fresh one (PLAN M3).
//! * It does **not** keep the plugin alive. The plugin's DLL dies with the
//!   process; the fresh host starts empty, and the app loads nothing by itself
//!   (it has no plugin registry yet — see `docs/tasks/P3-dll-acceptance.md`).
//! * It does **not** claim the fault was the plugin's fault. An exception
//!   outside a plugin call is reported as a host fault, and the line says so.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

use rustamp_plugin_abi::PluginKind;

/// The exit code the host uses when a plugin fault ended it.
///
/// The supervisor reads the host's exit code and classifies it (see
/// [`crate::restart::classify_exit`]): this code means "the host caught a
/// plugin fault itself and said so on stderr". It is deliberately outside the
/// Windows exception range (`0xC0000000..=0xC000FFFF`), so a host that died
/// from an exception the filter never saw is never mistaken for one it
/// reported.
pub const PLUGIN_FAULT_EXIT_CODE: u32 = 0xC0DE;

/// The identity of one loaded plugin, kept where a fault report can read it.
///
/// The loader holds it in a `Box`, so its address does not move while the
/// plugin is loaded: the crash handler can read it from the thread-local
/// record even while the process is going down.
pub struct PluginIdentity {
    /// The plugin's source path, exactly as the loader resolved it.
    path: PathBuf,
    /// The plugin's kind.
    kind: PluginKind,
}

impl PluginIdentity {
    /// The identity of the plugin in `path`.
    pub fn new(path: &Path, kind: PluginKind) -> Box<Self> {
        Box::new(Self {
            path: path.to_path_buf(),
            kind,
        })
    }

    /// The plugin's source path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The plugin's kind.
    pub fn kind(&self) -> PluginKind {
        self.kind
    }
}

/// The plugin call that is running on this thread, if any.
#[derive(Clone, Copy)]
struct FaultSite {
    /// The plugin the call belongs to. Valid for as long as the plugin is
    /// loaded, and the scope below only exists while the plugin's own call
    /// runs.
    identity: NonNull<PluginIdentity>,
    /// Which call it is (`"init"`, `"config"`, `"render"`, …).
    call: &'static str,
}

thread_local! {
    /// The plugin call running on this thread.
    ///
    /// A `Cell` with a constant initializer: no lazy allocation and no
    /// destructor, so the crash handler can read it without touching the
    /// allocator.
    static CURRENT_SITE: Cell<Option<FaultSite>> = const { Cell::new(None) };
}

/// Marks a plugin call as running on this thread.
///
/// The scope restores the previous record when it is dropped, so nested calls
/// (a plugin callback that re-enters the host) name the innermost plugin.
///
/// The scopes of one thread must therefore be dropped in the **reverse order
/// of their creation** — the way the call stack unwinds. A scope that is
/// dropped out of order (one that was *replaced* rather than unwound)
/// restores a record that names a plug-in no longer in a call, and the next
/// fault would blame it.
pub struct FaultScope {
    previous: Option<FaultSite>,
}

/// Record that a call into `identity`'s plugin is running on this thread.
///
/// Every call the host makes into plugin code belongs in one of these: it is
/// what lets the crash report name the plugin that faulted.
pub fn plugin_call_scope(identity: &PluginIdentity, call: &'static str) -> FaultScope {
    let site = FaultSite {
        identity: NonNull::from(identity),
        call,
    };
    let previous = CURRENT_SITE.with(|cell| cell.replace(Some(site)));
    FaultScope { previous }
}

impl Drop for FaultScope {
    fn drop(&mut self) {
        let previous = self.previous;
        // `try_with` never panics: the thread-local is gone only while the
        // thread is being destroyed, and a missing restore is better than a
        // panic in a drop.
        let _ = CURRENT_SITE.try_with(|cell| cell.set(previous));
    }
}

/// The plugin call running on this thread, if any.
fn current_site() -> Option<FaultSite> {
    // `try_with` never panics: the thread-local is gone only while the thread
    // is being destroyed, and a report without a name is better than a fault
    // inside the fault handler.
    CURRENT_SITE.try_with(|cell| cell.get()).unwrap_or(None)
}

/// Capacity of the diagnostic line the crash handler writes.
const FAULT_LINE_CAPACITY: usize = 256;

/// A fixed-capacity byte buffer the crash handler fills without allocating.
struct LineWriter<'a> {
    buffer: &'a mut [u8],
    len: usize,
}

impl<'a> LineWriter<'a> {
    fn new(buffer: &'a mut [u8]) -> Self {
        Self { buffer, len: 0 }
    }

    /// Append `text`, stopping at the buffer's end.
    fn push_str(&mut self, text: &str) {
        for byte in text.as_bytes() {
            if self.len == self.buffer.len() {
                return;
            }
            self.buffer[self.len] = *byte;
            self.len += 1;
        }
    }

    /// Append `text`'s raw bytes, stopping at the buffer's end.
    fn push_bytes(&mut self, text: &[u8]) {
        for byte in text {
            if self.len == self.buffer.len() {
                return;
            }
            self.buffer[self.len] = *byte;
            self.len += 1;
        }
    }

    /// Append `value` as eight hexadecimal digits behind `0x`.
    fn push_hex(&mut self, value: u32) {
        const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
        self.push_str("0x");
        for shift in (0..8).rev() {
            let digit = ((value >> (shift * 4)) & 0xF) as usize;
            if self.len == self.buffer.len() {
                return;
            }
            self.buffer[self.len] = DIGITS[digit];
            self.len += 1;
        }
    }
}

/// The plugin kind as the report spells it.
fn kind_label(kind: PluginKind) -> &'static str {
    match kind {
        PluginKind::Input => "input",
        PluginKind::Output => "output",
        PluginKind::Visualization => "visualization",
        PluginKind::Dsp => "DSP",
        PluginKind::General => "general-purpose",
    }
}

/// The name of a well-known Windows exception status, if it has one.
fn exception_name(code: u32) -> Option<&'static str> {
    Some(match code {
        0xC000_0005 => "access violation",
        0xC000_0006 => "in-page error",
        0xC000_001D => "illegal instruction",
        0xC000_0094 => "integer divide by zero",
        0xC000_00FD => "stack overflow",
        0xC000_0135 => "module not found",
        0xC000_0139 => "entry point not found",
        0xC000_0374 => "heap corruption",
        0xC000_0409 => "stack buffer overrun",
        _ => return None,
    })
}

/// Write the diagnostic line for a fault into `buffer`; returns its length.
///
/// The line names the plugin and the call when the fault happened inside one:
///
/// ```text
/// rustamp-plughost32: plugin fault in C:\plugins\bad.dll (input plugin, call init): Windows exception 0xC0000005 (access violation)
/// ```
///
/// and says plainly that the host itself faulted when it did not happen in a
/// plugin call:
///
/// ```text
/// rustamp-plughost32: unhandled exception in the plugin host: Windows exception 0xC0000005 (access violation)
/// ```
fn write_fault_line(buffer: &mut [u8], code: u32, site: Option<FaultSite>) -> usize {
    let mut line = LineWriter::new(buffer);
    line.push_str("rustamp-plughost32: ");
    match site {
        Some(site) => {
            // SAFETY: the identity is alive for as long as the plugin is
            // loaded, and the scope that points at it only exists while that
            // plugin's own call runs.
            let identity = unsafe { site.identity.as_ref() };
            line.push_str("plugin fault in ");
            line.push_bytes(identity.path().as_os_str().as_encoded_bytes());
            line.push_str(" (");
            line.push_str(kind_label(identity.kind()));
            line.push_str(" plugin, call ");
            line.push_str(site.call);
            line.push_str(")");
        }
        None => line.push_str("unhandled exception in the plugin host"),
    }
    line.push_str(": Windows exception ");
    line.push_hex(code);
    if let Some(name) = exception_name(code) {
        line.push_str(" (");
        line.push_str(name);
        line.push_str(")");
    }
    line.push_str("\n");
    line.len
}

#[cfg(windows)]
mod windows_filter {
    use windows::Win32::Storage::FileSystem::WriteFile;
    use windows::Win32::System::Console::{GetStdHandle, STD_ERROR_HANDLE};
    use windows::Win32::System::Diagnostics::Debug::{
        EXCEPTION_EXECUTE_HANDLER, EXCEPTION_POINTERS, LPTOP_LEVEL_EXCEPTION_FILTER,
        SetUnhandledExceptionFilter,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};

    use super::{FAULT_LINE_CAPACITY, PLUGIN_FAULT_EXIT_CODE, current_site, write_fault_line};

    /// The process-wide filter: report the fault, then end the process.
    ///
    /// Returning `EXCEPTION_EXECUTE_HANDLER` (instead of continuing the
    /// search) would let the system end the process with the original
    /// exception status; that is the fallback for the case where ending the
    /// process ourselves fails.
    unsafe extern "system" fn fault_filter(exception_info: *const EXCEPTION_POINTERS) -> i32 {
        let code = exception_code(exception_info);
        let mut buffer = [0u8; FAULT_LINE_CAPACITY];
        let len = write_fault_line(&mut buffer, code, current_site());
        write_stderr(&buffer[..len]);
        // The supervisor reads this exit code and knows that the host caught a
        // plugin fault (see `crate::restart`).
        let _ = unsafe { TerminateProcess(GetCurrentProcess(), PLUGIN_FAULT_EXIT_CODE) };
        EXCEPTION_EXECUTE_HANDLER
    }

    /// The exception status of the exception being reported.
    fn exception_code(exception_info: *const EXCEPTION_POINTERS) -> u32 {
        if exception_info.is_null() {
            return 0;
        }
        // SAFETY: the filter is called with the OS's own exception pointers.
        let record = unsafe { (*exception_info).ExceptionRecord };
        if record.is_null() {
            return 0;
        }
        // SAFETY: the record belongs to the exception being reported.
        unsafe { (*record).ExceptionCode.0 as u32 }
    }

    /// Write the diagnostic to the process's standard error.
    ///
    /// `WriteFile` on the standard-error handle: no allocation, no lock and
    /// no buffering, so it is safe to call from a crash handler.
    fn write_stderr(bytes: &[u8]) {
        // SAFETY: `GetStdHandle` takes a constant and returns the handle.
        let Ok(handle) = (unsafe { GetStdHandle(STD_ERROR_HANDLE) }) else {
            return;
        };
        if handle.is_invalid() {
            return;
        }
        let mut written = 0u32;
        // SAFETY: the buffer is valid for the call and the handle is the
        // process's standard error.
        let _ = unsafe { WriteFile(handle, Some(bytes), Some(&mut written), None) };
    }

    /// The process-wide fault filter, installed for as long as the guard lives.
    pub struct FaultGuard {
        previous: LPTOP_LEVEL_EXCEPTION_FILTER,
    }

    /// Install the process-wide fault filter.
    ///
    /// Call this **before any plugin code can run**: from then on an
    /// unhandled exception is reported on stderr and ends the process with
    /// [`PLUGIN_FAULT_EXIT_CODE`], which the supervisor recognises.
    pub fn install_fault_guard() -> FaultGuard {
        // SAFETY: the filter is a plain function pointer with the signature
        // the OS calls it with.
        let previous = unsafe { SetUnhandledExceptionFilter(Some(fault_filter)) };
        FaultGuard { previous }
    }

    impl Drop for FaultGuard {
        fn drop(&mut self) {
            // SAFETY: restoring the filter the OS had before us.
            let _ = unsafe { SetUnhandledExceptionFilter(self.previous) };
        }
    }
}

#[cfg(windows)]
pub use windows_filter::{FaultGuard, install_fault_guard};

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(path: &str, kind: PluginKind) -> Box<PluginIdentity> {
        PluginIdentity::new(Path::new(path), kind)
    }

    #[test]
    fn the_scope_records_the_running_call_and_restores_the_previous_one() {
        let plugin = identity(r"C:\plugins\gen_bad.dll", PluginKind::General);
        assert!(current_site().is_none(), "no call runs outside a scope");

        {
            let _scope = plugin_call_scope(&plugin, "init");
            let site = current_site().expect("the scope records the call");
            // SAFETY: the identity is alive for the whole scope.
            let identity = unsafe { site.identity.as_ref() };
            assert_eq!(identity.path(), Path::new(r"C:\plugins\gen_bad.dll"));
            assert_eq!(identity.kind(), PluginKind::General);
            assert_eq!(site.call, "init");
        }
        assert!(
            current_site().is_none(),
            "the scope must restore the previous record when it ends"
        );
    }

    #[test]
    fn nested_scopes_name_the_innermost_call() {
        let outer = identity(r"C:\plugins\outer.dll", PluginKind::Input);
        let inner = identity(r"C:\plugins\inner.dll", PluginKind::Dsp);
        let _outer_scope = plugin_call_scope(&outer, "init");
        {
            let _inner_scope = plugin_call_scope(&inner, "process");
            let site = current_site().expect("the inner scope records the call");
            // SAFETY: the identity is alive for the whole scope.
            let identity = unsafe { site.identity.as_ref() };
            assert_eq!(identity.path(), Path::new(r"C:\plugins\inner.dll"));
            assert_eq!(site.call, "process");
        }
        let site = current_site().expect("the outer scope is back");
        // SAFETY: the identity is alive for the whole scope.
        let identity = unsafe { site.identity.as_ref() };
        assert_eq!(identity.path(), Path::new(r"C:\plugins\outer.dll"));
        assert_eq!(site.call, "init");
    }

    #[test]
    fn the_fault_line_names_the_plugin_and_the_call() {
        let plugin = identity(r"C:\plugins\bad.dll", PluginKind::Input);
        let _scope = plugin_call_scope(&plugin, "init");
        let mut buffer = [0u8; FAULT_LINE_CAPACITY];
        let len = write_fault_line(&mut buffer, 0xC000_0005, current_site());
        let line = String::from_utf8_lossy(&buffer[..len]);
        assert!(
            line.starts_with("rustamp-plughost32: plugin fault in C:\\plugins\\bad.dll"),
            "the line must name the plugin: {line}"
        );
        assert!(
            line.contains("(input plugin, call init)"),
            "the line must name the kind and the call: {line}"
        );
        assert!(
            line.contains("Windows exception 0xC0000005 (access violation)"),
            "the line must name the exception: {line}"
        );
        assert!(line.ends_with('\n'), "the line must end the report: {line}");
    }

    #[test]
    fn a_fault_outside_a_plugin_call_says_so() {
        let mut buffer = [0u8; FAULT_LINE_CAPACITY];
        let len = write_fault_line(&mut buffer, 0xC000_001D, None);
        let line = String::from_utf8_lossy(&buffer[..len]);
        assert!(
            line.starts_with("rustamp-plughost32: unhandled exception in the plugin host"),
            "the line must not blame a plugin: {line}"
        );
        assert!(
            line.contains("0xC000001D (illegal instruction)"),
            "the line must name the exception: {line}"
        );
    }

    #[test]
    fn the_fault_line_is_bounded_and_never_overflows_the_buffer() {
        let long = identity(
            &format!(r"C:\plugins\{}.dll", "x".repeat(400)),
            PluginKind::General,
        );
        let _scope = plugin_call_scope(&long, "init");
        let mut buffer = [0u8; 64];
        let len = write_fault_line(&mut buffer, 0xC000_0005, current_site());
        assert!(len <= buffer.len(), "the line must fit the buffer");
        let line = String::from_utf8_lossy(&buffer[..len]);
        assert!(
            line.starts_with("rustamp-plughost32: plugin fault in"),
            "the line keeps its head even when the path does not fit: {line}"
        );
    }

    #[test]
    fn the_fault_exit_code_is_recognised_by_the_supervisor_policy() {
        // The two processes agree on one exit code: the host writes it, the
        // supervisor's policy reads it. The classification lives in
        // `crate::restart`, and it must not confuse the host's own code with
        // a Windows exception status.
        assert_eq!(
            crate::restart::classify_exit(Some(PLUGIN_FAULT_EXIT_CODE as i32)),
            crate::restart::HostExit::PluginFault
        );
        assert_eq!(
            crate::restart::classify_exit(Some(0xC000_0005u32 as i32)),
            crate::restart::HostExit::Crashed(0xC000_0005)
        );
    }
}

/// Property tests for the fault attribution.
///
/// The report is the only thing a crash can still say reliably, so it must
/// say the truth: a fault is attributed to the plug-in whose call was
/// running when it happened, and a fault that happened outside every
/// plug-in call blames nobody. The properties drive a random sequence of
/// plug-in calls (nested and not) and check the report after every step.
///
/// The report writer and the current-call record are private to this
/// module, so the properties live here rather than in an integration test.
#[cfg(test)]
mod props {
    use super::*;

    /// The seeds of the property runs. A failing run prints its seed, so
    /// the sequence is replayable.
    const SEEDS: [u64; 12] = [
        0x0000_0001,
        0x0000_00a3,
        0x0000_07ff,
        0x0000_ffff,
        0x0001_2345,
        0x00c0_ffee,
        0x0bad_c0de,
        0x5eed_0001,
        0x5eed_0002,
        0xdead_beef,
        0xface_b00c,
        0xffff_ffff,
    ];

    /// The number of steps in one random sequence.
    const STEPS: usize = 32;

    /// The number of steps in the storm: a sequence that keeps coming must
    /// never panic and never blame the wrong plug-in.
    const STORM: usize = 4_000;

    /// The exception the reports are written for.
    const EXCEPTION: u32 = 0xC000_0005;

    /// A xorshift64\* generator: deterministic, seeded and dependency-free.
    ///
    /// The same generator the property tests of the other crates use, kept
    /// local so this module needs no path into the test tree.
    struct Rng(u64);

    impl Rng {
        /// A generator for `seed`.
        fn new(seed: u64) -> Self {
            Self(seed | 1)
        }

        /// The next 64 bits.
        fn next_u64(&mut self) -> u64 {
            let mut state = self.0;
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            self.0 = state;
            state.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        /// A number in `0..bound`.
        fn below(&mut self, bound: u64) -> u64 {
            self.next_u64() % bound
        }
    }

    /// The plug-ins the sequences call into.
    ///
    /// The identities are boxed because the scope holds a pointer to them:
    /// the address of a plug-in must not move while its call runs (see
    /// [`PluginIdentity`]). A fixed array keeps them apart.
    fn plugins() -> [Box<PluginIdentity>; 4] {
        [
            PluginIdentity::new(Path::new(r"C:\plugins\gen_bad.dll"), PluginKind::General),
            PluginIdentity::new(Path::new(r"C:\plugins\in_bad.dll"), PluginKind::Input),
            PluginIdentity::new(Path::new(r"C:\plugins\out_bad.dll"), PluginKind::Output),
            PluginIdentity::new(
                Path::new(r"C:\plugins\vis_bad.dll"),
                PluginKind::Visualization,
            ),
        ]
    }

    /// The calls a sequence may make.
    const CALLS: [&str; 4] = ["init", "config", "render", "process"];

    /// The report the crash handler would write right now.
    fn report() -> String {
        let mut buffer = [0u8; FAULT_LINE_CAPACITY];
        let len = write_fault_line(&mut buffer, EXCEPTION, current_site());
        String::from_utf8_lossy(&buffer[..len]).into_owned()
    }

    /// Check the report against the call stack the model keeps.
    ///
    /// `stack` holds the plug-ins whose calls are running, outermost first.
    fn check(stack: &[(usize, &'static str)], plugins: &[Box<PluginIdentity>], label: &str) {
        let line = report();
        assert!(
            line.contains(&format!("0x{EXCEPTION:08X}")),
            "{label}: the report must name the exception: {line}"
        );
        match stack.last() {
            None => {
                assert!(
                    line.contains("unhandled exception in the plugin host"),
                    "{label}: a fault outside every plug-in call is the host's own: {line}"
                );
                for plugin in plugins {
                    assert!(
                        !line.contains(plugin.path().to_string_lossy().as_ref()),
                        "{label}: the report must not blame a plug-in: {line}"
                    );
                }
            }
            Some(&(index, call)) => {
                let plugin = &plugins[index];
                assert!(
                    line.contains(plugin.path().to_string_lossy().as_ref()),
                    "{label}: the report must name the plug-in the fault came from: {line}"
                );
                assert!(
                    line.contains(call),
                    "{label}: the report must name the call: {line}"
                );
                assert!(
                    line.contains(kind_label(plugin.kind())),
                    "{label}: the report must name the kind: {line}"
                );
                // The plug-ins that are further out are not to blame — unless
                // it is the same plug-in, which a re-entrant call nests.
                for (other, _) in stack.iter().rev().skip(1) {
                    if *other == index {
                        continue;
                    }
                    let other = &plugins[*other];
                    assert!(
                        !line.contains(other.path().to_string_lossy().as_ref()),
                        "{label}: the report must not blame an outer plug-in: {line}"
                    );
                }
            }
        }
    }

    /// The fault is attributed to the plug-in whose call was running, and an
    /// unattributable fault blames nobody.
    #[test]
    fn a_fault_is_attributed_to_the_plug_in_it_came_from() {
        for seed in SEEDS {
            let plugins = plugins();
            let mut rng = Rng::new(seed);
            let mut scopes: Vec<FaultScope> = Vec::new();
            let mut stack: Vec<(usize, &'static str)> = Vec::new();

            for step in 1..=STEPS {
                let label = format!("seed {seed}, step {step}");
                // Enter a call, or leave the innermost one.
                if stack.is_empty() || rng.below(3) == 0 {
                    let index = rng.below(plugins.len() as u64) as usize;
                    let call = CALLS[rng.below(CALLS.len() as u64) as usize];
                    scopes.push(plugin_call_scope(&plugins[index], call));
                    stack.push((index, call));
                } else {
                    scopes.pop();
                    stack.pop();
                }
                check(&stack, &plugins, &label);
            }
            // The scopes go out the way the call stack unwinds: the last one
            // first, so no record of a plug-in that is no longer in a call
            // survives the sequence.
            while let Some(scope) = scopes.pop() {
                drop(scope);
            }
        }
    }

    /// No panic, whatever the sequence: an empty one and a storm of calls.
    #[test]
    fn no_sequence_of_plug_in_calls_panics() {
        // An empty sequence: no call runs, so the report blames nobody.
        let fixtures = plugins();
        check(&[], &fixtures, "an empty sequence");

        // A storm: the calls keep coming and the report stays honest.
        for seed in SEEDS {
            let fixtures = plugins();
            let mut rng = Rng::new(seed);
            let mut scopes: Vec<FaultScope> = Vec::new();
            let mut stack: Vec<(usize, &'static str)> = Vec::new();
            for step in 1..=STORM {
                if stack.is_empty() || rng.below(3) == 0 {
                    let index = rng.below(fixtures.len() as u64) as usize;
                    let call = CALLS[rng.below(CALLS.len() as u64) as usize];
                    scopes.push(plugin_call_scope(&fixtures[index], call));
                    stack.push((index, call));
                } else {
                    scopes.pop();
                    stack.pop();
                }
                let line = report();
                assert!(!line.is_empty(), "seed {seed}, step {step}: the report");
                check(&stack, &fixtures, &format!("seed {seed}, step {step}"));
            }
            // The scopes go out the way the call stack unwinds.
            while let Some(scope) = scopes.pop() {
                drop(scope);
            }
        }
    }
}
