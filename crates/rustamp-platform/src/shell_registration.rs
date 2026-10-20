// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The shell registration: the verbs a drop from Explorer offers (S13 §3).
//!
//! # What is registered
//!
//! S13 §3 gives the handler ProgIDs and the verbs under them. This module is
//! the model of that table plus the Win32 half that writes it.
//!
//! | Handler | Key the verbs live under | Verbs (S13 §3) |
//! |---|---|---|
//! | media file | `Rustamp.File` | `open`, `Play`, `Enqueue`, `ListBookmark` |
//! | playlist file | `Rustamp.PlayList` | `open`, `Play`, `Enqueue`, `ListBookmark` |
//! | skin archive | `Rustamp.SkinZip` | `install` |
//! | directory | `Directory` (the shell's own handler) | `Play`, `Enqueue` |
//!
//! So a media file and a playlist file get **four** verbs each, a skin
//! archive gets **one**, and a directory gets **two** — the counts S13 §3
//! states.
//!
//! # The ProgIDs are ours, not the reference's (deliberate deviation)
//!
//! S13 §3 records the reference's own ProgIDs (`Winamp.File`,
//! `Winamp.PlayList`, `Winamp.SkinZip`). **We register `Rustamp.*` instead**
//! (owner decision, 2026-10-20): a player must not claim another product's
//! ProgIDs, and writing them would collide with a real Winamp installation on
//! the same machine — the extension associations would then point at whichever
//! program wrote last. The shell integration is unaffected, because this module
//! writes **both** the ProgID tree and the extension associations: Explorer
//! follows the association, not the name of the class it points at.
//!
//! # Conformance with S13 §3
//!
//! | S13 fact | Our behaviour | State |
//! |---|---|---|
//! | the reference's ProgIDs | ours are `Rustamp.File`, `Rustamp.PlayList`, `Rustamp.SkinZip` ([`MEDIA_PROG_ID`], [`PLAYLIST_PROG_ID`], [`SKIN_PROG_ID`]) | **deliberate deviation** (see above) |
//! | four verbs per media and playlist file | [`Handler::MediaFile`], [`Handler::PlaylistFile`] carry `open`, `Play`, `Enqueue`, `ListBookmark` | matches |
//! | one verb (`install`) per skin archive | [`Handler::SkinArchive`] | matches |
//! | two verbs (`Play`, `Enqueue`) per directory | [`Handler::Directory`] | matches |
//! | the verbs live under the ProgIDs (S13 §4) | every key starts with the handler's key | matches |
//! | `…\shell\Play\DropTarget` and `…\shell\Enqueue\DropTarget` exist | the model writes exactly those two key shapes per handler that has the verbs | matches (key shape) |
//! | registration writes the same key set each time, and `/UNREG` is the matching removal (S13 §3, a reading) | [`install_into`] writes the same entries every time, [`uninstall_from`] takes exactly those back and leaves every foreign key | matches (reading) |
//! | what the `DropTarget` entries carry | [`DROP_TARGET_VALUE`] | own choice (S13 §5.12) |
//! | `Enqueue` is the display name of that verb | [`Verb::label`] is `Enqueue` | matches |
//! | the display names of the other verbs | the verb name | own choice (S13 §5.11) |
//! | the directory handler's own identifiers | `Directory`, the shell's own directory handler | own choice (S13 §5.10) |
//! | what `Play` and `Enqueue` do | `Play` plays the file, `Enqueue` adds it to the playlist | own choice (S13 §3 marks the readings) |
//! | what the `install` verb of a skin archive does | it loads the dropped archive as the current skin (`--skin`); Rustamp has no skin installer yet | own choice (S13 §3 marks the reading) |
//! | the command each verb runs | [`Verb::arguments`] | own choice (S13 records only the bookmark form) |
//! | which file types point at the handlers | [`MEDIA_EXTENSIONS`], [`PLAYLIST_EXTENSIONS`], [`SKIN_EXTENSIONS`] | own choice (S13 §3 names the handlers, not the extensions) |
//!
//! # The registration step
//!
//! S13 §1 names four switches around one step — the shell registration:
//! `/INSTALL` performs it, `/NOREG` skips it, `/UNREG` removes the result and
//! `/ONLYINSTALL` performs it and then exits. [`registration_step`] maps the
//! switches to that decision, and it is a pure function, so the decision is
//! unit-tested without touching a registry.
//!
//! When the step runs is an **own choice**: S13 §1 says what the switches
//! pair with, not when the step runs. The step runs when the command line
//! asks for it (`/INSTALL`, `/ONLYINSTALL`); `/NOREG` skips it even when
//! `/INSTALL` asks for it; `/UNREG` removes the registration instead of
//! writing it.
//!
//! # Where the keys live
//!
//! The registration lives under `HKEY_CURRENT_USER\Software\Classes`, the
//! per-user classes root: a player that registers its own associations must
//! not need administrator rights. The root is an own choice; S13 §3 records
//! the key *shape* (`…\shell\Play\DropTarget`) with the prefix elided.
//!
//! # The headless seam
//!
//! [`registry_entries`] is the whole registration as plain data — the keys and
//! the values — and it is unit-tested without an operating system. Writing it
//! needs the registry, so [`install`] and [`uninstall`] are VM-only.
//!
//! [`RegistryModel`] is the registry as plain data too: the values a
//! registration owns. [`install_into`] and [`uninstall_from`] are the model's
//! own install and removal steps, so that the properties can ask what the
//! registration writes and what the removal takes with it — and what it
//! leaves alone — without an operating system.

