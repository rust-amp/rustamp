//! RFC 0008: the payload enums of the re-exported [`Command`] variants are
//! reachable through the contract crate.
//!
//! `rustamp-core`'s `command` module re-exports an explicit item list (RFC 0008
//! §6 Q1: no glob re-export), so that list is the only thing standing between a
//! consumer and the types it has to name to match on [`Command::OpenMedia`],
//! [`Command::OpenJump`], [`Command::OpenPrefsPage`] and RFC 0020's
//! [`Command::ConfigurePlugin`]. This file is that consumer: every path goes
//! through `rustamp_core::command`, never through `rustamp_proto`. Drop one of
//! the payload types from the re-export in `src/command.rs` and this test target
//! stops compiling with an unresolved-import error, so the suite fails instead
//! of a reader eventually noticing the gap.

use rustamp_core::command::{Command, JumpTarget, OpenMediaKind, PluginKind, PrefsPage};

/// Label a command through its payload, the way a consumer's `match` names it.
///
/// All payload types appear here with every one of their variants, so a
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
        // RFC 0020: the configure request names the plug-in's kind, so a
        // consumer matching on it must be able to name the kind here too.
        Command::ConfigurePlugin { kind, .. } => Some(match kind {
            PluginKind::Input => "configure-plugin: input",
            PluginKind::Output => "configure-plugin: output",
            PluginKind::Visualization => "configure-plugin: visualization",
            PluginKind::Dsp => "configure-plugin: dsp",
            PluginKind::General => "configure-plugin: general",
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
    // RFC 0020: every kind a configure request can name is reachable here.
    for (kind, label) in [
        (PluginKind::Input, "configure-plugin: input"),
        (PluginKind::Output, "configure-plugin: output"),
        (PluginKind::Visualization, "configure-plugin: visualization"),
        (PluginKind::Dsp, "configure-plugin: dsp"),
        (PluginKind::General, "configure-plugin: general"),
    ] {
        assert_eq!(
            payload_label(&Command::ConfigurePlugin { plugin_id: 7, kind }),
            Some(label)
        );
    }
    // A command that carries no payload from this set is unaffected.
    assert_eq!(payload_label(&Command::Play), None);
}

#[test]
fn payload_enums_are_the_proto_definitions_not_copies() {
    // RFC 0004 / RFC 0008 §4: the definitions stay in `rustamp-proto` and the
    // core path names those very types. Re-declaring the enums inside
    // `rustamp-core` would make the two sides of each assert different types and
    // stop this file from compiling. RFC 0020 option A goes one crate further:
    // the kind is `rustamp-plugin-abi`'s own type, not a mirror, so the wire
    // carries one kind and this path names that one.
    assert_eq!(
        OpenMediaKind::Location,
        rustamp_proto::command::OpenMediaKind::Location
    );
    assert_eq!(JumpTarget::Time, rustamp_proto::command::JumpTarget::Time);
    assert_eq!(
        PrefsPage::VisOptions,
        rustamp_proto::command::PrefsPage::VisOptions
    );
    assert_eq!(
        PluginKind::Visualization,
        rustamp_plugin_abi::PluginKind::Visualization,
        "the core path must name the ABI's kind type, not a copy of it"
    );
}
