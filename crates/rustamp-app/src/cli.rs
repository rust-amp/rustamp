// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Command-line parsing for the `rustamp` binary (task card N1, reconciled
//! with S13).
//!
//! # The two switch families
//!
//! The command line carries two families of switches:
//!
//! * the **reference switches** of S13 §1 — `/CLASS=`, `/CONFIG=`,
//!   `/BOOKMARK`, `/ADD`, `/INSTALL`, `/UNREG`, `/NOREG`, `/ONLYINSTALL` —
//!   spelled with a leading `/`, exactly as the audited spec records them;
//! * **Rustamp's own switches** (`--null`, `--no-audio`, `--volume`, `--skin`,
//!   `--seconds`, `--enqueue`, `--control`, `--import-winamp-ini`, `--help`),
//!   spelled with a leading `--`. They are the product's own test and
//!   convenience aids; the reference has no such switches.
//!
//! S13 §1's list is the reference's **own** switch list, not a closed one: the
//! second audit pass (2026-10-20) withdrew the completeness claim and moved
//! the question into §5.13. So nothing is added to the `/` family here beyond
//! the switches the spec names — and `/ADD`, which the first pass put on the
//! negative list, **is** a switch: the reference registers its `Enqueue` shell
//! verb with the command line `"%s" /ADD "%%1"` (S13 §3).
//!
//! A leading `/` starts a switch, `=` separates a switch's value, and
//! **everything else is a file argument** (S13 §1). An unknown `/` token is
//! therefore a file argument and not an error — that is the spec's rule, and it
//! is why `/PLAY`, `/NEW`, `/EJECT` and `/TASKBAR` land in the playlist
//! instead of being refused.
//!
//! Two switches take their path in the argument that **follows** them:
//! `/ADD <path>` and `/BOOKMARK <path>` (S13 §1). Both have the same shape,
//! down to the path that arrives in one quoted argument.
//!
//! The optional path of `--import-winamp-ini` is the one place where the two
//! families meet: the path is the argument behind the switch, when that
//! argument is not a switch of either family. So the import switch never
//! swallows a `/CLASS=` or a `--null`, and it has to come **before** the file
//! to play.
//!
//! # The URL on the command line (own choice on top of S13 §1)
//!
//! S13 §1 says that everything which is not a switch is a **file argument**.
//! A URL is not a file: it names a stream, and the player can read one
//! (S19 §1: the request template is plain `http://`). So the bare argument -
//! and every other path the command line names, the path of `/ADD` and the
//! path of `/BOOKMARK` - is read by one rule, [`media_argument`]:
//!
//! * an argument that names an `http://` URL is a **URL**;
//! * an `https://` URL is a **URL** too: TLS is **our own extension**, not an
//!   S19 fact (S19 §6 question 8 leaves the other schemes open, and the spec
//!   records no evidence that the reference reads a TLS stream at all);
//! * an argument that names **any other scheme** is refused **by name**, so a
//!   typo cannot silently become a file path;
//! * everything else is a **file path**, untouched.
//!
//! The refusal is a parse error: the player says which scheme it cannot
//! read and starts nothing, rather than taking the URL for a file name.
//!
//! The same rule decides what a **hand-over to a running instance** carries
//! (see [`forwarded_location`]): a stream URL travels in the hand-off
//! frame and is streamed by the running player, and another scheme is
//! refused with the very same words.
//!
//! # Conformance with S13
//!
//! | S13 fact | Our behaviour | State | Where |
//! |---|---|---|---|
//! | §1 everything else on the command line is a file argument | an argument that names an `http://` or `https://` URL is a URL; every other argument is a file path | differs (own choice: a URL is not a file, and S19 gives the player a stream to read; `https` is our extension) | `cli.rs`, [`media_argument`] |
//! | §1 `/CLASS=<name>` overrides the class the single-instance check looks for | [`Args::class`] carries the value; [`Args::instance_class`] hands it to the check | matches | `cli.rs`, [`rustamp_platform::single_instance`] |
//! | §2 the default class is the main window's class `Winamp v1.x` (S09 §1, §8 item 1) | [`rustamp_platform::single_instance::DEFAULT_CLASS`] | matches | `rustamp-platform/src/single_instance.rs` |
//! | §2 the class name is case-sensitive | the class half of the instance key is not case-folded | matches | `rustamp-platform/src/single_instance.rs` |
//! | §1 `/CONFIG=<path>` is the configuration file to use instead of the default | [`Args::config`] replaces the resolved default | matches | `cli.rs`, `lib.rs` |
//! | §1 `/BOOKMARK <path>` adds the path as a bookmark rather than playing it | [`Args::bookmark`] carries the path; the run adds it and does not play it | matches (see the note below) | `cli.rs`, `lib.rs` |
//! | §1 `/ADD <path>` enqueues the path instead of playing it — the `Enqueue` verb of §3, which the reference registers with the command line `"%s" /ADD "%%1"` (second audit pass, 2026-10-20) | [`Args::enqueue`] carries the path; the run adds it to the playlist and does not play it | matches | `cli.rs`, `lib.rs` |
//! | §1 `/INSTALL` is the installer association step | [`RegistrationFlags::install`] | matches | `rustamp-platform/src/shell_registration.rs` |
//! | §1 `/UNREG` removes the shell registration | [`RegistrationFlags::unreg`] | matches | `rustamp-platform/src/shell_registration.rs` |
//! | §1 `/NOREG` skips the registration step | [`RegistrationFlags::noreg`] | matches | `rustamp-platform/src/shell_registration.rs` |
//! | §1 `/ONLYINSTALL` is the install step, then exit | [`RegistrationFlags::only_install`] | matches | `main.rs` |
//! | §1 `/PLAY`, `/NEW`, `/EJECT` and `/TASKBAR` are not in the reference's switch list | not implemented; such a token is a file argument | matches (the scope of the negative list is the reference's own switch list, S13 §5.13) | `cli.rs` |
//! | §1 a command line may carry several paths | the parser takes one file argument and refuses a second; the path of `/ADD` is a path of its own and does not take the file argument's place | differs (own choice) | `cli.rs` |
//! | §2 a second start looks for a **window** of that class | the check is a named mutex named after the class | differs (own choice, S13 §5.8) | `rustamp-platform/src/single_instance.rs` |
//! | §2 the player registers a window class, and the class name is what `/CLASS=` overrides | the registered main-window class stays `Winamp v1.x` (S09 §1 and §8 item 1: an ABI fact plugins and external tools depend on); `/CLASS=` overrides the class the **check** looks for | own choice (the task's wording: "the class the single-instance check looks for") | `rustamp-app/src/window.rs`, `rustamp-platform/src/single_instance.rs` |
//! | §2 the hand-over is a window message | the hand-over travels over a named pipe | differs (own choice, S13 §5.8) | `rustamp-platform/src/single_instance.rs` |
//! | §1 (no fact) — there is no local control channel | `--control[=<name>]` opens a named pipe named after the instance key and answers typed JSON lines (ping, snapshot, wait, commands) | own choice (a test and tool aid; gated by the same window rail as the run) | `cli.rs`, [`Control`], `src/control.rs` |
//! | §3 the four verbs per media and playlist file | four verbs each | matches | `rustamp-platform/src/shell_registration.rs` |
//! | §3 the two verbs per directory | `Play` and `Enqueue` | matches | `rustamp-platform/src/shell_registration.rs` |
//! | §3 the one verb of a skin archive | `install` | matches | `rustamp-platform/src/shell_registration.rs` |
//!
//! The note on `/BOOKMARK`: the path goes into the bookmark store
//! ([`crate::bookmarks`]) and is not played, as S13 §1 settles. A second launch
//! writes the store itself, because the running player reads it before it
//! shows it.
//!
//! The note on `/ADD`: the reference registers its `Enqueue` shell verb with
//! the command line `"%s" /ADD "%%1"` (S13 §3), so the verb **is** the switch.
//! Rustamp's own `--enqueue` asks the same thing about the file argument, and
//! both spellings end in one [`Enqueue`] value, so the verb and the switch ask
//! for the same thing in the same words and cannot drift apart.
//!
//! The open questions of S13 §5 stay open here: the in-window drop rule
//! (§5.1), the drop of a folder (§5.2), `/BOOKMARK` without a path (§5.3),
//! whether `/CONFIG=` moves the skin directories (§5.4), what a busy instance
//! does (§5.5), whether `/CONFIG=` affects the class (§5.6), the reference's
//! own default for `/CLASS=` (§5.7), the hand-over message (§5.8), the
//! quoting of the bookmark command line (§5.9), the directory identifiers
//! (§5.10), the display names of the other verbs (§5.11), what the
//! `DropTarget` entries carry (§5.12), whether the reference's switch list is
//! complete (§5.13) and the name of the verb that registers `/BOOKMARK`
//! (§5.14).