use std::collections::BTreeMap;
use std::path::Path;

/// The ProgID of the media file handler (S13 §3).
pub const MEDIA_PROG_ID: &str = "Rustamp.File";

/// The ProgID of the playlist file handler (S13 §3).
pub const PLAYLIST_PROG_ID: &str = "Rustamp.PlayList";

/// The ProgID of the skin archive handler (S13 §3).
pub const SKIN_PROG_ID: &str = "Rustamp.SkinZip";

/// The shell's own directory handler (S13 §3, the directory row).
///
/// The spec does not name the directory handler's identifiers (S13 §5.10 asks
/// what `Rustamp.Play` and `Rustamp.Enqueue` are), so the shell's own
/// `Directory` key is used: it is the key the shell itself looks at when it
/// offers verbs for a folder.
pub const DIRECTORY_HANDLER: &str = "Directory";

/// What the `DropTarget` entries carry.
///
/// S13 §3 records the key shape (`…\shell\Play\DropTarget`) and states that
/// what the entries carry is not recorded (S13 §5.12). The value names the
/// player's own drop target; the in-window drop rule itself is open
/// (S13 §5.1), so the player does not act on a drop yet.
pub const DROP_TARGET_VALUE: &str = "Rustamp.DropTarget";

/// The file types the media handler is registered for.
///
/// Own choice: S13 §3 names the handlers, not the extensions. The list is the
/// player's own media list (the same one the open dialog offers).
pub const MEDIA_EXTENSIONS: &[&str] = &[
    "mp3", "ogg", "oga", "opus", "flac", "wav", "m4a", "aac", "wma", "mp2", "mpc", "ape",
];

/// The file types the playlist handler is registered for.
///
/// Own choice, like [`MEDIA_EXTENSIONS`].
pub const PLAYLIST_EXTENSIONS: &[&str] = &["m3u", "m3u8", "pls", "xspf"];

/// The file types the skin handler is registered for.
///
/// Own choice, like [`MEDIA_EXTENSIONS`].
pub const SKIN_EXTENSIONS: &[&str] = &["wsz"];

/// One shell verb of a handler (S13 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verb {
    /// The verb's key name (`open`, `Play`, `Enqueue`, `ListBookmark`,
    /// `install`).
    pub name: &'static str,
    /// The name the shell shows.
    ///
    /// S13 §3 records `Enqueue` and nothing else (S13 §5.11), so the other
    /// verbs show their own name.
    pub label: &'static str,
    /// The arguments the verb's command carries.
    ///
    /// `%1` stands for the path the shell hands over — the shell's own
    /// placeholder. The reference's recorded bookmark template spells its
    /// placeholder `%s` (S13 §1), but the spec does not establish what
    /// substitutes it, and the shell substitutes `%1`.
    pub arguments: &'static str,
    /// Whether the verb's key carries a `DropTarget` value (S13 §3).
    pub drop_target: bool,
}

