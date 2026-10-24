// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The equalizer's automatic per-track preset selection (the AUTO button).
//!
//! # What the specification settles
//!
//! S07 §12 item 13 ("Engagement setter") settles exactly one thing about the
//! AUTO button: `autoload_eq` is the saved-state key behind it, and it
//! toggles the AUTO indicator cell (S01 §3.1, S07 §2.1). S07 §1 lists that
//! key among the equalizer's saved-state keys (`preamp`, `eq_data`,
//! `autoload_eq`, `use_eq`). S07 §12 item 12 (preset load semantics) leaves
//! open whether loading a preset touches `preamp`, `eq_data`, `autoload_eq` or
//! `use_eq`.
//!
//! That is the whole of it. **The specification names the flag, not the
//! behaviour behind it.** Everything below is therefore Rustamp's own design,
//! stated plainly so it can be checked and changed:
//!
//! * **Trigger.** The lookup runs once per track start, when the engine
//!   reports the new track, and only while the AUTO flag is set.
//! * **The key.** A rule matches a track's genre, artist or album tag. The
//!   rules are tried in the order the file lists them and the first match
//!   decides.
//! * **Fallback.** A track that matches no rule gets the file's `default`
//!   preset; a file without a default leaves the current curve alone.
//! * **Precedence.** The auto-load runs at the track start only, so a manual
//!   change during the track is never overwritten. The next track start
//!   decides again, and then the auto-load wins over the manual curve.
//! * **What a load changes.** Only the curve: the ten bands and, for a saved
//!   preset, its preamp. The ON and AUTO flags are left alone (S07 §12 item
//!   12 leaves this open).
//!
//! # The rules file
//!
//! The rules live in `eq-autoload.toml`, beside the configuration file and
//! next to the user's `presets.eqf` library. The file is written by hand and
//! read once at start-up; a change takes effect at the next start.
//!
//! ```toml
//! # The preset for a track that matches no rule. Without it, such a track
//! # keeps the curve it has.
//! default = "Flat"
//!
//! # The rules, in the order they are tried. The first match decides.
//! [[rule]]
//! tag = "genre"          # genre, artist or album
//! value = "Rock"
//! preset = "Rock"
//!
//! [[rule]]
//! tag = "artist"
//! value = "Miles Davis"
//! preset = "Jazz"
//! ```
//!
//! `tag` names the track metadata field to match, `value` is the value to
//! match (ASCII case-insensitive, surrounding space ignored) and `preset` is
//! the name of a built-in preset or of one of the user's saved presets. An
//! unknown `tag`, an unknown key or a malformed file is reported and leaves
//! the table empty — the file is hand-written, so a mistake must be visible
//! rather than silently ignored. An empty `value` or `default` names nothing
//! and selects nothing.

use std::path::PathBuf;

use rustamp_core::meta::TrackMetadata;
use serde::Deserialize;

use crate::app::App;

/// The file name of the auto-load rules, beside the configuration.
pub const AUTO_LOAD_FILE: &str = "eq-autoload.toml";

/// The track metadata field a rule matches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleTag {
    /// The track's genre tag.
    Genre,
    /// The track's artist tag.
    Artist,
    /// The track's album tag.
    Album,
}

impl RuleTag {
    /// The name the rules file uses, parsed (ASCII case-insensitive).
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("genre") {
            Some(RuleTag::Genre)
        } else if text.eq_ignore_ascii_case("artist") {
            Some(RuleTag::Artist)
        } else if text.eq_ignore_ascii_case("album") {
            Some(RuleTag::Album)
        } else {
            None
        }
    }

    /// The metadata value this tag reads.
    fn value_of(self, metadata: &TrackMetadata) -> Option<&str> {
        match self {
            RuleTag::Genre => metadata.genre.as_deref(),
            RuleTag::Artist => metadata.artist.as_deref(),
            RuleTag::Album => metadata.album.as_deref(),
        }
    }
}

/// One rule: a metadata value that selects a preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoLoadRule {
    /// The metadata field the rule matches.
    pub tag: RuleTag,
    /// The value it matches (ASCII case-insensitive, surrounding space ignored).
    pub value: String,
    /// The name of the preset it selects.
    pub preset: String,
}