use std::fmt;

use std::path::PathBuf;

use rustamp_core::playlist::MediaLocation;
use rustamp_platform::shell_registration::RegistrationFlags;
use rustamp_platform::single_instance::{ForwardRequest, instance_key};

/// Where the audio goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputChoice {
    /// The best device for the host: WASAPI shared mode, `cpal` fallback
    /// (`rustamp_output::default_output`).
    #[default]
    Default,
    /// The deterministic null device. This is the test mode: the virtual
    /// device's clock is driven in real time by the app and **no window is
    /// opened** (so the binary can run in a headless test).
    Null,
    /// The deterministic null device, but the window still opens. This is the
    /// silent windowed mode (`--no-audio`): playback runs without any audio
    /// hardware (a headless VM has no endpoint), so the window tests can prove
    /// the window opened without depending on a device. The virtual clock is
    /// driven once per frame by the windowed presenter, exactly as
    /// [`crate::app::App::run_headless`] drives it headless.
    NoAudio,
}

/// What `/BOOKMARK` asks for (S13 §1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bookmark {
    /// `/BOOKMARK <path>`: the media is added as a bookmark rather than
    /// played. The path is read by [`media_argument`], so it may be an
    /// `http://` URL.
    Location(MediaLocation),
    /// `/BOOKMARK` without a path.
    ///
    /// S13 §5.3 leaves this form open, so the player says so and does
    /// nothing rather than guessing what the reference does.
    WithoutPath,
}

/// What the command line asks to be enqueued instead of played.
///
/// S13 §1: `/ADD <path>` enqueues the path instead of playing it — the
/// `Enqueue` verb of §3, which the reference registers with the command line
/// `"%s" /ADD "%%1"`. Rustamp's own `--enqueue` asks the same thing about the
/// file argument, so both spellings end in this one value: the shell's verb
/// and the reference's switch ask for the same thing in the same words.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Enqueue {
    /// Nothing is enqueued.
    #[default]
    None,
    /// The media argument is enqueued instead of played (`--enqueue`).
    Argument,
    /// This media is enqueued instead of played (`/ADD <path>`). The path is
    /// read by [`media_argument`], so it may be an `http://` URL.
    Location(MediaLocation),
    /// `/ADD` without a path.
    ///
    /// S13 §5.3 leaves the form of a path-taking switch without a path open
    /// (it asks it about `/BOOKMARK`), so the player says so and does
    /// nothing rather than guessing.
    WithoutPath,
}

/// What the `--import-winamp-ini` switch asks for (S12).
///
/// The switch is an explicit action: it imports a Winamp 2.95 `winamp.ini`
/// into our configuration. The file is only read; the player never writes to
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WinampIniImport {
    /// The switch was not given: nothing is imported.
    #[default]
    None,
    /// The switch was given without a path: the player asks for the file
    /// with the native file dialog.
    Ask,
    /// The switch named the file to import.
    File(PathBuf),
}

/// What `--control[=<name>]` asks for: the local control and inspection
/// channel.
///
/// Own choice on top of S13 §1, next to the other non-reference switches
/// (`--null`, `--no-audio`, `--seconds`): the reference has no local control
/// channel. The switch is a **gate**, never a default — without it the player
/// opens no pipe at all, and it is a command-line switch rather than an
/// environment variable or a `debug_assertions` test, so a release build has
/// exactly the same rule as a debug build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    /// `--control`: the channel is named after the running instance — the
    /// [`instance_key`] of the class the single-instance check looks for
    /// (`/CLASS=`) and the configuration the run uses (`/CONFIG=`), exactly
    /// the key the single-instance lock is named after.
    Instance,
    /// `--control=<name>`: the channel is named after `name`.
    ///
    /// A test names its own channel, so it never reaches — and never collides
    /// with — a player the developer is running. The name is one path
    /// component: letters, digits, `-`, `_` and `.`.
    Named(String),
}

/// The parsed command line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args {
    /// The media to load and play: a file path or an `http://` URL (see
    /// [`media_argument`]). `None` starts with nothing loaded (S02 §3 shows
    /// the program name in the marquee). With [`Enqueue::Argument`] the media
    /// is loaded but **not** played.
    pub media: Option<MediaLocation>,
    /// Output device choice.
    pub output: OutputChoice,
    /// Initial volume `0..=255`; `None` uses the config's `audio.volume`.
    pub volume: Option<u8>,
    /// Skin path (folder, `.wsz` or `.zip`); `None` uses the config's
    /// `ui.skin`, then the built-in default skin.
    pub skin: Option<PathBuf>,
    /// Stop after this many seconds of playback (test aid).
    pub seconds: Option<u64>,
    /// The local control and inspection channel (`--control`, see
    /// [`Control`]); `None` opens no channel at all.
    pub control: Option<Control>,
    /// The Winamp configuration to import (S12).
    pub import_winamp_ini: WinampIniImport,
    /// `--help`/`-h` was given.
    pub help: bool,
    /// `--console`: keep a console window for the notes.
    ///
    /// A release build is linked as a GUI program, so it has no console of its
    /// own (owner report, 2026-10-21). Own choice on top of S13 §1, which has no
    /// switch for this; the alternative was a debug window in the menu, and a
    /// console that the notes already go to is the smaller change.
    pub console: bool,
    /// What is enqueued instead of played (`--enqueue`, `/ADD <path>`).
    pub enqueue: Enqueue,
    /// The window class the single-instance check looks for (`/CLASS=`).
    pub class: Option<String>,
    /// The configuration file to use instead of the default (`/CONFIG=`).
    pub config: Option<PathBuf>,
    /// The bookmark the command line asks for (`/BOOKMARK`).
    pub bookmark: Option<Bookmark>,
    /// The registration switches (`/INSTALL`, `/UNREG`, `/NOREG`,
    /// `/ONLYINSTALL`).
    pub registration: RegistrationFlags,
}

impl Args {
    /// The window class the single-instance check looks for (S13 §2).
    ///
    /// `/CLASS=` overrides the default, which is the main window's class
    /// (`Winamp v1.x`, S09 §1 and §8 item 1).
    pub fn instance_class(&self) -> &str {
        self.class
            .as_deref()
            .unwrap_or(rustamp_platform::single_instance::DEFAULT_CLASS)
    }

