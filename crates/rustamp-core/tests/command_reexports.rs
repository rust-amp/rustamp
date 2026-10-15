//! RFC 0008: the payload enums of the re-exported [`Command`] variants are
//! reachable through the contract crate.
//!
//! `rustamp-core`'s `command` module re-exports an explicit item list (RFC 0008
//! §6 Q1: no glob re-export), so that list is the only thing standing between a
//! consumer and the types it has to name to match on [`Command::OpenMedia`],
//! [`Command::OpenJump`] and [`Command::OpenPrefsPage`]. This file is that
//! consumer: every path goes through `rustamp_core::command`, never through
//! `rustamp_proto`. Drop one of the three payload enums from the re-export in
//! `src/command.rs` and this test target stops compiling with an
//! unresolved-import error, so the suite fails instead of a reader eventually
//! noticing the gap.

use rustamp_core::command::{Command, JumpTarget, OpenMediaKind, PrefsPage};

/// Label a command through its payload, the way a consumer's `match` names it.
///
/// All three payload enums appear here with every one of their variants, so a
/// payload type that stops being reachable through the core path stops this file
/// from compiling, not just reading.
fn payload_label(command: &Command) -> Option<&'static str> {
    match command {
        Command::OpenMedia { kind, .. } => Some(match kind {
            OpenMediaKind::Files => "open-media: files",
            OpenMediaKind::Location => "open-media: location",
            OpenMediaKind::Directory => "open-media: directory",
        }),
        Command::OpenJump { target } => Some(match target {
            JumpTarget::File => "open-jump: file",
            JumpTarget::Time => "open-jump: time",
        }),
        Command::OpenPrefsPage(page) => Some(match page {
            PrefsPage::VisPlugin => "open-prefs-page: vis-plugin",
            PrefsPage::VisOptions => "open-prefs-page: vis-options",
        }),
        // Every other command carries no payload from this set; the arm keeps the
        // match total as `Command` grows by appending.
        _ => None,
    }
}

#[test]
fn payload_enums_are_reachable_through_the_core_path() {
    // RFC 0008 acceptance 1: `rustamp_core::command::{OpenMediaKind, JumpTarget,
    // PrefsPage}` resolve, and a consumer can build and match every variant of
    // all three through the contract crate.
    assert_eq!(
        payload_label(&Command::OpenMedia {
            kind: OpenMediaKind::Files,
            play: true,
        }),
        Some("open-media: files")
    );
    assert_eq!(
        payload_label(&Command::OpenMedia {
            kind: OpenMediaKind::Location,
            play: false,
        }),
        Some("open-media: location")
    );
    assert_eq!(
        payload_label(&Command::OpenMedia {
            kind: OpenMediaKind::Directory,
            play: true,
        }),
        Some("open-media: directory")
    );
    assert_eq!(
        payload_label(&Command::OpenJump {
            target: JumpTarget::File,
        }),
        Some("open-jump: file")
    );
    assert_eq!(
        payload_label(&Command::OpenJump {
            target: JumpTarget::Time,
        }),
        Some("open-jump: time")
    );
    assert_eq!(
        payload_label(&Command::OpenPrefsPage(PrefsPage::VisPlugin)),
        Some("open-prefs-page: vis-plugin")
    );
    assert_eq!(
        payload_label(&Command::OpenPrefsPage(PrefsPage::VisOptions)),
        Some("open-prefs-page: vis-options")
    );
    // A command that carries no payload from this set is unaffected.
    assert_eq!(payload_label(&Command::Play), None);
}

#[test]
fn payload_enums_are_the_proto_definitions_not_copies() {
    // RFC 0004 / RFC 0008 §4: the definitions stay in `rustamp-proto` and the
    // core path names those very types. Re-declaring the enums inside
    // `rustamp-core` would make the two sides of each assert different types and
    // stop this file from compiling.
    assert_eq!(
        OpenMediaKind::Location,
        rustamp_proto::command::OpenMediaKind::Location
    );
    assert_eq!(JumpTarget::Time, rustamp_proto::command::JumpTarget::Time);
    assert_eq!(
        PrefsPage::VisOptions,
        rustamp_proto::command::PrefsPage::VisOptions
    );
}
