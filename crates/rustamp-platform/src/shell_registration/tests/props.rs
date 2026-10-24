// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Seeded property tests for the shell registration's model.
//!
//! The unit tests next to the model pin hand-written cases. This file states
//! the invariants the model promises as properties over randomly drawn
//! installations, with a seeded generator, so that a failure is reproducible
//! and the crate needs no dependency.
//!
//! # The invariants
//!
//! * **The written key set is exactly the model's** ([`registry_entries`],
//!   [`install_into`]): the registration writes the keys S13 §3's verb table
//!   means - no key more, no key less - and the file type associations on
//!   top of them.
//! * **The install is idempotent** ([`install_into`]): installing twice
//!   yields the same keys with the same values, and the order of the entries
//!   does not matter. This is the reading of S13 §3 that "registration
//!   writes the same key set each time".
//! * **The removal takes what the install wrote** ([`uninstall_from`]): the
//!   keys the install created are gone afterwards, and a foreign key is
//!   still there.
//! * **The removal finds a key in any spelling** ([`uninstall_from`]): the
//!   registry's key names are case-insensitive, so a key that is written in
//!   another spelling is still the key the removal takes.
//! * **Every verb the spec names is there, and no other** ([`Handler`]): four
//!   verbs for a media file, four for a playlist file, one for a skin
//!   archive, two for a directory (S13 §3).
//! * **The `DropTarget` key has the documented shape**: the key is
//!   `…\shell\Play\DropTarget` or `…\shell\Enqueue\DropTarget`, and only
//!   those two verbs carry one (S13 §3).
//! * **Nothing panics**: an empty model, a duplicate verb, a path with a
//!   trailing separator and a very long path all leave the model alone.
//!
//! # What is not claimed
//!
//! * The properties are about the **model**, not about the registry: no key
//!   is written. The Win32 half that writes it is exercised by the VM-only
//!   test in `rustamp-app`.
//! * The properties do not say that the registration is the reference's: the
//!   file type extensions are own choices (S13 §3 names the handlers, not
//!   the extensions), and so is the key of the directory handler (S13 §5.10).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::shell_registration::{
    DROP_TARGET_VALUE, Handler, MEDIA_PROG_ID, RegistryModel, Verb, file_type_entries,
    install_into, registry_entries, uninstall_from, verb_entries, write_entries,
};

// ---------------------------------------------------------------------------
// The seeded generator
// ---------------------------------------------------------------------------

/// How many random scenarios one property runs.
const SEEDS: u64 = 24;

/// The seed base of the property "the written key set is exactly the model's".
const KEYS_BASE: u64 = 0xB1_0000;

/// The seed base of the property "the install is idempotent".
const IDEMPOTENT_BASE: u64 = 0xB2_0000;

/// The seed base of the property "the removal takes what the install wrote".
const REMOVE_BASE: u64 = 0xB3_0000;

/// The seed base of the property "every verb the spec names is there".
const VERBS_BASE: u64 = 0xB4_0000;

/// The seed base of the property "the DropTarget key has the documented shape".
const DROP_BASE: u64 = 0xB5_0000;

/// The seed base of the property "the removal finds a key in any spelling".
const CASE_BASE: u64 = 0xB6_0000;

/// The seed base of the property "nothing panics".
const PANIC_BASE: u64 = 0xB7_0000;

/// A seeded xorshift64 generator.
///
/// The property suite must be reproducible from its seed (a failure has to
/// be replayable) and the crate takes no dependency, so the generator lives
/// here. It is not a quality generator: it only has to spread the drawn
/// values evenly enough for the properties to mean something.
struct Rng(u64);

impl Rng {
    /// A generator for `seed`. The state must not be zero.
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A uniform index in `0..n`; `0` when there is nothing to choose from.
    fn index(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.next_u64() % n as u64) as usize
    }

    /// One of `items`.
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.index(items.len())]
    }

    /// Shuffles `items` (Fisher-Yates).
    fn shuffle<T>(&mut self, items: &mut [T]) {
        for index in (1..items.len()).rev() {
            let other = self.index(index + 1);
            items.swap(index, other);
        }
    }
}

// ---------------------------------------------------------------------------
// The spec's table
// ---------------------------------------------------------------------------

/// The four verbs of a media file and of a playlist file (S13 §3).
const SPEC_FILE_VERBS: &[&str] = &["open", "Play", "Enqueue", "ListBookmark"];

/// The one verb of a skin archive (S13 §3).
const SPEC_SKIN_VERBS: &[&str] = &["install"];

/// The two verbs of a directory (S13 §3).
const SPEC_DIRECTORY_VERBS: &[&str] = &["Play", "Enqueue"];