/// The four verbs of a media file and of a playlist file (S13 §3).
const FILE_VERBS: &[Verb] = &[
    Verb {
        name: "open",
        label: "open",
        arguments: "\"%1\"",
        drop_target: false,
    },
    Verb {
        name: "Play",
        label: "Play",
        arguments: "\"%1\"",
        drop_target: true,
    },
    Verb {
        name: "Enqueue",
        label: "Enqueue",
        arguments: "--enqueue \"%1\"",
        drop_target: true,
    },
    Verb {
        name: "ListBookmark",
        label: "ListBookmark",
        arguments: "/BOOKMARK \"%1\"",
        drop_target: false,
    },
];

/// The one verb of a skin archive (S13 §3).
const SKIN_VERBS: &[Verb] = &[Verb {
    name: "install",
    label: "install",
    arguments: "--skin \"%1\"",
    drop_target: false,
}];

/// The two verbs of a directory (S13 §3).
const DIRECTORY_VERBS: &[Verb] = &[
    Verb {
        name: "Play",
        label: "Play",
        arguments: "\"%1\"",
        drop_target: true,
    },
    Verb {
        name: "Enqueue",
        label: "Enqueue",
        arguments: "--enqueue \"%1\"",
        drop_target: true,
    },
];

/// What the player registers shell verbs for (S13 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handler {
    /// A media file: `Rustamp.File`, four verbs.
    MediaFile,
    /// A playlist file: `Rustamp.PlayList`, four verbs.
    PlaylistFile,
    /// A skin archive: `Rustamp.SkinZip`, one verb.
    SkinArchive,
    /// A directory: the shell's own handler, two verbs.
    Directory,
}

impl Handler {
    /// Every handler, in the order of S13 §3's table.
    pub const ALL: [Handler; 4] = [
        Handler::MediaFile,
        Handler::PlaylistFile,
        Handler::SkinArchive,
        Handler::Directory,
    ];

    /// The key the handler's verbs live under (S13 §3).
    pub const fn key(self) -> &'static str {
        match self {
            Handler::MediaFile => MEDIA_PROG_ID,
            Handler::PlaylistFile => PLAYLIST_PROG_ID,
            Handler::SkinArchive => SKIN_PROG_ID,
            Handler::Directory => DIRECTORY_HANDLER,
        }
    }

    /// The handler's verbs (S13 §3): four, four, one, two.
    pub const fn verbs(self) -> &'static [Verb] {
        match self {
            Handler::MediaFile => FILE_VERBS,
            Handler::PlaylistFile => FILE_VERBS,
            Handler::SkinArchive => SKIN_VERBS,
            Handler::Directory => DIRECTORY_VERBS,
        }
    }

    /// The ProgID the file types of this handler point at.
    ///
    /// The directory handler has no ProgID of its own (S13 §5.10); it is the
    /// shell's own key.
    pub const fn prog_id(self) -> Option<&'static str> {
        match self {
            Handler::MediaFile => Some(MEDIA_PROG_ID),
            Handler::PlaylistFile => Some(PLAYLIST_PROG_ID),
            Handler::SkinArchive => Some(SKIN_PROG_ID),
            Handler::Directory => None,
        }
    }
}

/// One registry entry of the registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The key path below the classes root.
    pub key: String,
    /// The value name; empty is the key's default value.
    pub name: String,
    /// The value's data.
    pub value: String,
}

/// The entries a handler's verbs write (S13 §3).
///
/// The key of a verb is the key the handler's verbs live under, the verb's
/// own name and the shape the shell expects: the `command` key of the verb
/// and, for a verb that has one, its `DropTarget` key (S13 §3).
pub fn verb_entries(handler: &str, verbs: &[Verb], exe: &Path) -> Vec<Entry> {
    let exe = exe.display().to_string();
    let mut entries = Vec::new();
    for verb in verbs {
        entries.push(Entry {
            key: format!(r"{}\shell\{}\command", handler, verb.name),
            name: String::new(),
            value: format!("\"{exe}\" {}", verb.arguments),
        });
        if verb.drop_target {
            entries.push(Entry {
                key: format!(r"{}\shell\{}\DropTarget", handler, verb.name),
                name: String::new(),
                value: DROP_TARGET_VALUE.to_owned(),
            });
        }
    }
    entries
}

