// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! S12 conformance for the import of a Winamp 2.95 `winamp.ini`.
//!
//! The import has tests of its own (`src/winamp_ini.rs` and
//! `tests/winamp_ini.rs`), but they test **our** mapping: which key of the
//! reference file becomes which of our settings. This suite tests the
//! **spec's** facts. Every case names the section of
//! `specs/S12-winamp-ini.md` it comes from, quotes what that section says,
//! and asserts the spec's wording or the spec's numbers - never our
//! implementation's shape.
//!
//! | Case | The spec's section |
//! |---|---|
//! | `s12_section_2_*` | §2 - the shape of the file |
//! | `s12_section_3_*` | §3 - the observed keys |
//! | `s12_section_4_*` | §4 - how the player reads a key |
//! | `s12_section_5_*` | §5 - the recognised key set |
//! | `s12_section_6_*` | §6 - `eq_data` |
//! | `s12_section_7_*` | §7 - `prefs_last_page` |
//! | `s12_section_8_*` | §8 - the plug-in sections |
//! | `s12_section_9_*` | §9 - the practical notes for an implementer |
//!
//! Where our design deliberately does not follow the spec, the case says so in
//! its own words and gives the reason. A spec fact is never weakened to fit
//! the code: either the code follows the fact, or the case names the
//! difference and why it is deliberate.
//!
//! The suite is headless: it drives the import's pure seam (`plan`), so no
//! window is opened and no file is written.

use std::cell::RefCell;
use std::path::PathBuf;

use rustamp_app::winamp_ini::{ImportPlan, IniFile, Outcome, Reading, plan as import_plan};
use rustamp_core::config::{Config, WindowLayout};
use rustamp_core::state::{WindowId, eq_classic_from_db};

// ---------------------------------------------------------------------------
// The helpers
// ---------------------------------------------------------------------------

/// A resolver that finds no skin: the skins folder holds none.
fn no_skin(_name: &str) -> Option<PathBuf> {
    None
}

/// The import of a file's text, starting from the default configuration.
fn plan_of(text: &str) -> ImportPlan {
    import_plan(text, &Config::default(), None, &no_skin)
}

/// The layout of the window `id` in a configuration.
fn window(config: &Config, id: WindowId) -> &WindowLayout {
    config
        .ui
        .windows
        .iter()
        .find(|window| window.id == id)
        .unwrap_or_else(|| panic!("the configuration has a layout for {id:?}"))
}

/// The outcome the import gave the key `key` of the section `section`.
fn outcome_in<'a>(plan: &'a ImportPlan, section: &str, key: &str) -> &'a Outcome {
    plan.report
        .keys
        .iter()
        .find(|entry| {
            entry.section.eq_ignore_ascii_case(section) && entry.key.eq_ignore_ascii_case(key)
        })
        .map(|entry| &entry.outcome)
        .unwrap_or_else(|| panic!("the report has no key `{key}` in the section `{section}`"))
}

/// The reason the import did not apply the key `key` of the section
/// `section`.
///
/// A key that was applied has no reason: the case fails instead.
fn reason_in(plan: &ImportPlan, section: &str, key: &str) -> String {
    match outcome_in(plan, section, key) {
        Outcome::NotApplied { reason } => reason.clone(),
        Outcome::Applied { setting, .. } => {
            panic!("the key `{key}` was applied as `{setting}`, it has no reason")
        }
    }
}

/// The value the file gave the key `key` of the section `section`.
fn value_in(plan: &ImportPlan, section: &str, key: &str) -> String {
    plan.report
        .keys
        .iter()
        .find(|entry| {
            entry.section.eq_ignore_ascii_case(section) && entry.key.eq_ignore_ascii_case(key)
        })
        .map(|entry| entry.value.clone())
        .unwrap_or_else(|| panic!("the report has no key `{key}` in the section `{section}`"))
}

/// The setting the key `key` of the section `section` became.
fn applied_as(plan: &ImportPlan, section: &str, key: &str) -> String {
    match outcome_in(plan, section, key) {
        Outcome::Applied { setting, .. } => setting.clone(),
        Outcome::NotApplied { reason } => {
            panic!("the key `{key}` was not applied: {reason}")
        }
    }
}

/// The names the player recognises, spelled as they appear in the file
/// (S12 §5.1).
///
/// The list is the fact; the count that goes with it is open (S12 §5.3: the
/// counts do not add up and a recount is needed). The count asserted below
/// pins **this transcription** against the number the spec's prose gives, not
/// the reference's key table.
const RECOGNISED_KEYS: &[&str] = &[
    // skin, output, cwd
    "skin",
    "outname",
    "randskin",
    "cwd",
    "defext",
    // main window
    "wx",
    "wy",
    "mw_open",
    "minimized",
    "aot",
    "dsize",
    "easymove",
    "windowshade",
    "keeponscreen",
    "snap",
    "snaplen",
    "parent",
    "hilite",
    // equalizer
    "eq_wx",
    "eq_wy",
    "eq_open",
    "eq_ws",
    "eqdsize",
    "use_eq",
    "autoload_eq",
    "preamp",
    "eq_data",
    // playlist
    "pe_wx",
    "pe_wy",
    "pe_open",
    "pe_width",
    "pe_height",
    "pe_height_ws",
    "pe_fontsize",
    "shownumsinpl",
    "pladv",
    "addtolist",
    // minibrowser
    "mb_wx",
    "mb_wy",
    "mb_open",
    "mb_width",
    "mb_height",
    // video window
    "video_wx",
    "video_wy",
    "video_open",
    "video_width",
    "video_height",
    "video_ratio1",
    "video_ratio2",
    "video_useratio",
    "video_vsync",
    "video_aspectadj",
    "video_overlays",
    "video_ddraw",
    "video_updsize",
    "video_autoopen",
    "video_autoclose",
    "video_noss",
    "video_osd",
    "video_yv12",
    "video_stopclose",
    "video_remove_fs_on_stop",
    // transport
    "volume",
    "pan",
    "shuffle",
    "repeat",
    "shuffle_morph_rate",
    "timeleftmode",
    "autoscrollname",
    // spectrum analyser
    "sa",
    "safire",
    "saref",
    "safalloff",
    "sa_peaks",
    "sa_peak_falloff",
    // plug-ins
    "visplugin_num",
    "visplugin_name",
    "visplugin_autoexec",
    "visplugin_priority",
    "dspplugin_num",
    "dspplugin_name",
    // preferences, misc
    "prefs_last_page",
    "last_shortdesc",
    "last_shorturl",
    "proxy",
    "inet_mode",
    "browserbrand",
    "langpack",
    "extlist",
    "newverchk",
    "newverchk2",
    "check_ft_startup",
    "usecursors",
    "fixtitles",
    "ttips",
    "ascb_new",
    "taskbar",
    "splash",
    "sticon",
    "riol",
    "minst",
    "whichicon",
    "whichicon2",
    "disvis",
    "priority",
    "ospb",
    "bifont",
    "rofiob",
    "pilp",
];

