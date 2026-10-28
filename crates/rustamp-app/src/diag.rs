// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Diagnostics for the binary: the panic hook, the `tracing` subscriber and the
//! startup banner.
//!
//! Everything here writes to **stderr**, never to stdout. With `--null` the app
//! is headless and its stdout carries the protocol a test reads
//! (`tests/headless.rs`), so a diagnostic line on stdout would be read as a
//! protocol line. `tracing_subscriber`'s fmt layer defaults to stdout; the
//! layer below is built with [`std::io::stderr`] explicitly.
//!
//! The functions are called once, at the very top of `main`, before the command
//! line is parsed - so a panic during parsing, during the registration step or
//! inside the event loop names its place and carries a backtrace. A release
//! build is a GUI program and has no console by default; `--console` attaches
//! one (see `rustamp_platform::console`), which is how these lines are read.

use std::fmt;
use std::io::IsTerminal;
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::PathBuf;

use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// The `tracing` filter used when `RUST_LOG` says nothing.
///
/// `info` keeps the startup banner visible in every run while leaving the
/// per-frame chatter of the dependencies (and our own `debug`/`trace` events)
/// off unless asked for.
pub const DEFAULT_LOG_FILTER: &str = "info";

/// Installs the panic hook: the panic message, its source location and a
/// backtrace, all on stderr.
///
/// The backtrace is captured unconditionally with
/// [`std::backtrace::Backtrace::force_capture`]; `RUST_BACKTRACE` only decides
/// whether the runtime *also* prints one. Writes are best-effort: a closed
/// stderr must not turn a panic into a second panic inside the hook.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let mut stderr = std::io::stderr().lock();
        let location = info
            .location()
            .map_or_else(|| "<unknown location>".to_string(), ToString::to_string);
        let _ = writeln!(
            stderr,
            "rustamp: panic at {location}: {}",
            panic_payload(info)
        );
        let _ = writeln!(stderr, "{}", std::backtrace::Backtrace::force_capture());
        let _ = stderr.flush();
    }));
}

/// The text a panic carries, for the payload types `panic!` can produce.
fn panic_payload<'a>(info: &'a PanicHookInfo<'_>) -> &'a str {
    if let Some(text) = info.payload().downcast_ref::<&'static str>() {
        return text;
    }
    if let Some(text) = info.payload().downcast_ref::<String>() {
        return text.as_str();
    }
    // `panic_any` with a non-string payload.
    "<panic payload is not a string>"
}

/// Installs the `tracing` subscriber: a registry with one fmt layer on stderr,
/// filtered by `RUST_LOG` (default [`DEFAULT_LOG_FILTER`]).
///
/// Every event carries its target, source file, line and thread id, which is
/// what turns a bare "something went wrong" from a test run into a place to
/// look. An already installed subscriber (an in-process test, an embedding
/// caller) is kept as it is; installing twice is never fatal.
pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_already_logging| EnvFilter::new(DEFAULT_LOG_FILTER));
    let subscriber = tracing_subscriber::registry().with(filter).with(
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(true)
            .with_file(true)
            .with_line_number(true)
            .with_thread_ids(true)
            // Escape codes belong on a terminal, not in a redirected log.
            .with_ansi(std::io::stderr().is_terminal()),
    );
    if subscriber.try_init().is_err() {
        // Only a `SetGlobalDefaultError`: a subscriber is already installed.
    }
}

/// Logs what is starting, where from and with which arguments.
///
/// This is the first line of every run at the default filter: a VM report that
/// only says "it crashed" can be matched to the build, the machine, the working
/// directory and the exact command line that produced it.
pub fn log_startup_banner() {
    let banner = StartupBanner::capture();
    tracing::info!(
        target: "rustamp",
        version = %banner.version,
        os = %banner.os,
        arch = %banner.arch,
        args = ?banner.args,
        exe = %banner.exe,
        cwd = %banner.cwd,
        "starting"
    );
}

/// The facts of one start, in a fixed order.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StartupBanner {
    version: &'static str,
    os: &'static str,
    arch: &'static str,
    args: Vec<String>,
    exe: String,
    cwd: String,
}

impl StartupBanner {
    /// Reads the facts from the running process.
    fn capture() -> Self {
        StartupBanner {
            version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            args: std::env::args().skip(1).collect(),
            exe: describe_path(std::env::current_exe()),
            cwd: describe_path(std::env::current_dir()),
        }
    }
}

impl fmt::Display for StartupBanner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "rustamp {} on {} {} ({}) args {:?} cwd {}",
            self.version, self.os, self.arch, self.exe, self.args, self.cwd
        )
    }
}

/// A path, or `<unavailable: ...>` when the OS refuses to name it.
fn describe_path(path: Result<PathBuf, std::io::Error>) -> String {
    path.map_or_else(
        |error| format!("<unavailable: {error}>"),
        |path| path.display().to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn banner() -> StartupBanner {
        StartupBanner {
            version: "1.2.3",
            os: "windows",
            arch: "x86_64",
            args: vec!["--null".to_string(), "track.flac".to_string()],
            exe: "C:\\rustamp.exe".to_string(),
            cwd: "C:\\music".to_string(),
        }
    }

    #[test]
    fn the_banner_names_every_startup_fact() {
        let text = banner().to_string();
        for expected in [
            "1.2.3",
            "windows",
            "x86_64",
            "--null",
            "track.flac",
            "C:\\rustamp.exe",
            "C:\\music",
        ] {
            assert!(
                text.contains(expected),
                "{expected:?} missing from {text:?}"
            );
        }
    }

    #[test]
    fn capturing_the_banner_always_names_a_version_and_a_platform() {
        let captured = StartupBanner::capture();
        assert_eq!(captured.version, env!("CARGO_PKG_VERSION"));
        assert!(!captured.os.is_empty());
        assert!(!captured.arch.is_empty());
        // A path may be unavailable, but the field is never empty: the failure
        // is described instead.
        assert!(!captured.exe.is_empty());
        assert!(!captured.cwd.is_empty());
    }

    #[test]
    fn an_unavailable_path_is_described_not_dropped() {
        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        assert!(describe_path(Err(error)).starts_with("<unavailable: gone>"));
        assert_eq!(describe_path(Ok(PathBuf::from("C:\\music"))), "C:\\music");
    }

    #[test]
    fn logging_the_banner_without_a_subscriber_is_harmless() {
        // Unit tests do not install a subscriber; an event with no subscriber
        // must simply be dropped.
        log_startup_banner();
    }

    #[test]
    fn the_default_filter_is_a_parsable_directive() {
        // `EnvFilter::new` panics on a malformed directive, so this also
        // guards the constant against a typo.
        assert_eq!(DEFAULT_LOG_FILTER, "info");
        assert_eq!(EnvFilter::new(DEFAULT_LOG_FILTER).to_string(), "info");
    }
}
