//! A write that changes nothing must not touch the file.
//!
//! The byte layout of an ID3v2 tag is not deterministic: `lofty` keeps the
//! text frames of a tag in a hash map, so two writes of the same metadata
//! produce the same frames in a different order. A write that rewrites the
//! file with the values it already has therefore changes the file — its
//! modification time moves, and every tool that watches the file sees a new
//! file. The writer asks before it touches the file whether the request
//! would change the tags the file has (see the `unchanged` module of the
//! crate), and leaves the file alone when it would not.
//!
//! The tests are headless: they copy a fixture into a temporary directory
//! and work on the copy.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rustamp_core::meta::TrackMetadata;
use rustamp_meta::{
    R128_ALBUM_GAIN_KEY, R128_TRACK_GAIN_KEY, REFERENCE_LOUDNESS_KEY, TagField, read_tags,
    write_tags_report, write_tags_with_report,
};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The formats the writer supports, with the fixture of each.
const FORMATS: &[&str] = &[
    "sine.mp3",
    "sine.flac",
    "sine.ogg",
    "sine.opus",
    "sine_aac.m4a",
    "sine_alac.m4a",
];

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../rustamp-decoders/tests/fixtures")
        .join(name)
}

/// Copies a fixture into `dir` so the test never modifies the fixture.
fn copy_fixture(dir: &TempDir, name: &str) -> PathBuf {
    let dst = dir.path().join(name);
    std::fs::copy(fixture(name), &dst).expect("copy the fixture");
    dst
}

/// The bytes of a file.
fn bytes_of(path: &Path) -> Vec<u8> {
    std::fs::read(path).expect("read the file")
}

/// The modification time of a file.
fn mtime_of(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .expect("stat the file")
        .modified()
        .expect("the modification time of the file")
}

/// How often `text` is in `bytes`.
fn occurrences(bytes: &[u8], text: &str) -> usize {
    bytes
        .windows(text.len())
        .filter(|window| *window == text.as_bytes())
        .count()
}

/// A metadata that names every common field.
///
/// The `extra` entry is one every format can store, so the comparison of
/// the `extra` entries is part of the test.
fn full_metadata() -> TrackMetadata {
    let mut metadata = TrackMetadata {
        title: Some("Round Trip Title".into()),
        artist: Some("Round Trip Artist".into()),
        album: Some("Round Trip Album".into()),
        album_artist: Some("Round Trip Album Artist".into()),
        genre: Some("Round Trip Genre".into()),
        comment: Some("Round Trip Comment".into()),
        year: Some(2024),
        track_no: Some(3),
        track_total: Some(9),
        disc_no: Some(1),
        ..TrackMetadata::default()
    };
    metadata
        .extra
        .insert("REPLAYGAIN_TRACK_GAIN".into(), "-3.50 dB".into());
    metadata
}

/// A metadata that names the values the MP3 fixture already has.
fn fixture_metadata() -> TrackMetadata {
    TrackMetadata {
        title: Some("Sine Title".into()),
        artist: Some("Test Artist".into()),
        year: Some(2001),
        track_no: Some(3),
        track_total: Some(9),
        ..TrackMetadata::default()
    }
}

/// A 128-byte ID3v1.1 tag.
fn id3v1(title: &str) -> Vec<u8> {
    let mut tag = vec![0u8; 128];
    tag[0..3].copy_from_slice(b"TAG");
    let bytes = title.as_bytes();
    let len = bytes.len().min(30);
    tag[3..3 + len].copy_from_slice(&bytes[..len]);
    tag[127] = 255; // the genre: none
    tag
}

// ---------------------------------------------------------------------------
// The rule
// ---------------------------------------------------------------------------

#[test]
fn an_identical_write_leaves_the_file_alone() {
    for name in FORMATS {
        let dir = TempDir::new().unwrap();
        let path = copy_fixture(&dir, name);
        let metadata = full_metadata();

        // The first write puts the values into the file.
        let report = write_tags_report(&path, &metadata).unwrap();
        assert!(!report.unchanged, "{name}: the first write changed nothing");
        assert!(report.is_complete(), "{name}: {report:?}");

        let bytes = bytes_of(&path);
        let mtime = mtime_of(&path);

        // The second write asks for the same values: the file is not
        // touched, so its bytes and its modification time stay the ones it
        // had.
        let report = write_tags_report(&path, &metadata).unwrap();
        assert!(report.unchanged, "{name}: {report:?}");
        assert!(report.is_complete(), "{name}: {report:?}");
        assert_eq!(bytes_of(&path), bytes, "{name}: the file was rewritten");
        assert_eq!(mtime_of(&path), mtime, "{name}: the file was touched");
    }
}