    /// The configuration file the run uses (S13 §1: `/CONFIG=`).
    ///
    /// `/CONFIG=` replaces the default path; without it the default is
    /// resolved as usual (the `%RUSTAMP_CONFIG%` override, then the
    /// per-user configuration file).
    pub fn resolved_config_path(&self) -> Option<PathBuf> {
        self.config.clone().or_else(crate::config::default_path)
    }

    /// The instance key the control channel is named after, or `None` when the
    /// run was asked for no channel (`--control`).
    ///
    /// Without a name the key is the single-instance key
    /// ([`instance_key`]): the class the single-instance check looks for and
    /// the configuration the run uses. The control channel therefore follows
    /// the player it belongs to, and a second player with its own
    /// `/CONFIG=`/`/CLASS=` is a different channel.
    pub fn control_key(&self) -> Option<String> {
        match self.control.as_ref()? {
            Control::Instance => Some(instance_key(
                self.instance_class(),
                self.resolved_config_path().as_deref(),
            )),
            Control::Named(name) => Some(name.clone()),
        }
    }
}

/// The usage text (`--help`).
pub const USAGE: &str = "\
usage: rustamp [<file|url>] [--null] [--no-audio] [--volume <0..=255>] [--skin <path>]
                 [--seconds <n>] [--enqueue] [--control[=<name>]]
                 [--import-winamp-ini [path]] [-h|--help]
       rustamp [/CLASS=<name>] [/CONFIG=<path>] [/BOOKMARK <path>] [/ADD <path>]
              [/INSTALL] [/UNREG] [/NOREG] [/ONLYINSTALL]
       rustamp --approve-plugin [/CONFIG=<path>] <plug-in.dll> [<plug-in.dll> ...]

  <file|url>        media file to load and play, or an http(s):// URL to stream (optional)
  --null            use the deterministic null output and run headless (no window)
  --console         keep a console window for the player's notes (release builds
                    are GUI programs; without this, notes go nowhere)
  --no-audio        open the window but play on the null output (silent; no audio device needed)
  --volume <0..=255> initial volume (default: the config's audio.volume)
  --skin <path>     skin folder, .wsz or .zip (default: config's ui.skin, then built-in)
  --seconds <n>     stop after n seconds of playback (test aid)
  --enqueue         add the file to the playlist instead of playing it
  --control[=<name>]
                    open the local control and inspection channel on the named pipe
                    \\\\.\\pipe\\rustamp-ctl-<name> (without a name: the instance key of
                    this player); one JSON object per line, requests carry an id
  --import-winamp-ini [path]
                    import a Winamp 2.95 winamp.ini (the file is only read); without a path
                    the player asks for the file with the native dialog
  -h, --help        print this help
  --approve-plugin  put the exact image of each plug-in on the allowlist
                    (plugins-approved.toml beside the configuration file) and exit; the
                    plug-in then may run inside the plug-in host. Nothing is run.

  /CLASS=<name>     the window class the single-instance check looks for
  /CONFIG=<path>    the configuration file to use instead of the default
  /BOOKMARK <path>  add the path as a bookmark rather than playing it
  /ADD <path>       add the path to the playlist instead of playing it
  /INSTALL          write the shell registration
  /UNREG            remove the shell registration
  /NOREG            skip the registration step
  /ONLYINSTALL      write the shell registration, then exit

  An argument that names an http:// or https:// URL is streamed. Another scheme
  (ftp:// for instance) is refused.";

/// Why the command line could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    /// An option that is not in the table.
    UnknownOption(String),
    /// An option that needs a value was the last argument.
    MissingValue(&'static str),
    /// An option's value could not be parsed.
    BadValue {
        /// The option.
        option: &'static str,
        /// The rejected value.
        value: String,
    },
    /// More than one input file was given.
    TooManyFiles(String),
    /// A URL named a scheme this build cannot read.
    UnsupportedUrl {
        /// The URL as it was written.
        url: String,
        /// The scheme of the URL, lower case.
        scheme: String,
    },
    /// An `http://` or `https://` URL that names no host.
    MalformedUrl(String),
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::UnknownOption(option) => write!(f, "unknown option: {option}"),
            CliError::MissingValue(option) => write!(f, "{option} needs a value"),
            CliError::BadValue { option, value } => {
                write!(f, "bad value for {option}: {value}")
            }
            CliError::TooManyFiles(extra) => {
                write!(f, "only one input file is supported, got another: {extra}")
            }
            CliError::UnsupportedUrl { url, scheme } => write!(
                f,
                "{url} was refused: unsupported URL scheme {scheme:?}: this build reads http:// \
                 and https:// URLs (S19 §6 question 8 leaves the other schemes open)"
            ),
            CliError::MalformedUrl(url) => write!(
                f,
                "{url} was refused: it is not a usable http(s):// URL (it names no host)"
            ),
        }
    }
}

impl std::error::Error for CliError {}

/// Parses the argument list (without `argv[0]`).
pub fn parse<I>(args: I) -> Result<Args, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut parsed = Args::default();
    let mut it = args.into_iter().peekable();
    while let Some(arg) = it.next() {
        // The reference switches of S13 §1 all start with a leading `/`.
        if let Some(rest) = arg.strip_prefix('/') {
            match rest {
                "INSTALL" => parsed.registration.install = true,
                "UNREG" => parsed.registration.unreg = true,
                "NOREG" => parsed.registration.noreg = true,
                "ONLYINSTALL" => parsed.registration.only_install = true,
                "BOOKMARK" => {
                    parsed.bookmark = Some(match path_argument(&mut it)? {
                        Some(location) => Bookmark::Location(location),
                        None => Bookmark::WithoutPath,
                    })
                }
                "ADD" => {
                    parsed.enqueue = match path_argument(&mut it)? {
                        Some(location) => Enqueue::Location(location),
                        None => Enqueue::WithoutPath,
                    }
                }
                _ => {
                    if let Some(value) = rest.strip_prefix("CLASS=") {
                        parsed.class = Some(class_value(value)?);
                    } else if let Some(value) = rest.strip_prefix("CONFIG=") {
                        parsed.config = Some(PathBuf::from(value));
                    } else if let Some(path) = rest.strip_prefix("BOOKMARK ") {
                        // The shell registers the bookmark form as one quoted
                        // argument (S13 §1), so the path can arrive in the
                        // same argument as the switch.
                        parsed.bookmark = Some(Bookmark::Location(media_argument(unquote(path))?));
                    } else if let Some(path) = rest.strip_prefix("ADD ") {
                        // The same whole form for `/ADD` (S13 §1): the shell
                        // registers the verb with `"%s" /ADD "%%1"`, so the
                        // switch and its path can arrive in one argument.
                        parsed.enqueue = Enqueue::Location(media_argument(unquote(path))?);
                    } else {
                        // Anything else is a file argument (S13 §1): a token
                        // the reference's switch list does not name is not a
                        // switch at all.
                        set_media(&mut parsed, arg)?;
                    }
                }
            }
            continue;
        }
        match arg.as_str() {
            "--null" => parsed.output = OutputChoice::Null,
            "--console" => parsed.console = true,
            "--no-audio" => parsed.output = OutputChoice::NoAudio,
            "--enqueue" => parsed.enqueue = Enqueue::Argument,
            "--volume" => {
                let value = it.next().ok_or(CliError::MissingValue("--volume"))?;
                parsed.volume = Some(value.parse::<u8>().map_err(|_| CliError::BadValue {
                    option: "--volume",
                    value,
                })?);
            }
            "--skin" => {
                let value = it.next().ok_or(CliError::MissingValue("--skin"))?;
                parsed.skin = Some(PathBuf::from(value));
            }
            "--seconds" => {
                let value = it.next().ok_or(CliError::MissingValue("--seconds"))?;
                parsed.seconds = Some(value.parse::<u64>().map_err(|_| CliError::BadValue {
                    option: "--seconds",
                    value,
                })?);
            }
            "--control" => parsed.control = Some(Control::Instance),
            option if option.starts_with("--control=") => {
                // `--control=<name>`: the channel is named after the value, so
                // a test never reaches the player the developer is running.
                let name = &option["--control=".len()..];
                if name.is_empty() {
                    return Err(CliError::MissingValue("--control"));
                }
                if !is_channel_name(name) {
                    return Err(CliError::BadValue {
                        option: "--control",
                        value: name.to_owned(),
                    });
                }
                parsed.control = Some(Control::Named(name.to_owned()));
            }
            "--import-winamp-ini" => {
                // The path is optional: it is the argument right behind the
                // switch, when that argument is not another switch. Without
                // a path the player asks for the file with the native
                // dialog, so the switch has to come before the file to
                // play.
                //
                // "Switch" is the same rule the rest of the parser uses
                // (see `is_switch`): a leading `/` starts a reference
                // switch (S13 §1) and a leading `-` one of Rustamp's own.
                // The import switch therefore never swallows a `/CLASS=`
                // or a `--null`.
                let path_follows = matches!(it.peek(), Some(next) if !is_switch(next));
                parsed.import_winamp_ini = if path_follows {
                    let path = it.next().expect("the argument was peeked");
                    WinampIniImport::File(PathBuf::from(path))
                } else {
                    WinampIniImport::Ask
                };
            }
            "--help" | "-h" => parsed.help = true,
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(CliError::UnknownOption(other.to_owned()));
            }
            other => set_media(&mut parsed, other.to_owned())?,
        }
    }
    Ok(parsed)
}

