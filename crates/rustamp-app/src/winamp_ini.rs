// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Import of a Winamp 2.95 `winamp.ini` into our configuration (S12).
//!
//! A user who switches over from the reference player brings a `winamp.ini`
//! with them. This module reads that file and maps the settings it carries
//! onto ours. It is deliberately split in two:
//!
//! * the **parser** ([`IniFile`]) reads the file's shape: `[Section]`
//!   headers, `key=value` pairs, an empty value written as `key=`, CRLF line
//!   endings, no quoting and no escaping (S12 §2). It is pure: no file
//!   system, no configuration, no player.
//! * the **mapper** ([`plan`]) turns the parsed file into a [`Config`] and a
//!   report. It is pure as well: the one thing it cannot decide on its own —
//!   which file the classic skin name refers to — is handed in as a
//!   [`SkinResolver`].
//!
//! The file is only ever **read**. Nothing here writes to it, and the
//! configuration it produces is ours (`rustamp.toml`), not the reference's.
//!
//! # The mapping table
//!
//! The table is the honest core of this module. It says what a key of the
//! reference file becomes in our configuration, what we cannot honour, and
//! which of our settings the reference has no key for.
//!
//! ## The keys we honour
//!
//! | Reference key | Our setting | How | Basis |
//! |---|---|---|---|
//! | `volume` | `audio.volume` | the value, clamped to `0..=255` | S12 §3.2, §9 |
//! | `pan` | `audio.balance` | the value, clamped to `-127..=127`, `0` = centre | S12 §3.2 — **our reading**, see below |
//! | `use_eq` | `eq.state.enabled` | `0` = off, any other number = on | S12 §3.2, S01 §3.1 |
//! | `autoload_eq` | `eq.state.auto` | `0` = off, any other number = on | S12 §3.2, S01 §3.1 |
//! | `preamp` | `eq.state.preamp` | the classic `0..=63` value as a gain in dB | S12 §3.2, S07 §7 |
//! | `eq_data` | `eq.state.bands` | up to ten classic `0..=63` values as gains in dB | S12 §6 |
//! | `shuffle` | `playlist.shuffle` | `0` = off, any other number = on | S12 §3.3, S15 |
//! | `repeat` | `playlist.repeat` | `0` = off, any other number = on | S12 §3.3, S15 |
//! | `pe_fontsize` | `ui.playlist_font_size` | the value, when it is a size the playlist can use | S12 §3.3, S03 §2 |
//! | `wx`, `wy` | `ui.windows[Main].x` / `.y` | the value | S12 §3.4 |
//! | `eq_wx`, `eq_wy` | `ui.windows[Equalizer].x` / `.y` | the value | S12 §3.4 |
//! | `pe_wx`, `pe_wy` | `ui.windows[Playlist].x` / `.y` | the value | S12 §3.4 |
//! | `video_wx`, `video_wy` | `ui.windows[Video].x` / `.y` | the value | S12 §3.4 |
//! | `mw_open` | `ui.windows[Main].visible` | `0` = hidden, any other number = shown | S12 §3.4 |
//! | `eq_open` | `ui.windows[Equalizer].visible` | `0` = hidden, any other number = shown | S12 §3.4 |
//! | `pe_open` | `ui.windows[Playlist].visible` | `0` = hidden, any other number = shown | S12 §3.4 |
//! | `video_open` | `ui.windows[Video].visible` | `0` = hidden, any other number = shown | S12 §3.4 |
//! | `windowshade` | `ui.windows[Main].shaded` | `0` = not shaded, any other number = shaded | S12 §3.4 |
//! | `eq_ws` | `ui.windows[Equalizer].shaded` | `0` = not shaded, any other number = shaded | S12 §3.4 |
//! | `pe_width`, `pe_height` | `ui.windows[Playlist].width` / `.height` | the value | S12 §3.4 |
//! | `video_width`, `video_height` | `ui.windows[Video].width` / `.height` | the value | S12 §3.4 |
//! | `aot` | `ui.always_on_top` | `0` = off, any other number = on | S12 §3.4 |
//! | `dsize` | `ui.double_size` | `0` = off, any other number = on | S12 §3.4 |
//! | `easymove` | `ui.easy_move` | `0` = off, any other number = on | S12 §3.4 |
//! | `skin` | `ui.skin` | the file name, resolved in our skins folder | S12 §3.1 |
//!
//! ### The two readings that are ours and not the spec's
//!
//! * **`pan`.** The spec is explicit that the file's balance key has an
//!   **open range and an open direction** (S12 §3.2 and its open question 4):
//!   one sample (`pan=0`) cannot settle either. We read the key as a signed
//!   balance in our own domain: `0` is the centre, a negative value is the
//!   left, and the value is clamped to `-127..=127`. The *magnitude* is
//!   therefore applied as the reference recorded it, but the *side* rests on
//!   our convention (the contract's `balance` is negative = left) and not on
//!   a measurement. The report names the key as applied, so the user can see
//!   that the balance came from the file.
//! * **The classic `0..=63` gain scale.** The preamp and the band values are
//!   saved in the slider's own unit (`0..=63`). We turn them into dB with
//!   [`rustamp_ui::saved_to_gain_db`], the conversion the equalizer window
//!   already uses (`0` = +12 dB, `31` = flat, `63` = −12 dB; S07 §7).
//!
//! ### The caveat the spec itself carries
//!
//! `use_eq` is mapped onto the equalizer's enabled state. The spec links the
//! key to the ON button and says the link was not independently re-verified
//! (S12 §3.2, S01 §3.1): the mapping follows the spec's reading, and the
//! caveat travels with it. `autoload_eq` is on firmer ground — S01 §3.1
//! measured that the key toggles exactly the AUTO indicator.
//!
//! ### The main window's visibility
//!
//! `mw_open` hides or shows the main window **in the running player**. The
//! player always opens the main window when it starts — an unavailable tray
//! must never strand the player — so the key is not a "start hidden"
//! instruction. The window list records the state the player was in, the way
//! it records the positions.
//!
//! ## The keys we cannot honour
//!
//! Every key the reference file carries that is not in the table above is
//! **reported**, never dropped silently. The reasons:
//!
//! | Keys | Why not |
//! |---|---|
//! | `outname` | the output plug-in is not ours: the backend comes from the command line |
//! | `randskin`, `cwd`, `defext`, `proxy`, `inet_mode`, `browserbrand`, `langpack`, `extlist`, `check_ft_startup`, `usecursors`, `fixtitles`, `ttips`, `ascb_new`, `taskbar`, `splash`, `sticon`, `riol`, `minst`, `whichicon`, `whichicon2`, `disvis`, `priority`, `ospb`, `bifont`, `rofiob` | the player has no setting for them |
//! | `minimized` | the player does not keep a minimized state |
//! | `keeponscreen`, `parent`, `hilite` | the player has no setting for them |
//! | `snap` | our snapping is a distance, not a flag, and the classic unit is open (S12 §9) |
//! | `snaplen` | the classic length has no established unit or range (S12 §9), so it cannot become our pixel distance |
//! | `eqdsize` | the player has one double-size flag for the whole window set, not one for the equalizer |
//! | `pe_height_ws` | the editor's shaded height is fixed by the layout (S01 §1) |
//! | `shownumsinpl` | the player does not keep a track-number column setting |
//! | `pladv` | the setting behind it (`playlist.manual_advance`) is not applied by the player yet |
//! | `addtolist` | the player has no add-to-list setting |
//! | `mb_wx`, `mb_wy`, `mb_open`, `mb_width`, `mb_height` | the player has no minibrowser window |
//! | `video_ratio1`, `video_ratio2`, `video_useratio`, `video_vsync`, `video_aspectadj`, `video_overlays`, `video_ddraw`, `video_updsize`, `video_autoopen`, `video_autoclose`, `video_noss`, `video_osd`, `video_yv12`, `video_stopclose`, `video_remove_fs_on_stop` | the player has no setting for the video window's playback |
//! | `shuffle_morph_rate` | the player has no shuffle morph rate |
//! | `timeleftmode` | the player does not keep the time display's mode |
//! | `autoscrollname` | the player does not keep the title scroll setting |
//! | `sa`, `safalloff`, `sa_peaks`, `sa_peak_falloff` | the player has no setting for the built-in analyser |
//! | `safire`, `saref` | the reference's own meaning for these keys is unknown (S12 §10) |
//! | `pilp` | the reference's own meaning for this key is unknown (S12 §10) |
//! | `visplugin_num`, `visplugin_name`, `visplugin_autoexec`, `visplugin_priority` | the player does not load visualisation plug-ins yet |
//! | `dspplugin_num`, `dspplugin_name` | the player has no DSP plug-in selection |
//! | `prefs_last_page` | the player does not keep the preferences page. The key carries two numbers (the page index in the low byte, a second number above it, S12 §7); we could only ever honour the low byte |
//! | `last_shortdesc`, `last_shorturl` | the player keeps no shortcut description or URL |
//! | `newverchk`, `newverchk2` | the player does not check for a new version |
//! | every key of a plug-in's own section | the player does not interpret them (S12 §8): the section belongs to the plug-in |
//! | every key the reference does not recognise | the reference file held keys the player does not know (S12 §5.3) |
//!
//! ## Our settings the reference has no key for
//!
//! | Our setting | Why there is no key |
//! |---|---|
//! | `version` | our own schema marker |
//! | `audio.fadeout_ms` | the reference has no fade-out setting |
//! | `audio.output_mode` | the reference's output is a plug-in, not a mode |
//! | `audio.replaygain` | the reference has no ReplayGain setting |
//! | `audio.device` | the reference has no device setting |
//! | `ui.snap_distance` | the reference has `snap` and `snaplen`, but the classic unit and range are open (S12 §9) |
//! | `ui.playlist_title_format` | the reference has no global title-format setting: the format belongs to the input plug-ins (S14 §1) |
//! | `plugins.plugin_dir`, `plugins.disabled` | the reference keeps its plug-in list in its own files |
//! | `audio.output`, `audio.buffer_ms`, `audio.gapless`, `playlist.manual_advance`, `playlist.last_playlist`, `plugins.vis_plugin`, `plugins.vis_preset` | the settings the player does not apply (see `crate::config`) |
//!
//! # The report the page shows
//!
//! The preferences page shows one line after an import: the file that was
//! read, the settings that were taken over and the keys that could not be
//! ([`report_line`]). The keys the player read in its own way are named in
//! that line, so the split of the report survives on the page and the user
//! is not told that the reference's meaning was applied.