/// The recognised keys the observed file did **not** carry (S12 §5.2).
///
/// The six plug-in-selection keys are named in §5.2 as well; they are already
/// in [`RECOGNISED_KEYS`], so the list here holds the thirty names the
/// section itemises one by one.
const ABSENT_KEYS: &[&str] = &[
    "bifont",
    "eqdsize",
    "check_ft_startup",
    "usecursors",
    "fixtitles",
    "keeponscreen",
    "snap",
    "snaplen",
    "parent",
    "hilite",
    "disvis",
    "minst",
    "riol",
    "ttips",
    "ascb_new",
    "taskbar",
    "splash",
    "sticon",
    "whichicon",
    "whichicon2",
    "addtolist",
    "rofiob",
    "priority",
    "ospb",
    "proxy",
    "inet_mode",
    "langpack",
    "extlist",
    "defext",
    "browserbrand",
];

// ---------------------------------------------------------------------------
// §2 - the shape of the file
// ---------------------------------------------------------------------------

/// S12 §2 - "An INI file: `[Section]` headers, `key=value`, one pair per
/// line, CRLF line endings, no quoting, no escaping. An empty value is
/// written as `key=` (e.g. `last_shortdesc=`)."
///
/// The code: the parser reads the sections in the order the file wrote them
/// and the keys of a section in the order it wrote them; a CRLF file and an
/// LF file read the same; an empty value is the empty string, not a missing
/// key.
///
/// The assertion: the shape the spec describes is the shape the parser
/// reports.
#[test]
fn s12_section_2_the_file_is_sections_and_key_value_pairs() {
    let text = "[Winamp]\r\n\
                volume=255\r\n\
                last_shortdesc=\r\n\
                \r\n\
                [Nullsoft Module Decoder]\r\n\
                cfg_title_format=$if2(%title%,%filename%)[ (%part%)]\r\n";
    let file = IniFile::parse(text);

    assert!(file.problems.is_empty(), "{:?}", file.problems);

    let sections: Vec<&str> = file.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(sections, ["Winamp", "Nullsoft Module Decoder"]);

    let player = file.section("Winamp").expect("the player's own section");
    let keys: Vec<&str> = player.entries.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(keys, ["volume", "last_shortdesc"]);
    assert_eq!(player.get("volume"), Some("255"));
    // An empty value is an empty value, not a missing key.
    assert_eq!(player.get("last_shortdesc"), Some(""));
}

/// S12 §2 - "no quoting, no escaping" and §9 - "values run to the end of the
/// line".
///
/// The code: the value of a pair is everything behind the first `=`; the
/// parser knows no quoting rule, so a quote in the file is a character of the
/// value.
///
/// The assertion: a quoted value is taken literally, and a second `=` belongs
/// to the value.
#[test]
fn s12_section_2_a_value_runs_to_the_end_of_the_line() {
    let file = IniFile::parse("[Winamp]\nskin=\"base-2.91.wsz\"\nlast_shortdesc=a=b\n");
    let player = file.section("Winamp").expect("the player's own section");

    assert_eq!(player.get("skin"), Some("\"base-2.91.wsz\""));
    assert_eq!(player.get("last_shortdesc"), Some("a=b"));
}

/// S12 §2 - "CRLF line endings".
///
/// The code: the parser reads the line endings the file uses; a CRLF file
/// and an LF file hold the same pairs.
///
/// The assertion: both spellings give the same sections, keys and values.
#[test]
fn s12_section_2_crlf_and_lf_read_the_same() {
    let crlf = IniFile::parse("[Winamp]\r\nvolume=200\r\nskin=base-2.91.wsz\r\n");
    let lf = IniFile::parse("[Winamp]\nvolume=200\nskin=base-2.91.wsz\n");

    assert_eq!(crlf.problems.is_empty(), lf.problems.is_empty());
    assert_eq!(
        crlf.section("Winamp").map(|s| s.entries.clone()),
        lf.section("Winamp").map(|s| s.entries.clone())
    );
}

/// S12 §2 - "The first section is `[Winamp]` and holds **all** the player's
/// own settings" and "Every further section belongs to **one** plug-in".
///
/// The code: the keys of the player's section are the player's; the keys of
/// every further section belong to a plug-in and are never applied.
///
/// The assertion: the player's own section is honoured, and the same key
/// name in a plug-in's section is not.
#[test]
fn s12_section_2_the_first_section_is_the_players_own() {
    let text = "[Winamp]\r\nvolume=200\r\n\r\n[IN_WAVE]\r\nvolume=255\r\n";
    let plan = plan_of(text);

    assert_eq!(
        plan.config.audio.volume, 200,
        "the player's own section counts"
    );
    assert_eq!(
        applied_as(&plan, "Winamp", "volume"),
        "audio.volume",
        "the player's own section is mapped"
    );
    // The plug-in's section is not the player's: the key is reported.
    assert!(
        matches!(
            outcome_in(&plan, "IN_WAVE", "volume"),
            Outcome::NotApplied { .. }
        ),
        "the plug-in's key was applied"
    );
    assert_eq!(
        plan.config.audio.volume, 200,
        "the plug-in's key changed the volume"
    );
}