/// The class name of `/CLASS=`.
fn class_value(value: &str) -> Result<String, CliError> {
    if value.is_empty() {
        return Err(CliError::MissingValue("/CLASS"));
    }
    Ok(value.to_owned())
}

/// The path of `/ADD` or `/BOOKMARK`: the next argument, when there is one.
///
/// Both switches take their path in the argument that follows them (S13 §1),
/// so they share this one rule. A missing path is left open by S13 §5.3, so
/// the caller records that the switch was given without a path instead of
/// guessing. A path that names a URL is read by [`media_argument`], so it
/// may be an `http://` or `https://` URL - and a URL this build cannot read is refused.
fn path_argument<I>(it: &mut std::iter::Peekable<I>) -> Result<Option<MediaLocation>, CliError>
where
    I: Iterator<Item = String>,
{
    match it.peek() {
        Some(next) if !is_switch(next) => {
            let path = it.next().expect("peeked");
            Ok(Some(media_argument(unquote(&path))?))
        }
        _ => Ok(None),
    }
}

/// Reads a command-line argument as the media it names.
///
/// S13 §1 says that everything which is not a switch is a file argument. A
/// URL is not a file: it names a stream, and the player can read one (S19).
/// This is an **own choice** on top of S13 §1, and it is the same rule for
/// every path the command line names - the bare argument, the path of `/ADD`
/// and the path of `/BOOKMARK`:
///
/// * an argument that names an `http://` URL is a **URL** (S19 §1: the
///   request template is plain `http://`), and an `https://` one is a URL too
///   (TLS is our own extension - S19 establishes no evidence for it); the
///   host must be there, or the argument is refused;
/// * an argument that names **any other scheme** is refused **by name**, so a
///   typo cannot silently become a file path;
/// * everything else is a **file path**, untouched.
///
/// A one-letter scheme is a Windows drive letter (`C://Music`), not a scheme,
/// so such a path stays a path.
pub fn media_argument(argument: &str) -> Result<MediaLocation, CliError> {
    let trimmed = argument.trim();
    let Some((scheme, rest)) = split_scheme(trimmed) else {
        return Ok(MediaLocation::File(PathBuf::from(argument)));
    };
    let readable = scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https");
    if !readable {
        return Err(CliError::UnsupportedUrl {
            url: trimmed.to_owned(),
            scheme: scheme.to_ascii_lowercase(),
        });
    }
    if !names_a_host(rest) {
        return Err(CliError::MalformedUrl(trimmed.to_owned()));
    }
    Ok(MediaLocation::Url(trimmed.to_owned()))
}

/// The location a hand-off frame can carry, or a refusal when the media is a
/// URL this build cannot read.
///
/// The frame of the single-instance hand-over carries a **location**: a file
/// path or a URL (S19 §1). The URL is read by the same rule as the command
/// line reads one - only an `http(s)://` URL that names a host can be read - and
/// a URL that fails it is refused with the command line's own wording, so
/// the two cannot drift apart. A media that is no media at all carries
/// nothing and refuses nothing.
pub fn forwarded_location(
    location: Option<MediaLocation>,
    refusals: &mut Vec<String>,
) -> Option<MediaLocation> {
    match location {
        None => None,
        Some(MediaLocation::File(path)) => Some(MediaLocation::File(path)),
        Some(MediaLocation::Url(url)) => match media_argument(&url) {
            Ok(MediaLocation::Url(url)) => Some(MediaLocation::Url(url)),
            // The location claims to be a URL but names no scheme: there is
            // nothing to read, so it is refused like a malformed URL.
            Ok(MediaLocation::File(_)) => {
                refusals.push(CliError::MalformedUrl(url.clone()).to_string());
                None
            }
            Err(error) => {
                refusals.push(error.to_string());
                None
            }
        },
    }
}

/// The part of a second launch's command line the running instance can apply.
///
/// The media and an initial volume still make sense; the output mode, the
/// skin and the `--seconds` test aid were chosen when the first instance
/// started, so they become refusals. A media that is only **added**
/// (`--enqueue`, S13 §1's `/ADD` and `/BOOKMARK`) travels as such, so the
/// running instance does not start playing it. The media itself may be a
/// file path or a URL (S19 §1): the frame carries the kind, so the running
/// instance applies a URL the way the command line applies one.
pub fn forward_request(args: &Args) -> ForwardRequest {
    let mut refusals = Vec::new();
    if args.output == OutputChoice::NoAudio {
        refusals.push(
            "--no-audio is ignored: the running instance fixed its output at start-up".to_owned(),
        );
    }
    if args.skin.is_some() {
        refusals
            .push("--skin is ignored: the running instance chose its skin at start-up".to_owned());
    }
    if args.seconds.is_some() {
        refusals.push(
            "--seconds is ignored: it is a start-up test aid on the running instance".to_owned(),
        );
    }
    // A bookmark with a path does not travel: this launch writes the store
    // ([`crate::bookmarks`]). One without a path is left open by S13 §5.3, so it
    // is reported rather than guessed.
    if let Some(Bookmark::WithoutPath) = &args.bookmark {
        refusals.push(
            "/BOOKMARK without a path is not defined (S13 §5.3); nothing was bookmarked".to_owned(),
        );
    }
    // The enqueued media (S13 §1): `/ADD <path>` names its own media and
    // `--enqueue` names the media argument. Both are added and not played. A
    // `/ADD` without a path is left open by S13 §5.3, so it is reported
    // rather than guessed.
    let (file, enqueued) = match &args.enqueue {
        Enqueue::None => (args.media.clone(), None),
        Enqueue::Argument => (None, args.media.clone()),
        Enqueue::Location(location) => (args.media.clone(), Some(location.clone())),
        Enqueue::WithoutPath => {
            refusals.push(
                "/ADD without a path is not defined (S13 §5.3); nothing was enqueued".to_owned(),
            );
            (args.media.clone(), None)
        }
    };
    // The frame carries a location: a path or a URL. A URL this build cannot
    // read is reported instead of being handed over (see
    // [`forwarded_location`]).
    let file = forwarded_location(file, &mut refusals);
    let added = forwarded_location(enqueued, &mut refusals);
    ForwardRequest {
        file,
        added,
        volume: args.volume,
        refusals,
    }
}