/// The whole rule table: the ordered rules and the fallback.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoLoadRules {
    /// The rules, in the order the file lists them.
    pub rules: Vec<AutoLoadRule>,
    /// The preset for a track that matches no rule.
    pub default: Option<String>,
}

impl AutoLoadRules {
    /// The preset name the table selects for `metadata`, if any.
    ///
    /// The rules are tried in order and the first match decides. A track that
    /// matches no rule gets the table's default; a table without a default
    /// selects nothing, and the track keeps its curve.
    pub fn decide(&self, metadata: &TrackMetadata) -> Option<&str> {
        for rule in &self.rules {
            let value = rule.value.trim();
            if value.is_empty() {
                // An empty value would match an empty tag; it selects nothing.
                continue;
            }
            let Some(actual) = rule.tag.value_of(metadata) else {
                continue;
            };
            if actual.trim().eq_ignore_ascii_case(value) {
                return Some(&rule.preset);
            }
        }
        self.default.as_deref()
    }
}

/// The rules file as it is written on disk.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRules {
    /// The preset for a track that matches no rule.
    #[serde(default)]
    default: Option<String>,
    /// The rules, in the order they are tried.
    #[serde(default)]
    rule: Vec<RawRule>,
}

/// One rule as it is written on disk.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    /// The metadata field: `genre`, `artist` or `album`.
    tag: String,
    /// The value to match.
    value: String,
    /// The name of the preset to select.
    preset: String,
}

/// Parses the rules file's text.
///
/// An unknown tag, an unknown key or malformed TOML is an error: the file is
/// written by hand, so a mistake has to be reported rather than ignored.
pub fn parse(text: &str) -> Result<AutoLoadRules, String> {
    let raw: RawRules = toml::from_str(text).map_err(|error| error.to_string())?;
    let mut rules = Vec::with_capacity(raw.rule.len());
    for (index, rule) in raw.rule.iter().enumerate() {
        let Some(tag) = RuleTag::parse(&rule.tag) else {
            return Err(format!(
                "rule {}: unknown tag '{}' (expected genre, artist or album)",
                index + 1,
                rule.tag
            ));
        };
        rules.push(AutoLoadRule {
            tag,
            value: rule.value.trim().to_string(),
            preset: rule.preset.trim().to_string(),
        });
    }
    Ok(AutoLoadRules {
        rules,
        // An empty default names no preset, so it is no default at all.
        default: raw.default.filter(|name| !name.trim().is_empty()),
    })
}

impl App {
    /// The auto-load rules file, beside the configuration.
    fn auto_load_rules_path(&self) -> Option<PathBuf> {
        Some(self.config_path()?.parent()?.join(AUTO_LOAD_FILE))
    }