/// S12 §2 - "Window positions are stored as an x then a y pair in `*_wx` /
/// `*_wy` keys; sizes as `*_width` / `*_height`."
///
/// The code: the position is two keys, and they become the x and the y of
/// the window; the size is two keys as well.
///
/// The assertion: the main window lands at the pair the file wrote, and the
/// playlist at the size the file wrote.
#[test]
fn s12_section_2_a_position_is_two_keys_and_a_size_is_two() {
    let plan = plan_of("[Winamp]\nwx=26\nwy=29\npe_width=275\npe_height=116\n");

    let main = window(&plan.config, WindowId::Main);
    assert_eq!((main.x, main.y), (26, 29));

    let playlist = window(&plan.config, WindowId::Playlist);
    assert_eq!((playlist.width, playlist.height), (Some(275), Some(116)));
}

/// S12 §2 - "Lists are comma-separated without spaces (`eq_data=31,31,…,31`)"
/// and "Colours and binary blobs are hex strings without a prefix
/// (`cfg_wfx=0100…`)".
///
/// The code: the parser keeps the list and the hex string as the file wrote
/// them; the list is read entry by entry by the equalizer's mapping (§6).
///
/// The assertion: the list is kept whole, and the hex string keeps its digits
/// and carries no prefix of its own.
#[test]
fn s12_section_2_a_list_and_a_hex_blob_are_kept_as_they_were_written() {
    let text = "[Winamp]\neq_data=31,31,31,31,31,31,31,31,31,31\r\n\
                \r\n\
                [Nullsoft Module Decoder]\r\n\
                cfg_wfx=0100\r\n";
    let plan = plan_of(text);

    assert_eq!(
        value_in(&plan, "Winamp", "eq_data"),
        "31,31,31,31,31,31,31,31,31,31",
        "the list keeps no spaces"
    );
    assert_eq!(
        value_in(&plan, "Nullsoft Module Decoder", "cfg_wfx"),
        "0100"
    );
}

// ---------------------------------------------------------------------------
// §3 - the observed keys
// ---------------------------------------------------------------------------

/// S12 §3.1 - `skin` holds "the skin file name (not a path)" and `outname`
/// the selected output plug-in "by file name".
///
/// The code: the skin's file name is handed to the caller's resolver and the
/// resolved path becomes our `ui.skin`; the output plug-in is not ours - our
/// backend comes from the command line - so the key is reported.
///
/// The assertion: the resolver is asked for the file name alone, and the
/// output plug-in is not applied.
#[test]
fn s12_section_3_1_the_skin_is_a_file_name_and_the_output_plug_in_is_not_ours() {
    let asked: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let text = "[Winamp]\nskin=base-2.91.wsz\noutname=out_wave.dll\nrandskin=0\n";
    let plan = import_plan(text, &Config::default(), None, &|name: &str| {
        asked.borrow_mut().push(name.to_string());
        Some(PathBuf::from("skins").join(name))
    });

    assert_eq!(
        asked.borrow().as_slice(),
        ["base-2.91.wsz"],
        "the resolver is asked for the file name, not for a path"
    );
    assert_eq!(
        plan.config.ui.skin,
        Some(PathBuf::from("skins").join("base-2.91.wsz"))
    );

    let outname = reason_in(&plan, "Winamp", "outname");
    assert!(outname.contains("command line"), "{outname}");
    assert!(
        matches!(
            outcome_in(&plan, "Winamp", "outname"),
            Outcome::NotApplied { .. }
        ),
        "the output plug-in was applied"
    );
    // `randskin` is the reference's own "pick a random skin" flag; the player
    // has no such setting, so the key is reported.
    assert!(matches!(
        outcome_in(&plan, "Winamp", "randskin"),
        Outcome::NotApplied { .. }
    ));
}

/// S12 §3.2 - the observed values of the volume, the balance and the
/// equalizer: `volume=0`, `pan=0`, `use_eq=1`, `autoload_eq=1`, `preamp=31`
/// and `eq_data=31,31,31,31,31,31,31,31,31,31`.
///
/// The code: the volume is our volume, the balance our balance, the two
/// equalizer flags our flags, and the preamp and the ten bands the slider's
/// own values turned into gains (S07 §7: `31` is flat, `0` is +12 dB, `63` is
/// −12 dB).
///
/// The assertion: the observed numbers land as the spec's numbers say.
#[test]
fn s12_section_3_2_the_observed_volume_and_equalizer_values_land() {
    let text = "[Winamp]\nvolume=0\npan=0\nuse_eq=1\nautoload_eq=1\npreamp=31\n\
                eq_data=31,31,31,31,31,31,31,31,31,31\n";
    let plan = plan_of(text);

    assert_eq!(plan.config.audio.volume, 0);
    assert_eq!(plan.config.audio.balance, 0, "0 is the centre");
    assert!(plan.config.eq.state.enabled);
    assert!(plan.config.eq.state.auto);
    assert_eq!(plan.config.eq.state.preamp, 0.0, "31 is flat");
    assert_eq!(plan.config.eq.state.bands, [0.0; 10], "every band is flat");
}

/// S12 §3.2 - the balance's range and direction are **open** (S12 §3.2's note
/// and S12 §10, open question 4), and the classic gain scale is ours (S07
/// §7).
///
/// The code: the report marks the keys it read in **our** way, so the user
/// is never told that the reference's meaning was applied.
///
/// The assertion: the balance and the gain keys are marked as our own
/// reading, the volume is not - the volume's range is the spec's.
#[test]
fn s12_section_3_2_the_keys_we_read_in_our_own_way_are_marked() {
    let plan = plan_of("[Winamp]\npan=0\npreamp=31\neq_data=31\nvolume=0\n");

    for key in ["pan", "preamp", "eq_data"] {
        assert!(
            matches!(
                outcome_in(&plan, "Winamp", key),
                Outcome::Applied {
                    reading: Reading::Own,
                    ..
                }
            ),
            "the key `{key}` is not marked as our own reading"
        );
    }
    assert!(
        matches!(
            outcome_in(&plan, "Winamp", "volume"),
            Outcome::Applied {
                reading: Reading::Spec,
                ..
            }
        ),
        "the volume is not the spec's reading alone"
    );
}

