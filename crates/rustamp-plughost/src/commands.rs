//! The plugin host's `WM_COMMAND` id table (RFC 0005 §3, §12, §15.5).
//!
//! The host process owns a fake "Winamp v1.x" main window; a classic plug-in
//! (or an external tool) sends it a public `WM_COMMAND` id, and the host has to
//! turn that id into a protocol [`Command`] for the player. This module is that
//! table.
//!
//! Two rules from RFC 0005 §3 drive it:
//!
//! * **Forward** iff S03 records a public id for the action *and* the host can
//!   build the command without any state or constant that belongs to the GPL
//!   player. The ids are interoperability facts; the table is therefore safe in
//!   this permissively licensed crate (RFC 0005 §13).
//! * **Log and drop** everything else. The deferred actions (minibrowser,
//!   video fullscreen, playlist search), the app-local `NewInstance`, the
//!   UI-local elapsed/remaining toggle and the volume/seek keys — whose payload
//!   needs a step constant that stays in `rustamp-ui` (RFC 0005 §8, §13) — have
//!   no command the host may construct. They are recorded in [`DROPPED`] with
//!   the reason, so the M3 message loop can log them instead of guessing.
//!
//! `PUBLIC_IDS` is the S03 id list written out once; the tests check that it is
//! exactly the union of [`TABLE`] and [`DROPPED`], so an id cannot be silently
//! forgotten (RFC 0005 §13's drift risk).

use rustamp_proto::command::{Command, JumpTarget, OpenMediaKind, PrefsPage};
use rustamp_proto::state::WindowId;

/// The step of the "10 tracks back/forward" keys, mirrored from S03 §1's two
/// rows (`Num1` = back, `Num3` = forward). The value is the action's own id
/// table entry (RFC 0005 §5.1), not a player constant.
const TEN_TRACK_STEP: i32 = 10;

/// Every distinct public `WM_COMMAND` id S03 §1/§4 records (55 ids).
///
/// These are interoperability facts (the ids plug-ins and external tools post).
pub const PUBLIC_IDS: &[u32] = &[
    40012, 40019, 40022, 40023, 40029, 40036, 40040, 40041, 40043, 40044, 40045, 40046, 40047,
    40048, 40058, 40059, 40060, 40061, 40063, 40064, 40147, 40154, 40157, 40165, 40185, 40186,
    40187, 40188, 40191, 40192, 40193, 40194, 40195, 40197, 40219, 40221, 40224, 40242, 40258,
    40291, 40298, 40299, 40300, 40301, 40302, 40303, 40305, 40309, 40317, 40318, 40320, 40321,
    40328, 40334, 40337,
];

/// The ids the host forwards, and the command each becomes.
///
/// The `Command`s carry no player state, so the host builds them from the id
/// alone. The ids with a payload decision in them (`SkipBy`) keep the step here
/// rather than in the player (RFC 0005 §5.1).
pub const TABLE: &[(u32, Command)] = &[
    // Pre-RFC-0005 actions (the S03 rows that already had a variant).
    (40012, Command::OpenPrefs),
    (40019, Command::ToggleAlwaysOnTop),
    (40022, Command::ToggleRepeat),
    (40023, Command::ToggleShuffle),
    (40036, Command::ToggleWindow(WindowId::Equalizer)),
    (40040, Command::ToggleWindow(WindowId::Playlist)),
    (40044, Command::Prev),
    (40045, Command::Play),
    (40046, Command::TogglePause),
    (40047, Command::Stop),
    (40048, Command::Next),
    (40063, Command::FocusNextWindow),
    (40064, Command::ToggleWindowShade(WindowId::Main)),
    (40147, Command::StopWithFadeout),
    (40154, Command::PlayIndex(0)),
    (40157, Command::ToggleStopAfterCurrent),
    (40165, Command::ToggleDoubleSize),
    // RFC 0005 §4 rows.
    (
        40029,
        Command::OpenMedia {
            kind: OpenMediaKind::Files,
            play: true,
        },
    ),
    (40041, Command::OpenAbout),
    (40043, Command::OpenMainMenu),
    (
        40185,
        Command::OpenMedia {
            kind: OpenMediaKind::Location,
            play: true,
        },
    ),
    (40186, Command::ToggleEasyMove),
    (
        40187,
        Command::OpenMedia {
            kind: OpenMediaKind::Directory,
            play: true,
        },
    ),
    (40188, Command::OpenFileInfo),
    (40191, Command::OpenPrefsPage(PrefsPage::VisPlugin)),
    (40192, Command::ToggleVisPlugin),
    (
        40193,
        Command::OpenJump {
            target: JumpTarget::Time,
        },
    ),
    (
        40194,
        Command::OpenJump {
            target: JumpTarget::File,
        },
    ),
    (
        40195,
        Command::SkipBy {
            delta: TEN_TRACK_STEP,
        },
    ),
    (
        40197,
        Command::SkipBy {
            delta: -TEN_TRACK_STEP,
        },
    ),
    (40219, Command::OpenSkinBrowser),
    (40221, Command::ConfigureVisPlugin),
    (40224, Command::CloseFocusedWindow),
    (40258, Command::ShowMainWindow),
    (40291, Command::RefreshSkin),
    (40317, Command::OpenPrefsPage(PrefsPage::VisOptions)),
    (40320, Command::OpenBookmarkEditor),
    (40321, Command::AddBookmark),
    (40328, Command::ToggleWindow(WindowId::Video)),
    (40334, Command::Minimize),
];

