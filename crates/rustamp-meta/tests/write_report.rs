//! The write report names the values the target format could not store.
//!
//! The MP4 `trkn` and `disk` atoms hold a pair of 16-bit numbers, and a
//! zero means "no number at all": the MP4 family carries neither a zero nor
//! a number above 65535. The other formats store the number as text and
//! carry every `u32`. The report tells the two apart
//! ([`WriteReport::unstoreable`]), so a lost value is never silent.
//!
//! The fixtures are copied into a temporary directory first: a test never
//! writes into the fixture tree.

use std::path::PathBuf;

use rustamp_core::meta::TrackMetadata;
use rustamp_meta::{TagField, read_tags, write_tags_report, write_tags_with_report};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

/// A metadata that names one number.
fn number(field: &str, value: u32) -> TrackMetadata {
    let mut metadata = TrackMetadata::default();
    match field {
        "track" => metadata.track_no = Some(value),
        "track_total" => metadata.track_total = Some(value),
        "disc" => metadata.disc_no = Some(value),
        other => panic!("no such field: {other}"),
    }
    metadata
}

// ---------------------------------------------------------------------------
// The MP4 family
// ---------------------------------------------------------------------------

#[test]
fn the_mp4_family_reports_the_numbers_it_cannot_hold() {
    let dir = TempDir::new().unwrap();

    for name in ["sine_aac.m4a", "sine_alac.m4a"] {
        let path = copy_fixture(&dir, name);

        // A number the format can hold is not reported.
        let report = write_tags_report(&path, &number("track", 5)).unwrap();
        assert!(
            report.unstoreable.is_empty(),
            "{name}: {:?}",
            report.unstoreable
        );
        assert!(report.is_complete(), "{name}");
        assert_eq!(read_tags(&path).unwrap().track_no, Some(5), "{name}");

        // A number above 65535 is not in the file afterwards, and the report
        // says so.
        let report = write_tags_report(&path, &number("track", 70_000)).unwrap();
        assert_eq!(report.unstoreable, vec![TagField::Track], "{name}");
        assert!(!report.is_complete(), "{name}");

        // The number the file had is gone with it: the writer replaces the
        // number, and the format has no room for the new one.
        assert_eq!(read_tags(&path).unwrap().track_no, None, "{name}");

        // A zero is "no number at all" in the MP4 family, so it is lost the
        // same way.
        let report = write_tags_report(&path, &number("track", 0)).unwrap();
        assert_eq!(report.unstoreable, vec![TagField::Track], "{name}");
        assert!(!report.is_complete(), "{name}");
        assert_eq!(read_tags(&path).unwrap().track_no, None, "{name}");

        // The disc number is the other half of the same pair.
        let report = write_tags_report(&path, &number("disc", 70_000)).unwrap();
        assert_eq!(report.unstoreable, vec![TagField::Disc], "{name}");
        assert!(!report.is_complete(), "{name}");
        assert_eq!(read_tags(&path).unwrap().disc_no, None, "{name}");

        // The track total is the third number of the pair.
        let report = write_tags_report(&path, &number("track_total", 70_000)).unwrap();
        assert_eq!(report.unstoreable, vec![TagField::Track], "{name}");
        assert!(!report.is_complete(), "{name}");
        assert_eq!(read_tags(&path).unwrap().track_total, None, "{name}");
    }
}

#[test]
fn the_mp4_family_reports_the_track_and_the_disc_separately() {
    let dir = TempDir::new().unwrap();
    let path = copy_fixture(&dir, "sine_aac.m4a");

    let mut metadata = number("track", 70_000);
    metadata.disc_no = Some(70_000);

    let report = write_tags_report(&path, &metadata).unwrap();
    assert_eq!(
        report.unstoreable,
        vec![TagField::Track, TagField::Disc],
        "the report names the wrong fields"
    );
}

// ---------------------------------------------------------------------------
// The other formats
// ---------------------------------------------------------------------------

#[test]
fn the_id3v2_and_vorbis_formats_carry_the_same_numbers() {
    let dir = TempDir::new().unwrap();

    for name in ["sine.mp3", "sine.flac", "sine.ogg", "sine.opus"] {
        let path = copy_fixture(&dir, name);

        for value in [0u32, 70_000, u32::MAX] {
            let mut metadata = number("track", value);
            metadata.disc_no = Some(value);

            let report = write_tags_report(&path, &metadata).unwrap();
            assert!(
                report.unstoreable.is_empty(),
                "{name}: {:?} was reported for {value}",
                report.unstoreable
            );
            assert!(report.is_complete(), "{name}: {value}");

            // And the value is in the file.
            let after = read_tags(&path).unwrap();
            assert_eq!(after.track_no, Some(value), "{name}: {value}");
            assert_eq!(after.disc_no, Some(value), "{name}: {value}");
        }
    }
}

#[test]
fn a_normal_write_reports_nothing() {
    let dir = TempDir::new().unwrap();

    for name in [
        "sine.mp3",
        "sine.flac",
        "sine.ogg",
        "sine.opus",
        "sine_aac.m4a",
        "sine_alac.m4a",
    ] {
        let path = copy_fixture(&dir, name);

        let metadata = TrackMetadata {
            title: Some("Round Trip Title".into()),
            artist: Some("Round Trip Artist".into()),
            album: Some("Round Trip Album".into()),
            year: Some(2024),
            genre: Some("Round Trip Genre".into()),
            comment: Some("Round Trip Comment".into()),
            track_no: Some(3),
            track_total: Some(9),
            disc_no: Some(1),
            ..TrackMetadata::default()
        };

        let report = write_tags_report(&path, &metadata).unwrap();
        assert!(
            report.unstoreable.is_empty(),
            "{name}: {:?}",
            report.unstoreable
        );
        assert!(report.is_complete(), "{name}: {:?}", report);

        // The numbers are in the file.
        let after = read_tags(&path).unwrap();
        assert_eq!(after.track_no, Some(3), "{name}");
        assert_eq!(after.track_total, Some(9), "{name}");
        assert_eq!(after.disc_no, Some(1), "{name}");
    }
}

// ---------------------------------------------------------------------------
// The removals
// ---------------------------------------------------------------------------

#[test]
fn a_removed_field_is_not_reported_as_unstoreable() {
    // The caller asked for the field to be gone, so the format is not the
    // reason it is not in the file.
    let dir = TempDir::new().unwrap();
    let path = copy_fixture(&dir, "sine_aac.m4a");

    let report =
        write_tags_with_report(&path, &number("track", 70_000), &[TagField::Track]).unwrap();
    assert!(report.unstoreable.is_empty(), "{:?}", report.unstoreable);
    assert!(report.unremoved.is_empty(), "{:?}", report.unremoved);
    assert!(report.is_complete(), "{report:?}");
    assert_eq!(read_tags(&path).unwrap().track_no, None);
}