/// The scheme and the rest of an argument that names a URL, or `None` when
/// the argument names no scheme.
///
/// A scheme starts with a letter and carries at least two characters: a
/// one-letter scheme is a Windows drive letter (`C://Music`), which is a
/// path and not a URL.
fn split_scheme(argument: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = argument.split_once("://")?;
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() || scheme.len() < 2 {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return None;
    }
    Some((scheme, rest))
}

/// Whether the part of an `http://` URL behind the scheme names a host.
///
/// The authority may carry a userinfo and a port; what is left of it is the
/// host, and a URL without a host is not a usable one.
fn names_a_host(rest: &str) -> bool {
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_userinfo, host)| host);
    let host = host.rsplit_once(':').map_or(host, |(host, _port)| host);
    !host.is_empty()
}

/// The path without the quotes the shell puts around it.
///
/// The registered command lines quote the path (`"%s" /ADD "%%1"`,
/// `"%s" /BOOKMARK "%%1"`, S13 §1), so a path that arrives inside a quoted
/// argument keeps its quotes unless they are taken off here.
fn unquote(path: &str) -> &str {
    path.strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(path)
}

/// Whether an argument carries a switch marker rather than being a path.
///
/// A leading `/` starts a reference switch (S13 §1) and a leading `-` one of
/// Rustamp's own — the same rule the parser applies to an unknown option, so
/// a token that would be refused as an option is never taken as a path.
/// Anything else is a path.
fn is_switch(arg: &str) -> bool {
    arg.starts_with('/') || (arg.starts_with('-') && arg.len() > 1)
}

/// Records the media argument; a second one is refused.
fn set_media(parsed: &mut Args, value: String) -> Result<(), CliError> {
    if parsed.media.is_some() {
        return Err(CliError::TooManyFiles(value));
    }
    parsed.media = Some(media_argument(&value)?);
    Ok(())
}