/// The verb table of S13 §3, as **we** write it.
///
/// The verb sets are the spec's; the **ProgIDs are ours** (`Rustamp.File`,
/// `Rustamp.PlayList`, `Rustamp.SkinZip`) — a deliberate deviation from the
/// reference's `Winamp.*` ones, see the module doc: a player must not claim
/// another product's ProgIDs and must not collide with a real Winamp
/// installation. `Directory` is the shell's own key (S13 §5.10 leaves the
/// directory handler's identifiers open).
const SPEC_TABLE: &[(&str, &[&str])] = &[
    ("Rustamp.File", SPEC_FILE_VERBS),
    ("Rustamp.PlayList", SPEC_FILE_VERBS),
    ("Rustamp.SkinZip", SPEC_SKIN_VERBS),
    ("Directory", SPEC_DIRECTORY_VERBS),
];

/// The verbs that carry a `DropTarget` key (S13 §3).
const DROP_TARGET_VERBS: &[&str] = &["Play", "Enqueue"];

/// The keys S13 §3's verb table means the registration writes.
fn expected_verb_keys() -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for (handler, verbs) in SPEC_TABLE {
        for verb in *verbs {
            keys.insert(format!(r"{handler}\shell\{verb}\command"));
            if DROP_TARGET_VERBS.contains(verb) {
                keys.insert(format!(r"{handler}\shell\{verb}\DropTarget"));
            }
        }
    }
    keys
}

/// The handler whose verbs live under `key`.
fn handler_of(key: &str) -> Handler {
    Handler::ALL
        .into_iter()
        .find(|handler| handler.key() == key)
        .unwrap_or_else(|| panic!("no handler lives under {key}"))
}

// ---------------------------------------------------------------------------
// The random world
// ---------------------------------------------------------------------------

/// A random path to the player's own executable.
fn random_exe(rng: &mut Rng) -> PathBuf {
    let mut path = match rng.index(4) {
        0 => r"C:\Program Files\Rustamp".to_string(),
        1 => r"C:\Games\Rustamp".to_string(),
        2 => r"D:\Spiele\Rustamp".to_string(),
        _ => r"\\nas\share\Rustamp".to_string(),
    };
    path.push('\\');
    path.push_str(match rng.index(4) {
        0 => "rustamp.exe",
        1 => "Rustamp.exe",
        2 => "rustamp-2.95.exe",
        _ => "rüstamp.exe",
    });
    PathBuf::from(path)
}

/// The foreign keys the registry holds next to the player's own.
///
/// None of them is below a tree of the player's own verbs, so the removal
/// must leave every one of them alone.
fn decoy_keys(rng: &mut Rng) -> Vec<(String, String)> {
    let mut decoys = Vec::new();
    // Another verb under one of the player's own ProgIDs.
    let (handler, _) = SPEC_TABLE[rng.index(SPEC_TABLE.len())];
    decoys.push((format!(r"{handler}\shell\OtherVerb\command"), String::new()));
    // Another player's own ProgID.
    decoys.push((
        format!(r"Other.Player{0}\shell\Play\command", rng.index(4)),
        String::new(),
    ));
    // A file type the player does not register for.
    decoys.push((
        format!(".{}", rng.pick(&["xyz", "mp3x", "opusx"])),
        String::new(),
    ));
    decoys
}

/// The value a foreign key carries.
fn decoy_value(key: &str) -> String {
    if key.starts_with('.') {
        "Other.Player".to_string()
    } else {
        r#""C:\other\player.exe" "%1""#.to_string()
    }
}

// ---------------------------------------------------------------------------
// The properties
// ---------------------------------------------------------------------------

/// The registration writes the keys of S13 §3's table - no key more, no key
/// less.
#[test]
fn the_written_key_set_is_exactly_the_specs_key_set() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(KEYS_BASE + seed);
        let exe = random_exe(&mut rng);
        let entries = registry_entries(&exe);

        let written: BTreeSet<&str> = entries.iter().map(|entry| entry.key.as_str()).collect();
        let expected_keys = expected_verb_keys();
        let expected: BTreeSet<&str> = expected_keys.iter().map(String::as_str).collect();
        assert_eq!(
            written, expected,
            "seed {seed}: the registration writes other keys than the spec's table"
        );
        // A key is written once: two entries under one key would make the
        // result depend on the order of the writes.
        assert_eq!(
            written.len(),
            entries.len(),
            "seed {seed}: a key is written twice"
        );

        // The install writes the verb keys and the file type associations on
        // top of them - and nothing else.
        let mut model = RegistryModel::new();
        install_into(&mut model, &exe);
        let installed: BTreeSet<&str> = model.keys().map(|(key, _)| key.as_str()).collect();
        let mut expected_install = expected_verb_keys();
        expected_install.extend(file_type_entries().iter().map(|entry| entry.key.clone()));
        let expected_install: BTreeSet<&str> =
            expected_install.iter().map(String::as_str).collect();
        assert_eq!(
            installed, expected_install,
            "seed {seed}: the install writes other keys than the model"
        );
    }
}