/// The registration's verb keys and values (S13 §3).
///
/// The whole registration as plain data: the model is what the tests read,
/// and [`install`] is what writes it.
pub fn registry_entries(exe: &Path) -> Vec<Entry> {
    let mut entries = Vec::new();
    for handler in Handler::ALL {
        entries.extend(verb_entries(handler.key(), handler.verbs(), exe));
    }
    entries
}

/// The verb trees the removal step removes (S13 §3).
///
/// The tree of a verb is the key its own keys live under
/// (`Rustamp.File\shell\Play`): the removal step takes the whole tree with
/// it, not only the keys the player wrote under it.
pub fn verb_trees() -> Vec<String> {
    let mut trees = Vec::new();
    for handler in Handler::ALL {
        for verb in handler.verbs() {
            trees.push(format!(r"{}\shell\{}", handler.key(), verb.name));
        }
    }
    trees
}

/// The file type entries of the registration (own choice; S13 §3 names the
/// handlers, not the extensions).
pub fn file_type_entries() -> Vec<Entry> {
    file_type_associations()
        .into_iter()
        .map(|(extension, prog_id)| Entry {
            key: format!(".{extension}"),
            name: String::new(),
            value: prog_id.to_owned(),
        })
        .collect()
}

/// The file types and the handler each points at.
pub fn file_type_associations() -> Vec<(&'static str, &'static str)> {
    let mut out = Vec::new();
    for extension in MEDIA_EXTENSIONS {
        out.push((*extension, MEDIA_PROG_ID));
    }
    for extension in PLAYLIST_EXTENSIONS {
        out.push((*extension, PLAYLIST_PROG_ID));
    }
    for extension in SKIN_EXTENSIONS {
        out.push((*extension, SKIN_PROG_ID));
    }
    out
}

// ---------------------------------------------------------------------------
// The registry as plain data
// ---------------------------------------------------------------------------

/// The registry as the model holds it.
///
/// The model is flat: an entry is a key path, a value name and the value's
/// data — the three things [`Entry`] carries. The keys above them
/// (`Rustamp.File\shell\open`) exist in a real registry, but the registry
/// creates them on the way, so the model does not hold them.
///
/// A key path is held as it was written. The registry's key names are
/// case-insensitive, so the removal finds a key in any spelling; the model
/// does not fold the spelling on the way in.
pub type RegistryModel = BTreeMap<(String, String), String>;

/// Writes `entries` into the model (the model's write step).
///
/// The same step as the Win32 half's write, but on plain data: a key that is
/// written twice keeps the value of the last write, as the registry does.
pub fn write_entries(model: &mut RegistryModel, entries: &[Entry]) {
    for entry in entries {
        model.insert((entry.key.clone(), entry.name.clone()), entry.value.clone());
    }
}

/// Writes the registration into the model (the model's install step).
///
/// The same step as [`install`], without a registry: the model is the plain
/// data the Win32 half writes.
pub fn install_into(model: &mut RegistryModel, exe: &Path) {
    write_entries(model, &registry_entries(exe));
    write_entries(model, &file_type_entries());
}

/// Removes the registration from the model (the model's removal step).
///
/// The same step as [`uninstall`]: the verb trees of the player's own
/// handlers go, and so does a file type association that points at the
/// player. A foreign key stays — the player removes what it wrote, not what
/// it finds.
pub fn uninstall_from(model: &mut RegistryModel) {
    for tree in verb_trees() {
        model.retain(|(key, _), _| !is_below(key, &tree));
    }
    for (extension, prog_id) in file_type_associations() {
        let association = format!(".{extension}");
        // The association is the key's default value, and the key's name is
        // case-insensitive in the registry: the association is found in any
        // spelling. The value itself is compared as it is.
        let ours: Vec<(String, String)> = model
            .iter()
            .filter(|((key, name), value)| {
                name.is_empty() && is_the_key(key, &association) && value.as_str() == prog_id
            })
            .map(|((key, name), _)| (key.clone(), name.clone()))
            .collect();
        for key in ours {
            model.remove(&key);
        }
    }
}

/// Whether the key `key` is the key `tree`, whatever its spelling.
///
/// The registry's key names are case-insensitive, so the comparison is too.
fn is_the_key(key: &str, tree: &str) -> bool {
    key.to_lowercase() == tree.to_lowercase()
}