//! # What a missing key means
//!
//! The reference reads a missing key as the caller's default, which is the
//! reference's own starting value (S12 §4.2). Our default is **our current
//! configuration**: a key the file does not carry leaves the setting exactly
//! as it is, and the report does not mention it at all (there is nothing to
//! report about a key that is not there).
//!
//! # The file's shape, as far as we read it
//!
//! * The first section is `[Winamp]` and holds the player's own settings;
//!   every other section belongs to a plug-in (S12 §2).
//! * Key and section names are matched without regard to case: the reference
//!   reads the file through the profile API (S12 §4.3), whose names are case
//!   insensitive.
//! * A value is everything behind the first `=` on the line; the whitespace
//!   around the key and the value is dropped.
//! * A line the parser cannot read (no `=`, an unterminated section header,
//!   an empty key) is **reported** and skipped: it never stops the import.
//! * A key that appears twice keeps its **first** value and the repetition is
//!   reported. The spec does not say which value wins, so this is our choice.

use std::path::PathBuf;

use rustamp_core::config::{Config, WindowLayout};
use rustamp_core::state::{EQ_BANDS, WindowId};

/// The section the reference keeps its own settings in (S12 §2).
pub const PLAYER_SECTION: &str = "Winamp";

/// The classic gain scale's lowest value (S12 §9).
pub const CLASSIC_MIN: i64 = 0;

/// The classic gain scale's highest value (S12 §9).
pub const CLASSIC_MAX: i64 = 63;

/// One key of an INI file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IniEntry {
    /// The key's name, as the file wrote it.
    pub key: String,
    /// The key's value: everything behind the first `=`, without the
    /// whitespace around it.
    pub value: String,
}

/// One section of an INI file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IniSection {
    /// The section's name, without the brackets.
    pub name: String,
    /// The section's keys, in the order the file wrote them.
    pub entries: Vec<IniEntry>,
}

impl IniSection {
    /// The value of the first entry named `key`, if the section has one.
    ///
    /// The name is matched without regard to case (see the module docs).
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|entry| entry.key.eq_ignore_ascii_case(key))
            .map(|entry| entry.value.as_str())
    }
}

/// A parsed INI file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IniFile {
    /// The sections, in the order the file wrote them.
    pub sections: Vec<IniSection>,
    /// The lines the parser could not read, each with the reason.
    pub problems: Vec<String>,
}