/// Installing twice changes nothing, and the order of the entries does not
/// matter.
#[test]
fn installing_twice_changes_nothing() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(IDEMPOTENT_BASE + seed);
        let exe = random_exe(&mut rng);

        let mut model = RegistryModel::new();
        install_into(&mut model, &exe);
        let after_first = model.clone();

        // The second install writes the same keys with the same values.
        install_into(&mut model, &exe);
        assert_eq!(
            model, after_first,
            "seed {seed}: the second install changed the registry"
        );

        // The order of the entries does not matter: no key is written twice
        // with a different value.
        let mut entries = registry_entries(&exe);
        entries.extend(file_type_entries());
        rng.shuffle(&mut entries);
        let mut shuffled = RegistryModel::new();
        write_entries(&mut shuffled, &entries);
        assert_eq!(
            shuffled, after_first,
            "seed {seed}: the order of the entries matters"
        );
    }
}

/// The removal takes the keys the install wrote and leaves the foreign keys
/// alone.
#[test]
fn the_removal_takes_what_the_install_wrote_and_leaves_the_rest() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(REMOVE_BASE + seed);
        let exe = random_exe(&mut rng);

        // The registry as it is before the install: the player's own keys
        // are not there yet, and somebody else's are.
        let decoys = decoy_keys(&mut rng);
        let decoy_paths: Vec<String> = decoys.iter().map(|(key, _)| key.clone()).collect();
        let mut model = RegistryModel::new();
        for (key, name) in &decoys {
            model.insert((key.clone(), name.clone()), decoy_value(key));
        }

        install_into(&mut model, &exe);
        let ours: Vec<(String, String)> = model
            .keys()
            .map(|(key, name)| (key.clone(), name.clone()))
            .filter(|(key, _)| !decoy_paths.contains(key))
            .collect();
        assert!(!ours.is_empty(), "seed {seed}: the install wrote nothing");

        uninstall_from(&mut model);

        // Every key the install wrote is gone.
        for (key, name) in &ours {
            assert!(
                !model.contains_key(&(key.clone(), name.clone())),
                "seed {seed}: the removal left the key {key}"
            );
        }
        // Every foreign key is still there.
        for (key, name) in &decoys {
            assert!(
                model.contains_key(&(key.clone(), name.clone())),
                "seed {seed}: the removal took the foreign key {key}"
            );
        }
        // And nothing else is left.
        assert_eq!(
            model.len(),
            decoys.len(),
            "seed {seed}: the removal left other keys behind"
        );
    }
}

/// Every verb the spec names is there for its ProgID, and no verb the spec
/// does not name is there.
#[test]
fn every_verb_the_spec_names_is_present_and_no_other_is() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(VERBS_BASE + seed);
        let exe = random_exe(&mut rng);
        for (key, verbs) in SPEC_TABLE {
            let handler = handler_of(key);

            // The verbs of the handler are the spec's verbs - no more, no
            // less (S13 §3: four, four, one, two).
            let names: BTreeSet<&str> = handler.verbs().iter().map(|verb| verb.name).collect();
            let expected: BTreeSet<&str> = verbs.iter().copied().collect();
            assert_eq!(
                names, expected,
                "seed {seed}: the handler {key} has other verbs than the spec's table"
            );
            assert_eq!(
                handler.verbs().len(),
                verbs.len(),
                "seed {seed}: the handler {key} has the wrong number of verbs"
            );

            // The keys the verbs wrote are the keys of the table.
            let written: BTreeSet<String> = registry_entries(&exe)
                .iter()
                .filter(|entry| entry.key.starts_with(&format!(r"{key}\shell\")))
                .map(|entry| entry.key.clone())
                .collect();
            let mut expected_keys: BTreeSet<String> = verbs
                .iter()
                .map(|verb| format!(r"{key}\shell\{verb}\command"))
                .collect();
            for verb in *verbs {
                if DROP_TARGET_VERBS.contains(verb) {
                    expected_keys.insert(format!(r"{key}\shell\{verb}\DropTarget"));
                }
            }
            assert_eq!(
                written, expected_keys,
                "seed {seed}: the handler {key} writes other keys than the spec's table"
            );
        }
    }
}

/// The `DropTarget` keys have the shape S13 §3 documents.
#[test]
fn the_drop_target_keys_have_the_specs_shape() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(DROP_BASE + seed);
        let exe = random_exe(&mut rng);
        let entries = registry_entries(&exe);

        for handler in Handler::ALL {
            for verb in handler.verbs() {
                let command = format!(r"{}\shell\{}\command", handler.key(), verb.name);
                assert!(
                    entries.iter().any(|entry| entry.key == command),
                    "seed {seed}: the verb {} of {} writes no command",
                    verb.name,
                    handler.key()
                );
                let drop = format!(r"{}\shell\{}\DropTarget", handler.key(), verb.name);
                let drop_entry = entries.iter().find(|entry| entry.key == drop);
                assert_eq!(
                    drop_entry.is_some(),
                    verb.drop_target,
                    "seed {seed}: the verb {} of {} has the wrong DropTarget",
                    verb.name,
                    handler.key()
                );
                if let Some(entry) = drop_entry {
                    // S13 §3: only `Play` and `Enqueue` carry a `DropTarget`.
                    assert!(
                        DROP_TARGET_VERBS.contains(&verb.name),
                        "seed {seed}: the verb {} of {} carries a DropTarget",
                        verb.name,
                        handler.key()
                    );
                    assert_eq!(
                        entry.name, "",
                        "seed {seed}: the DropTarget key carries a named value"
                    );
                    assert_eq!(
                        entry.value, DROP_TARGET_VALUE,
                        "seed {seed}: the DropTarget key carries another value"
                    );
                }
            }
        }

        // No key carries a `DropTarget` outside that shape.
        for entry in &entries {
            if !entry.key.contains("DropTarget") {
                continue;
            }
            let parts: Vec<&str> = entry.key.split('\\').collect();
            assert_eq!(
                parts.len(),
                4,
                "seed {seed}: the key {} has another shape",
                entry.key
            );
            assert_eq!(parts[1], "shell", "seed {seed}: the key {}", entry.key);
            assert_eq!(
                parts[3], "DropTarget",
                "seed {seed}: the key {} has another shape",
                entry.key
            );
            let handler = handler_of(parts[0]);
            assert!(
                handler.verbs().iter().any(|verb| verb.name == parts[2]),
                "seed {seed}: the key {} names a verb the handler does not have",
                entry.key
            );
        }
    }
}

