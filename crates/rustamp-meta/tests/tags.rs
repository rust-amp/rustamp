//! Integration tests for `rustamp-meta`.
//!
//! The compressed fixtures live in `crates/rustamp-decoders/tests/fixtures/`
//! and are 1.5 s, 440 Hz, 44.1 kHz stereo sine waves made with ffmpeg (see
//! `FIXTURES.txt` there). They are only ever read; a test that writes copies
//! the fixture into a temporary directory first.
//!
//! WAV/AIFF and raw tag inputs are built in memory, so no external tool is
//! needed.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use rustamp_core::meta::TrackMetadata;
use rustamp_meta::{
    ALBUM_GAIN_KEY, ALBUM_PEAK_KEY, Artwork, R128_ALBUM_GAIN_KEY, R128_TRACK_GAIN_KEY,
    REFERENCE_LOUDNESS_KEY, ReplayGainTags, read_artwork, read_tags, scan_paths,
    supported_extensions, supports_extension, write_replaygain, write_tags, write_tags_report,
};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../rustamp-decoders/tests/fixtures")
        .join(name)
}

/// Copies a fixture into `dir` so tests never modify the fixture in place.
fn copy_fixture(dir: &TempDir, name: &str) -> PathBuf {
    let dst = dir.path().join(name);
    std::fs::copy(fixture(name), &dst).expect("copy fixture");
    dst
}

fn write_file(dir: &TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).expect("write temp file");
    path
}