impl IniFile {
    /// Parses the INI text.
    ///
    /// The parser never fails: a line it cannot read becomes a
    /// [`problem`](Self::problems) and the rest of the file is read as usual.
    pub fn parse(text: &str) -> IniFile {
        let mut file = IniFile::default();
        // The index of the section the keys belong to; `None` means the key
        // came before the first section header.
        let mut current: Option<usize> = None;
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(header) = line.strip_prefix('[') {
                match header.strip_suffix(']') {
                    Some(name) => {
                        let name = name.trim();
                        if name.is_empty() {
                            file.problems.push(format!(
                                "line {}: a section header without a name",
                                number + 1
                            ));
                            continue;
                        }
                        file.sections.push(IniSection {
                            name: name.to_string(),
                            entries: Vec::new(),
                        });
                        current = Some(file.sections.len() - 1);
                    }
                    None => file.problems.push(format!(
                        "line {}: a section header without a closing bracket",
                        number + 1
                    )),
                }
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                file.problems
                    .push(format!("line {}: a line without `=`", number + 1));
                continue;
            };
            let key = key.trim();
            if key.is_empty() {
                file.problems
                    .push(format!("line {}: a key without a name", number + 1));
                continue;
            }
            let value = value.trim().to_string();
            let section = match current {
                Some(index) => index,
                // A key before the first section header belongs to no section:
                // it lands in the unnamed one, and the mapper reports it.
                None => {
                    file.sections.push(IniSection::default());
                    let index = file.sections.len() - 1;
                    current = Some(index);
                    index
                }
            };
            let mut problem = None;
            {
                let entries = &mut file.sections[section].entries;
                if entries
                    .iter()
                    .any(|entry| entry.key.eq_ignore_ascii_case(key))
                {
                    problem = Some(format!(
                        "line {}: the key `{key}` appears more than once; the first value is used",
                        number + 1
                    ));
                } else {
                    entries.push(IniEntry {
                        key: key.to_string(),
                        value,
                    });
                }
            }
            if let Some(problem) = problem {
                file.problems.push(problem);
            }
        }
        file
    }

    /// The section named `name`, if the file has one.
    ///
    /// The name is matched without regard to case (see the module docs).
    pub fn section(&self, name: &str) -> Option<&IniSection> {
        self.sections
            .iter()
            .find(|section| section.name.eq_ignore_ascii_case(name))
    }
}

/// When an imported setting takes effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// The running player takes the setting over at once.
    Now,
    /// The configuration keeps the setting; the player restores the window
    /// placement when it starts.
    NextStart,
    /// The configuration keeps the setting, but the player does not act on
    /// it: a window it does not have, or a size the layout fixes.
    Stored,
}

impl Effect {
    /// The effect, in the words the report uses.
    pub fn description(self) -> &'static str {
        match self {
            Effect::Now => "applied now",
            Effect::NextStart => "saved; the player places the windows at the next start",
            Effect::Stored => "kept in the configuration; the player does not act on it",
        }
    }
}

/// How the player read a key of the reference file.
///
/// The mapping table reads most keys the way the spec states it. Two of
/// them are read in **our** way: the balance's side (S12 §3.2) and the
/// classic gain scale (S07 §7). The report keeps the two apart, so the
/// user is never told that the reference's meaning was applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// The key means what the spec says it means.
    Spec,
    /// The key's meaning is our reading, not the spec's.
    Own,
}

/// What happened to one key of the reference file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The key became one of our settings.
    Applied {
        /// The setting it became, named as in our configuration.
        setting: String,
        /// When the setting takes effect.
        effect: Effect,
        /// How the player read the key.
        reading: Reading,
    },
    /// The key was not applied.
    NotApplied {
        /// Why it was not applied.
        reason: String,
    },
}

/// What the import did with one key of the reference file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyOutcome {
    /// The section the key was in.
    pub section: String,
    /// The key's name, as the file wrote it.
    pub key: String,
    /// The key's value, as the file wrote it.
    pub value: String,
    /// What happened to the key.
    pub outcome: Outcome,
}

/// What an import did with the reference file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// What happened to every key of the file, in the order the file wrote
    /// them.
    pub keys: Vec<KeyOutcome>,
    /// The lines the parser could not read.
    pub problems: Vec<String>,
}

impl ImportReport {
    /// The keys that were applied.
    pub fn applied(&self) -> impl Iterator<Item = &KeyOutcome> {
        self.keys
            .iter()
            .filter(|entry| matches!(entry.outcome, Outcome::Applied { .. }))
    }

    /// The keys that were not applied.
    pub fn not_applied(&self) -> impl Iterator<Item = &KeyOutcome> {
        self.keys
            .iter()
            .filter(|entry| matches!(entry.outcome, Outcome::NotApplied { .. }))
    }

    /// The keys the player read in its own way, not the spec's.
    ///
    /// The module docs name the two readings: the balance's side (S12
    /// §3.2) and the classic gain scale (S07 §7). They are applied like any
    /// other key, but the report names them, so the user sees that these
    /// settings rest on our reading and not on a measurement.
    pub fn own_readings(&self) -> impl Iterator<Item = &KeyOutcome> {
        self.applied().filter(|entry| {
            matches!(
                entry.outcome,
                Outcome::Applied {
                    reading: Reading::Own,
                    ..
                }
            )
        })
    }

    /// Whether the key `key` was applied.
    pub fn applied_key(&self, key: &str) -> bool {
        self.applied()
            .any(|entry| entry.key.eq_ignore_ascii_case(key))
    }

    /// The reason the key `key` was not applied, if it was not.
    pub fn reason_for(&self, key: &str) -> Option<&str> {
        self.not_applied()
            .find(|entry| entry.key.eq_ignore_ascii_case(key))
            .and_then(|entry| match &entry.outcome {
                Outcome::NotApplied { reason } => Some(reason.as_str()),
                Outcome::Applied { .. } => None,
            })
    }

    /// One line per key, for the report the user reads.
    pub fn lines(&self) -> Vec<String> {
        let mut lines = Vec::with_capacity(self.keys.len());
        for entry in &self.keys {
            let line = match &entry.outcome {
                Outcome::Applied {
                    setting,
                    effect,
                    reading,
                } => {
                    let mut line = format!(
                        "{}[{}] = {} -> {setting} ({})",
                        entry.section,
                        entry.key,
                        entry.value,
                        effect.description()
                    );
                    if *reading == Reading::Own {
                        line.push_str("; our own reading, not the spec's");
                    }
                    line
                }
                Outcome::NotApplied { reason } => format!(
                    "{}[{}] = {} -> not applied: {reason}",
                    entry.section, entry.key, entry.value
                ),
            };
            lines.push(line);
        }
        for problem in &self.problems {
            lines.push(format!("not read: {problem}"));
        }
        lines
    }

    /// A one-line summary of the import.
    pub fn summary(&self) -> String {
        format!(
            "{} applied, {} not applied, {} lines not read",
            self.applied().count(),
            self.not_applied().count(),
            self.problems.len()
        )
    }
}

/// The configuration an import produces.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportPlan {
    /// The configuration the import produces.
    pub config: Config,
    /// The playlist editor's runtime font size the import produces (`None`
    /// means the skin's size is in use).
    pub playlist_font_size: Option<u32>,
    /// What the import did with the file's keys.
    pub report: ImportReport,
}

/// Looks a classic skin file name up in our skins folder.
///
/// The reference's `skin` key holds a **file name**, not a path (S12 §3.1);
/// our `ui.skin` holds a path. The mapper cannot decide on its own which
/// file the name refers to, so the caller hands in the lookup: the app
/// resolves it in the skins folder beside the configuration file.
pub type SkinResolver<'a> = &'a dyn Fn(&str) -> Option<PathBuf>;

