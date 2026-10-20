// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Command-line parsing for the `rustamp` binary (task card N1, reconciled
//! with S13).
//!
//! # The two switch families
//!
//! The command line carries two families of switches:
//!
//! * the **reference switches** of S13 §1 — `/CLASS=`, `/CONFIG=`,
//!   `/BOOKMARK`, `/INSTALL`, `/UNREG`, `/NOREG`, `/ONLYINSTALL` — spelled
//!   with a leading `/`, exactly as the audited spec records them;
//! * **Rustamp's own switches** (`--null`, `--no-audio`, `--volume`, `--skin`,
//!   `--seconds`, `--enqueue`, `--import-winamp-ini`, `--help`), spelled with
//!   a leading `--`. They are the product's own test and convenience aids; the
//!   reference has no such switches.
//!
//! S13 §1's list is complete for the reference, so nothing is added to the
//! `/` family: `/ADD`, `/PLAY`, `/NEW`, `/EJECT` and `/TASKBAR` are **not**
//! switches here.
//!
//! A leading `/` starts a switch, `=` separates a switch's value, and
//! **everything else is a file argument** (S13 §1). An unknown `/` token is
//! therefore a file argument and not an error — that is the spec's rule, and
//! it is why `/ADD` and friends land in the playlist instead of being
//! refused.
//!
//! The optional path of `--import-winamp-ini` is the one place where the two
//! families meet: the path is the argument behind the switch, when that
//! argument is not a switch of either family. So the import switch never
//! swallows a `/CLASS=` or a `--null`, and it has to come **before** the file
//! to play.
//!
//! # Conformance with S13
//!
//! | S13 fact | Our behaviour | State | Where |
//! |---|---|---|---|
//! | §1 `/CLASS=<name>` overrides the class the single-instance check looks for | [`Args::class`] carries the value; [`Args::instance_class`] hands it to the check | matches | `cli.rs`, [`rustamp_platform::single_instance`] |
//! | §2 the default class is the main window's class `Winamp v1.x` (S09 §1, §8 item 1) | [`rustamp_platform::single_instance::DEFAULT_CLASS`] | matches | `rustamp-platform/src/single_instance.rs` |
//! | §2 the class name is case-sensitive | the class half of the instance key is not case-folded | matches | `rustamp-platform/src/single_instance.rs` |
//! | §1 `/CONFIG=<path>` is the configuration file to use instead of the default | [`Args::config`] replaces the resolved default | matches | `cli.rs`, `lib.rs` |
//! | §1 `/BOOKMARK <path>` adds the path as a bookmark rather than playing it | [`Args::bookmark`] carries the path; the run adds it and does not play it | matches (see the note below) | `cli.rs`, `lib.rs` |
//! | §1 `/INSTALL` is the installer association step | [`RegistrationFlags::install`] | matches | `rustamp-platform/src/shell_registration.rs` |
//! | §1 `/UNREG` removes the shell registration | [`RegistrationFlags::unreg`] | matches | `rustamp-platform/src/shell_registration.rs` |
//! | §1 `/NOREG` skips the registration step | [`RegistrationFlags::noreg`] | matches | `rustamp-platform/src/shell_registration.rs` |
//! | §1 `/ONLYINSTALL` is the install step, then exit | [`RegistrationFlags::only_install`] | matches | `main.rs` |
//! | §1 no `/ADD`, `/PLAY`, `/NEW`, `/EJECT`, `/TASKBAR` | not implemented; such a token is a file argument | matches | `cli.rs` |
//! | §1 a command line may carry several paths | the parser takes one file argument and refuses a second | differs (own choice) | `cli.rs` |
//! | §2 a second start looks for a **window** of that class | the check is a named mutex named after the class | differs (own choice, S13 §5.8) | `rustamp-platform/src/single_instance.rs` |
//! | §2 the player registers a window class, and the class name is what `/CLASS=` overrides | the registered main-window class stays `Winamp v1.x` (S09 §1 and §8 item 1: an ABI fact plugins and external tools depend on); `/CLASS=` overrides the class the **check** looks for | own choice (the task's wording: "the class the single-instance check looks for") | `rustamp-app/src/window.rs`, `rustamp-platform/src/single_instance.rs` |
//! | §2 the hand-over is a window message | the hand-over travels over a named pipe | differs (own choice, S13 §5.8) | `rustamp-platform/src/single_instance.rs` |
//! | §3 the four verbs per media and playlist file | four verbs each | matches | `rustamp-platform/src/shell_registration.rs` |
//! | §3 the two verbs per directory | `Play` and `Enqueue` | matches | `rustamp-platform/src/shell_registration.rs` |
//! | §3 the one verb of a skin archive | `install` | matches | `rustamp-platform/src/shell_registration.rs` |
//!
//! The note on `/BOOKMARK`: the reference adds a **bookmark**, and Rustamp has
//! no bookmark store yet (RFC 0005 O12). The path is therefore added to the
//! playlist **without starting playback** — added rather than played, which is
//! what S13 §1 settles — and the run says that the bookmark store does not
//! exist yet.
//!
//! The open questions of S13 §5 stay open here: the in-window drop rule
//! (§5.1), the drop of a folder (§5.2), `/BOOKMARK` without a path (§5.3),
//! whether `/CONFIG=` moves the skin directories (§5.4), what a busy instance
//! does (§5.5), whether `/CONFIG=` affects the class (§5.6), the reference's
//! own default for `/CLASS=` (§5.7), the hand-over message (§5.8), the
//! quoting of the bookmark command line (§5.9), the directory identifiers
//! (§5.10), the display names of the other verbs (§5.11) and what the
//! `DropTarget` entries carry (§5.12).