/// S12 §3.3 - the observed values of the transport and the playlist:
/// `shuffle=0`, `repeat=0` and `pe_fontsize=10`.
///
/// The code: the two flags are our flags, and the font size becomes the
/// playlist editor's size.
///
/// The assertion: the observed numbers land as the spec's numbers say.
#[test]
fn s12_section_3_3_the_observed_transport_and_playlist_values_land() {
    let plan = plan_of("[Winamp]\nshuffle=0\nrepeat=0\npe_fontsize=10\n");

    assert!(!plan.config.playlist.shuffle);
    assert!(!plan.config.playlist.repeat);
    assert_eq!(plan.playlist_font_size, Some(10));
}

/// S12 §3.3 - `pladv`, `shownumsinpl`, `timeleftmode` and `autoscrollname`
/// are the player's own settings, and `pilp` is "unknown - no meaning was
/// established; **open**" (S12 §10, open question 2).
///
/// The code: the player has no setting for the first four, and the
/// reference's own meaning for the fifth is unknown, so all five are
/// reported - never applied silently.
///
/// The assertion: every one of them is reported with a reason.
#[test]
fn s12_section_3_3_the_keys_the_player_has_no_setting_for_are_reported() {
    let text = "[Winamp]\npladv=1\nshownumsinpl=1\ntimeleftmode=0\nautoscrollname=1\npilp=0\n";
    let plan = plan_of(text);

    for key in [
        "pladv",
        "shownumsinpl",
        "timeleftmode",
        "autoscrollname",
        "pilp",
    ] {
        assert!(
            matches!(outcome_in(&plan, "Winamp", key), Outcome::NotApplied { .. }),
            "the key `{key}` was applied"
        );
        assert!(
            !reason_in(&plan, "Winamp", key).is_empty(),
            "`{key}` has no reason"
        );
    }
    // The reference's own meaning for `pilp` is unknown to the spec as well.
    assert!(reason_in(&plan, "Winamp", "pilp").contains("unknown"));
}

/// S12 §3.4 - the observed window placements: `wx=26`, `wy=29`,
/// `eq_wx=26`, `eq_wy=145`, `pe_wx=26`, `pe_wy=261`, `video_wx=26`,
/// `video_wy=145`.
///
/// The code: each pair becomes the position of the window its key names.
///
/// The assertion: every window lands at the pair the file wrote.
#[test]
fn s12_section_3_4_the_observed_window_placements_land() {
    let text = "[Winamp]\nwx=26\nwy=29\neq_wx=26\neq_wy=145\npe_wx=26\npe_wy=261\n\
                video_wx=26\nvideo_wy=145\n";
    let plan = plan_of(text);

    let main = window(&plan.config, WindowId::Main);
    assert_eq!((main.x, main.y), (26, 29));

    let equalizer = window(&plan.config, WindowId::Equalizer);
    assert_eq!((equalizer.x, equalizer.y), (26, 145));

    let playlist = window(&plan.config, WindowId::Playlist);
    assert_eq!((playlist.x, playlist.y), (26, 261));

    let video = window(&plan.config, WindowId::Video);
    assert_eq!((video.x, video.y), (26, 145));
}

/// S12 §3.4 - the observed sizes and shade flags: `pe_width=275`,
/// `pe_height=116`, `windowshade=0`, `eq_ws=0`, and the visibility keys
/// `mw_open=1`, `eq_open=1`, `pe_open=1`, `video_open=0`.
///
/// The code: the sizes become the window's size, the shade flags its shade
/// state and the visibility keys its visibility.
///
/// The assertion: the observed numbers land as the spec's numbers say.
#[test]
fn s12_section_3_4_the_observed_sizes_shade_and_visibility_land() {
    let text = "[Winamp]\npe_width=275\npe_height=116\nwindowshade=0\neq_ws=0\n\
                mw_open=1\neq_open=1\npe_open=1\nvideo_open=0\n";
    let plan = plan_of(text);

    let playlist = window(&plan.config, WindowId::Playlist);
    assert_eq!((playlist.width, playlist.height), (Some(275), Some(116)));

    assert!(!window(&plan.config, WindowId::Main).shaded);
    assert!(!window(&plan.config, WindowId::Equalizer).shaded);

    assert!(window(&plan.config, WindowId::Main).visible);
    assert!(window(&plan.config, WindowId::Equalizer).visible);
    assert!(window(&plan.config, WindowId::Playlist).visible);
    assert!(!window(&plan.config, WindowId::Video).visible);
}

/// S12 §3.4 - the minibrowser's keys (`mb_wx=301`, `mb_wy=29`,
/// `mb_width=350`, `mb_height=348`, `mb_open=1`), the playlist's shaded
/// height (`pe_height_ws=0`) and the minimized flag (`minimized=0`).
///
/// The code: the player has no minibrowser window, the editor's shaded
/// height is fixed by the layout (S01 §1) and the player keeps no minimized
/// state, so all six are reported.
///
/// The assertion: every one of them is reported with a reason.
#[test]
fn s12_section_3_4_the_window_keys_the_player_has_no_setting_for_are_reported() {
    let text = "[Winamp]\nmb_wx=301\nmb_wy=29\nmb_width=350\nmb_height=348\nmb_open=1\n\
                pe_height_ws=0\nminimized=0\n";
    let plan = plan_of(text);

    for key in [
        "mb_wx",
        "mb_wy",
        "mb_width",
        "mb_height",
        "mb_open",
        "pe_height_ws",
        "minimized",
    ] {
        assert!(
            matches!(outcome_in(&plan, "Winamp", key), Outcome::NotApplied { .. }),
            "the key `{key}` was applied"
        );
        assert!(
            !reason_in(&plan, "Winamp", key).is_empty(),
            "`{key}` has no reason"
        );
    }
}