/// Why a Winamp configuration could not be imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// The file could not be read.
    ///
    /// The import only ever reads: a file it cannot read is reported, and
    /// the configuration stays as it was.
    Read {
        /// The file's path.
        path: PathBuf,
        /// The operating system's message.
        error: String,
    },
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Read { path, error } => {
                write!(f, "could not read {}: {error}", path.display())
            }
        }
    }
}

impl std::error::Error for ImportError {}

/// What an import of a Winamp configuration did.
///
/// The preferences page shows this: the file it read and what it made of
/// it. A cancelled import is not an outcome - nothing happened, so there is
/// nothing to show.
#[derive(Debug, Clone, PartialEq)]
pub enum ImportOutcome {
    /// The file was read and its settings were taken over.
    Imported {
        /// The file that was read.
        path: PathBuf,
        /// What the import did with the file's keys.
        report: ImportReport,
    },
    /// The file could not be read; the configuration stayed as it was.
    Failed {
        /// The error: it names the file that could not be read.
        error: ImportError,
    },
}

impl ImportOutcome {
    /// The line the preferences page shows for this outcome.
    ///
    /// The line names the file and what the import did with it.
    pub fn line(&self) -> String {
        match self {
            ImportOutcome::Imported { path, report } => report_line(path, report),
            // The error names the file it could not read.
            ImportOutcome::Failed { error } => format!("not imported: {error}"),
        }
    }
}

/// The line the preferences page shows after an import.
///
/// The line names the file, the settings that were taken over and the keys
/// that could not be. It keeps the split the report makes: the keys the
/// player read in its own way are named, so the user is never told that
/// the reference's meaning was applied.
pub fn report_line(path: &std::path::Path, report: &ImportReport) -> String {
    let mut line = format!(
        "{} — {} applied, {} not applied",
        path.display(),
        report.applied().count(),
        report.not_applied().count()
    );
    let own = report.own_readings().count();
    if own > 0 {
        line.push_str(&format!(
            " ({own} read in our own way, not the reference's)"
        ));
    }
    if !report.problems.is_empty() {
        line.push_str(&format!(", {} lines not read", report.problems.len()));
    }
    line
}

/// Reads the Winamp configuration at `path` and maps it onto `current`.
///
/// The file is only **read**: the import never writes to it, and the result
/// is our configuration, not the reference's.
///
/// `resolve_skin` looks a classic skin file name up (see [`SkinResolver`]).
pub fn import_file(
    path: &std::path::Path,
    current: &Config,
    playlist_font_size: Option<u32>,
    resolve_skin: SkinResolver<'_>,
) -> Result<ImportPlan, ImportError> {
    let text = std::fs::read_to_string(path).map_err(|error| ImportError::Read {
        path: path.to_path_buf(),
        error: error.to_string(),
    })?;
    Ok(plan(&text, current, playlist_font_size, resolve_skin))
}

/// Maps the reference file onto our configuration.
///
/// `current` is the configuration the import starts from: a key the file does
/// not carry leaves the setting as it is (see the module docs). The result
/// is a new configuration; the input is not changed.
///
/// `resolve_skin` looks a classic skin file name up (see [`SkinResolver`]).
pub fn plan(
    text: &str,
    current: &Config,
    playlist_font_size: Option<u32>,
    resolve_skin: SkinResolver<'_>,
) -> ImportPlan {
    let mut file = IniFile::parse(text);
    let mut config = current.clone();
    let mut font_size = playlist_font_size;
    let mut report = ImportReport {
        keys: Vec::new(),
        problems: std::mem::take(&mut file.problems),
    };

    for section in &file.sections {
        let players_own = section.name.eq_ignore_ascii_case(PLAYER_SECTION);
        for entry in &section.entries {
            let outcome = if players_own {
                apply_key(entry, &mut config, &mut font_size, resolve_skin)
            } else {
                Outcome::NotApplied {
                    reason: if section.name.is_empty() {
                        "the key is not in a section".to_string()
                    } else {
                        format!(
                            "the key belongs to the section `[{}]`, whose keys the player does \
                             not interpret (S12 §8)",
                            section.name
                        )
                    },
                }
            };
            report.keys.push(KeyOutcome {
                section: section.name.clone(),
                key: entry.key.clone(),
                value: entry.value.clone(),
                outcome,
            });
        }
    }

    ImportPlan {
        config,
        playlist_font_size: font_size,
        report,
    }
}