use std::fmt;

use std::path::PathBuf;

use rustamp_platform::shell_registration::RegistrationFlags;

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
    /// `/BOOKMARK <path>`: the path is added as a bookmark rather than played.
    Path(PathBuf),
    /// `/BOOKMARK` without a path.
    ///
    /// S13 §5.3 leaves this form open, so the player says so and does
    /// nothing rather than guessing what the reference does.
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

/// The parsed command line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Args {
    /// The file to load and play. `None` starts with nothing loaded (S02 §3
    /// shows the program name in the marquee).
    pub file: Option<PathBuf>,
    /// Output device choice.
    pub output: OutputChoice,
    /// Initial volume `0..=255`; `None` uses the config's `audio.volume`.
    pub volume: Option<u8>,
    /// Skin path (folder, `.wsz` or `.zip`); `None` uses the config's
    /// `ui.skin`, then the built-in default skin.
    pub skin: Option<PathBuf>,
    /// Stop after this many seconds of playback (test aid).
    pub seconds: Option<u64>,
    /// The Winamp configuration to import (S12).
    pub import_winamp_ini: WinampIniImport,
    /// `--help`/`-h` was given.
    pub help: bool,
    /// The file is added to the playlist instead of being played (`--enqueue`).
    pub enqueue: bool,
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
}

