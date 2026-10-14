//! Property tests with a deterministic seed.
//!
//! They use the crate's own [`Rng`] instead of a `proptest`/`quickcheck`
//! dependency, as task card E1 requires no new dependencies.

use std::path::PathBuf;

use rustamp_core::playlist::{EntryId, MediaLocation, Playlist};
use rustamp_playlist::{
    LoadedEntry, PlaylistFormat, Rng, randomize, read_playlist, read_playlist_as,
    read_playlist_path, write_playlist,
};

const SEED: u64 = 0x5EED_1234_ABCD_0001;

const FORMATS: [PlaylistFormat; 4] = [
    PlaylistFormat::M3u,
    PlaylistFormat::M3u8,
    PlaylistFormat::Pls,
    PlaylistFormat::Xspf,
];

/// Characters that are legal in a file name but need escaping in at least one
/// of the formats. `\` is left out because it is a path separator on Windows
/// and a legal character on Unix, so it cannot round-trip on both.
const NAME_CHARS: &[char] = &[
    'a', 'b', 'c', 'z', '0', '9', ' ', '#', '%', '&', '<', '>', '=', ';', '\'', '(', ')', ',', '.',
    '-', '_', '+', '[', ']', '!', '@', '$', '^', '~', '\u{00E4}', '\u{00F6}', '\u{20AC}',
];

const TITLE_CHARS: &[char] = &[
    'a', 'B', 'c', '0', '1', ' ', ',', '.', ':', ';', '-', '_', '&', '<', '>', '"', '\'', '(', ')',
    '[', ']', '!', '?', '%', '#', '=', '+', '\u{00E4}', '\u{00FC}', '\u{20AC}',
];

fn random_text(rng: &mut Rng, chars: &[char], min: usize, max: usize) -> String {
    let len = min + rng.below(max - min + 1);
    (0..len)
        .map(|_| chars[rng.below(chars.len())])
        .collect::<String>()
        .trim()
        .to_owned()
}

/// A random name for a file or a URL tail.
fn random_name(rng: &mut Rng) -> String {
    let name = random_text(rng, NAME_CHARS, 1, 12);
    if name.is_empty() {
        "x".to_owned()
    } else {
        name
    }
}

fn random_title(rng: &mut Rng) -> Option<String> {
    let title = random_text(rng, TITLE_CHARS, 1, 20);
    (!title.is_empty()).then_some(title)
}

fn random_location(rng: &mut Rng) -> MediaLocation {
    if rng.below(4) == 0 {
        MediaLocation::Url(format!("http://example.com/{}", random_name(rng)))
    } else {
        MediaLocation::File(PathBuf::from(format!("C:/Music/{}.mp3", random_name(rng))))
    }
}

fn random_entries(rng: &mut Rng, count: usize) -> Vec<LoadedEntry> {
    (0..count)
        .map(|_| {
            let mut entry = LoadedEntry::new(random_location(rng));
            entry.title = random_title(rng);
            // Whole seconds, sub-second values and "unknown" are all covered.
            entry.duration_ms = match rng.below(4) {
                0 => None,
                1 => Some(rng.below(1_000_000) as u64),
                2 => Some(rng.below(1000) as u64 * 1000),
                _ => Some(rng.below(1_000_000) as u64),
            };
            entry
        })
        .collect()
}

fn ids(playlist: &Playlist) -> Vec<EntryId> {
    playlist.iter().map(|e| e.id).collect()
}

fn sorted(mut ids: Vec<EntryId>) -> Vec<EntryId> {
    ids.sort();
    ids
}

#[test]
fn random_lists_survive_a_write_read_round_trip() {
    let mut rng = Rng::new(SEED);
    for round in 0..300 {
        let count = rng.below(40);
        let entries = random_entries(&mut rng, count);
        for format in FORMATS {
            let bytes = write_playlist(format, &entries).expect("writing never fails");
            let path = PathBuf::from(format!("C:/Music/lists/list.{}", format.extension()));
            let back = read_playlist_path(&bytes, &path).expect("reading never fails");
            assert_eq!(
                back.len(),
                entries.len(),
                "round {round}, format {format}: entry count changed"
            );
            assert_eq!(back, entries, "round {round}, format {format}");
        }
    }
}

#[test]
fn random_lists_survive_a_round_trip_through_the_playlist_model() {
    let mut rng = Rng::new(SEED ^ 0x1234);
    for _ in 0..100 {
        let count = rng.below(25);
        let entries = random_entries(&mut rng, count);
        let mut playlist = Playlist::new();
        rustamp_playlist::append_loaded(&mut playlist, &entries);
        let snapshot = rustamp_playlist::loaded_entries(&playlist);
        assert_eq!(snapshot, entries);
    }
}

#[test]
fn randomize_is_always_a_permutation_of_exactly_the_same_ids() {
    let mut rng = Rng::new(SEED ^ 0xABCD);
    for round in 0..200 {
        let count = rng.below(50);
        let mut playlist = Playlist::new();
        for i in 0..count {
            playlist.push(MediaLocation::File(PathBuf::from(format!("t{i}.mp3"))));
        }
        let before = ids(&playlist);
        randomize(&mut playlist, rng.next_u64());
        let after = ids(&playlist);

        assert_eq!(after.len(), before.len(), "round {round}: length changed");
        assert_eq!(
            sorted(after.clone()),
            sorted(before.clone()),
            "round {round}: ids changed"
        );
        assert_eq!(
            after.len(),
            after
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            "round {round}: duplicate ids"
        );

        // Randomizing again keeps the same set and stays a permutation.
        randomize(&mut playlist, rng.next_u64());
        assert_eq!(sorted(ids(&playlist)), sorted(before));
    }
}

#[test]
fn randomize_with_the_same_seed_is_reproducible() {
    let mut rng = Rng::new(SEED ^ 0x9999);
    for _ in 0..50 {
        let count = 1 + rng.below(30);
        let seed = rng.next_u64();
        let mut a = Playlist::new();
        let mut b = Playlist::new();
        for i in 0..count {
            let location = MediaLocation::File(PathBuf::from(format!("t{i}.mp3")));
            a.push(location.clone());
            b.push(location);
        }
        randomize(&mut a, seed);
        randomize(&mut b, seed);
        assert_eq!(ids(&a), ids(&b));
    }
}

#[test]
fn random_bytes_never_panic_any_reader() {
    let mut rng = Rng::new(0xBEEF_CAFE);
    let base = PathBuf::from("C:/Music");
    for round in 0..400 {
        let len = rng.below(300);
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            bytes.push((rng.next_u64() >> 24) as u8);
        }
        for format in FORMATS {
            let _ = read_playlist_as(format, &bytes, &base);
        }
        let _ = read_playlist(&bytes, &base);
        assert!(round < 1000);
    }
}

/// Random text built from XML/INI metacharacters reaches much deeper into the
/// parsers than uniform random bytes do.
#[test]
fn random_markup_never_panics_any_reader() {
    const ALPHABET: &[u8] = b"<>/=\"'&;#[]()!%\\ \t\r\nabc0123playlisttracklocationtitle";
    let mut rng = Rng::new(0xDEAD_BEEF);
    let base = PathBuf::from("C:/Music");
    for _ in 0..600 {
        let len = rng.below(400);
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            bytes.push(ALPHABET[rng.below(ALPHABET.len())]);
        }
        for format in FORMATS {
            if let Ok(entries) = read_playlist_as(format, &bytes, &base) {
                for entry in entries {
                    let _ = entry.location.to_display_string();
                }
            }
        }
        let _ = read_playlist(&bytes, &base);
    }
}