#[test]
fn a_changed_field_is_written() {
    for name in FORMATS {
        let dir = TempDir::new().unwrap();
        let path = copy_fixture(&dir, name);
        let metadata = full_metadata();
        write_tags_report(&path, &metadata).unwrap();
        let bytes = bytes_of(&path);

        // One field changes: the file is written again.
        let changed = TrackMetadata {
            title: Some("Another Title".into()),
            ..TrackMetadata::default()
        };
        let report = write_tags_report(&path, &changed).unwrap();
        assert!(!report.unchanged, "{name}: {report:?}");
        assert_ne!(bytes_of(&path), bytes, "{name}: the file was not written");
        assert_eq!(
            read_tags(&path).unwrap().title.as_deref(),
            Some("Another Title"),
            "{name}"
        );

        // And the values that were written before are still there.
        let after = read_tags(&path).unwrap();
        assert_eq!(after.artist.as_deref(), Some("Round Trip Artist"), "{name}");
        assert_eq!(after.year, Some(2024), "{name}");
        assert_eq!(after.track_no, Some(3), "{name}");
    }
}

#[test]
fn a_removal_of_an_absent_field_does_not_write() {
    for name in FORMATS {
        let dir = TempDir::new().unwrap();
        let path = copy_fixture(&dir, name);

        // A genre in the file, so that the removal has something to remove.
        let genre = TrackMetadata {
            genre: Some("A Genre".into()),
            ..TrackMetadata::default()
        };
        write_tags_report(&path, &genre).unwrap();

        // The first removal takes the genre out: the file changes.
        let report =
            write_tags_with_report(&path, &TrackMetadata::default(), &[TagField::Genre]).unwrap();
        assert!(!report.unchanged, "{name}: {report:?}");
        assert_eq!(read_tags(&path).unwrap().genre, None, "{name}");

        let bytes = bytes_of(&path);
        let mtime = mtime_of(&path);

        // The second removal names a field the file does not have: nothing
        // changes, so the file is not touched.
        let report =
            write_tags_with_report(&path, &TrackMetadata::default(), &[TagField::Genre]).unwrap();
        assert!(report.unchanged, "{name}: {report:?}");
        assert_eq!(bytes_of(&path), bytes, "{name}: the file was rewritten");
        assert_eq!(mtime_of(&path), mtime, "{name}: the file was touched");
    }
}

#[test]
fn a_value_that_only_the_id3v1_tag_holds_is_still_a_change() {
    const TITLE: &str = "Only In The V1 Tag";

    let dir = TempDir::new().unwrap();
    let path = copy_fixture(&dir, "sine.mp3");

    // The ID3v2 tag loses its title, and an ID3v1 tag takes it over: the
    // title is now only in the second tag of the file.
    write_tags_with_report(&path, &TrackMetadata::default(), &[TagField::Title]).unwrap();
    let mut bytes = bytes_of(&path);
    bytes.extend_from_slice(&id3v1(TITLE));
    std::fs::write(&path, &bytes).unwrap();

    // The reader sees the title — it comes from the ID3v1 tag, because the
    // ID3v2 tag does not have it.
    assert_eq!(
        read_tags(&path).unwrap().title.as_deref(),
        Some(TITLE),
        "the ID3v1 tag does not fill the gap"
    );
    // The title is in the file once: in the ID3v1 tag at its end.
    assert_eq!(
        occurrences(&bytes, TITLE),
        1,
        "the title is in the ID3v2 tag as well"
    );

    // A request for the value of the ID3v1 tag is a change: the write puts
    // the value into the tag the reader prefers.
    let request = TrackMetadata {
        title: Some(TITLE.into()),
        ..TrackMetadata::default()
    };
    let report = write_tags_report(&path, &request).unwrap();
    assert!(!report.unchanged, "{report:?}");
    assert_eq!(
        read_tags(&path).unwrap().title.as_deref(),
        Some(TITLE),
        "the title is not in the file"
    );
    assert_eq!(
        occurrences(&bytes_of(&path), TITLE),
        2,
        "the title is not in the ID3v2 tag"
    );

    // And now that the primary tag holds the value, the same request
    // changes nothing.
    let bytes = bytes_of(&path);
    let mtime = mtime_of(&path);
    let report = write_tags_report(&path, &request).unwrap();
    assert!(report.unchanged, "{report:?}");
    assert_eq!(bytes_of(&path), bytes, "the file was rewritten");
    assert_eq!(mtime_of(&path), mtime, "the file was touched");
}