/// The usage text (`--help`).
pub const USAGE: &str = "\
usage: rustamp [<file>] [--null] [--no-audio] [--volume <0..=255>] [--skin <path>]
                 [--seconds <n>] [--enqueue] [--import-winamp-ini [path]] [-h|--help]
       rustamp [/CLASS=<name>] [/CONFIG=<path>] [/BOOKMARK <path>]
              [/INSTALL] [/UNREG] [/NOREG] [/ONLYINSTALL]

  <file>            media file to load and play (optional)
  --null            use the deterministic null output and run headless (no window)
  --no-audio        open the window but play on the null output (silent; no audio device needed)
  --volume <0..=255> initial volume (default: the config's audio.volume)
  --skin <path>     skin folder, .wsz or .zip (default: config's ui.skin, then built-in)
  --seconds <n>     stop after n seconds of playback (test aid)
  --enqueue         add the file to the playlist instead of playing it
  --import-winamp-ini [path]
                    import a Winamp 2.95 winamp.ini (the file is only read); without a path
                    the player asks for the file with the native dialog
  -h, --help        print this help

  /CLASS=<name>     the window class the single-instance check looks for
  /CONFIG=<path>    the configuration file to use instead of the default
  /BOOKMARK <path>  add the path as a bookmark rather than playing it
  /INSTALL          write the shell registration
  /UNREG            remove the shell registration
  /NOREG            skip the registration step
  /ONLYINSTALL      write the shell registration, then exit";

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
                "BOOKMARK" => parsed.bookmark = Some(bookmark_path(&mut it)),
                _ => {
                    if let Some(value) = rest.strip_prefix("CLASS=") {
                        parsed.class = Some(class_value(value)?);
                    } else if let Some(value) = rest.strip_prefix("CONFIG=") {
                        parsed.config = Some(PathBuf::from(value));
                    } else if let Some(path) = rest.strip_prefix("BOOKMARK ") {
                        // The shell registers the bookmark form as one quoted
                        // argument (S13 §1), so the path can arrive in the
                        // same argument as the switch.
                        parsed.bookmark = Some(Bookmark::Path(PathBuf::from(path)));
                    } else {
                        // Anything else is a file argument (S13 §1): the
                        // switch list is complete, so an unknown `/` token
                        // is not a switch at all.
                        set_file(&mut parsed, arg)?;
                    }
                }
            }
            continue;
        }
        match arg.as_str() {
            "--null" => parsed.output = OutputChoice::Null,
            "--no-audio" => parsed.output = OutputChoice::NoAudio,
            "--enqueue" => parsed.enqueue = true,
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
            other => set_file(&mut parsed, other.to_owned())?,
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

/// The path of `/BOOKMARK`: the next argument, when there is one.
///
/// A missing path is left open by S13 §5.3, so the parser records that the
/// switch was given without a path instead of guessing.
fn bookmark_path<I>(it: &mut std::iter::Peekable<I>) -> Bookmark
where
    I: Iterator<Item = String>,
{
    match it.peek() {
        Some(next) if !is_switch(next) => {
            let path = it.next().expect("peeked");
            Bookmark::Path(PathBuf::from(path))
        }
        _ => Bookmark::WithoutPath,
    }
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

/// Records the file argument; a second one is refused.
fn set_file(parsed: &mut Args, value: String) -> Result<(), CliError> {
    if parsed.file.is_some() {
        return Err(CliError::TooManyFiles(value));
    }
    parsed.file = Some(PathBuf::from(value));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Args, CliError> {
        parse(args.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn a_bare_file_is_the_input() {
        let args = parse_args(&["music.mp3"]).unwrap();
        assert_eq!(args.file, Some(PathBuf::from("music.mp3")));
        assert_eq!(args.output, OutputChoice::Default);
        assert_eq!(args.volume, None);
        assert_eq!(args.skin, None);
        assert!(!args.help);
    }

    #[test]
    fn null_selects_the_test_output() {
        let args = parse_args(&["--null", "music.flac"]).unwrap();
        assert_eq!(args.output, OutputChoice::Null);
        assert_eq!(args.file, Some(PathBuf::from("music.flac")));
    }

    #[test]
    fn no_audio_selects_the_silent_windowed_output() {
        let args = parse_args(&["--no-audio", "music.flac"]).unwrap();
        assert_eq!(args.output, OutputChoice::NoAudio);
        assert_eq!(args.file, Some(PathBuf::from("music.flac")));
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

    #[test]
    fn no_file_is_allowed() {
        let args = parse_args(&["--null"]).unwrap();
        assert_eq!(args.file, None);
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
            "--import-winamp-ini",
            "--help",
            "/CLASS=",
            "/CONFIG=",
            "/BOOKMARK",
            "/INSTALL",
            "/UNREG",
            "/NOREG",
            "/ONLYINSTALL",
        ] {
            assert!(USAGE.contains(option), "usage does not mention {option}");
        }
    }

    // ----- S12: the Winamp configuration import -----

    /// The import switch without a path asks for the file: the native
    /// dialog needs a window, so the headless run cannot answer it.
    #[test]
    fn the_import_switch_without_a_path_asks_for_the_file() {
        let args = parse_args(&["--import-winamp-ini"]).unwrap();
        assert_eq!(args.import_winamp_ini, WinampIniImport::Ask);
        assert_eq!(args.file, None);
    }

    /// The path behind the switch is the file to import.
    #[test]
    fn the_import_switch_names_the_file_to_import() {
        let args = parse_args(&["--import-winamp-ini", "C:/old/winamp.ini"]).unwrap();
        assert_eq!(
            args.import_winamp_ini,
            WinampIniImport::File(PathBuf::from("C:/old/winamp.ini"))
        );
        assert_eq!(args.file, None);
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
            Some(Bookmark::Path(PathBuf::from("C:/music/a.mp3")))
        );
    }

    #[test]
    fn bookmark_also_accepts_the_quoted_whole_form() {
        // S13 §1: the shell registers the bookmark form as one quoted
        // argument, so the path can arrive in the same argument.
        let args = parse_args(&["/BOOKMARK C:/music/a.mp3"]).unwrap();
        assert_eq!(
            args.bookmark,
            Some(Bookmark::Path(PathBuf::from("C:/music/a.mp3")))
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
        // S13 §1: the switch list is complete, so an unknown `/` token is
        // not a switch.
        let args = parse_args(&["/ADD"]).unwrap();
        assert_eq!(args.file, Some(PathBuf::from("/ADD")));
        assert_eq!(args.class, None);
        assert_eq!(args.registration, RegistrationFlags::default());
    }

    #[test]
    fn the_switches_the_reference_does_not_have_are_file_arguments() {
        for token in ["/ADD", "/PLAY", "/NEW", "/EJECT", "/TASKBAR"] {
            let args = parse_args(&[token]).unwrap();
            assert_eq!(
                args.file,
                Some(PathBuf::from(token)),
                "{token} must not be a switch"
            );
        }
    }

    #[test]
    fn switches_and_paths_are_mixed() {
        let args = parse_args(&["/CLASS=Foo", "song.mp3", "/NOREG"]).unwrap();
        assert_eq!(args.class.as_deref(), Some("Foo"));
        assert_eq!(args.file, Some(PathBuf::from("song.mp3")));
        assert!(args.registration.noreg);
    }

    #[test]
    fn enqueue_adds_the_file_instead_of_playing_it() {
        let args = parse_args(&["--enqueue", "song.mp3"]).unwrap();
        assert!(args.enqueue);
        assert_eq!(args.file, Some(PathBuf::from("song.mp3")));
    }
}