    /// Reads the auto-load rules from disk (called when the config path is
    /// set). A missing file means no rules; an unreadable or malformed one is
    /// noted and leaves the table empty.
    pub(crate) fn load_eq_auto_rules(&mut self) {
        self.eq_auto_rules = AutoLoadRules::default();
        let Some(path) = self.auto_load_rules_path() else {
            return;
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match parse(&text) {
                Ok(rules) => self.eq_auto_rules = rules,
                Err(error) => self.note(format!("{}: {error}", path.display())),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => self.note(format!("{}: {error}", path.display())),
        }
    }

    /// Applies the auto-loaded preset for the track that just started.
    ///
    /// The AUTO flag is the gate: S07 §12 item 13 settles that `autoload_eq`
    /// is the saved-state key behind the AUTO button, so the lookup only runs
    /// while that flag is set. The lookup runs once per track start, so a
    /// manual change during the track is never overwritten.
    pub(crate) fn auto_load_eq_preset(&mut self, metadata: &TrackMetadata) {
        let eq = self.engine().eq();
        if !eq.auto {
            return;
        }
        let Some(name) = self.eq_auto_rules.decide(metadata).map(str::to_owned) else {
            // No rule matched and the file names no default: the track keeps
            // the curve it has.
            return;
        };
        let Some((bands, preamp)) = crate::eq_presets::named_curve(&name, &self.eq_user_presets)
        else {
            self.note(format!("EQ auto-load: no preset named '{name}'"));
            return;
        };
        let mut next = eq;
        next.bands = bands;
        if let Some(preamp) = preamp {
            next.preamp = preamp;
        }
        // `enabled` and `auto` are carried over unchanged: loading a curve is
        // not a change of the ON or AUTO state.
        self.engine_mut().set_eq(next);
        self.sync_eq_from_engine();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_core::audio::DecoderRegistry;
    use rustamp_core::command::Command;
    use rustamp_core::config::Config;
    use rustamp_core::event::Event;

    use crate::app::AppOptions;
    use crate::cli::OutputChoice;

    const GENRE_RULE: &str = r#"
[[rule]]
tag = "genre"
value = "Rock"
preset = "Rock"
"#;

    fn metadata(genre: Option<&str>, artist: Option<&str>, album: Option<&str>) -> TrackMetadata {
        TrackMetadata {
            genre: genre.map(str::to_owned),
            artist: artist.map(str::to_owned),
            album: album.map(str::to_owned),
            ..TrackMetadata::default()
        }
    }

    fn test_app() -> App {
        let options = AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            media: None,
            seconds: None,
        };
        App::with_registry(options, DecoderRegistry::new()).unwrap()
    }

    /// An app whose auto-load rules are `text`.
    fn app_with_rules(dir: &tempfile::TempDir, text: &str) -> App {
        std::fs::write(dir.path().join(AUTO_LOAD_FILE), text).unwrap();
        let mut app = test_app();
        app.set_config_path(Some(dir.path().join("rustamp.toml")));
        app
    }

    /// The event the engine publishes when a track starts.
    fn track_started(metadata: TrackMetadata) -> Event {
        Event::TrackChanged {
            id: None,
            index: Some(0),
            metadata: Some(Box::new(metadata)),
            duration_ms: None,
        }
    }

    fn rock_bands() -> [f32; rustamp_core::state::EQ_BANDS] {
        crate::eq_presets::find_preset("Rock").unwrap().bands
    }

    // ----- the decision table -----

    #[test]
    fn the_first_matching_rule_decides() {
        let rules = parse(
            r#"
[[rule]]
tag = "genre"
value = "Rock"
preset = "Rock"

[[rule]]
tag = "artist"
value = "Miles Davis"
preset = "Jazz"
"#,
        )
        .unwrap();
        // The genre rule is listed first, so it wins even though the artist
        // matches too.
        assert_eq!(
            rules.decide(&metadata(Some("Rock"), Some("Miles Davis"), None)),
            Some("Rock")
        );
        // Without a genre, the artist rule decides.
        assert_eq!(
            rules.decide(&metadata(Some("Jazz"), Some("Miles Davis"), None)),
            Some("Jazz")
        );
        // And without either, nothing matches.
        assert_eq!(rules.decide(&metadata(Some("Jazz"), None, None)), None);
    }

    #[test]
    fn every_tag_selects_its_own_metadata_field() {
        for (tag, metadata) in [
            (RuleTag::Genre, metadata(Some("Rock"), None, None)),
            (RuleTag::Artist, metadata(None, Some("Miles Davis"), None)),
            (RuleTag::Album, metadata(None, None, Some("Kind of Blue"))),
        ] {
            let rules = AutoLoadRules {
                rules: vec![AutoLoadRule {
                    tag,
                    value: match tag {
                        RuleTag::Genre => "Rock",
                        RuleTag::Artist => "Miles Davis",
                        RuleTag::Album => "Kind of Blue",
                    }
                    .to_string(),
                    preset: "Jazz".to_string(),
                }],
                default: None,
            };
            assert_eq!(rules.decide(&metadata), Some("Jazz"), "{tag:?}");
        }
    }

    #[test]
    fn a_value_matches_ignoring_case_and_surrounding_space() {
        let rules = parse(
            r#"
[[rule]]
tag = "genre"
value = "  rock "
preset = "Rock"
"#,
        )
        .unwrap();
        assert_eq!(
            rules.decide(&metadata(Some("ROCK"), None, None)),
            Some("Rock")
        );
        assert_eq!(
            rules.decide(&metadata(Some(" Rock "), None, None)),
            Some("Rock")
        );
        assert_eq!(
            rules.decide(&metadata(Some("Rock and Roll"), None, None)),
            None
        );
    }

    #[test]
    fn a_track_without_the_tag_does_not_match() {
        let rules = parse(GENRE_RULE).unwrap();
        assert_eq!(rules.decide(&metadata(None, Some("Rock"), None)), None);
        assert_eq!(rules.decide(&metadata(None, None, Some("Rock"))), None);
    }

    #[test]
    fn a_track_that_matches_no_rule_gets_the_default() {
        let rules = parse(
            r#"
default = "Flat"

[[rule]]
tag = "genre"
value = "Rock"
preset = "Rock"
"#,
        )
        .unwrap();
        assert_eq!(
            rules.decide(&metadata(Some("Jazz"), None, None)),
            Some("Flat")
        );
        // A matching rule still wins over the default.
        assert_eq!(
            rules.decide(&metadata(Some("Rock"), None, None)),
            Some("Rock")
        );
    }

    #[test]
    fn a_table_without_a_default_selects_nothing_for_an_unmatched_track() {
        let rules = parse(GENRE_RULE).unwrap();
        assert_eq!(rules.default, None);
        assert_eq!(rules.decide(&metadata(Some("Jazz"), None, None)), None);
    }

    #[test]
    fn an_empty_value_or_default_selects_nothing() {
        let rules = parse(
            r#"
default = ""

[[rule]]
tag = "genre"
value = "  "
preset = "Rock"
"#,
        )
        .unwrap();
        assert_eq!(rules.default, None);
        assert_eq!(rules.decide(&metadata(Some("Rock"), None, None)), None);
    }

    // ----- the file -----

    #[test]
    fn the_file_is_read_into_the_table() {
        let rules = parse(
            r#"
default = "Flat"

[[rule]]
tag = "album"
value = "Kind of Blue"
preset = "Jazz"
"#,
        )
        .unwrap();
        assert_eq!(rules.rules.len(), 1);
        assert_eq!(rules.rules[0].tag, RuleTag::Album);
        assert_eq!(rules.rules[0].value, "Kind of Blue");
        assert_eq!(rules.rules[0].preset, "Jazz");
        assert_eq!(rules.default.as_deref(), Some("Flat"));
    }

    #[test]
    fn an_unknown_tag_is_reported() {
        let error = parse(
            r#"
[[rule]]
tag = "genra"
value = "Rock"
preset = "Rock"
"#,
        )
        .unwrap_err();
        assert!(error.contains("genra"), "{error}");
        assert!(error.contains("genre, artist or album"), "{error}");
    }

    #[test]
    fn an_unknown_key_and_malformed_text_are_reported() {
        assert!(parse("rulle = 1\n").unwrap_err().contains("rulle"));
        assert!(!parse("this is = = not toml").unwrap_err().is_empty());
    }

    #[test]
    fn a_missing_file_leaves_no_rules_and_no_note() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = test_app();
        app.set_config_path(Some(dir.path().join("rustamp.toml")));
        assert!(app.eq_auto_rules.rules.is_empty());
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn a_malformed_file_is_noted_and_leaves_no_rules() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(AUTO_LOAD_FILE), "not toml = =").unwrap();
        let mut app = test_app();
        app.set_config_path(Some(dir.path().join("rustamp.toml")));
        assert!(app.eq_auto_rules.rules.is_empty());
        let notes = app.drain_notes();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains(AUTO_LOAD_FILE), "{notes:?}");
    }

    // ----- the track-start path -----

    #[test]
    fn a_matching_rule_applies_the_preset_when_the_track_starts() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rules(&dir, GENRE_RULE);
        app.submit(Command::SetEqAuto(true)).unwrap();
        app.handle_event(&track_started(metadata(Some("Rock"), None, None)));
        assert_eq!(app.engine().eq().bands, rock_bands());
    }

    #[test]
    fn an_artist_rule_selects_the_preset_from_the_track_s_artist() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rules(
            &dir,
            r#"
[[rule]]
tag = "artist"
value = "Miles Davis"
preset = "Jazz"
"#,
        );
        app.submit(Command::SetEqAuto(true)).unwrap();
        app.handle_event(&track_started(metadata(None, Some("Miles Davis"), None)));
        let jazz = crate::eq_presets::find_preset("Jazz").unwrap().bands;
        assert_eq!(app.engine().eq().bands, jazz);
        // Another artist keeps the curve.
        app.apply_eq_preset("Rock").unwrap();
        app.handle_event(&track_started(metadata(None, Some("Someone Else"), None)));
        assert_eq!(app.engine().eq().bands, rock_bands());
    }

    #[test]
    fn the_auto_flag_gates_the_auto_load() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rules(&dir, GENRE_RULE);
        // AUTO is off: the curve stays flat.
        app.handle_event(&track_started(metadata(Some("Rock"), None, None)));
        assert_eq!(
            app.engine().eq().bands,
            [0.0; rustamp_core::state::EQ_BANDS]
        );
        // Switching AUTO on makes the next track load the preset.
        app.submit(Command::SetEqAuto(true)).unwrap();
        app.handle_event(&track_started(metadata(Some("Rock"), None, None)));
        assert_eq!(app.engine().eq().bands, rock_bands());
    }

    #[test]
    fn a_track_without_a_rule_keeps_the_current_curve() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rules(&dir, GENRE_RULE);
        app.submit(Command::SetEqAuto(true)).unwrap();
        app.apply_eq_preset("Jazz").unwrap();
        let jazz = crate::eq_presets::find_preset("Jazz").unwrap().bands;
        app.handle_event(&track_started(metadata(Some("Jazz"), None, None)));
        assert_eq!(app.engine().eq().bands, jazz);
    }

    #[test]
    fn the_default_preset_is_applied_when_no_rule_matches() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rules(
            &dir,
            r#"
default = "Flat"

[[rule]]
tag = "genre"
value = "Rock"
preset = "Rock"
"#,
        );
        app.submit(Command::SetEqAuto(true)).unwrap();
        app.handle_event(&track_started(metadata(Some("Rock"), None, None)));
        assert_eq!(app.engine().eq().bands, rock_bands());
        // A track that matches nothing gets the default.
        app.handle_event(&track_started(metadata(Some("Jazz"), None, None)));
        assert_eq!(
            app.engine().eq().bands,
            [0.0; rustamp_core::state::EQ_BANDS]
        );
    }

    #[test]
    fn a_manual_change_during_the_track_survives_until_the_next_track_starts() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rules(&dir, GENRE_RULE);
        app.submit(Command::SetEqAuto(true)).unwrap();
        app.handle_event(&track_started(metadata(Some("Rock"), None, None)));
        // The user changes a band by hand while the track plays.
        app.submit(Command::SetBand {
            band: 0,
            gain_db: 9.0,
        })
        .unwrap();
        assert_eq!(app.engine().eq().bands[0], 9.0);
        // The auto-load runs at the track start only, so nothing overwrites the
        // manual change while the track plays.
        app.pump().unwrap();
        assert_eq!(app.engine().eq().bands[0], 9.0);
        // The next track start decides again, and then the auto-load wins.
        app.handle_event(&track_started(metadata(Some("Rock"), None, None)));
        assert_eq!(app.engine().eq().bands[0], rock_bands()[0]);
    }

    #[test]
    fn the_auto_load_leaves_the_on_and_auto_flags_alone() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rules(&dir, GENRE_RULE);
        app.submit(Command::SetEqAuto(true)).unwrap();
        let before = app.engine().eq();
        app.handle_event(&track_started(metadata(Some("Rock"), None, None)));
        let after = app.engine().eq();
        assert_eq!(
            after.enabled, before.enabled,
            "the ON flag survives the auto-load"
        );
        assert!(after.auto, "the AUTO flag survives the auto-load");
        // The curve did change: the auto-load did something.
        assert_eq!(after.bands, rock_bands());
    }

    #[test]
    fn a_rule_naming_an_unknown_preset_is_noted_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_rules(
            &dir,
            r#"
[[rule]]
tag = "genre"
value = "Rock"
preset = "No Such Preset"
"#,
        );
        app.submit(Command::SetEqAuto(true)).unwrap();
        app.handle_event(&track_started(metadata(Some("Rock"), None, None)));
        assert_eq!(
            app.engine().eq().bands,
            [0.0; rustamp_core::state::EQ_BANDS]
        );
        let notes = app.drain_notes();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("No Such Preset"), "{notes:?}");
    }
}