#[test]
fn an_extra_entry_the_format_cannot_store_is_not_a_change() {
    let dir = TempDir::new().unwrap();
    let path = copy_fixture(&dir, "sine.mp3");

    // The values the file already has, plus a key no tag format has a field
    // for: the key cannot be in the file and cannot be put there.
    let mut metadata = fixture_metadata();
    metadata
        .extra
        .insert(REFERENCE_LOUDNESS_KEY.into(), "89 dB SPL".into());

    let bytes = bytes_of(&path);
    let mtime = mtime_of(&path);

    let report = write_tags_report(&path, &metadata).unwrap();
    assert_eq!(
        report.unwritten_extra,
        vec![REFERENCE_LOUDNESS_KEY.to_owned()],
        "{report:?}"
    );
    // The refused key is reported, but it is not a change: the file holds
    // every value the request names.
    assert!(report.unchanged, "{report:?}");
    assert_eq!(bytes_of(&path), bytes, "the file was rewritten");
    assert_eq!(mtime_of(&path), mtime, "the file was touched");
}

#[test]
fn the_r128_keys_are_compared() {
    // The R128 keys of an ID3v2 tag live in `TXXX` frames the generic model
    // cannot see, so the comparison has to ask the concrete tag for them.
    for name in ["sine.mp3", "sine.flac"] {
        let dir = TempDir::new().unwrap();
        let path = copy_fixture(&dir, name);

        let mut metadata = TrackMetadata::default();
        metadata
            .extra
            .insert(R128_TRACK_GAIN_KEY.into(), "-768".into());
        metadata
            .extra
            .insert(R128_ALBUM_GAIN_KEY.into(), "-1280".into());

        let report = write_tags_report(&path, &metadata).unwrap();
        assert!(report.unwritten_extra.is_empty(), "{name}: {report:?}");
        assert!(!report.unchanged, "{name}: {report:?}");

        let bytes = bytes_of(&path);
        let mtime = mtime_of(&path);

        let report = write_tags_report(&path, &metadata).unwrap();
        assert!(report.unchanged, "{name}: {report:?}");
        assert_eq!(bytes_of(&path), bytes, "{name}: the file was rewritten");
        assert_eq!(mtime_of(&path), mtime, "{name}: the file was touched");

        // The key may be spelled differently: the frames and the comments
        // carry the canonical name, so the value is the same one.
        let mut other = TrackMetadata::default();
        other.extra.insert("r128_track_gain".into(), "-768".into());
        other.extra.insert("r128_album_gain".into(), "-1280".into());
        let report = write_tags_report(&path, &other).unwrap();
        assert!(report.unwritten_extra.is_empty(), "{name}: {report:?}");
        assert!(report.unchanged, "{name}: {report:?}");
        assert_eq!(bytes_of(&path), bytes, "{name}: the file was rewritten");
        assert_eq!(mtime_of(&path), mtime, "{name}: the file was touched");
    }
}

#[test]
fn a_field_the_request_does_not_name_cannot_decide() {
    // `None` means "keep": a field the request does not name is not
    // compared, so the values of the file stay as they are.
    let dir = TempDir::new().unwrap();
    let path = copy_fixture(&dir, "sine.flac");

    let metadata = TrackMetadata {
        title: Some("Sine Title".into()),
        ..TrackMetadata::default()
    };
    let report = write_tags_report(&path, &metadata).unwrap();
    assert!(report.unchanged, "{report:?}");
    assert_eq!(
        read_tags(&path).unwrap().title.as_deref(),
        Some("Sine Title")
    );
    assert_eq!(
        read_tags(&path).unwrap().artist.as_deref(),
        Some("Test Artist")
    );
}