/// S12 §3.5 - the six analyser keys (`sa=1`, `safire=4`, `saref=2`,
/// `safalloff=2`, `sa_peaks=1`, `sa_peak_falloff=1`) are the player's own
/// settings; `safire` and `saref` have an **unknown** meaning (S12 §10,
/// open question 2).
///
/// The code: the player has no setting for the built-in analyser, and the
/// reference's own meaning for two of the keys is unknown, so all six are
/// reported.
///
/// The assertion: every one of them is reported with a reason.
#[test]
fn s12_section_3_5_the_analyser_keys_are_reported() {
    let text = "[Winamp]\nsa=1\nsafire=4\nsaref=2\nsafalloff=2\nsa_peaks=1\nsa_peak_falloff=1\n";
    let plan = plan_of(text);

    for key in [
        "sa",
        "safire",
        "saref",
        "safalloff",
        "sa_peaks",
        "sa_peak_falloff",
    ] {
        assert!(
            matches!(outcome_in(&plan, "Winamp", key), Outcome::NotApplied { .. }),
            "the key `{key}` was applied"
        );
        assert!(
            !reason_in(&plan, "Winamp", key).is_empty(),
            "`{key}` has no reason"
        );
    }
    for key in ["safire", "saref"] {
        assert!(reason_in(&plan, "Winamp", key).contains("unknown"));
    }
}

/// S12 §3.6 - the video window's playback keys (`video_ratio1`,
/// `video_ratio2`, `video_useratio`, `video_vsync`, `video_aspectadj`,
/// `video_overlays`, `video_ddraw`, `video_updsize`, `video_autoopen`,
/// `video_autoclose`, `video_noss`, `video_osd`, `video_yv12`,
/// `video_stopclose`, `video_remove_fs_on_stop`).
///
/// The code: the player has no setting for the video window's playback, so
/// every one of them is reported.
///
/// The assertion: every one of them is reported with a reason.
#[test]
fn s12_section_3_6_the_video_playback_keys_are_reported() {
    let keys = [
        "video_ratio1",
        "video_ratio2",
        "video_useratio",
        "video_vsync",
        "video_aspectadj",
        "video_overlays",
        "video_ddraw",
        "video_updsize",
        "video_autoopen",
        "video_autoclose",
        "video_noss",
        "video_osd",
        "video_yv12",
        "video_stopclose",
        "video_remove_fs_on_stop",
    ];
    let mut text = String::from("[Winamp]\n");
    for key in keys {
        text.push_str(key);
        text.push_str("=1\n");
    }
    let plan = plan_of(&text);

    for key in keys {
        assert!(
            matches!(outcome_in(&plan, "Winamp", key), Outcome::NotApplied { .. }),
            "the key `{key}` was applied"
        );
        assert!(
            !reason_in(&plan, "Winamp", key).is_empty(),
            "`{key}` has no reason"
        );
    }
}

/// S12 §3.7 - the miscellaneous keys: `cwd`, `last_shortdesc`,
/// `last_shorturl` (both empty in the observed run), `newverchk` and
/// `newverchk2`.
///
/// The code: the empty values are read as empty values; the player keeps no
/// shortcut description or URL and does not check for a new version, so all
/// five are reported.
///
/// The assertion: the empty values are read as the empty string, and every
/// key is reported.
#[test]
fn s12_section_3_7_the_miscellaneous_keys_are_reported() {
    let text =
        "[Winamp]\ncwd=C:\\Temp\nlast_shortdesc=\nlast_shorturl=\nnewverchk=1\nnewverchk2=1\n";
    let plan = plan_of(text);

    assert_eq!(value_in(&plan, "Winamp", "last_shortdesc"), "");
    assert_eq!(value_in(&plan, "Winamp", "last_shorturl"), "");

    for key in [
        "cwd",
        "last_shortdesc",
        "last_shorturl",
        "newverchk",
        "newverchk2",
    ] {
        assert!(
            matches!(outcome_in(&plan, "Winamp", key), Outcome::NotApplied { .. }),
            "the key `{key}` was applied"
        );
    }
}

// ---------------------------------------------------------------------------
// §4 - how the player reads a key
// ---------------------------------------------------------------------------

/// S12 §4.1 - "The names the player looks up carry a seven-character
/// `config_` prefix; **the name in the file is the name without that
/// prefix**. `config_volume` reads `volume=`, so a file that wrote
/// `config_volume=` would not be found."
///
/// The code: the import looks a key up by the name the file spells, so a file
/// that wrote `config_volume=` names a key the reference does not know. The
/// volume then keeps the value it had - the reference falls back to the
/// caller's default (S12 §4.2), we fall back to our current configuration.
///
/// The assertion: `config_volume=` is **not** honoured as `volume`, the
/// volume stays as it was, and the report says the key is not one the player
/// recognises.
#[test]
fn s12_section_4_1_the_file_spells_the_key_without_the_config_prefix() {
    let mut current = Config::default();
    current.audio.volume = 77;

    let plan = import_plan("[Winamp]\nconfig_volume=200\n", &current, None, &no_skin);

    assert!(
        !plan.report.applied_key("config_volume"),
        "the prefixed name was applied"
    );
    assert_eq!(
        plan.config.audio.volume, 77,
        "the volume the file does not name in the right way stays"
    );
    let reason = plan
        .report
        .reason_for("config_volume")
        .expect("the key is in the report");
    assert!(reason.contains("does not recognise"), "{reason}");

    // The name the file may write is honoured.
    let honoured = plan_of("[Winamp]\nvolume=200\n");
    assert!(honoured.report.applied_key("volume"));
    assert_eq!(honoured.config.audio.volume, 200);
}

/// S12 §4.2 - "A key that is not in the file yields the **default** that the
/// caller passes, and that default is the player's own initial value for the
/// setting. A missing key therefore leaves the program at its own starting
/// value."
///
/// The code: our default is our current configuration, so a key the file does
/// not carry leaves the setting as it is. The report does not mention such a
/// key at all: there is nothing to report about a key that is not there.
///
/// The assertion: `snap` - one of the keys the observed file did **not**
/// carry (S12 §5.2) - changes nothing and is not in the report.
#[test]
fn s12_section_4_2_a_key_the_file_does_not_carry_leaves_the_setting_alone() {
    let mut current = Config::default();
    current.ui.snap_distance = 25;

    let plan = import_plan("[Winamp]\nvolume=200\n", &current, None, &no_skin);

    assert_eq!(
        plan.config.ui.snap_distance, 25,
        "the snap distance the file does not name stays"
    );
    assert!(
        !plan
            .report
            .keys
            .iter()
            .any(|entry| entry.key.eq_ignore_ascii_case("snap")),
        "the report mentions a key the file does not carry"
    );
    assert!(!plan.report.applied_key("snap"));
}