/// The ids the host logs and drops, with the reason (RFC 0005 §3, §9, §10, §11).
pub const DROPPED: &[(u32, &str)] = &[
    (
        40058,
        "volume-up key: the payload is the absolute volume and the step is a UI constant (RFC 0005 §8)",
    ),
    (40059, "volume-down key: same as 40058 (RFC 0005 §8)"),
    (
        40060,
        "forward-5s key: the payload needs the UI-local seek step (RFC 0005 §13)",
    ),
    (40061, "rewind-5s key: same as 40060 (RFC 0005 §13)"),
    (
        40242,
        "elapsed/remaining toggle is UI-local and has no variant (RFC 0005 §9)",
    ),
    (
        40298,
        "minibrowser is deferred: no window variant and no owning spec (RFC 0005 §11.1)",
    ),
    (40299, "minibrowser command, unconfirmed (RFC 0005 §11.1)"),
    (40300, "minibrowser command, unconfirmed (RFC 0005 §11.1)"),
    (40301, "minibrowser command, unconfirmed (RFC 0005 §11.1)"),
    (
        40302,
        "minibrowser command, unconfirmed; conflicts with 40185 (RFC 0005 §11.1)",
    ),
    (40303, "minibrowser command, unconfirmed (RFC 0005 §11.1)"),
    (
        40305,
        "new instance is app-local and has no variant (RFC 0005 §10)",
    ),
    (40309, "minibrowser command, unconfirmed (RFC 0005 §11.1)"),
    (
        40318,
        "playlist search is deferred until the playlist window exists (RFC 0005 §11.3)",
    ),
    (
        40337,
        "video fullscreen is deferred: the video window is unspecified (RFC 0005 §11.2)",
    ),
];

/// What the host does with one `WM_COMMAND` id.
#[derive(Debug, Clone, PartialEq)]
pub enum Disposition {
    /// Forward this command to the player.
    Forward(Command),
    /// Log the id and the reason, and drop it.
    Drop(&'static str),
}

/// The command for a public id, or `None` when the host must not forward it.
pub fn command_for_id(id: u32) -> Option<Command> {
    TABLE
        .iter()
        .find(|(entry, _)| *entry == id)
        .map(|(_, command)| command.clone())
}

/// What the host does with `id` (RFC 0005 §15.5).
pub fn disposition(id: u32) -> Disposition {
    match command_for_id(id) {
        Some(command) => Disposition::Forward(command),
        None => Disposition::Drop(drop_reason(id)),
    }
}

/// Why an id is dropped; ids the table has never heard of are named as such.
pub fn drop_reason(id: u32) -> &'static str {
    DROPPED
        .iter()
        .find(|(entry, _)| *entry == id)
        .map(|(_, reason)| *reason)
        .unwrap_or("not a public S03 command id")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_public_id_list_is_the_s03_surface() {
        // S03 §1 records 62 rows and §4 29; they bind 55 distinct ids.
        assert_eq!(PUBLIC_IDS.len(), 55);
        let distinct: std::collections::BTreeSet<_> = PUBLIC_IDS.iter().collect();
        assert_eq!(distinct.len(), PUBLIC_IDS.len(), "no duplicate ids");
    }

    #[test]
    fn every_public_id_has_exactly_one_verdict() {
        for id in PUBLIC_IDS {
            let forwarded = TABLE.iter().any(|(entry, _)| entry == id);
            let dropped = DROPPED.iter().any(|(entry, _)| entry == id);
            assert!(
                forwarded ^ dropped,
                "id {id}: forwarded={forwarded} dropped={dropped}"
            );
        }
    }