/// Whether the key `key` is the tree `tree` or lies below it.
fn is_below(key: &str, tree: &str) -> bool {
    let key = key.to_lowercase();
    let tree = tree.to_lowercase();
    key == tree || key.starts_with(&format!("{tree}\\"))
}

/// The registration switches of the command line (S13 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RegistrationFlags {
    /// `/INSTALL`: perform the registration step.
    pub install: bool,
    /// `/UNREG`: remove the registration.
    pub unreg: bool,
    /// `/NOREG`: skip the registration step.
    pub noreg: bool,
    /// `/ONLYINSTALL`: perform the registration step, then exit.
    pub only_install: bool,
}

/// What the start-up does with the shell registration (S13 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationStep {
    /// Write the registration.
    Install,
    /// Remove the registration.
    Uninstall,
    /// Leave the registry alone.
    Skip,
    /// Write the registration, then exit without starting the player.
    InstallAndExit,
}

/// The registration step the switches ask for (S13 §1).
///
/// Pure, so the decision is unit-tested without a registry:
///
/// * `/ONLYINSTALL` wins over everything: the step runs and the player does
///   not start;
/// * `/UNREG` removes the registration instead of writing it;
/// * `/NOREG` skips the step, also when `/INSTALL` asks for it;
/// * `/INSTALL` (or the plain start-up) writes the registration.
pub fn registration_step(flags: RegistrationFlags) -> RegistrationStep {
    if flags.only_install {
        RegistrationStep::InstallAndExit
    } else if flags.unreg {
        RegistrationStep::Uninstall
    } else if flags.noreg {
        RegistrationStep::Skip
    } else if flags.install {
        RegistrationStep::Install
    } else {
        RegistrationStep::Skip
    }
}

/// Why a registration write or removal failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistrationError {
    /// The registry refused the operation.
    #[error("the registry refused {key} (error {code})")]
    Registry {
        /// The key the operation was about.
        key: String,
        /// The Win32 error code.
        code: u32,
    },
    /// The player's own path could not be read.
    #[error("the player's path is unknown: {0}")]
    Path(String),
    /// The platform has no registry.
    #[error("the shell registration needs Windows")]
    Unsupported,
}

/// Writes the registration (S13 §1, `/INSTALL`).
///
/// The verbs of S13 §3 are written under the ProgIDs of S13 §3, the
/// `DropTarget` values under the `Play` and the `Enqueue` key, and the file
/// types point at their handler.
#[cfg(windows)]
pub fn install(exe: &Path) -> Result<(), RegistrationError> {
    imp::install(&registry_entries(exe), &file_type_entries())
}

/// Removes the registration (S13 §1, `/UNREG`).
///
/// The verb keys the player wrote are removed. A file type that points at
/// another handler is left alone; one that points at a handler of the
/// player loses the association.
#[cfg(windows)]
pub fn uninstall() -> Result<(), RegistrationError> {
    imp::uninstall()
}

/// Writes the registration (S13 §1, `/INSTALL`).
#[cfg(not(windows))]
pub fn install(_exe: &Path) -> Result<(), RegistrationError> {
    Err(RegistrationError::Unsupported)
}

/// Removes the registration (S13 §1, `/UNREG`).
#[cfg(not(windows))]
pub fn uninstall() -> Result<(), RegistrationError> {
    Err(RegistrationError::Unsupported)
}

/// Reads the value the registration wrote under `key`, or `None` when there
/// is none.
///
/// The counterpart of [`install`]: it is how a VM test proves that the
/// registration really landed in the registry. `key` is a path below the
/// classes root, e.g. `Rustamp.File\shell\open\command`.
#[cfg(windows)]
pub fn registered_value(key: &str) -> Option<String> {
    imp::read_value(key)
}

/// Reads the value the registration wrote under `key`, or `None` when there
/// is none.
#[cfg(not(windows))]
pub fn registered_value(_key: &str) -> Option<String> {
    None
}