/// S12 §4.3 - "A string value is read through a 2048-byte staging buffer,
/// and the caller passes its own size; a value longer than the buffer is
/// truncated by the profile API. In practice a value beyond 2048 bytes is cut
/// short." and §9 - "A value longer than 2048 bytes is cut short (§4.3)."
///
/// The code: the parser cuts a value at the buffer's size, on a character
/// boundary - the file is read as text, and half a character is not a value.
///
/// The assertion: a 3000-byte value is cut at 2048 bytes, both in the report
/// and in what the mapper saw; a value of exactly 2048 bytes is kept whole.
#[test]
fn s12_section_4_3_a_value_longer_than_2048_bytes_is_cut_short() {
    let asked: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let long = "a".repeat(3000);
    let plan = import_plan(
        &format!("[Winamp]\nskin={long}\n"),
        &Config::default(),
        None,
        &|name: &str| {
            asked.borrow_mut().push(name.to_string());
            None
        },
    );

    assert_eq!(
        value_in(&plan, "Winamp", "skin").len(),
        2048,
        "the value was not cut short"
    );
    assert_eq!(
        asked.borrow()[0].len(),
        2048,
        "the mapper did not see the cut value"
    );

    // A value of exactly the buffer's size is kept whole.
    let exact = "b".repeat(2048);
    let plan = plan_of(&format!("[Winamp]\nskin={exact}\n"));
    assert_eq!(value_in(&plan, "Winamp", "skin").len(), 2048);
}

// ---------------------------------------------------------------------------
// §5 - the complete recognised key set
// ---------------------------------------------------------------------------

/// S12 §5.1 - "The player recognises the following keys, spelled as they
/// appear **in the file** (that is, without the `config_` prefix). This is
/// the complete set the player recognises."
///
/// The code: the import knows every one of them: a recognised key is either
/// applied or reported with a reason of our own. Only a key **outside** the
/// set is reported as "the reference player does not recognise this key".
///
/// The assertion: no name of the spec's list falls into that branch.
#[test]
fn s12_section_5_1_every_recognised_key_is_known_to_the_import() {
    assert_eq!(
        RECOGNISED_KEYS.len(),
        109,
        "the transcription of §5.1 lost or gained a name"
    );

    let mut text = String::from("[Winamp]\r\n");
    for key in RECOGNISED_KEYS {
        text.push_str(key);
        text.push_str("=1\r\n");
    }
    let plan = plan_of(&text);

    for key in RECOGNISED_KEYS {
        let reason = plan.report.reason_for(key).unwrap_or_default();
        assert!(
            !reason.contains("does not recognise"),
            "the import does not know the key `{key}`"
        );
    }
}

/// S12 §5.2 - "The reference can leave a recognised key out entirely. The
/// observed file did **not** carry these: `bifont`, `eqdsize`,
/// `check_ft_startup`, ... `browserbrand` - and all six plug-in-selection
/// keys."
///
/// The code: a key the observed file lacked is still a key the player
/// recognises: if the file carries it, the import knows it and reports it
/// with a reason of its own; if the file does not carry it, the setting
/// stays as it was (S12 §4.2).
///
/// The assertion: none of the absent names is unknown to the import, and the
/// six plug-in-selection keys are known as well.
#[test]
fn s12_section_5_2_a_key_the_observed_file_lacked_is_still_recognised() {
    assert_eq!(
        ABSENT_KEYS.len(),
        30,
        "the transcription of §5.2 lost or gained a name"
    );

    let mut text = String::from("[Winamp]\r\n");
    for key in ABSENT_KEYS {
        text.push_str(key);
        text.push_str("=1\r\n");
    }
    for key in [
        "visplugin_num",
        "visplugin_name",
        "visplugin_autoexec",
        "visplugin_priority",
        "dspplugin_num",
        "dspplugin_name",
    ] {
        text.push_str(key);
        text.push_str("=1\r\n");
    }
    let plan = plan_of(&text);

    for key in ABSENT_KEYS {
        let reason = plan.report.reason_for(key).unwrap_or_default();
        assert!(
            !reason.contains("does not recognise"),
            "`{key}` is unknown to the import"
        );
        assert!(!reason.is_empty(), "the key `{key}` was applied");
    }
    for key in [
        "visplugin_num",
        "visplugin_name",
        "visplugin_autoexec",
        "visplugin_priority",
        "dspplugin_num",
        "dspplugin_name",
    ] {
        let reason = plan.report.reason_for(key).unwrap_or_default();
        assert!(
            !reason.contains("does not recognise"),
            "`{key}` is unknown to the import"
        );
        assert!(!reason.is_empty(), "the key `{key}` was applied");
    }
}

// ---------------------------------------------------------------------------
// §6 - `eq_data`: parsing and clamping
// ---------------------------------------------------------------------------

