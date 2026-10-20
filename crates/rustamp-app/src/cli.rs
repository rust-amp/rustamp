// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Command-line parsing for the `rustamp` binary (task card N1).
//!
//! The card's CLI is `<file> [--null] [--no-audio] [--volume <0..=255>]
//! [--skin <path>]`. Two additional switches exist for the headless test mode:
//! `--seconds <n>` bounds a run (the same aid the `rustamp-audio` `play`
//! example has) and `--help`/`-h` prints the usage. Parsing is a pure function
//! of the argument list, so it is unit-tested without opening anything.

use std::fmt;

use std::path::PathBuf;

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
}

/// The usage text (`--help`).
pub const USAGE: &str = "\
usage: rustamp [<file>] [--null] [--no-audio] [--volume <0..=255>] [--skin <path>] [--seconds <n>]
           [--import-winamp-ini [path]]

  <file>            media file to load and play (optional)
  --null            use the deterministic null output and run headless (no window)
  --no-audio        open the window but play on the null output (silent; no audio device needed)
  --volume <0..=255> initial volume (default: the config's audio.volume)
  --skin <path>     skin folder, .wsz or .zip (default: config's ui.skin, then built-in)
  --seconds <n>     stop after n seconds of playback (test aid)
  --import-winamp-ini [path]
                    import a Winamp 2.95 winamp.ini (the file is only read); without a path
                    the player asks for the file with the native dialog
  -h, --help        print this help";

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
        match arg.as_str() {
            "--null" => parsed.output = OutputChoice::Null,
            "--no-audio" => parsed.output = OutputChoice::NoAudio,
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
                // switch, when that argument is not another option. Without
                // a path the player asks for the file with the native
                // dialog, so the switch has to come before the file to
                // play.
                let path_follows = matches!(it.peek(), Some(next) if !next.starts_with('-'));
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
            other => {
                if parsed.file.is_some() {
                    return Err(CliError::TooManyFiles(other.to_owned()));
                }
                parsed.file = Some(PathBuf::from(other));
            }
        }
    }
    Ok(parsed)
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
            "--import-winamp-ini",
            "--help",
        ] {
            assert!(USAGE.contains(option), "usage does not mention {option}");
        }
    }

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

    /// Without the switch nothing is imported.
    #[test]
    fn no_import_switch_means_no_import() {
        let args = parse_args(&["--null"]).unwrap();
        assert_eq!(args.import_winamp_ini, WinampIniImport::None);
    }
}