/// Maps one key of the `[Winamp]` section onto our configuration.
fn apply_key(
    entry: &IniEntry,
    config: &mut Config,
    playlist_font_size: &mut Option<u32>,
    resolve_skin: SkinResolver<'_>,
) -> Outcome {
    let key = entry.key.to_ascii_lowercase();
    let mut outcome: Option<Outcome> = None;

    match key.as_str() {
        // ----- audio -----
        "volume" => {
            if let Some(value) = int_value(&entry.value) {
                config.audio.volume = value.clamp(0, 255) as u8;
                outcome = Some(applied("audio.volume", Effect::Now));
            }
        }
        "pan" => {
            if let Some(value) = int_value(&entry.value) {
                config.audio.balance = value.clamp(-127, 127) as i8;
                // Our reading: the file's balance key has an open range and
                // an open direction (S12 §3.2), so the side is our
                // convention and not a measurement.
                outcome = Some(applied_own("audio.balance", Effect::Now));
            }
        }
        // ----- equalizer -----
        "use_eq" => {
            if let Some(value) = flag_value(&entry.value) {
                config.eq.state.enabled = value;
                outcome = Some(applied("eq.state.enabled", Effect::Now));
            }
        }
        "autoload_eq" => {
            if let Some(value) = flag_value(&entry.value) {
                config.eq.state.auto = value;
                outcome = Some(applied("eq.state.auto", Effect::Now));
            }
        }
        "preamp" => {
            if let Some(value) = int_value(&entry.value) {
                config.eq.state.preamp = rustamp_ui::saved_to_gain_db(clamp_classic(value));
                // Our reading: the value is in the slider's own scale, and
                // the conversion to dB is ours (S07 §7).
                outcome = Some(applied_own("eq.state.preamp", Effect::Now));
            }
        }
        "eq_data" => {
            let values = parse_eq_data(&entry.value);
            if values.is_empty() {
                outcome = Some(Outcome::NotApplied {
                    reason: "the band list holds no readable value".to_string(),
                });
            } else {
                for (band, value) in values.iter().enumerate() {
                    config.eq.state.bands[band] = rustamp_ui::saved_to_gain_db(*value);
                }
                outcome = Some(applied_own(
                    &format!("eq.state.bands[0..{}]", values.len()),
                    Effect::Now,
                ));
            }
        }
        // ----- transport -----
        "shuffle" => {
            if let Some(value) = flag_value(&entry.value) {
                config.playlist.shuffle = value;
                outcome = Some(applied("playlist.shuffle", Effect::Now));
            }
        }
        "repeat" => {
            if let Some(value) = flag_value(&entry.value) {
                config.playlist.repeat = value;
                outcome = Some(applied("playlist.repeat", Effect::Now));
            }
        }
        // ----- the playlist editor -----
        "pe_fontsize" => {
            if let Some(value) = int_value(&entry.value) {
                let usable = u32::try_from(value).ok().filter(|size| {
                    (rustamp_ui::MIN_FONT_SIZE..=rustamp_ui::MAX_FONT_SIZE).contains(size)
                });
                match usable {
                    Some(size) => {
                        *playlist_font_size = Some(size);
                        outcome = Some(applied("ui.playlist_font_size", Effect::Now));
                    }
                    None => {
                        outcome = Some(Outcome::NotApplied {
                            reason: format!(
                                "the playlist font size must be a whole number of pixels between \
                                 {} and {}",
                                rustamp_ui::MIN_FONT_SIZE,
                                rustamp_ui::MAX_FONT_SIZE
                            ),
                        });
                    }
                }
            }
        }
        "pe_width" => {
            if let Some(value) = size_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Playlist).width = Some(value);
                outcome = Some(applied("ui.windows[Playlist].width", Effect::Stored));
            }
        }
        "pe_height" => {
            if let Some(value) = size_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Playlist).height = Some(value);
                outcome = Some(applied("ui.windows[Playlist].height", Effect::Stored));
            }
        }
        // ----- the windows -----
        "wx" => {
            if let Some(value) = position_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Main).x = value;
                outcome = Some(applied("ui.windows[Main].x", Effect::NextStart));
            }
        }
        "wy" => {
            if let Some(value) = position_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Main).y = value;
                outcome = Some(applied("ui.windows[Main].y", Effect::NextStart));
            }
        }
        "eq_wx" => {
            if let Some(value) = position_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Equalizer).x = value;
                outcome = Some(applied("ui.windows[Equalizer].x", Effect::NextStart));
            }
        }
        "eq_wy" => {
            if let Some(value) = position_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Equalizer).y = value;
                outcome = Some(applied("ui.windows[Equalizer].y", Effect::NextStart));
            }
        }
        "pe_wx" => {
            if let Some(value) = position_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Playlist).x = value;
                outcome = Some(applied("ui.windows[Playlist].x", Effect::NextStart));
            }
        }
        "pe_wy" => {
            if let Some(value) = position_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Playlist).y = value;
                outcome = Some(applied("ui.windows[Playlist].y", Effect::NextStart));
            }
        }
        "video_wx" => {
            if let Some(value) = position_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Video).x = value;
                outcome = Some(applied("ui.windows[Video].x", Effect::Stored));
            }
        }
        "video_wy" => {
            if let Some(value) = position_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Video).y = value;
                outcome = Some(applied("ui.windows[Video].y", Effect::Stored));
            }
        }
        "video_width" => {
            if let Some(value) = size_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Video).width = Some(value);
                outcome = Some(applied("ui.windows[Video].width", Effect::Stored));
            }
        }
        "video_height" => {
            if let Some(value) = size_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Video).height = Some(value);
                outcome = Some(applied("ui.windows[Video].height", Effect::Stored));
            }
        }
        "mw_open" => {
            if let Some(value) = flag_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Main).visible = value;
                outcome = Some(applied("ui.windows[Main].visible", Effect::Now));
            }
        }
        "eq_open" => {
            if let Some(value) = flag_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Equalizer).visible = value;
                outcome = Some(applied("ui.windows[Equalizer].visible", Effect::Now));
            }
        }
        "pe_open" => {
            if let Some(value) = flag_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Playlist).visible = value;
                outcome = Some(applied("ui.windows[Playlist].visible", Effect::Now));
            }
        }
        "video_open" => {
            if let Some(value) = flag_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Video).visible = value;
                outcome = Some(applied("ui.windows[Video].visible", Effect::Stored));
            }
        }
        "windowshade" => {
            if let Some(value) = flag_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Main).shaded = value;
                outcome = Some(applied("ui.windows[Main].shaded", Effect::Now));
            }
        }
        "eq_ws" => {
            if let Some(value) = flag_value(&entry.value) {
                layout_mut(&mut config.ui.windows, WindowId::Equalizer).shaded = value;
                outcome = Some(applied("ui.windows[Equalizer].shaded", Effect::Now));
            }
        }
        // ----- the window flags -----
        "aot" => {
            if let Some(value) = flag_value(&entry.value) {
                config.ui.always_on_top = value;
                outcome = Some(applied("ui.always_on_top", Effect::Now));
            }
        }
        "dsize" => {
            if let Some(value) = flag_value(&entry.value) {
                config.ui.double_size = value;
                outcome = Some(applied("ui.double_size", Effect::Now));
            }
        }
        "easymove" => {
            if let Some(value) = flag_value(&entry.value) {
                config.ui.easy_move = value;
                outcome = Some(applied("ui.easy_move", Effect::Now));
            }
        }
        // ----- the skin -----
        "skin" => {
            if entry.value.is_empty() {
                outcome = Some(Outcome::NotApplied {
                    reason: "the key names no skin".to_string(),
                });
            } else {
                match resolve_skin(&entry.value) {
                    Some(path) => {
                        config.ui.skin = Some(path);
                        outcome = Some(applied("ui.skin", Effect::Now));
                    }
                    None => {
                        outcome = Some(Outcome::NotApplied {
                            reason: format!(
                                "the skin file `{}` is not in our skins folder",
                                entry.value
                            ),
                        });
                    }
                }
            }
        }
        // ----- the keys the player has no setting for -----
        "outname" => not_honoured(
            &mut outcome,
            "the output plug-in is not ours: the backend comes from the command line",
        ),
        "pladv" => not_honoured(
            &mut outcome,
            "the setting behind it (playlist.manual_advance) is not applied by the player yet",
        ),
        "pe_height_ws" => not_honoured(
            &mut outcome,
            "the playlist editor's shaded height is fixed by the layout (S01 §1)",
        ),
        "eqdsize" => not_honoured(
            &mut outcome,
            "the player has one double-size flag for the whole window set, not one for the \
             equalizer",
        ),
        "snaplen" => not_honoured(
            &mut outcome,
            "the classic length has no established unit or range (S12 §9), so it cannot become \
             our pixel distance",
        ),
        "snap" => not_honoured(
            &mut outcome,
            "our snapping is a distance, not a flag, and the classic unit is open (S12 §9)",
        ),
        "minimized" => not_honoured(&mut outcome, "the player does not keep a minimized state"),
        "timeleftmode" => not_honoured(
            &mut outcome,
            "the player does not keep the time display's mode",
        ),
        "autoscrollname" => not_honoured(
            &mut outcome,
            "the player does not keep the title scroll setting",
        ),
        "shownumsinpl" => not_honoured(
            &mut outcome,
            "the player does not keep a track-number column setting",
        ),
        "prefs_last_page" => not_honoured(
            &mut outcome,
            "the player does not keep the preferences page; the key carries the page index in \
             its low byte and a second number above it (S12 §7)",
        ),
        "last_shortdesc" | "last_shorturl" => not_honoured(
            &mut outcome,
            "the player keeps no shortcut description or URL",
        ),
        "newverchk" | "newverchk2" => {
            not_honoured(&mut outcome, "the player does not check for a new version")
        }
        "safire" | "saref" | "pilp" => not_honoured(
            &mut outcome,
            "the reference's own meaning for this key is unknown (S12 §10)",
        ),
        "sa" | "safalloff" | "sa_peaks" | "sa_peak_falloff" => not_honoured(
            &mut outcome,
            "the player has no setting for the built-in analyser",
        ),
        "visplugin_num" | "visplugin_name" | "visplugin_autoexec" | "visplugin_priority" => {
            not_honoured(
                &mut outcome,
                "the player does not load visualisation plug-ins yet",
            )
        }
        "dspplugin_num" | "dspplugin_name" => {
            not_honoured(&mut outcome, "the player has no DSP plug-in selection")
        }
        "shuffle_morph_rate" => not_honoured(&mut outcome, "the player has no shuffle morph rate"),
        "mb_wx" | "mb_wy" | "mb_open" | "mb_width" | "mb_height" => {
            not_honoured(&mut outcome, "the player has no minibrowser window")
        }
        "video_ratio1"
        | "video_ratio2"
        | "video_useratio"
        | "video_vsync"
        | "video_aspectadj"
        | "video_overlays"
        | "video_ddraw"
        | "video_updsize"
        | "video_autoopen"
        | "video_autoclose"
        | "video_noss"
        | "video_osd"
        | "video_yv12"
        | "video_stopclose"
        | "video_remove_fs_on_stop" => not_honoured(
            &mut outcome,
            "the player has no setting for the video window's playback",
        ),
        "randskin" | "cwd" | "defext" | "keeponscreen" | "parent" | "hilite" | "addtolist"
        | "proxy" | "inet_mode" | "browserbrand" | "langpack" | "extlist" | "check_ft_startup"
        | "usecursors" | "fixtitles" | "ttips" | "ascb_new" | "taskbar" | "splash" | "sticon"
        | "riol" | "minst" | "whichicon" | "whichicon2" | "disvis" | "priority" | "ospb"
        | "bifont" | "rofiob" => not_honoured(&mut outcome, "the player has no setting for it"),
        _ => not_honoured(
            &mut outcome,
            "the reference player does not recognise this key (S12 §5.1)",
        ),
    }

    outcome.unwrap_or_else(|| Outcome::NotApplied {
        reason: "the value is not a number".to_string(),
    })
}