/// S12 §6 - "exactly **ten** values are taken; the parse stops at the first
/// non-numeric character; a value below 0 becomes **0**, a value above **63**
/// becomes **63**; a missing or unparsable entry leaves the band at whatever
/// it was - the parse stops, it does not skip to the next entry; the
/// separator counts as one character, so `1,2,3` and `1,,3` behave
/// differently: an empty entry stops the parse."
///
/// The code: the ten band values are read one by one; a value that is not a
/// number stops the parse, and the bands the parse did not reach keep the
/// value they had.
///
/// The assertion: the exact band values - the first band clamped down from
/// 64 to 63, the second clamped up from −5 to 0, and every band behind the
/// empty entry untouched.
#[test]
fn s12_section_6_eq_data_is_clamped_and_stops_at_the_first_entry_it_cannot_read() {
    let mut current = Config::default();
    // A value of its own for the bands the parse does not reach.
    current.eq.state.bands = [3.0; 10];

    let plan = import_plan("[Winamp]\neq_data=64,-5,,31\n", &current, None, &no_skin);

    // 64 is above 63, so it becomes 63 - which is −12 dB on the slider's
    // scale (S07 §7).
    assert_eq!(plan.config.eq.state.bands[0], -12.0);
    assert_eq!(eq_classic_from_db(plan.config.eq.state.bands[0]), 63);

    // −5 is below 0, so it becomes 0 - which is +12 dB.
    assert_eq!(plan.config.eq.state.bands[1], 12.0);
    assert_eq!(eq_classic_from_db(plan.config.eq.state.bands[1]), 0);

    // The empty entry stopped the parse: the third band and every band
    // behind it keep the value they had - including the `31` that stands
    // behind the empty entry.
    for band in 2..10 {
        assert_eq!(
            plan.config.eq.state.bands[band], 3.0,
            "the band {band} does not keep its value"
        );
    }
}

/// S12 §6 - "exactly **ten** values are taken".
///
/// The code: the parse stops at the tenth value; the rest of the list is
/// ignored.
///
/// The assertion: the ten bands take the first ten values of a twelve-value
/// list.
#[test]
fn s12_section_6_eq_data_takes_exactly_ten_values() {
    let plan = plan_of("[Winamp]\neq_data=1,2,3,4,5,6,7,8,9,10,11,12\n");

    for (band, value) in plan.config.eq.state.bands.iter().enumerate() {
        assert_eq!(
            eq_classic_from_db(*value),
            band as i32 + 1,
            "the band {band} does not hold the value the file wrote"
        );
    }
}

/// S12 §6 - "the parse stops at the first non-numeric character" and "the
/// separator counts as one character, so `1,2,3` and `1,,3` behave
/// differently".
///
/// The code: a character that is not a digit ends the parse; it does not skip
/// to the next entry.
///
/// The assertion: `1,2,3` fills three bands, `1,,3` fills one, and
/// `1,2,x,4` fills two.
#[test]
fn s12_section_6_eq_data_stops_at_the_first_non_numeric_character() {
    let mut current = Config::default();
    current.eq.state.bands = [3.0; 10];

    let plan = import_plan("[Winamp]\neq_data=1,2,3\n", &current, None, &no_skin);
    assert_eq!(eq_classic_from_db(plan.config.eq.state.bands[0]), 1);
    assert_eq!(eq_classic_from_db(plan.config.eq.state.bands[1]), 2);
    assert_eq!(eq_classic_from_db(plan.config.eq.state.bands[2]), 3);
    assert_eq!(
        plan.config.eq.state.bands[3], 3.0,
        "the fourth band was reached"
    );

    let plan = import_plan("[Winamp]\neq_data=1,,3\n", &current, None, &no_skin);
    assert_eq!(eq_classic_from_db(plan.config.eq.state.bands[0]), 1);
    assert_eq!(
        plan.config.eq.state.bands[1], 3.0,
        "the empty entry did not stop the parse"
    );
    assert_eq!(plan.config.eq.state.bands[2], 3.0);

    let plan = import_plan("[Winamp]\neq_data=1,2,x,4\n", &current, None, &no_skin);
    assert_eq!(eq_classic_from_db(plan.config.eq.state.bands[1]), 2);
    assert_eq!(
        plan.config.eq.state.bands[2], 3.0,
        "the parse skipped to the next entry"
    );
}

// ---------------------------------------------------------------------------
// §7 - `prefs_last_page`: two numbers in one key
// ---------------------------------------------------------------------------

/// S12 §7 - "The one integer that carries **two** numbers: its **low byte**
/// is the Preferences page index restored at start, and the bits from bit 8
/// upwards carry a second number, which the player keeps separately. The
/// second number's meaning was not established - **open**."
///
/// The code: the import reports the key and changes nothing.
///
/// **Deliberate difference.** The reference restores the page that sits in the
/// key's low byte; we restore no page at all. The reason: our configuration
/// keeps no preferences-page index (the dialog's pages live in the dialog,
/// not in the configuration file), and the spec's second number is open, so
/// there is nothing we could honour above bit 7 either. The spec's fact is
/// not weakened: the report names both parts of the key, so the user can see
/// that the key carries two numbers.
///
/// The assertion: the key is reported, the reason names the low byte and the
/// second number, the value is reported as the file wrote it, and the
/// configuration is untouched.
#[test]
fn s12_section_7_prefs_last_page_carries_two_numbers() {
    // 0x0102: the low byte is the page index (2), the bits from bit 8
    // upwards carry the second number (1).
    let plan = plan_of("[Winamp]\nprefs_last_page=258\n");

    assert!(
        matches!(
            outcome_in(&plan, "Winamp", "prefs_last_page"),
            Outcome::NotApplied { .. }
        ),
        "the key was applied"
    );
    let reason = reason_in(&plan, "Winamp", "prefs_last_page");
    assert!(reason.contains("low byte"), "{reason}");
    assert!(reason.contains("second number"), "{reason}");
    assert_eq!(value_in(&plan, "Winamp", "prefs_last_page"), "258");
    assert_eq!(plan.config, Config::default(), "the configuration changed");
}

// ---------------------------------------------------------------------------
// §8 - the plug-in sections
// ---------------------------------------------------------------------------