#[cfg(windows)]
mod imp {
    //! The Win32 half: the registry calls.

    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_ALL_ACCESS, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE,
        REG_SAM_FLAGS, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteKeyW,
        RegDeleteTreeW, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW,
    };
    use windows::core::PCWSTR;

    use super::{Entry, RegistrationError};

    /// The classes root the registration lives under (own choice).
    const CLASSES: &str = r"Software\Classes";

    /// Writes every entry below the classes root.
    pub fn install(entries: &[Entry], file_types: &[Entry]) -> Result<(), RegistrationError> {
        let root = open_key(HKEY_CURRENT_USER, CLASSES, KEY_ALL_ACCESS)?;
        let result = write_all(root, entries).and_then(|()| write_all(root, file_types));
        close_key(root);
        result
    }

    /// Removes the verb keys and the file type associations of the player.
    pub fn uninstall() -> Result<(), RegistrationError> {
        let root = open_key(HKEY_CURRENT_USER, CLASSES, KEY_ALL_ACCESS)?;
        let mut result = Ok(());
        for tree in super::verb_trees() {
            if let Err(error) = delete_tree(root, &tree) {
                result = Err(error);
            }
        }
        for handler in super::Handler::ALL {
            // The handler key itself is removed when the player wrote all of
            // it; a handler that carries more (another player's registration)
            // keeps its content and only loses the verbs above.
            let _ = delete_key(root, &format!(r"{}\shell", handler.key()));
            let _ = delete_key(root, handler.key());
        }
        for (extension, prog_id) in super::file_type_associations() {
            let key = format!(".{extension}");
            if read_default_value(root, &key).as_deref() == Some(prog_id) {
                if let Err(error) = delete_value(root, &key, "") {
                    result = Err(error);
                }
                let _ = delete_key(root, &key);
            }
        }
        close_key(root);
        result
    }

    /// Reads the default value of a key below the classes root.
    ///
    /// The key is only opened, never created: a read must not leave a key
    /// behind.
    pub fn read_value(key: &str) -> Option<String> {
        let root = open_existing_key(HKEY_CURRENT_USER, CLASSES)?;
        let value = read_default_value(root, key);
        close_key(root);
        value
    }

    /// Writes every entry below `root`.
    fn write_all(root: HKEY, entries: &[Entry]) -> Result<(), RegistrationError> {
        for entry in entries {
            let handle = open_key(root, &entry.key, KEY_WRITE)?;
            let status = unsafe {
                RegSetValueExW(
                    handle,
                    PCWSTR::null(),
                    None,
                    REG_SZ,
                    Some(&reg_sz_bytes(&entry.value)),
                )
            };
            close_key(handle);
            if status != ERROR_SUCCESS {
                return Err(RegistrationError::Registry {
                    key: entry.key.clone(),
                    code: status.0,
                });
            }
        }
        Ok(())
    }

    /// Opens (and creates) `path` below `root`.
    fn open_key(root: HKEY, path: &str, access: REG_SAM_FLAGS) -> Result<HKEY, RegistrationError> {
        let wide = to_wide(path);
        let mut handle = HKEY(std::ptr::null_mut());
        // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the
        // call, `handle` is a valid output slot, and the returned handle is
        // owned by the caller (who closes it with `close_key`).
        let status = unsafe {
            RegCreateKeyExW(
                root,
                PCWSTR(wide.as_ptr()),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                access,
                None,
                &mut handle,
                None,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(RegistrationError::Registry {
                key: path.to_owned(),
                code: status.0,
            });
        }
        Ok(handle)
    }

    /// Opens `path` below `root` without creating it.
    fn open_existing_key(root: HKEY, path: &str) -> Option<HKEY> {
        let wide = to_wide(path);
        let mut handle = HKEY(std::ptr::null_mut());
        // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the
        // call, `handle` is a valid output slot, and the returned handle is
        // owned by the caller (who closes it with `close_key`).
        let status =
            unsafe { RegOpenKeyExW(root, PCWSTR(wide.as_ptr()), None, KEY_READ, &mut handle) };
        (status == ERROR_SUCCESS).then_some(handle)
    }

    /// Closes a key handle.
    fn close_key(handle: HKEY) {
        // SAFETY: `handle` is a valid owned handle from `open_key` and is
        // closed exactly once.
        let _ = unsafe { RegCloseKey(handle) };
    }

    /// Removes `path` and everything below it.
    fn delete_tree(root: HKEY, path: &str) -> Result<(), RegistrationError> {
        let wide = to_wide(path);
        // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the
        // call.
        let status = unsafe { RegDeleteTreeW(root, PCWSTR(wide.as_ptr())) };
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(RegistrationError::Registry {
                key: path.to_owned(),
                code: status.0,
            })
        }
    }

    /// Removes `path`; fails when the key still has content.
    fn delete_key(root: HKEY, path: &str) -> Result<(), RegistrationError> {
        let wide = to_wide(path);
        // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the
        // call.
        let status = unsafe { RegDeleteKeyW(root, PCWSTR(wide.as_ptr())) };
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(RegistrationError::Registry {
                key: path.to_owned(),
                code: status.0,
            })
        }
    }

    /// Removes one value of a key; the empty name is the default value.
    fn delete_value(root: HKEY, path: &str, name: &str) -> Result<(), RegistrationError> {
        let handle = open_key(root, path, KEY_WRITE)?;
        let wide = to_wide(name);
        // SAFETY: `wide` is a NUL-terminated UTF-16 name that outlives the
        // call, and `handle` is a valid owned handle.
        let status = unsafe { RegDeleteValueW(handle, PCWSTR(wide.as_ptr())) };
        close_key(handle);
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(RegistrationError::Registry {
                key: path.to_owned(),
                code: status.0,
            })
        }
    }

    /// Reads the default value of a key, or `None` when there is none.
    fn read_default_value(root: HKEY, path: &str) -> Option<String> {
        let wide = to_wide(path);
        let mut size: u32 = 0;
        // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the
        // call; the first call only asks for the size.
        let status = unsafe {
            RegGetValueW(
                root,
                PCWSTR(wide.as_ptr()),
                PCWSTR::null(),
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let mut buffer = vec![0u16; size as usize / 2 + 1];
        let mut bytes = (buffer.len() * 2) as u32;
        // SAFETY: the buffer is `bytes` long and outlives the call.
        let status = unsafe {
            RegGetValueW(
                root,
                PCWSTR(wide.as_ptr()),
                PCWSTR::null(),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
        };
        if status != ERROR_SUCCESS {
            return None;
        }
        let length = (bytes as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(
            &buffer[..length.min(buffer.len())],
        ))
    }

    /// The bytes of a `REG_SZ` value: UTF-16, little endian, NUL terminated.
    fn reg_sz_bytes(value: &str) -> Vec<u8> {
        value
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    /// UTF-16, NUL terminated, for the `*W` Win32 calls.
    fn to_wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod props;

    /// The verbs of a handler, by name.
    fn verb_names(handler: Handler) -> Vec<&'static str> {
        handler.verbs().iter().map(|verb| verb.name).collect()
    }

    #[test]
    fn the_counts_are_the_specs_counts() {
        assert_eq!(
            verb_names(Handler::MediaFile).len(),
            4,
            "S13 §3: four verbs"
        );
        assert_eq!(
            verb_names(Handler::PlaylistFile).len(),
            4,
            "S13 §3: four verbs"
        );
        assert_eq!(
            verb_names(Handler::SkinArchive).len(),
            1,
            "S13 §3: one verb"
        );
        assert_eq!(verb_names(Handler::Directory).len(), 2, "S13 §3: two verbs");
    }

    #[test]
    fn the_verb_names_are_the_specs_verb_names() {
        assert_eq!(
            verb_names(Handler::MediaFile),
            vec!["open", "Play", "Enqueue", "ListBookmark"]
        );
        assert_eq!(
            verb_names(Handler::PlaylistFile),
            vec!["open", "Play", "Enqueue", "ListBookmark"]
        );
        assert_eq!(verb_names(Handler::SkinArchive), vec!["install"]);
        assert_eq!(verb_names(Handler::Directory), vec!["Play", "Enqueue"]);
    }

    #[test]
    fn the_prog_ids_are_the_specs_prog_ids() {
        assert_eq!(Handler::MediaFile.key(), "Rustamp.File");
        assert_eq!(Handler::PlaylistFile.key(), "Rustamp.PlayList");
        assert_eq!(Handler::SkinArchive.key(), "Rustamp.SkinZip");
    }

    #[test]
    fn only_play_and_enqueue_carry_a_drop_target() {
        for handler in Handler::ALL {
            for verb in handler.verbs() {
                let expected = verb.name == "Play" || verb.name == "Enqueue";
                assert_eq!(
                    verb.drop_target, expected,
                    "{}: the DropTarget key belongs to Play and Enqueue only",
                    verb.name
                );
            }
        }
    }

    #[test]
    fn the_drop_target_keys_have_the_specs_shape() {
        let entries = registry_entries(Path::new("C:/games/rustamp.exe"));
        let drop_targets: Vec<&str> = entries
            .iter()
            .filter(|entry| entry.key.ends_with("DropTarget"))
            .map(|entry| entry.key.as_str())
            .collect();
        assert_eq!(
            drop_targets,
            vec![
                r"Rustamp.File\shell\Play\DropTarget",
                r"Rustamp.File\shell\Enqueue\DropTarget",
                r"Rustamp.PlayList\shell\Play\DropTarget",
                r"Rustamp.PlayList\shell\Enqueue\DropTarget",
                r"Directory\shell\Play\DropTarget",
                r"Directory\shell\Enqueue\DropTarget",
            ]
        );
    }

    #[test]
    fn every_verb_writes_a_command_under_its_prog_id() {
        let entries = registry_entries(Path::new("C:/games/rustamp.exe"));
        let commands: Vec<&str> = entries
            .iter()
            .filter(|entry| entry.key.ends_with("command"))
            .map(|entry| entry.key.as_str())
            .collect();
        assert_eq!(
            commands,
            vec![
                r"Rustamp.File\shell\open\command",
                r"Rustamp.File\shell\Play\command",
                r"Rustamp.File\shell\Enqueue\command",
                r"Rustamp.File\shell\ListBookmark\command",
                r"Rustamp.PlayList\shell\open\command",
                r"Rustamp.PlayList\shell\Play\command",
                r"Rustamp.PlayList\shell\Enqueue\command",
                r"Rustamp.PlayList\shell\ListBookmark\command",
                r"Rustamp.SkinZip\shell\install\command",
                r"Directory\shell\Play\command",
                r"Directory\shell\Enqueue\command",
            ]
        );
    }

    #[test]
    fn the_bookmark_verb_carries_the_specs_bookmark_form() {
        let entries = registry_entries(Path::new("C:/games/rustamp.exe"));
        let command = entries
            .iter()
            .find(|entry| entry.key == r"Rustamp.File\shell\ListBookmark\command")
            .expect("the ListBookmark command");
        assert_eq!(
            command.value, r#""C:/games/rustamp.exe" /BOOKMARK "%1""#,
            "S13 §1: the shell registers the bookmark form"
        );
    }

    #[test]
    fn the_file_types_point_at_their_handler() {
        let associations = file_type_associations();
        assert!(
            associations.contains(&("mp3", MEDIA_PROG_ID)),
            "a media file points at the media handler"
        );
        assert!(
            associations.contains(&("m3u", PLAYLIST_PROG_ID)),
            "a playlist file points at the playlist handler"
        );
        assert!(
            associations.contains(&("wsz", SKIN_PROG_ID)),
            "a skin archive points at the skin handler"
        );
    }

    #[test]
    fn the_registration_step_follows_the_switches() {
        assert_eq!(
            registration_step(RegistrationFlags {
                install: true,
                ..RegistrationFlags::default()
            }),
            RegistrationStep::Install
        );
        assert_eq!(
            registration_step(RegistrationFlags {
                unreg: true,
                ..RegistrationFlags::default()
            }),
            RegistrationStep::Uninstall
        );
        assert_eq!(
            registration_step(RegistrationFlags {
                noreg: true,
                ..RegistrationFlags::default()
            }),
            RegistrationStep::Skip
        );
        assert_eq!(
            registration_step(RegistrationFlags {
                only_install: true,
                ..RegistrationFlags::default()
            }),
            RegistrationStep::InstallAndExit
        );
    }

    #[test]
    fn noreg_skips_the_registration_even_with_install() {
        assert_eq!(
            registration_step(RegistrationFlags {
                install: true,
                noreg: true,
                ..RegistrationFlags::default()
            }),
            RegistrationStep::Skip
        );
    }

    #[test]
    fn only_install_wins_over_the_other_switches() {
        assert_eq!(
            registration_step(RegistrationFlags {
                install: true,
                unreg: true,
                noreg: true,
                only_install: true,
            }),
            RegistrationStep::InstallAndExit
        );
    }

    #[test]
    fn a_plain_start_up_leaves_the_registry_alone() {
        assert_eq!(
            registration_step(RegistrationFlags::default()),
            RegistrationStep::Skip
        );
    }
}