    #[test]
    fn the_table_and_the_drop_list_have_no_extra_ids() {
        for (id, _) in TABLE {
            assert!(PUBLIC_IDS.contains(id), "table id {id} is not in S03");
        }
        for (id, _) in DROPPED {
            assert!(PUBLIC_IDS.contains(id), "dropped id {id} is not in S03");
        }
    }

    #[test]
    fn no_id_is_listed_twice() {
        for (i, (id, _)) in TABLE.iter().enumerate() {
            for (other, _) in &TABLE[i + 1..] {
                assert_ne!(id, other, "id {id} appears twice");
            }
        }
        for (i, (id, _)) in DROPPED.iter().enumerate() {
            for (other, _) in &DROPPED[i + 1..] {
                assert_ne!(id, other, "dropped id {id} appears twice");
            }
        }
    }

    #[test]
    fn the_new_rfc_0005_ids_forward_the_documented_commands() {
        // The RFC 0005 §4 master table, "Plugin host" column.
        let expected: [(u32, Command); 26] = [
            (40188, Command::OpenFileInfo),
            (40186, Command::ToggleEasyMove),
            (40043, Command::OpenMainMenu),
            (40321, Command::AddBookmark),
            (40320, Command::OpenBookmarkEditor),
            (
                40194,
                Command::OpenJump {
                    target: JumpTarget::File,
                },
            ),
            (
                40193,
                Command::OpenJump {
                    target: JumpTarget::Time,
                },
            ),
            (40191, Command::OpenPrefsPage(PrefsPage::VisPlugin)),
            (40221, Command::ConfigureVisPlugin),
            (40192, Command::ToggleVisPlugin),
            (
                40029,
                Command::OpenMedia {
                    kind: OpenMediaKind::Files,
                    play: true,
                },
            ),
            (
                40185,
                Command::OpenMedia {
                    kind: OpenMediaKind::Location,
                    play: true,
                },
            ),
            (
                40187,
                Command::OpenMedia {
                    kind: OpenMediaKind::Directory,
                    play: true,
                },
            ),
            (40334, Command::Minimize),
            (40317, Command::OpenPrefsPage(PrefsPage::VisOptions)),
            (40219, Command::OpenSkinBrowser),
            (40157, Command::ToggleStopAfterCurrent),
            (40328, Command::ToggleWindow(WindowId::Video)),
            (40041, Command::OpenAbout),
            (40224, Command::CloseFocusedWindow),
            (40291, Command::RefreshSkin),
            (40197, Command::SkipBy { delta: -10 }),
            (40195, Command::SkipBy { delta: 10 }),
            (40063, Command::FocusNextWindow),
            (40258, Command::ShowMainWindow),
            (40154, Command::PlayIndex(0)),
        ];
        for (id, command) in expected {
            assert_eq!(command_for_id(id), Some(command), "id {id}");
        }
    }

    #[test]
    fn deferred_app_local_and_ui_local_ids_are_dropped_with_a_reason() {
        for id in [
            40298, 40299, 40300, 40301, 40302, 40303, 40309, 40305, 40318, 40337,
        ] {
            assert_eq!(command_for_id(id), None, "id {id} must not be forwarded");
            let reason = drop_reason(id);
            assert!(
                reason.contains("RFC 0005"),
                "id {id} must carry a documented reason, got {reason:?}"
            );
            assert!(matches!(disposition(id), Disposition::Drop(_)));
        }
        // The UI-local toggle and the volume/seek keys are dropped too, but for
        // their own reasons.
        assert_eq!(command_for_id(40242), None);
        assert!(drop_reason(40242).contains("UI-local"));
        for id in [40058, 40059, 40060, 40061] {
            assert_eq!(command_for_id(id), None, "id {id}");
        }
        assert!(drop_reason(40058).contains("UI constant"));
    }

    #[test]
    fn an_unknown_id_is_dropped_and_named_as_unknown() {
        assert_eq!(command_for_id(1), None);
        assert_eq!(command_for_id(40000), None);
        assert_eq!(drop_reason(1), "not a public S03 command id");
        assert!(matches!(disposition(1), Disposition::Drop(_)));
    }

    #[test]
    fn dispositions_carry_the_command() {
        assert_eq!(disposition(40045), Disposition::Forward(Command::Play));
        assert_eq!(
            disposition(40328),
            Disposition::Forward(Command::ToggleWindow(WindowId::Video))
        );
    }
}