/// The outcome of an applied key, read the way the spec states it.
fn applied(setting: &str, effect: Effect) -> Outcome {
    Outcome::Applied {
        setting: setting.to_string(),
        effect,
        reading: Reading::Spec,
    }
}

/// The outcome of an applied key whose meaning is **our** reading.
///
/// The two such keys are named in the module docs: the balance's side
/// (S12 §3.2) and the classic gain scale (S07 §7).
fn applied_own(setting: &str, effect: Effect) -> Outcome {
    Outcome::Applied {
        setting: setting.to_string(),
        effect,
        reading: Reading::Own,
    }
}

/// The outcome of a key the player has no setting for.
fn not_honoured(outcome: &mut Option<Outcome>, reason: &str) {
    *outcome = Some(Outcome::NotApplied {
        reason: reason.to_string(),
    });
}

/// The integer a value holds, when it holds one.
///
/// The whole value has to be the number: the reference reads an integer or
/// falls back to its default (S12 §4.2), and a value with text behind the
/// number is not a number we can act on.
fn int_value(value: &str) -> Option<i64> {
    let text = value.trim();
    let (number, tail) = read_integer(text)?;
    tail.trim().is_empty().then_some(number)
}

/// The flag a value holds: `0` is off, any other number is on.
fn flag_value(value: &str) -> Option<bool> {
    int_value(value).map(|number| number != 0)
}

/// A window position, as a signed number.
fn position_value(value: &str) -> Option<i32> {
    int_value(value).and_then(|number| i32::try_from(number).ok())
}

/// A window size, as an unsigned number.
fn size_value(value: &str) -> Option<u32> {
    int_value(value).and_then(|number| u32::try_from(number).ok())
}

/// A classic gain value, clamped into the slider's own scale.
fn clamp_classic(value: i64) -> u8 {
    value.clamp(CLASSIC_MIN, CLASSIC_MAX) as u8
}

/// The band values a classic `eq_data` string holds (S12 §6).
///
/// The guarantees the spec states:
///
/// * exactly **ten** values are taken; the parse stops at the first
///   non-numeric character;
/// * a value below `0` becomes `0`, a value above `63` becomes `63`;
/// * a missing or unparsable entry leaves the band at whatever it was — the
///   parse stops, it does not skip to the next entry;
/// * the separator counts as one character, so `1,2,3` and `1,,3` behave
///   differently: an empty entry stops the parse.
///
/// The returned vector therefore holds **fewer than ten** values when the
/// parse stopped early; the bands it does not reach keep the value they had.
pub fn parse_eq_data(value: &str) -> Vec<u8> {
    let mut values = Vec::new();
    let mut rest = value;
    while values.len() < EQ_BANDS {
        let Some((number, tail)) = read_integer(rest) else {
            break;
        };
        values.push(clamp_classic(number));
        if values.len() == EQ_BANDS {
            break;
        }
        let Some(tail) = tail.strip_prefix(',') else {
            break;
        };
        rest = tail;
    }
    values
}

/// The leading integer of `text`, and the text behind it.
///
/// The sign may be `-` or `+`; the digits follow it. A number that does not
/// fit into an `i64` saturates, so a huge value still reaches the clamp
/// instead of being read as no number at all.
fn read_integer(text: &str) -> Option<(i64, &str)> {
    let bytes = text.as_bytes();
    let mut index = 0;
    let negative = match bytes.first() {
        Some(b'-') => {
            index = 1;
            true
        }
        Some(b'+') => {
            index = 1;
            false
        }
        _ => false,
    };
    let start = index;
    let mut number: i64 = 0;
    while let Some(byte) = bytes.get(index) {
        if !byte.is_ascii_digit() {
            break;
        }
        number = number
            .saturating_mul(10)
            .saturating_add(i64::from(byte - b'0'));
        index += 1;
    }
    if index == start {
        return None;
    }
    Some((if negative { -number } else { number }, &text[index..]))
}

/// The layout of the window `id`, created when the configuration has none.
fn layout_mut(windows: &mut Vec<WindowLayout>, id: WindowId) -> &mut WindowLayout {
    if let Some(index) = windows.iter().position(|window| window.id == id) {
        return &mut windows[index];
    }
    let (x, y) = default_position(id);
    windows.push(WindowLayout {
        id,
        x,
        y,
        visible: true,
        shaded: false,
        width: None,
        height: None,
    });
    windows.last_mut().expect("the layout was just pushed")
}