/// S12 §8 - "Each plug-in's section is named by its display name and its
/// keys are the plug-in's own; the player does not interpret them." and
/// §8.1 - the observed section names are the display names of the three
/// plug-ins: `WAV Writing Output Driver` (`out_wave.dll`), `Nullsoft Module
/// Decoder` (`in_mod.dll`) and `IN_WAVE` (`in_wave.dll`).
///
/// The code: a section that is not the player's belongs to a plug-in: every
/// key in it is reported, and none of them is applied - not even a key the
/// player's own section would honour.
///
/// The assertion: the plug-in's keys are reported as unhonourable, never
/// silently applied, and the configuration stays as it was.
#[test]
fn s12_section_8_a_plug_ins_section_is_not_the_players() {
    let text = "[Winamp]\r\n\
                volume=200\r\n\
                \r\n\
                [WAV Writing Output Driver]\r\n\
                volume=255\r\n\
                \r\n\
                [Nullsoft Module Decoder]\r\n\
                cfg_title_format=$if2(%title%,%filename%)[ (%part%)]\r\n\
                \r\n\
                [IN_WAVE]\r\n\
                cfg_wfx=0100\r\n";
    let plan = plan_of(text);

    // The player's own section is honoured.
    assert_eq!(plan.config.audio.volume, 200);

    // The plug-in's section is not: the same key name is reported there and
    // changes nothing.
    assert!(
        matches!(
            outcome_in(&plan, "WAV Writing Output Driver", "volume"),
            Outcome::NotApplied { .. }
        ),
        "the plug-in's key was applied"
    );
    let reason = reason_in(&plan, "WAV Writing Output Driver", "volume");
    assert!(reason.contains("does not interpret"), "{reason}");
    assert!(reason.contains("WAV Writing Output Driver"), "{reason}");
    assert_eq!(
        plan.config.audio.volume, 200,
        "the plug-in's key changed the volume"
    );
}

/// S12 §8.3 - the observed key of the module decoder's own section is
/// `cfg_title_format=$if2(%title%,%filename%)[ (%part%)]`, and §2 - a
/// colour or binary blob is a hex string without a prefix
/// (`cfg_wfx=0100…`).
///
/// The code: the plug-in's own keys keep the names the file gave them - the
/// `cfg_` prefix stays as it is written, in contrast to the player's
/// `config_` prefix, which the file must not write (S12 §4.1). The player
/// does not interpret them, so the report names them as the plug-in's.
///
/// The assertion: the key and the value are reported exactly as the file
/// wrote them.
#[test]
fn s12_section_8_3_a_plug_ins_own_keys_keep_the_names_they_were_written_with() {
    let text = "[Nullsoft Module Decoder]\r\n\
                cfg_title_format=$if2(%title%,%filename%)[ (%part%)]\r\n\
                cfg_wfx=0100\r\n";
    let plan = plan_of(text);

    assert_eq!(
        value_in(&plan, "Nullsoft Module Decoder", "cfg_title_format"),
        "$if2(%title%,%filename%)[ (%part%)]"
    );
    assert_eq!(
        value_in(&plan, "Nullsoft Module Decoder", "cfg_wfx"),
        "0100"
    );

    // The name in the report is the name in the file, prefix included.
    let names: Vec<&str> = plan
        .report
        .keys
        .iter()
        .map(|entry| entry.key.as_str())
        .collect();
    assert_eq!(names, ["cfg_title_format", "cfg_wfx"]);
}

// ---------------------------------------------------------------------------
// §9 - the practical notes for an implementer
// ---------------------------------------------------------------------------

/// S12 §9 - "Clamp only where a range is known: `volume` is `0..=255`,
/// `preamp` is `0..=63`, and an `eq_data` value is clamped to `0..=63` with
/// exactly ten entries taken (§6). Every other numeric key's range is open."
///
/// The code: the volume and the preamp are clamped into the ranges the spec
/// names; a key whose range is open keeps the number the file wrote.
///
/// **Deliberate difference.** The balance is clamped into our contract's
/// domain (`-127..=127`), not into a range the spec knows: the spec's range
/// and direction for `pan` are open (S12 §10, open question 4), and our
/// balance is a signed byte. The report marks the balance as a key we read in
/// our own way.
///
/// The assertion: the two known ranges are clamped, an open range is not, and
/// the balance is marked as our own reading.
#[test]
fn s12_section_9_the_clamp_is_only_where_a_range_is_known() {
    let plan = plan_of("[Winamp]\nvolume=300\npreamp=99\npan=300\nwx=1000\npe_width=9999\n");

    assert_eq!(plan.config.audio.volume, 255, "`volume` is 0..=255");
    assert_eq!(plan.config.eq.state.preamp, -12.0, "`preamp` is 0..=63");
    assert_eq!(
        eq_classic_from_db(plan.config.eq.state.preamp),
        63,
        "`preamp` is clamped to 63"
    );

    // An open range keeps the number the file wrote.
    assert_eq!(window(&plan.config, WindowId::Main).x, 1000);
    assert_eq!(window(&plan.config, WindowId::Playlist).width, Some(9999));

    // The balance is clamped into our own domain, and marked as ours.
    assert_eq!(plan.config.audio.balance, 127);
    assert!(
        matches!(
            outcome_in(&plan, "Winamp", "pan"),
            Outcome::Applied {
                reading: Reading::Own,
                ..
            }
        ),
        "the balance is not marked as our own reading"
    );
}

/// S12 §9 - "Round-trip what you read: preserve keys outside the recognised
/// set of §5 as well, and preserve unknown sections. The file held more keys
/// than the player recognises (§5.3) ... so a writer must not drop what it
/// did not understand."
///
/// The code: the import never writes the reference file - it reads the file
/// and produces **our** configuration - so there is no writer that could drop
/// what it did not understand. What the import does instead is report every
/// key it did not apply: the keys outside the recognised set of §5.1 and the
/// keys of a section it does not know.
///
/// The assertion: the unknown key and the unknown section's key are both in
/// the report, and the report holds one line per key of the file.
#[test]
fn s12_section_9_nothing_is_dropped_silently() {
    let text = "[Winamp]\r\nvolume=200\r\nnot_a_key=1\r\n\r\n[Some Plug-in]\r\nwhatever=2\r\n";
    let plan = plan_of(text);

    assert!(
        reason_in(&plan, "Winamp", "not_a_key").contains("does not recognise"),
        "the key outside the recognised set was dropped"
    );
    assert!(
        reason_in(&plan, "Some Plug-in", "whatever").contains("does not interpret"),
        "the unknown section's key was dropped"
    );
    assert_eq!(
        plan.report.keys.len(),
        3,
        "the report does not name every key"
    );
}