/// Whether `name` can be one component of a pipe name.
///
/// The name is the part behind `\\.\pipe\rustamp-ctl-`, so a separator (`\`,
/// `/`) or a character a client cannot type back would make the channel
/// unreachable or ambiguous. Letters, digits, `-`, `_` and `.` are enough for
/// a test name and for a key.
fn is_channel_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_console_switch_is_off_by_default_and_parses() {
        // A release build has no console of its own (owner report, 2026-10-21),
        // so the notes need an explicit switch; it must not read as an unknown
        // option and must not change what the rest of the line means.
        let plain = parse_args(&[]).unwrap();
        assert!(!plain.console, "no console unless asked for");
        let media = parse_args(&["music.flac"]).unwrap().media;

        let with_console = parse_args(&["--console", "music.flac"]).unwrap();
        assert!(with_console.console, "the switch is accepted");
        assert_eq!(with_console.media, media, "and changes nothing else");
    }

    fn parse_args(args: &[&str]) -> Result<Args, CliError> {
        parse(args.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn a_bare_file_is_the_input() {
        let args = parse_args(&["music.mp3"]).unwrap();
        assert_eq!(
            args.media,
            Some(MediaLocation::File(PathBuf::from("music.mp3")))
        );
        assert_eq!(args.output, OutputChoice::Default);
        assert_eq!(args.volume, None);
        assert_eq!(args.skin, None);
        assert!(!args.help);
    }

    #[test]
    fn null_selects_the_test_output() {
        let args = parse_args(&["--null", "music.flac"]).unwrap();
        assert_eq!(args.output, OutputChoice::Null);
        assert_eq!(
            args.media,
            Some(MediaLocation::File(PathBuf::from("music.flac")))
        );
    }

    #[test]
    fn no_audio_selects_the_silent_windowed_output() {
        let args = parse_args(&["--no-audio", "music.flac"]).unwrap();
        assert_eq!(args.output, OutputChoice::NoAudio);
        assert_eq!(
            args.media,
            Some(MediaLocation::File(PathBuf::from("music.flac")))
        );
    }

    #[test]
    fn volume_skin_and_seconds_are_parsed() {
        let args = parse_args(&[
            "song.ogg",
            "--volume",
            "200",
            "--skin",
            "C:/skins/base.wsz",
            "--seconds",
            "3",
        ])
        .unwrap();
        assert_eq!(args.volume, Some(200));
        assert_eq!(args.skin, Some(PathBuf::from("C:/skins/base.wsz")));
        assert_eq!(args.seconds, Some(3));
    }

    // ----- the local control channel (`--control`, own choice) -----

    #[test]
    fn without_the_control_switch_there_is_no_channel() {
        let args = parse_args(&["--null", "song.ogg"]).unwrap();
        assert_eq!(args.control, None, "the channel is opt-in, never a default");
        assert_eq!(args.control_key(), None);
    }

    #[test]
    fn the_control_switch_without_a_name_uses_the_instance_key() {
        // The channel follows the player it belongs to: the key is the one
        // the single-instance lock is named after.
        let args = parse_args(&["--control"]).unwrap();
        assert_eq!(args.control, Some(Control::Instance));
        assert_eq!(
            args.control_key(),
            Some(instance_key(
                args.instance_class(),
                args.resolved_config_path().as_deref()
            ))
        );
    }

    #[test]
    fn the_control_switch_named_names_its_own_channel() {
        let args = parse_args(&["--control=test-channel.1"]).unwrap();
        assert_eq!(args.control, Some(Control::Named("test-channel.1".into())));
        assert_eq!(args.control_key().as_deref(), Some("test-channel.1"));
    }

    #[test]
    fn an_empty_control_name_is_refused() {
        assert_eq!(
            parse_args(&["--control="]),
            Err(CliError::MissingValue("--control"))
        );
    }

    #[test]
    fn a_control_name_that_is_not_one_path_component_is_refused() {
        // The name is part of `\\.\pipe\rustamp-ctl-<name>`, so a separator
        // would make the channel unreachable or ambiguous.
        for name in ["a\\b", "a/b", "a b", "a:b"] {
            assert_eq!(
                parse_args(&[&format!("--control={name}")]),
                Err(CliError::BadValue {
                    option: "--control",
                    value: name.to_owned(),
                })
            );
        }
    }

    #[test]
    fn the_control_switch_does_not_eat_the_next_argument() {
        // The optional name is behind `=`, so the media argument keeps its
        // meaning (`--control s.mp3` names no channel `s.mp3`).
        let args = parse_args(&["--control", "song.mp3"]).unwrap();
        assert_eq!(args.control, Some(Control::Instance));
        assert_eq!(
            args.media,
            Some(MediaLocation::File(PathBuf::from("song.mp3")))
        );
    }

    #[test]
    fn no_file_is_allowed() {
        let args = parse_args(&["--null"]).unwrap();
        assert_eq!(args.media, None);
        assert_eq!(args.output, OutputChoice::Null);
    }

    #[test]
    fn help_is_recognized_in_both_forms() {
        assert!(parse_args(&["--help"]).unwrap().help);
        assert!(parse_args(&["-h"]).unwrap().help);
    }

    #[test]
    fn a_volume_outside_the_byte_range_is_rejected() {
        assert_eq!(
            parse_args(&["--volume", "256"]),
            Err(CliError::BadValue {
                option: "--volume",
                value: "256".into()
            })
        );
        assert_eq!(
            parse_args(&["--volume", "-1"]),
            Err(CliError::BadValue {
                option: "--volume",
                value: "-1".into()
            })
        );
        assert!(matches!(
            parse_args(&["--volume", "loud"]),
            Err(CliError::BadValue { .. })
        ));
    }

    #[test]
    fn a_missing_value_is_reported() {
        assert_eq!(
            parse_args(&["--skin"]),
            Err(CliError::MissingValue("--skin"))
        );
        assert_eq!(
            parse_args(&["--volume"]),
            Err(CliError::MissingValue("--volume"))
        );
    }

    #[test]
    fn unknown_options_and_extra_files_are_rejected() {
        assert_eq!(
            parse_args(&["--frobnicate"]),
            Err(CliError::UnknownOption("--frobnicate".into()))
        );
        assert_eq!(
            parse_args(&["a.mp3", "b.mp3"]),
            Err(CliError::TooManyFiles("b.mp3".into()))
        );
    }

    #[test]
    fn the_usage_text_names_every_option() {
        for option in [
            "--null",
            "--no-audio",
            "--volume",
            "--skin",
            "--seconds",
            "--enqueue",
            "--control",
            "--import-winamp-ini",
            "--help",
            "/CLASS=",
            "/CONFIG=",
            "/BOOKMARK",
            "/ADD",
            "/INSTALL",
            "/UNREG",
            "/NOREG",
            "/ONLYINSTALL",
        ] {
            assert!(USAGE.contains(option), "usage does not mention {option}");
        }
        // The URL rule is part of the usage: the user has to know that an
        // `http://` argument is streamed and that another scheme is refused.
        assert!(
            USAGE.contains("http://"),
            "usage does not mention the URL rule"
        );
    }

    // ----- S12: the Winamp configuration import -----

    /// The import switch without a path asks for the file: the native
    /// dialog needs a window, so the headless run cannot answer it.
    #[test]
    fn the_import_switch_without_a_path_asks_for_the_file() {
        let args = parse_args(&["--import-winamp-ini"]).unwrap();
        assert_eq!(args.import_winamp_ini, WinampIniImport::Ask);
        assert_eq!(args.media, None);
    }

    /// The path behind the switch is the file to import.
    #[test]
    fn the_import_switch_names_the_file_to_import() {
        let args = parse_args(&["--import-winamp-ini", "C:/old/winamp.ini"]).unwrap();
        assert_eq!(
            args.import_winamp_ini,
            WinampIniImport::File(PathBuf::from("C:/old/winamp.ini"))
        );
        assert_eq!(args.media, None);
    }

    /// Another option behind the switch is not a path: the switch asks for
    /// the file and the option keeps its own meaning.
    #[test]
    fn an_option_behind_the_import_switch_is_not_a_path() {
        let args = parse_args(&["--import-winamp-ini", "--null"]).unwrap();
        assert_eq!(args.import_winamp_ini, WinampIniImport::Ask);
        assert_eq!(args.output, OutputChoice::Null);
    }

    /// A reference switch behind the import switch is not a path either:
    /// the two switch families must not eat each other (S13 §1 against
    /// S12).
    #[test]
    fn a_reference_switch_behind_the_import_switch_is_not_a_path() {
        let args = parse_args(&["--import-winamp-ini", "/CLASS=Foo"]).unwrap();
        assert_eq!(args.import_winamp_ini, WinampIniImport::Ask);
        assert_eq!(args.class.as_deref(), Some("Foo"));
    }

    /// A token the parser refuses as an unknown option is not a path
    /// either: the import switch asks for the file and the option is
    /// refused.
    #[test]
    fn an_unknown_option_behind_the_import_switch_is_not_a_path() {
        assert_eq!(
            parse_args(&["--import-winamp-ini", "--frobnicate"]),
            Err(CliError::UnknownOption("--frobnicate".into()))
        );
    }

    /// Without the switch nothing is imported.
    #[test]
    fn no_import_switch_means_no_import() {
        let args = parse_args(&["--null"]).unwrap();
        assert_eq!(args.import_winamp_ini, WinampIniImport::None);
    }

    // ----- S13: the reference switches -----

    #[test]
    fn class_takes_the_value_after_the_equals_sign() {
        let args = parse_args(&["/CLASS=Rustamp second"]).unwrap();
        assert_eq!(args.class.as_deref(), Some("Rustamp second"));
        assert_eq!(
            args.instance_class(),
            "Rustamp second",
            "the class must reach the single-instance check"
        );
    }

    #[test]
    fn the_default_class_is_the_main_windows_class() {
        let args = parse_args(&[]).unwrap();
        assert_eq!(args.class, None);
        assert_eq!(args.instance_class(), "Winamp v1.x");
    }

    #[test]
    fn an_empty_class_is_refused() {
        assert_eq!(
            parse_args(&["/CLASS="]),
            Err(CliError::MissingValue("/CLASS"))
        );
    }

    #[test]
    fn config_takes_the_value_after_the_equals_sign() {
        let args = parse_args(&["/CONFIG=C:/other/rustamp.toml"]).unwrap();
        assert_eq!(args.config, Some(PathBuf::from("C:/other/rustamp.toml")));
    }

    #[test]
    fn the_config_switch_replaces_the_default_path() {
        let args = parse_args(&["/CONFIG=C:/other/rustamp.toml"]).unwrap();
        assert_eq!(
            args.resolved_config_path(),
            Some(PathBuf::from("C:/other/rustamp.toml"))
        );
    }

    #[test]
    fn bookmark_takes_the_path_that_follows_it() {
        let args = parse_args(&["/BOOKMARK", "C:/music/a.mp3"]).unwrap();
        assert_eq!(
            args.bookmark,
            Some(Bookmark::Location(MediaLocation::File(PathBuf::from(
                "C:/music/a.mp3"
            ))))
        );
    }

    #[test]
    fn bookmark_also_accepts_the_quoted_whole_form() {
        // S13 §1: the shell registers the bookmark form as one quoted
        // argument, so the path can arrive in the same argument.
        let args = parse_args(&["/BOOKMARK C:/music/a.mp3"]).unwrap();
        assert_eq!(
            args.bookmark,
            Some(Bookmark::Location(MediaLocation::File(PathBuf::from(
                "C:/music/a.mp3"
            ))))
        );
    }

    #[test]
    fn a_bookmark_without_a_path_is_recorded_as_such() {
        // S13 §5.3 leaves the form open; the parser does not guess.
        let args = parse_args(&["/BOOKMARK"]).unwrap();
        assert_eq!(args.bookmark, Some(Bookmark::WithoutPath));
    }

    #[test]
    fn a_bookmark_does_not_swallow_the_next_switch() {
        let args = parse_args(&["/BOOKMARK", "/INSTALL"]).unwrap();
        assert_eq!(args.bookmark, Some(Bookmark::WithoutPath));
        assert!(args.registration.install);
    }

    #[test]
    fn a_bookmark_does_not_swallow_a_rustamp_switch_either() {
        let args = parse_args(&["/BOOKMARK", "--volume", "128"]).unwrap();
        assert_eq!(args.bookmark, Some(Bookmark::WithoutPath));
        assert_eq!(args.volume, Some(128));
    }

    #[test]
    fn the_registration_switches_are_parsed() {
        let args = parse_args(&["/INSTALL"]).unwrap();
        assert!(args.registration.install);
        assert!(!args.registration.unreg);
        assert!(!args.registration.noreg);
        assert!(!args.registration.only_install);

        let args = parse_args(&["/UNREG"]).unwrap();
        assert!(args.registration.unreg);

        let args = parse_args(&["/NOREG"]).unwrap();
        assert!(args.registration.noreg);

        let args = parse_args(&["/ONLYINSTALL"]).unwrap();
        assert!(args.registration.only_install);
    }

    #[test]
    fn an_unknown_slash_token_is_a_file_argument() {
        // S13 §1: a token the reference's switch list does not name is not
        // a switch, it is a file argument.
        let args = parse_args(&["/UNKNOWN"]).unwrap();
        assert_eq!(
            args.media,
            Some(MediaLocation::File(PathBuf::from("/UNKNOWN")))
        );
        assert_eq!(args.class, None);
        assert_eq!(args.registration, RegistrationFlags::default());
    }

    #[test]
    fn the_switches_the_reference_does_not_have_are_file_arguments() {
        // S13 §1: the negative list is scoped to the reference's own switch
        // list (S13 §5.13). `/ADD` is not on it — it is a switch.
        for token in ["/PLAY", "/NEW", "/EJECT", "/TASKBAR"] {
            let args = parse_args(&[token]).unwrap();
            assert_eq!(
                args.media,
                Some(MediaLocation::File(PathBuf::from(token))),
                "{token} must not be a switch"
            );
        }
    }

    #[test]
    fn switches_and_paths_are_mixed() {
        let args = parse_args(&["/CLASS=Foo", "song.mp3", "/NOREG"]).unwrap();
        assert_eq!(args.class.as_deref(), Some("Foo"));
        assert_eq!(
            args.media,
            Some(MediaLocation::File(PathBuf::from("song.mp3")))
        );
        assert!(args.registration.noreg);
    }

    #[test]
    fn enqueue_adds_the_file_instead_of_playing_it() {
        let args = parse_args(&["--enqueue", "song.mp3"]).unwrap();
        assert_eq!(args.enqueue, Enqueue::Argument);
        assert_eq!(
            args.media,
            Some(MediaLocation::File(PathBuf::from("song.mp3")))
        );
    }

    // ----- S13 §1: the `/ADD` switch (second audit pass, 2026-10-20) -----

    #[test]
    fn add_takes_the_path_that_follows_it() {
        // S13 §1: `/ADD <path>` takes its path in the argument that follows
        // the switch — the same shape as `/BOOKMARK`.
        let args = parse_args(&["/ADD", "C:/music/a.mp3"]).unwrap();
        assert_eq!(
            args.enqueue,
            Enqueue::Location(MediaLocation::File(PathBuf::from("C:/music/a.mp3")))
        );
    }

    #[test]
    fn an_added_path_is_not_the_file_that_is_played() {
        // The path of `/ADD` is enqueued; it is not the file the player
        // loads and plays.
        let args = parse_args(&["/ADD", "C:/music/a.mp3"]).unwrap();
        assert_eq!(args.media, None);
    }

    #[test]
    fn add_also_accepts_the_quoted_whole_form() {
        // S13 §1: the shell registers the verb with `"%s" /ADD "%%1"`, so
        // the switch and its path can arrive in one argument.
        let args = parse_args(&["/ADD C:/music/a.mp3"]).unwrap();
        assert_eq!(
            args.enqueue,
            Enqueue::Location(MediaLocation::File(PathBuf::from("C:/music/a.mp3")))
        );
    }

    #[test]
    fn the_quotes_of_a_registered_path_are_taken_off() {
        // The registered command line quotes the path, so a path that
        // arrives inside a quoted argument keeps its quotes.
        let args = parse_args(&["/ADD \"C:/music/a.mp3\""]).unwrap();
        assert_eq!(
            args.enqueue,
            Enqueue::Location(MediaLocation::File(PathBuf::from("C:/music/a.mp3")))
        );
    }

    #[test]
    fn an_add_without_a_path_is_recorded_as_such() {
        // S13 §5.3 leaves the form open; the parser does not guess.
        let args = parse_args(&["/ADD"]).unwrap();
        assert_eq!(args.enqueue, Enqueue::WithoutPath);
        assert_eq!(args.media, None);
    }

    #[test]
    fn an_add_does_not_swallow_the_next_switch() {
        let args = parse_args(&["/ADD", "/INSTALL"]).unwrap();
        assert_eq!(args.enqueue, Enqueue::WithoutPath);
        assert!(args.registration.install);
    }

    #[test]
    fn an_add_does_not_swallow_a_rustamp_switch_either() {
        let args = parse_args(&["/ADD", "--volume", "128"]).unwrap();
        assert_eq!(args.enqueue, Enqueue::WithoutPath);
        assert_eq!(args.volume, Some(128));
    }

    #[test]
    fn an_add_and_a_file_argument_can_be_mixed() {
        // S13 §1: a command line may carry several paths, and the switches
        // and the paths are mixed.
        let args = parse_args(&["/ADD", "C:/music/a.mp3", "song.mp3"]).unwrap();
        assert_eq!(
            args.enqueue,
            Enqueue::Location(MediaLocation::File(PathBuf::from("C:/music/a.mp3")))
        );
        assert_eq!(
            args.media,
            Some(MediaLocation::File(PathBuf::from("song.mp3")))
        );
    }

    // ----- S19: the URL on the command line -----

    #[test]
    fn an_http_url_argument_is_a_url() {
        // S19 §1: the request template is plain `http://`, so an `http://`
        // URL names a stream the player can read.
        let args = parse_args(&["http://example.test/stream.mp3"]).unwrap();
        assert_eq!(
            args.media,
            Some(MediaLocation::Url(
                "http://example.test/stream.mp3".to_owned()
            ))
        );
    }

    #[test]
    fn the_scheme_of_a_url_is_not_case_sensitive() {
        // A URL's scheme is case-insensitive (RFC 3986), and the reader of
        // the stream compares it the same way.
        let args = parse_args(&["HTTP://example.test/stream.mp3"]).unwrap();
        assert_eq!(
            args.media,
            Some(MediaLocation::Url(
                "HTTP://example.test/stream.mp3".to_owned()
            ))
        );
    }

    #[test]
    fn an_https_url_is_a_stream_too() {
        // `https` is **our own extension**, not an S19 fact: S19 §1's template
        // is plain `http://`, and §6 question 8 leaves the other schemes open.
        // The rule is the same one that reads `http://`, so the URL is a URL
        // and not a file name.
        let args = parse_args(&["https://example.test/stream.mp3"]).unwrap();
        assert_eq!(
            args.media,
            Some(MediaLocation::Url(
                "https://example.test/stream.mp3".to_owned()
            ))
        );
    }

    #[test]
    fn a_scheme_this_build_cannot_read_is_refused_by_name() {
        // The refusal names the scheme; the URL is not taken for a file name.
        let error = parse_args(&["ftp://example.test/stream.mp3"]).unwrap_err();
        assert_eq!(
            error,
            CliError::UnsupportedUrl {
                url: "ftp://example.test/stream.mp3".to_owned(),
                scheme: "ftp".to_owned(),
            }
        );
        let message = error.to_string();
        assert!(message.contains("ftp"), "{message}");
        assert!(message.contains("https://"), "{message}");
    }

    #[test]
    fn another_scheme_is_refused_by_name() {
        // S19 §6 question 8 asks about the other schemes too, and the
        // answer is the same: only `http://` is read.
        let error = parse_args(&["ftp://example.test/stream.mp3"]).unwrap_err();
        assert_eq!(
            error,
            CliError::UnsupportedUrl {
                url: "ftp://example.test/stream.mp3".to_owned(),
                scheme: "ftp".to_owned(),
            }
        );
        assert!(error.to_string().contains("ftp"));
    }

    #[test]
    fn an_http_url_without_a_host_is_refused() {
        assert_eq!(
            parse_args(&["http://"]),
            Err(CliError::MalformedUrl("http://".to_owned()))
        );
    }

    #[test]
    fn a_path_stays_a_path() {
        let args = parse_args(&["song.mp3"]).unwrap();
        assert_eq!(
            args.media,
            Some(MediaLocation::File(PathBuf::from("song.mp3")))
        );
        let args = parse_args(&[r"C:\Music\song.mp3"]).unwrap();
        assert_eq!(
            args.media,
            Some(MediaLocation::File(PathBuf::from(r"C:\Music\song.mp3")))
        );
    }

    #[test]
    fn a_drive_letter_is_not_a_scheme() {
        // `C://Music` is a Windows path, not a URL with the scheme "c".
        let args = parse_args(&["C://Music/song.mp3"]).unwrap();
        assert_eq!(
            args.media,
            Some(MediaLocation::File(PathBuf::from("C://Music/song.mp3")))
        );
    }

    #[test]
    fn the_add_switch_reads_a_url_the_same_way() {
        // The rule is one rule: every path the command line names is read by
        // `media_argument`, so `/ADD` can enqueue a stream as well.
        let args = parse_args(&["/ADD", "http://example.test/stream.mp3"]).unwrap();
        assert_eq!(
            args.enqueue,
            Enqueue::Location(MediaLocation::Url(
                "http://example.test/stream.mp3".to_owned()
            ))
        );
    }

    #[test]
    fn the_add_switch_refuses_a_scheme_it_cannot_read() {
        let error = parse_args(&["/ADD", "ftp://example.test/stream.mp3"]).unwrap_err();
        assert_eq!(
            error,
            CliError::UnsupportedUrl {
                url: "ftp://example.test/stream.mp3".to_owned(),
                scheme: "ftp".to_owned(),
            }
        );
    }

    #[test]
    fn the_bookmark_switch_reads_a_url_the_same_way() {
        let args = parse_args(&["/BOOKMARK", "http://example.test/stream.mp3"]).unwrap();
        assert_eq!(
            args.bookmark,
            Some(Bookmark::Location(MediaLocation::Url(
                "http://example.test/stream.mp3".to_owned()
            )))
        );
    }

    // ----- the hand-over to a running instance -----

    #[test]
    fn a_file_is_handed_over_as_its_path() {
        let mut refusals = Vec::new();
        let location = forwarded_location(
            Some(MediaLocation::File(PathBuf::from("song.mp3"))),
            &mut refusals,
        );
        assert_eq!(
            location,
            Some(MediaLocation::File(PathBuf::from("song.mp3")))
        );
        assert!(refusals.is_empty(), "{refusals:?}");
    }

    #[test]
    fn a_url_is_handed_over_as_its_url() {
        // The frame carries a location, and the kind is in the frame: a URL
        // reaches the running instance instead of being refused.
        let mut refusals = Vec::new();
        let location = forwarded_location(
            Some(MediaLocation::Url(
                "http://example.test/stream.mp3".to_owned(),
            )),
            &mut refusals,
        );
        assert_eq!(
            location,
            Some(MediaLocation::Url(
                "http://example.test/stream.mp3".to_owned()
            ))
        );
        assert!(refusals.is_empty(), "{refusals:?}");
    }

    #[test]
    fn a_refused_scheme_is_refused_with_the_command_lines_wording() {
        // The command line refuses a scheme it cannot read by name (S19 §6
        // question 8 leaves the other schemes open), and the hand-over refuses
        // it with the same words.
        let mut refusals = Vec::new();
        let location = forwarded_location(
            Some(MediaLocation::Url(
                "ftp://example.test/stream.mp3".to_owned(),
            )),
            &mut refusals,
        );
        assert_eq!(location, None);
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert_eq!(
            refusals[0],
            CliError::UnsupportedUrl {
                url: "ftp://example.test/stream.mp3".to_owned(),
                scheme: "ftp".to_owned(),
            }
            .to_string(),
            "the refusal must be the command line's own wording"
        );
        // The command line refuses the same argument with the same words.
        assert_eq!(
            parse_args(&["ftp://example.test/stream.mp3"]),
            Err(CliError::UnsupportedUrl {
                url: "ftp://example.test/stream.mp3".to_owned(),
                scheme: "ftp".to_owned(),
            })
        );
    }

    #[test]
    fn a_hand_over_carries_an_https_url_too() {
        let mut refusals = Vec::new();
        let location = forwarded_location(
            Some(MediaLocation::Url(
                "https://example.test/stream.mp3".to_owned(),
            )),
            &mut refusals,
        );
        assert!(refusals.is_empty(), "{refusals:?}");
        assert_eq!(
            location,
            Some(MediaLocation::Url(
                "https://example.test/stream.mp3".to_owned()
            ))
        );
    }

    #[test]
    fn a_url_that_names_no_host_is_refused() {
        let mut refusals = Vec::new();
        let location = forwarded_location(
            Some(MediaLocation::Url("http://".to_owned())),
            &mut refusals,
        );
        assert_eq!(location, None);
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert_eq!(
            refusals[0],
            CliError::MalformedUrl("http://".to_owned()).to_string()
        );
    }

    #[test]
    fn a_location_that_names_no_scheme_is_refused() {
        // A location that claims to be a URL and names no scheme has nothing
        // to read, so it is refused instead of being read as a stream.
        let mut refusals = Vec::new();
        let location = forwarded_location(
            Some(MediaLocation::Url("song.mp3".to_owned())),
            &mut refusals,
        );
        assert_eq!(location, None);
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert_eq!(
            refusals[0],
            CliError::MalformedUrl("song.mp3".to_owned()).to_string()
        );
    }

    #[test]
    fn the_request_of_a_second_launch_carries_the_url() {
        // The whole way from the parsed command line to the frame: a second
        // launch that names a URL hands it over.
        let args = parse_args(&["http://example.test/stream.mp3"]).unwrap();
        let request = forward_request(&args);
        assert_eq!(
            request.file,
            Some(MediaLocation::Url(
                "http://example.test/stream.mp3".to_owned()
            ))
        );
        assert_eq!(request.added, None);
        assert!(request.refusals.is_empty(), "{:?}", request.refusals);
    }

    #[test]
    fn the_request_of_a_second_launch_carries_a_path() {
        let args = parse_args(&["song.mp3"]).unwrap();
        let request = forward_request(&args);
        assert_eq!(
            request.file,
            Some(MediaLocation::File(PathBuf::from("song.mp3")))
        );
        assert!(request.refusals.is_empty(), "{:?}", request.refusals);
    }

    #[test]
    fn an_added_url_travels_as_an_added_url() {
        // `/ADD <url>` adds the URL to the playlist and does not play it.
        let args = parse_args(&["/ADD", "http://example.test/stream.mp3"]).unwrap();
        let request = forward_request(&args);
        assert_eq!(request.file, None);
        assert_eq!(
            request.added,
            Some(MediaLocation::Url(
                "http://example.test/stream.mp3".to_owned()
            ))
        );
        assert!(request.refusals.is_empty(), "{:?}", request.refusals);
    }
}