/// The position a window starts at when the configuration names none.
fn default_position(id: WindowId) -> (i32, i32) {
    match id {
        WindowId::Main => (26, rustamp_ui::DEFAULT_MAIN_POSITION.y),
        WindowId::Equalizer => (26, rustamp_ui::DEFAULT_EQUALIZER_POSITION.y),
        WindowId::Playlist => (26, rustamp_ui::DEFAULT_PLAYLIST_POSITION.y),
        _ => (26, 26),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A resolver that finds no skin: the skins folder holds none.
    fn no_skin(_name: &str) -> Option<PathBuf> {
        None
    }

    /// The layout of the window `id` in a configuration.
    fn layout(config: &Config, id: WindowId) -> &WindowLayout {
        config
            .ui
            .windows
            .iter()
            .find(|window| window.id == id)
            .unwrap_or_else(|| panic!("the configuration has a layout for {id:?}"))
    }

    #[test]
    fn the_parser_reads_the_sections_the_keys_and_the_empty_values() {
        let text = "[Winamp]\r\nvolume=255\r\nlast_shortdesc=\r\n\r\n\
                    [Nullsoft Module Decoder]\r\ncfg_title_format=$if2(%title%,%filename%)\r\n";
        let file = IniFile::parse(text);
        assert!(file.problems.is_empty(), "{:?}", file.problems);

        let player = file.section("Winamp").expect("the player's own section");
        assert_eq!(player.get("volume"), Some("255"));
        assert_eq!(player.get("last_shortdesc"), Some(""));
        assert_eq!(player.entries.len(), 2);

        let decoder = file
            .section("Nullsoft Module Decoder")
            .expect("the plug-in's own section");
        assert_eq!(
            decoder.get("cfg_title_format"),
            Some("$if2(%title%,%filename%)")
        );
    }

    /// The reference reads the file through the profile API (S12 §4.3),
    /// whose names are case insensitive.
    #[test]
    fn the_names_are_matched_without_regard_to_case() {
        let file = IniFile::parse("[winamp]\nVolume=255\n");
        let player = file.section("WINAMP").expect("the section");
        assert_eq!(player.get("VOLUME"), Some("255"));
    }

    #[test]
    fn a_line_the_parser_cannot_read_is_reported_and_the_rest_is_read() {
        let text = "[Winamp]\nthis line has no equals sign\nvolume=200\n[Winamp\nshuffle=1\n";
        let file = IniFile::parse(text);
        assert_eq!(file.problems.len(), 2, "{:?}", file.problems);
        let player = file.section("Winamp").expect("the section");
        assert_eq!(player.get("volume"), Some("200"));
        assert_eq!(player.get("shuffle"), Some("1"));
    }

    #[test]
    fn a_repeated_key_keeps_the_first_value_and_is_reported() {
        let file = IniFile::parse("[Winamp]\nvolume=200\nvolume=10\n");
        assert_eq!(file.problems.len(), 1, "{:?}", file.problems);
        let player = file.section("Winamp").expect("the section");
        assert_eq!(player.get("volume"), Some("200"));
    }

    /// The band's values are clamped into the slider's own scale: below `0`
    /// becomes `0`, above `63` becomes `63` (S12 §6).
    #[test]
    fn a_band_value_is_clamped_into_the_classic_scale() {
        assert_eq!(parse_eq_data("0"), vec![0]);
        assert_eq!(parse_eq_data("63"), vec![63]);
        assert_eq!(parse_eq_data("64"), vec![63]);
        assert_eq!(parse_eq_data("-5"), vec![0]);
    }

    /// The parse stops at the first entry it cannot read, and it does not
    /// skip to the next one (S12 §6).
    #[test]
    fn the_band_parse_stops_at_the_first_entry_it_cannot_read() {
        // An empty entry stops the parse: `1,,3` differs from `1,2,3`.
        assert_eq!(parse_eq_data("1,,3"), vec![1]);
        assert_eq!(parse_eq_data("1,2,3"), vec![1, 2, 3]);
        // A value that is not a number stops the parse too.
        assert_eq!(parse_eq_data("1,2,x,4"), vec![1, 2]);
        // The separator counts as one character, so a space is not a digit.
        assert_eq!(parse_eq_data("1, 2"), vec![1]);
        // Ten values are taken; the rest of the list is ignored.
        let ten = parse_eq_data("1,2,3,4,5,6,7,8,9,10,11,12");
        assert_eq!(ten.len(), EQ_BANDS);
        assert_eq!(ten[9], 10);
    }

    /// The known keys land on the settings the mapping table names.
    #[test]
    fn the_known_keys_map_onto_the_configuration() {
        let text = "[Winamp]\n\
                    volume=200\n\
                    pan=-40\n\
                    use_eq=1\n\
                    autoload_eq=0\n\
                    preamp=63\n\
                    eq_data=0,63,31,31,31,31,31,31,31,31\n\
                    shuffle=1\n\
                    repeat=1\n\
                    pe_fontsize=14\n\
                    wx=100\n\
                    wy=200\n\
                    aot=1\n\
                    dsize=1\n\
                    easymove=0\n\
                    windowshade=1\n\
                    eq_ws=1\n";
        let plan = plan(text, &Config::default(), None, &no_skin);
        let config = &plan.config;

        assert_eq!(config.audio.volume, 200);
        assert_eq!(config.audio.balance, -40);
        assert!(config.eq.state.enabled);
        assert!(!config.eq.state.auto);
        assert_eq!(config.eq.state.preamp, rustamp_ui::saved_to_gain_db(63));
        assert_eq!(config.eq.state.bands[0], rustamp_ui::saved_to_gain_db(0));
        assert_eq!(config.eq.state.bands[1], rustamp_ui::saved_to_gain_db(63));
        assert_eq!(config.eq.state.bands[2], rustamp_ui::saved_to_gain_db(31));
        assert_eq!(config.eq.state.bands[9], rustamp_ui::saved_to_gain_db(31));
        assert!(config.playlist.shuffle);
        assert!(config.playlist.repeat);
        assert_eq!(plan.playlist_font_size, Some(14));
        assert!(config.ui.always_on_top);
        assert!(config.ui.double_size);
        assert!(!config.ui.easy_move);

        let main = layout(config, WindowId::Main);
        assert_eq!((main.x, main.y), (100, 200));
        assert!(main.shaded);
        let equalizer = layout(config, WindowId::Equalizer);
        assert!(equalizer.shaded);
    }

    /// A key the file does not carry leaves the setting as it is: our
    /// default is our current configuration (S12 §4.2 says the same about
    /// the reference's own default).
    #[test]
    fn a_missing_key_keeps_the_current_value() {
        let mut current = Config::default();
        current.audio.volume = 77;
        current.playlist.shuffle = true;
        current.eq.state.preamp = 3.0;
        current.ui.always_on_top = true;

        let plan = plan("[Winamp]\nrepeat=1\n", &current, Some(18), &no_skin);

        assert_eq!(plan.config.audio.volume, 77);
        assert!(plan.config.playlist.shuffle);
        assert_eq!(plan.config.eq.state.preamp, 3.0);
        assert!(plan.config.ui.always_on_top);
        // The key that is there is applied; the one that is not is not even
        // mentioned.
        assert!(plan.config.playlist.repeat);
        assert!(plan.report.applied_key("repeat"));
        assert!(!plan.report.applied_key("volume"));
        assert_eq!(plan.playlist_font_size, Some(18));
    }

    /// A key the player has no setting for is reported, never dropped
    /// silently.
    #[test]
    fn a_key_the_player_cannot_honour_is_reported() {
        let plan = plan(
            "[Winamp]\noutname=out_wave.dll\npilp=0\nvolume=200\n",
            &Config::default(),
            None,
            &no_skin,
        );

        assert!(!plan.report.applied_key("outname"));
        let reason = plan.report.reason_for("outname").expect("a reason");
        assert!(reason.contains("command line"), "{reason}");

        assert!(!plan.report.applied_key("pilp"));
        let reason = plan.report.reason_for("pilp").expect("a reason");
        assert!(reason.contains("unknown"), "{reason}");

        // The key that can be honoured still is.
        assert!(plan.report.applied_key("volume"));
        assert_eq!(plan.config.audio.volume, 200);
    }

    /// A plug-in's own keys are not the player's, and the player does not
    /// interpret them (S12 §8).
    #[test]
    fn a_plug_ins_own_keys_are_reported() {
        let plan = plan(
            "[Winamp]\nvolume=200\n\n[Nullsoft Module Decoder]\ncfg_title_format=%title%\n",
            &Config::default(),
            None,
            &no_skin,
        );
        assert!(plan.report.applied_key("volume"));
        let reason = plan
            .report
            .reason_for("cfg_title_format")
            .expect("a reason");
        assert!(reason.contains("does not interpret"), "{reason}");
    }

    /// The skin is a file name; the caller resolves it in our skins folder.
    #[test]
    fn the_skin_name_is_resolved_by_the_caller() {
        let text = "[Winamp]\nskin=base-2.91.wsz\n";
        let found = plan(text, &Config::default(), None, &|name: &str| {
            (name == "base-2.91.wsz").then(|| PathBuf::from("skins/base-2.91.wsz"))
        });
        assert_eq!(
            found.config.ui.skin,
            Some(PathBuf::from("skins/base-2.91.wsz"))
        );
        assert!(found.report.applied_key("skin"));

        // A skin our folder does not hold is reported, not guessed.
        let missing = plan(text, &Config::default(), None, &no_skin);
        assert_eq!(missing.config.ui.skin, None);
        let reason = missing.report.reason_for("skin").expect("a reason");
        assert!(reason.contains("skins folder"), "{reason}");
    }

    /// A font size the playlist cannot use is reported and the current one
    /// stays (the same rule the configuration file follows).
    #[test]
    fn a_font_size_the_playlist_cannot_use_is_reported() {
        let plan = plan(
            "[Winamp]\npe_fontsize=900\n",
            &Config::default(),
            Some(14),
            &no_skin,
        );
        assert_eq!(plan.playlist_font_size, Some(14));
        let reason = plan.report.reason_for("pe_fontsize").expect("a reason");
        assert!(reason.contains("between"), "{reason}");
    }

    /// A value that is not a number is reported; the setting keeps the
    /// value it had.
    #[test]
    fn a_value_that_is_not_a_number_is_reported() {
        let plan = plan(
            "[Winamp]\nvolume=loud\n",
            &Config::default(),
            None,
            &no_skin,
        );
        assert_eq!(plan.config.audio.volume, Config::default().audio.volume);
        let reason = plan.report.reason_for("volume").expect("a reason");
        assert!(reason.contains("not a number"), "{reason}");
    }

    /// The report says what was applied and what was not, in one line per
    /// key.
    #[test]
    fn the_report_names_every_key() {
        let plan = plan(
            "[Winamp]\nvolume=200\noutname=out_wave.dll\n",
            &Config::default(),
            None,
            &no_skin,
        );
        let lines = plan.report.lines();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].contains("audio.volume"), "{}", lines[0]);
        assert!(lines[0].contains("applied now"), "{}", lines[0]);
        assert!(lines[1].contains("not applied"), "{}", lines[1]);
        assert!(
            plan.report.summary().contains("1 applied"),
            "{}",
            plan.report.summary()
        );
    }

    /// The keys the player reads in its own way are named, so the user is
    /// not told that the reference's meaning was applied (see the module
    /// docs).
    #[test]
    fn the_keys_we_read_in_our_own_way_are_named() {
        let plan = plan(
            "[Winamp]\npan=-40\npreamp=63\neq_data=31\nvolume=200\n",
            &Config::default(),
            None,
            &no_skin,
        );
        let own: Vec<&str> = plan
            .report
            .own_readings()
            .map(|entry| entry.key.as_str())
            .collect();
        assert_eq!(own, ["pan", "preamp", "eq_data"]);
        // The line of such a key says so.
        let lines = plan.report.lines();
        let balance = lines
            .iter()
            .find(|line| line.contains("audio.balance"))
            .expect("the balance's line");
        assert!(balance.contains("our own reading"), "{balance}");
        // A key read the way the spec states it is not marked.
        let volume = lines
            .iter()
            .find(|line| line.contains("audio.volume"))
            .expect("the volume's line");
        assert!(!volume.contains("our own reading"), "{volume}");
    }

    /// A key that could not be read is not one of our own readings: the
    /// split only counts what was applied.
    #[test]
    fn a_key_that_could_not_be_read_is_not_one_of_our_own_readings() {
        let plan = plan("[Winamp]\npan=loud\n", &Config::default(), None, &no_skin);
        assert_eq!(plan.report.own_readings().count(), 0);
        assert_eq!(plan.report.not_applied().count(), 1);
    }

    /// The line the preferences page shows names the file, the settings that
    /// were taken over and the keys that could not be.
    #[test]
    fn the_report_line_names_the_file_and_the_counts() {
        let plan = plan(
            "[Winamp]\nvolume=200\npan=-40\noutname=out_wave.dll\n",
            &Config::default(),
            None,
            &no_skin,
        );
        let line = report_line(std::path::Path::new(r"C:\winamp.ini"), &plan.report);
        // The file, the two counts and the honest split: the balance is
        // read in our own way, not the reference's.
        assert_eq!(
            line,
            r"C:\winamp.ini — 2 applied, 1 not applied (1 read in our own way, not the reference's)"
        );
    }

    /// A file without a key of our own reading says nothing about one.
    #[test]
    fn the_report_line_stays_short_without_a_key_of_our_own_reading() {
        let plan = plan("[Winamp]\nvolume=200\n", &Config::default(), None, &no_skin);
        let line = report_line(std::path::Path::new(r"C:\winamp.ini"), &plan.report);
        assert_eq!(line, r"C:\winamp.ini — 1 applied, 0 not applied");
    }
}