/// The removal finds a key whatever case it carries, as the registry does.
#[test]
fn the_removal_finds_a_key_whatever_case_it_carries() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(CASE_BASE + seed);
        let exe = random_exe(&mut rng);

        // The registry's key names are case-insensitive: the same key in
        // another spelling is still the same key.
        let mut model = RegistryModel::new();
        let mut entries = registry_entries(&exe);
        entries.extend(file_type_entries());
        for entry in entries {
            let spelled = if rng.index(2) == 0 {
                entry.key.to_uppercase()
            } else {
                entry.key.to_lowercase()
            };
            model.insert((spelled, entry.name), entry.value);
        }

        uninstall_from(&mut model);
        assert!(
            model.is_empty(),
            "seed {seed}: the removal missed a key in another spelling"
        );
    }
}

/// Nothing panics.
#[test]
fn nothing_panics() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(PANIC_BASE + seed);

        // An empty model: the removal has nothing to remove.
        let mut empty = RegistryModel::new();
        uninstall_from(&mut empty);
        assert!(empty.is_empty(), "seed {seed}: the removal wrote something");

        // An empty model: the install writes everything, the removal takes it
        // all away again.
        let exe = random_exe(&mut rng);
        let mut model = RegistryModel::new();
        install_into(&mut model, &exe);
        assert!(!model.is_empty(), "seed {seed}: the install wrote nothing");
        uninstall_from(&mut model);
        assert!(
            model.is_empty(),
            "seed {seed}: the removal left the player's own keys"
        );

        // A duplicate verb: the same verb twice under one handler.
        let verb = Verb {
            name: "Play",
            label: "Play",
            arguments: "\"%1\"",
            drop_target: true,
        };
        let entries = verb_entries(MEDIA_PROG_ID, &[verb, verb], Path::new("C:/rustamp.exe"));
        let mut model = RegistryModel::new();
        write_entries(&mut model, &entries);
        assert_eq!(
            model.len(),
            2,
            "seed {seed}: the duplicate verb writes more than its own two keys"
        );
        uninstall_from(&mut model);
        assert!(
            model.is_empty(),
            "seed {seed}: the removal left the duplicate verb"
        );

        // A path with a trailing separator.
        let exe = PathBuf::from(r"C:\Games\Rustamp\");
        let mut model = RegistryModel::new();
        install_into(&mut model, &exe);
        assert!(!model.is_empty(), "seed {seed}: the install wrote nothing");
        uninstall_from(&mut model);
        assert!(
            model.is_empty(),
            "seed {seed}: the removal left the player's own keys"
        );

        // A very long path.
        let exe = PathBuf::from(format!(r"C:\{}\rustamp.exe", "a".repeat(4096)));
        let mut model = RegistryModel::new();
        install_into(&mut model, &exe);
        uninstall_from(&mut model);
        assert!(
            model.is_empty(),
            "seed {seed}: the removal left the player's own keys"
        );
    }
}