/// A metadata set that exercises every common field.
fn full_metadata() -> TrackMetadata {
    let mut metadata = TrackMetadata {
        title: Some("Round Trip Title".into()),
        artist: Some("Round Trip Artist".into()),
        album: Some("Round Trip Album".into()),
        album_artist: Some("Round Trip Album Artist".into()),
        year: Some(2024),
        genre: Some("Round Trip Genre".into()),
        comment: Some("Round Trip Comment".into()),
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

// --- raw tag builders -------------------------------------------------------

fn put_fixed(field: &mut [u8], value: &str) {
    let bytes = value.as_bytes();
    let n = bytes.len().min(field.len());
    field[..n].copy_from_slice(&bytes[..n]);
}

/// A 128-byte ID3v1.1 tag.
fn id3v1(title: &str, artist: &str, album: &str, year: &str, comment: &str) -> Vec<u8> {
    let mut tag = vec![0u8; 128];
    tag[0..3].copy_from_slice(b"TAG");
    put_fixed(&mut tag[3..33], title);
    put_fixed(&mut tag[33..63], artist);
    put_fixed(&mut tag[63..93], album);
    put_fixed(&mut tag[93..97], year);
    put_fixed(&mut tag[97..125], comment);
    tag[127] = 255; // genre: none
    tag
}

/// A file that is only a raw ID3v1 tag, prefixed with a little silence so the
/// MP3 reader's end-of-file seeks stay in range.
fn id3v1_only_file(title: &str) -> Vec<u8> {
    let mut file = vec![0u8; 64];
    file.extend_from_slice(&id3v1(title, "", "", "", ""));
    file
}

fn synchsafe(size: u32) -> [u8; 4] {
    [
        ((size >> 21) & 0x7F) as u8,
        ((size >> 14) & 0x7F) as u8,
        ((size >> 7) & 0x7F) as u8,
        (size & 0x7F) as u8,
    ]
}

fn id3v2_frame(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.extend_from_slice(id);
    frame.extend_from_slice(&(data.len() as u32).to_be_bytes()); // ID3v2.3: plain big-endian
    frame.extend_from_slice(&[0, 0]); // flags
    frame.extend_from_slice(data);
    frame
}

fn id3v2_tag(frames: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = frames.concat();
    let mut tag = Vec::new();
    tag.extend_from_slice(b"ID3");
    tag.extend_from_slice(&[0x03, 0x00, 0x00]); // v2.3, no flags
    tag.extend_from_slice(&synchsafe(body.len() as u32));
    tag.extend_from_slice(&body);
    tag
}

fn id3v2_text_frame(id: &[u8; 4], text: &str) -> Vec<u8> {
    let mut data = vec![0x00]; // ISO-8859-1
    data.extend_from_slice(text.as_bytes());
    id3v2_frame(id, &data)
}

fn id3v2_apic_frame(mime: &str, kind: u8, description: &str, picture: &[u8]) -> Vec<u8> {
    let mut data = vec![0x00]; // ISO-8859-1
    data.extend_from_slice(mime.as_bytes());
    data.push(0x00);
    data.push(kind);
    data.extend_from_slice(description.as_bytes());
    data.push(0x00);
    data.extend_from_slice(picture);
    id3v2_frame(b"APIC", &data)
}

/// A footer-only APEv2 tag (the MP3 reader finds it at the end of the file).
fn apev2_tag(items: &[(&str, &str)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (key, value) in items {
        body.extend_from_slice(&(value.len() as u32).to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes()); // flags: text, not read-only
        body.extend_from_slice(key.as_bytes());
        body.push(0x00);
        body.extend_from_slice(value.as_bytes());
    }

    let mut footer = Vec::new();
    footer.extend_from_slice(b"APETAGEX");
    footer.extend_from_slice(&2000u32.to_le_bytes()); // version
    footer.extend_from_slice(&((body.len() + 32) as u32).to_le_bytes()); // size: items + footer
    footer.extend_from_slice(&(items.len() as u32).to_le_bytes());
    footer.extend_from_slice(&0u32.to_le_bytes()); // flags
    footer.extend_from_slice(&[0u8; 8]); // reserved

    let mut tag = body;
    tag.extend_from_slice(&footer);
    tag
}

fn riff_chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(id);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// A minimal 8-bit mono PCM WAV with a RIFF `LIST INFO` chunk.
fn wav_with_info(title: &str, artist: &str) -> Vec<u8> {
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes()); // PCM
    fmt.extend_from_slice(&1u16.to_le_bytes()); // mono
    fmt.extend_from_slice(&44100u32.to_le_bytes());
    fmt.extend_from_slice(&44100u32.to_le_bytes()); // byte rate
    fmt.extend_from_slice(&1u16.to_le_bytes()); // block align
    fmt.extend_from_slice(&8u16.to_le_bytes()); // bits per sample

    let mut info = b"INFO".to_vec();
    let mut inam = title.as_bytes().to_vec();
    inam.push(0);
    let mut iart = artist.as_bytes().to_vec();
    iart.push(0);
    info.extend_from_slice(&riff_chunk(b"INAM", &inam));
    info.extend_from_slice(&riff_chunk(b"IART", &iart));

    let mut body = b"WAVE".to_vec();
    body.extend_from_slice(&riff_chunk(b"fmt ", &fmt));
    body.extend_from_slice(&riff_chunk(b"data", &[0u8; 16]));
    body.extend_from_slice(&riff_chunk(b"LIST", &info));

    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&(body.len() as u32).to_le_bytes());
    file.extend_from_slice(&body);
    file
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

#[test]
fn reads_a_tagged_file_of_each_family() {
    let flac = read_tags(fixture("sine.flac")).unwrap();
    assert_eq!(flac.title.as_deref(), Some("Sine Title"));
    assert_eq!(flac.artist.as_deref(), Some("Test Artist"));
    assert_eq!(flac.album.as_deref(), Some("Test Album"));
    assert_eq!(flac.year, Some(2001));
    assert_eq!(flac.genre.as_deref(), Some("Test"));
    assert_eq!(flac.comment.as_deref(), Some("hello"));
    assert_eq!(flac.track_no, Some(3));
    assert_eq!(flac.track_total, None);

    let mp3 = read_tags(fixture("sine.mp3")).unwrap();
    assert_eq!(mp3.title.as_deref(), Some("Sine Title"));
    assert_eq!(mp3.artist.as_deref(), Some("Test Artist"));
    assert_eq!(mp3.year, Some(2001));
    assert_eq!(mp3.track_no, Some(3));
    assert_eq!(mp3.track_total, Some(9));

    let ogg = read_tags(fixture("sine.ogg")).unwrap();
    assert_eq!(ogg.title.as_deref(), Some("Sine Title"));
    assert_eq!(ogg.artist.as_deref(), Some("Test Artist"));

    // The MP4 fixtures carry no tags at all.
    for name in ["sine_aac.m4a", "sine_alac.m4a"] {
        let metadata = read_tags(fixture(name)).unwrap();
        assert!(metadata.is_empty(), "{name}: {metadata:?}");
    }
}

#[test]
fn absent_tags_yield_default_metadata_without_error() {
    let metadata = read_tags(fixture("sine_aac.m4a")).unwrap();
    assert_eq!(metadata, TrackMetadata::default());
}

#[test]
fn reads_riff_info_from_wav() {
    let dir = TempDir::new().unwrap();
    let path = write_file(
        &dir,
        "info.wav",
        &wav_with_info("Info Title", "Info Artist"),
    );

    let metadata = read_tags(&path).unwrap();
    assert_eq!(metadata.title.as_deref(), Some("Info Title"));
    assert_eq!(metadata.artist.as_deref(), Some("Info Artist"));
}

#[test]
fn reads_ape_tag() {
    // Append a footer-only APEv2 tag to a copy of the MP3 fixture. The ID3v2
    // tag has no comment, so the APE tag fills that field.
    let dir = TempDir::new().unwrap();
    let path = copy_fixture(&dir, "sine.mp3");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.extend_from_slice(&apev2_tag(&[("Comment", "Ape Comment")]));
    std::fs::write(&path, &bytes).unwrap();

    let metadata = read_tags(&path).unwrap();
    assert_eq!(metadata.comment.as_deref(), Some("Ape Comment"));
    // The ID3v2 tag still wins where both are present.
    assert_eq!(metadata.title.as_deref(), Some("Sine Title"));
}

#[test]
fn non_utf8_tag_bytes_never_panic() {
    // ID3v1 is Latin-1; every byte maps to a character, so a field full of
    // high bytes decodes without panicking and without being dropped.
    let dir = TempDir::new().unwrap();
    let mut bytes = vec![0u8; 64];
    let mut tag = vec![0u8; 128];
    tag[0..3].copy_from_slice(b"TAG");
    for (i, byte) in tag[3..33].iter_mut().enumerate() {
        *byte = 0x80 + (i as u8 % 0x80);
    }
    tag[127] = 255; // genre: none
    bytes.extend_from_slice(&tag);
    let path = write_file(&dir, "latin1.mp3", &bytes);

    let metadata = read_tags(&path).expect("Latin-1 tag must not panic");
    let title = metadata.title.expect("title decoded");
    assert_eq!(title.chars().count(), 30, "{title:?}");
}

#[test]
fn tags_are_read_without_any_audio() {
    // A file that contains nothing but an ID3v2 tag: there is no audio to
    // decode at all, yet the tags come back.
    let dir = TempDir::new().unwrap();
    let tag = id3v2_tag(&[id3v2_text_frame(b"TIT2", "No Audio Here")]);
    let path = write_file(&dir, "tag_only.mp3", &tag);

    let metadata = read_tags(&path).unwrap();
    assert_eq!(metadata.title.as_deref(), Some("No Audio Here"));

    let only_v1 = write_file(&dir, "v1_only.mp3", &id3v1_only_file("V1 Only"));
    assert_eq!(
        read_tags(&only_v1).unwrap().title.as_deref(),
        Some("V1 Only")
    );
}

#[test]
fn truncated_file_still_reads_tags() {
    // The FLAC metadata blocks (STREAMINFO, VORBIS_COMMENT) sit before the
    // audio frames, so cutting the audio off does not affect tag reading.
    let dir = TempDir::new().unwrap();
    let bytes = std::fs::read(fixture("sine.flac")).unwrap();
    let truncated = &bytes[..bytes.len() * 2 / 3];
    let path = write_file(&dir, "truncated.flac", truncated);

    let metadata = read_tags(&path).unwrap();
    assert_eq!(metadata.title.as_deref(), Some("Sine Title"));
}

#[test]
fn non_media_files_error_or_stay_default() {
    let dir = TempDir::new().unwrap();
    let random = write_file(&dir, "random.bin", &[0x37u8; 4096]);
    let text = write_file(&dir, "notes.txt", b"hello, this is not audio");
    let empty = write_file(&dir, "empty.mp3", &[]);
    let fake = write_file(&dir, "fake.mp3", &[0x11u8; 1024]);

    for path in [&random, &text, &empty, &fake] {
        if let Ok(metadata) = read_tags(path) {
            assert!(metadata.is_empty(), "{path:?}: {metadata:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Precedence and number forms
// ---------------------------------------------------------------------------

#[test]
fn id3v2_wins_over_id3v1_and_id3v1_fills_gaps() {
    let dir = TempDir::new().unwrap();
    let path = copy_fixture(&dir, "sine.mp3");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.extend_from_slice(&id3v1(
        "V1 Title",
        "V1 Artist",
        "V1 Album",
        "1999",
        "V1 Comment",
    ));
    std::fs::write(&path, &bytes).unwrap();

    let metadata = read_tags(&path).unwrap();
    // ID3v2 (the fixture) wins for every field it has.
    assert_eq!(metadata.title.as_deref(), Some("Sine Title"));
    assert_eq!(metadata.artist.as_deref(), Some("Test Artist"));
    assert_eq!(metadata.album.as_deref(), Some("Test Album"));
    assert_eq!(metadata.year, Some(2001));
    // ID3v1 fills what ID3v2 does not have.
    assert_eq!(metadata.comment.as_deref(), Some("V1 Comment"));
}

#[test]
fn track_number_accepts_the_pair_and_the_bare_form() {
    // The MP3 fixture stores ID3v2 `TRCK` as "3/9".
    let mp3 = read_tags(fixture("sine.mp3")).unwrap();
    assert_eq!((mp3.track_no, mp3.track_total), (Some(3), Some(9)));

    // The FLAC fixture stores a bare Vorbis `TRACKNUMBER`.
    let flac = read_tags(fixture("sine.flac")).unwrap();
    assert_eq!((flac.track_no, flac.track_total), (Some(3), None));
}

// ---------------------------------------------------------------------------
// Artwork
// ---------------------------------------------------------------------------

/// A PNG signature followed by junk: enough for the tag formats, which never
/// inspect the image beyond storing its MIME type.
const PNG_LIKE: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x01,
];

#[test]
fn artwork_is_exposed_with_unchanged_bytes() {
    let dir = TempDir::new().unwrap();
    let tag = id3v2_tag(&[
        id3v2_text_frame(b"TIT2", "Art Title"),
        id3v2_apic_frame("image/png", 3, "front cover", PNG_LIKE),
    ]);
    let path = write_file(&dir, "art.mp3", &tag);

    assert_eq!(
        read_tags(&path).unwrap().title.as_deref(),
        Some("Art Title")
    );

    let artwork = read_artwork(&path).unwrap();
    assert_eq!(
        artwork,
        vec![Artwork {
            kind: 3,
            mime_type: Some("image/png".into()),
            description: Some("front cover".into()),
            data: PNG_LIKE.to_vec(),
        }]
    );
}

#[test]
fn files_without_artwork_yield_an_empty_list() {
    assert!(read_artwork(fixture("sine.flac")).unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

#[test]
fn write_read_roundtrip_on_each_family() {
    let dir = TempDir::new().unwrap();
    for name in [
        "sine.flac",
        "sine.mp3",
        "sine.ogg",
        "sine_aac.m4a",
        "sine_alac.m4a",
    ] {
        let path = copy_fixture(&dir, name);
        let expected = full_metadata();

        write_tags(&path, &expected).unwrap();
        let got = read_tags(&path).unwrap();

        assert_eq!(got.title, expected.title, "{name}: title");
        assert_eq!(got.artist, expected.artist, "{name}: artist");
        assert_eq!(got.album, expected.album, "{name}: album");
        assert_eq!(
            got.album_artist, expected.album_artist,
            "{name}: album artist"
        );
        assert_eq!(got.year, expected.year, "{name}: year");
        assert_eq!(got.genre, expected.genre, "{name}: genre");
        assert_eq!(got.comment, expected.comment, "{name}: comment");
        assert_eq!(got.track_no, expected.track_no, "{name}: track no");
        assert_eq!(got.track_total, expected.track_total, "{name}: track total");
        assert_eq!(got.disc_no, expected.disc_no, "{name}: disc no");
        // The four classic ReplayGain keys round-trip in every family: ID3v2
        // holds them as TXXX frames, Vorbis comments and APEv2 as plain
        // keys, and MP4 ilst as freeform
        // `----:com.apple.iTunes:replaygain_*` atoms (written by ItemKey,
        // not by short key name — a short-name lookup finds nothing in the
        // ilst map).
        assert_eq!(
            got.extra.get("REPLAYGAIN_TRACK_GAIN"),
            expected.extra.get("REPLAYGAIN_TRACK_GAIN"),
            "{name}: replaygain"
        );
    }
}

#[test]
fn writing_to_a_file_without_tags_creates_them() {
    let dir = TempDir::new().unwrap();
    // The MP4 fixture is untagged to begin with.
    let path = copy_fixture(&dir, "sine_aac.m4a");
    assert!(read_tags(&path).unwrap().is_empty());

    let expected = TrackMetadata {
        title: Some("Fresh Title".into()),
        artist: Some("Fresh Artist".into()),
        ..TrackMetadata::default()
    };
    write_tags(&path, &expected).unwrap();

    let got = read_tags(&path).unwrap();
    assert_eq!(got.title.as_deref(), Some("Fresh Title"));
    assert_eq!(got.artist.as_deref(), Some("Fresh Artist"));
}

#[test]
fn writing_leaves_unset_fields_untouched() {
    let dir = TempDir::new().unwrap();
    let path = copy_fixture(&dir, "sine.flac");

    // Only change the album; everything else must survive.
    let patch = TrackMetadata {
        album: Some("Patched Album".into()),
        ..TrackMetadata::default()
    };
    write_tags(&path, &patch).unwrap();

    let got = read_tags(&path).unwrap();
    assert_eq!(got.album.as_deref(), Some("Patched Album"));
    assert_eq!(got.title.as_deref(), Some("Sine Title"));
    assert_eq!(got.artist.as_deref(), Some("Test Artist"));
    assert_eq!(got.track_no, Some(3));
}

#[test]
fn writing_creates_an_id3v2_chunk_in_wav() {
    let dir = TempDir::new().unwrap();
    let path = write_file(
        &dir,
        "info.wav",
        &wav_with_info("Info Title", "Info Artist"),
    );

    let mut metadata = read_tags(&path).unwrap();
    metadata.album = Some("Written Album".into());
    write_tags(&path, &metadata).unwrap();

    let got = read_tags(&path).unwrap();
    assert_eq!(got.album.as_deref(), Some("Written Album"));
    assert_eq!(got.title.as_deref(), Some("Info Title"));
    assert_eq!(got.artist.as_deref(), Some("Info Artist"));
}

#[test]
fn unicode_survives_a_roundtrip() {
    let dir = TempDir::new().unwrap();
    for name in ["sine.mp3", "sine.flac"] {
        let path = copy_fixture(&dir, name);
        let metadata = TrackMetadata {
            title: Some("Ünïcødé – 日本語 🎵".into()),
            artist: Some("Ævar Þór".into()),
            comment: Some("Ω≈ç√∫".into()),
            ..TrackMetadata::default()
        };

        write_tags(&path, &metadata).unwrap();
        let got = read_tags(&path).unwrap();
        assert_eq!(got.title, metadata.title, "{name}: title");
        assert_eq!(got.artist, metadata.artist, "{name}: artist");
        assert_eq!(got.comment, metadata.comment, "{name}: comment");
    }
}

// ---------------------------------------------------------------------------
// Batch scan
// ---------------------------------------------------------------------------

#[test]
fn batch_scan_reports_each_path_separately() {
    let dir = TempDir::new().unwrap();
    let good = fixture("sine.flac");
    let broken = PathBuf::from("this/path/does/not/exist.mp3");
    let text = write_file(&dir, "notes.txt", b"not audio");

    let results = scan_paths([good.clone(), broken.clone(), text.clone()]);
    assert_eq!(results.len(), 3);

    assert_eq!(results[0].0, good);
    assert_eq!(
        results[0].1.as_ref().unwrap().title.as_deref(),
        Some("Sine Title")
    );
    assert_eq!(results[1].0, broken);
    assert!(results[1].1.is_err());
    assert_eq!(results[2].0, text);
    assert!(results[2].1.is_err());
}

#[test]
fn batch_scan_is_usable_from_a_background_thread() {
    let paths: Vec<PathBuf> = ["sine.flac", "sine.mp3", "sine.ogg"]
        .iter()
        .map(|name| fixture(name))
        .collect();
    let handle = std::thread::spawn(move || scan_paths(paths));
    let results = handle.join().expect("scan thread must not panic");
    assert_eq!(results.len(), 3);
    for (path, result) in results {
        assert!(result.is_ok(), "{path:?}");
    }
}

// ---------------------------------------------------------------------------
// Probe list and timing
// ---------------------------------------------------------------------------

#[test]
fn extension_list_matches_the_reader() {
    for ext in [
        "mp3", "FLAC", "ogg", "m4a", "wav", "aiff", "opus", "wv", "ape",
    ] {
        assert!(supports_extension(ext), "{ext}");
    }
    assert!(!supports_extension("txt"));
    assert!(!supports_extension("exe"));
    for ext in supported_extensions() {
        assert!(supports_extension(ext), "{ext}");
    }
}

#[test]
fn tag_reads_are_fast() {
    let names = [
        "sine.flac",
        "sine.mp3",
        "sine.ogg",
        "sine_aac.m4a",
        "sine_alac.m4a",
    ];
    let start = Instant::now();
    for name in names {
        read_tags(fixture(name)).unwrap();
    }
    let elapsed = start.elapsed();
    println!("rustamp-meta: read tags of 5 fixtures in {elapsed:?}");
    // Generous bound; the point is that nothing scans or decodes the audio.
    assert!(
        elapsed < Duration::from_secs(5),
        "tag reads took {elapsed:?}"
    );
}

// ---------------------------------------------------------------------------
// Audit (task card E2): refusals are loud
// ---------------------------------------------------------------------------

#[test]
fn an_unrepresentable_year_is_refused_before_the_file_is_touched() {
    let dir = TempDir::new().unwrap();
    let path = copy_fixture(&dir, "sine.flac");
    let before = std::fs::read(&path).unwrap();

    let too_big = TrackMetadata {
        title: Some("Should Not Be Written".into()),
        year: Some(70_000),
        ..TrackMetadata::default()
    };
    let error = write_tags(&path, &too_big).unwrap_err();
    assert!(
        matches!(error, rustamp_core::Error::InvalidArgument(_)),
        "{error:?}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before, "file was modified");

    // The largest four-digit year still goes through and reads back.
    let largest = TrackMetadata {
        year: Some(9999),
        ..TrackMetadata::default()
    };
    write_tags(&path, &largest).unwrap();
    assert_eq!(read_tags(&path).unwrap().year, Some(9999));
}

#[test]
fn missing_paths_are_errors_for_every_entry_point() {
    let dir = TempDir::new().unwrap();
    let missing = dir.path().join("missing.mp3");
    assert!(read_tags(&missing).is_err());
    assert!(read_artwork(&missing).is_err());
    assert!(write_tags(&missing, &full_metadata()).is_err());
    assert!(!missing.exists(), "a failed write must not create the file");
}

#[test]
fn a_file_that_is_not_media_is_never_written_to() {
    let dir = TempDir::new().unwrap();
    let notes = write_file(&dir, "notes.txt", b"hello, this is not audio");
    assert!(write_tags(&notes, &full_metadata()).is_err());
    assert_eq!(std::fs::read(&notes).unwrap(), b"hello, this is not audio");
}

// ---------------------------------------------------------------------------
// ReplayGain (task card E4)
// ---------------------------------------------------------------------------

/// A ReplayGain set that exercises every carried key.
fn full_replaygain() -> ReplayGainTags {
    ReplayGainTags {
        track_gain_db: Some(-3.5),
        track_peak: Some(0.987654),
        album_gain_db: Some(-5.1),
        album_peak: Some(0.999),
        r128_track_gain_q8: Some(-1152),
        r128_album_gain_q8: Some(-1280),
    }
}

#[test]
fn classic_replaygain_keys_roundtrip_on_every_family() {
    let dir = TempDir::new().unwrap();
    for name in [
        "sine.flac",
        "sine.mp3",
        "sine.ogg",
        "sine_aac.m4a",
        "sine_alac.m4a",
    ] {
        let path = copy_fixture(&dir, name);
        let expected = ReplayGainTags {
            track_gain_db: Some(-3.5),
            track_peak: Some(0.987654),
            album_gain_db: Some(-5.1),
            album_peak: Some(0.999),
            r128_track_gain_q8: None,
            r128_album_gain_q8: None,
        };

        let report = write_replaygain(&path, &expected).unwrap();
        assert!(
            report.is_complete(),
            "{name}: skipped {:?}",
            report.skipped_unsupported
        );

        let got = ReplayGainTags::from_metadata(&read_tags(&path).unwrap());
        assert_eq!(got, expected, "{name}");
    }
}

#[test]
fn r128_keys_roundtrip_on_vorbis_and_are_refused_loudly_elsewhere() {
    let dir = TempDir::new().unwrap();

    for name in ["sine.flac", "sine.ogg"] {
        let path = copy_fixture(&dir, name);
        let tags = ReplayGainTags {
            r128_track_gain_q8: Some(-1152),
            r128_album_gain_q8: Some(-1280),
            ..ReplayGainTags::default()
        };
        let report = write_replaygain(&path, &tags).unwrap();
        assert!(
            report.is_complete(),
            "{name}: skipped {:?}",
            report.skipped_unsupported
        );
        assert_eq!(
            ReplayGainTags::from_metadata(&read_tags(&path).unwrap()),
            tags,
            "{name}"
        );
    }

    // ID3v2 and MP4 ilst have no field for R128: the write lands the classic
    // keys and reports the R128 ones instead of dropping them silently.
    for name in ["sine.mp3", "sine_aac.m4a"] {
        let path = copy_fixture(&dir, name);
        let report = write_replaygain(&path, &full_replaygain()).unwrap();
        assert!(!report.is_complete(), "{name}");
        assert_eq!(
            report.skipped_unsupported,
            vec![
                R128_TRACK_GAIN_KEY.to_owned(),
                R128_ALBUM_GAIN_KEY.to_owned()
            ],
            "{name}"
        );
        // The classic keys still landed.
        let got = ReplayGainTags::from_metadata(&read_tags(&path).unwrap());
        assert_eq!(got.track_gain_db, Some(-3.5), "{name}");
        assert!(got.r128_track_gain_q8.is_none(), "{name}");
    }
}

#[test]
fn reference_loudness_is_reported_as_skipped_not_dropped() {
    // No tag format lofty 0.25.4 supports carries this key, so it is always
    // reported — on every family — while the rest of the write still lands.
    let dir = TempDir::new().unwrap();
    for name in ["sine.flac", "sine.mp3", "sine_aac.m4a"] {
        let path = copy_fixture(&dir, name);
        let mut metadata = TrackMetadata {
            title: Some("Loudness Title".into()),
            ..TrackMetadata::default()
        };
        metadata
            .extra
            .insert(REFERENCE_LOUDNESS_KEY.into(), "89 dB SPL".into());

        let report = write_tags_report(&path, &metadata).unwrap();
        assert_eq!(
            report.unwritten_extra,
            vec![REFERENCE_LOUDNESS_KEY.to_owned()],
            "{name}"
        );

        let got = read_tags(&path).unwrap();
        assert_eq!(got.title.as_deref(), Some("Loudness Title"), "{name}");
    }
}

#[test]
fn writing_replaygain_leaves_other_tags_untouched() {
    let dir = TempDir::new().unwrap();
    let path = copy_fixture(&dir, "sine.mp3");
    let before = read_tags(&path).unwrap();

    let tags = ReplayGainTags {
        track_gain_db: Some(-3.5),
        track_peak: Some(0.987654),
        ..ReplayGainTags::default()
    };
    let report = write_replaygain(&path, &tags).unwrap();
    assert!(report.is_complete());
    assert_eq!(
        report.written,
        vec![
            "REPLAYGAIN_TRACK_GAIN".to_owned(),
            "REPLAYGAIN_TRACK_PEAK".to_owned()
        ]
    );

    let got = read_tags(&path).unwrap();
    assert_eq!(got.title, before.title);
    assert_eq!(got.artist, before.artist);
    assert_eq!(got.album, before.album);
    assert_eq!(got.track_no, before.track_no);
    assert_eq!(got.track_total, before.track_total);
    let replaygain = ReplayGainTags::from_metadata(&got);
    assert_eq!(replaygain.track_gain_db, Some(-3.5));
    assert_eq!(replaygain.track_peak, Some(0.987654));
    assert!(replaygain.album_gain_db.is_none());
}

#[test]
fn album_keys_and_full_metadata_share_one_roundtrip() {
    // Album gain/peak travel through the same path; check them together with
    // the plain `write_tags` entry point on an MP4 file (the family that used
    // to lose these keys).
    let dir = TempDir::new().unwrap();
    let path = copy_fixture(&dir, "sine_alac.m4a");
    let mut metadata = TrackMetadata::default();
    metadata
        .extra
        .insert(ALBUM_GAIN_KEY.into(), "-5.10 dB".into());
    metadata
        .extra
        .insert(ALBUM_PEAK_KEY.into(), "0.999000".into());

    let report = write_tags_report(&path, &metadata).unwrap();
    assert!(report.is_complete());

    let got = read_tags(&path).unwrap();
    assert_eq!(
        got.extra.get(ALBUM_GAIN_KEY).map(String::as_str),
        Some("-5.10 dB")
    );
    assert_eq!(
        got.extra.get(ALBUM_PEAK_KEY).map(String::as_str),
        Some("0.999000")
    );
}
