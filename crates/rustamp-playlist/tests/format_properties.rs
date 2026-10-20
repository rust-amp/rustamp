//! Property tests for the playlist file formats.
//!
//! The invariants are derived from the crate's contract
//! (`crates/rustamp-playlist/CONTRACT_NOTES.md` §3 and §4) and are stated once
//! per test:
//!
//! * **R1 — round trip**: `parse(write(entries, f))` yields the entries again,
//!   for every format, with exactly the losses the contract documents.
//! * **R2 — detection**: `detect(write(entries, f), hint) == f`. The two M3U
//!   encodings are byte-identical, so only the file name tells them apart.
//! * **R3 — preamble**: a long comment in front of the file changes neither
//!   the detected format nor the entries (the wave132 fix: the marker search
//!   covers the whole file).
//! * **M1 — malformed input never panics**: random bytes, truncated prefixes
//!   of a valid file and markers at wrong offsets are answered with entries
//!   or with an error, never with a panic.
//! * **M2 — a contradicting format is refused**: reading explicitly as `f`
//!   fails when the content carries another format's marker.
//! * **W1 — writing is a fixed point**: writing the parsed entries again
//!   yields the same bytes.
//! * **W2 — the file converges**: after one write → read → write round the
//!   bytes do not change any more.
//! * **D1 — the readers are deterministic**: the same bytes always give the
//!   same report.
//!
//! Two instabilities that the contract already documents are pinned at the
//! end of the file, so they cannot change silently.
//!
//! The generator is the crate's own seeded [`Rng`]; there is no new
//! dependency. Every seed is fixed, so a failure is reproducible.

use std::path::PathBuf;

use rustamp_core::playlist::MediaLocation;
use rustamp_playlist::{
    LoadedEntry, PlaylistError, PlaylistFormat, Rng, detect, import_playlist_path, read_playlist,
    read_playlist_as, read_playlist_path, read_playlist_report_as, sniff, write_playlist,
};

/// The seed of every property in this file.
const SEED: u64 = 0x2460_0BAD_F00D_0001;

/// The formats this crate can read and write. B4S is recognized but not
/// implemented, so it is not part of a round trip.
const SUPPORTED: [PlaylistFormat; 4] = [
    PlaylistFormat::M3u,
    PlaylistFormat::M3u8,
    PlaylistFormat::Pls,
    PlaylistFormat::Xspf,
];

/// Characters a location may contain: no separator, no control character, no
/// character that a reader trims. The characters that need escaping in one
/// format or the other (`#`, `%`, `&`, `<`, `>`, `=`) are in on purpose.
const NAME_CHARS: &[char] = &[
    'a', 'b', 'c', 'z', 'A', 'Z', '0', '9', ' ', '#', '%', '&', '<', '>', '=', ';', '\'', '(', ')',
    ',', '.', '-', '_', '+', '[', ']', '!', '@', '$', '^', '~', '\u{00E4}', '\u{00F6}', '\u{20AC}',
];

/// Characters a title may contain. The comma and the equal sign are the
/// interesting ones: they are the separators of the formats.
const TITLE_CHARS: &[char] = &[
    'a', 'B', 'c', '0', '1', ' ', ',', '.', ':', ';', '-', '_', '&', '<', '>', '"', '\'', '(', ')',
    '[', ']', '!', '?', '%', '#', '=', '+', '\u{00E4}', '\u{00FC}', '\u{20AC}',
];

/// The content markers of the formats, plus the B4S root element.
const MARKERS: &[&str] = &["#EXTM3U", "[playlist]", "File1=", "<playlist", "<winamp"];

/// Bytes that reach deep into the parsers: markup characters, whitespace and
/// the letters the formats use.
const ALPHABET: &[u8] = b"<>/=\"'&;#[]()!%\\ \t\r\nabc0123playlisttracklocationtitleEXTM3UFile";

// ---------------------------------------------------------------------------
// the generator
// ---------------------------------------------------------------------------

/// The directory the playlists are read against.
fn base() -> PathBuf {
    PathBuf::from("C:/Music/Lists")
}

/// The file a playlist of this format is read from. Its name carries the
/// format hint.
fn path_of(format: PlaylistFormat) -> PathBuf {
    base().join(format!("list.{}", format.extension()))
}

/// The spelling of a path after the readers' separator normalization.
///
/// The readers turn `/` into `\` on Windows and `\` into `/` everywhere else.
/// A location written in the other spelling comes back changed, so the
/// generator produces the spelling the reader hands back.
fn normalized(path: &str) -> String {
    if cfg!(windows) {
        path.replace('/', "\\")
    } else {
        path.replace('\\', "/")
    }
}

/// A random string of `chars`, trimmed and never empty.
fn random_text(rng: &mut Rng, chars: &[char], min: usize, max: usize) -> String {
    let len = min + rng.below(max - min + 1);
    let text: String = (0..len).map(|_| chars[rng.below(chars.len())]).collect();
    let trimmed = text.trim();
    if trimmed.is_empty() {
        "x".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// What the generator has produced so far.
///
/// The properties are worthless if the generator stops producing the shapes
/// they are about, so every property checks its coverage at the end.
#[derive(Debug, Default)]
struct Coverage {
    file_location: bool,
    url_location: bool,
    with_title: bool,
    with_duration: bool,
    without_metadata: bool,
    padded_title: bool,
    empty_title: bool,
    title_with_line_break: bool,
}

/// A random location: an absolute file path or a URL.
fn random_location(rng: &mut Rng, coverage: &mut Coverage) -> MediaLocation {
    if rng.below(4) == 0 {
        coverage.url_location = true;
        MediaLocation::Url(format!(
            "http://example.com/{}",
            random_text(rng, NAME_CHARS, 1, 12)
        ))
    } else {
        coverage.file_location = true;
        MediaLocation::File(PathBuf::from(normalized(&format!(
            "C:/Music/Lists/{}.mp3",
            random_text(rng, NAME_CHARS, 1, 12)
        ))))
    }
}

/// An entry in canonical form: a trimmed, non-empty title (or no title at
/// all), a duration in whole milliseconds and an absolute location.
///
/// This is the form the contract calls "resolved"; a writer stores it without
/// changing it, so such a playlist is a fixed point of write → read → write.
fn random_entry(rng: &mut Rng, coverage: &mut Coverage) -> LoadedEntry {
    let mut entry = LoadedEntry::new(random_location(rng, coverage));
    entry.title = if rng.below(3) == 0 {
        None
    } else {
        coverage.with_title = true;
        Some(random_text(rng, TITLE_CHARS, 1, 24))
    };
    entry.duration_ms = match rng.below(3) {
        0 => None,
        // Whole seconds.
        1 => {
            coverage.with_duration = true;
            Some(rng.below(86_400_000) as u64)
        }
        // Any number of milliseconds, well below the 2^53 ms at which the
        // seconds-based formats stop being exact.
        _ => {
            coverage.with_duration = true;
            Some(rng.below(1_000_000_000_000) as u64)
        }
    };
    if entry.title.is_none() && entry.duration_ms.is_none() {
        coverage.without_metadata = true;
    }
    entry
}

/// A random list of canonical entries.
fn random_entries(rng: &mut Rng, count: usize, coverage: &mut Coverage) -> Vec<LoadedEntry> {
    (0..count).map(|_| random_entry(rng, coverage)).collect()
}

/// An entry whose title is deliberately *not* canonical: padded, empty or
/// with a line break in it.
fn lossy_entry(rng: &mut Rng, coverage: &mut Coverage) -> LoadedEntry {
    let mut entry = random_entry(rng, coverage);
    let title = random_text(rng, TITLE_CHARS, 1, 12);
    match rng.below(4) {
        0 => {
            coverage.padded_title = true;
            entry.title = Some(format!("  {title}  "));
        }
        1 => {
            coverage.empty_title = true;
            entry.title = Some(" ".repeat(1 + rng.below(3)));
        }
        2 => {
            coverage.title_with_line_break = true;
            entry.title = Some(format!("{title}\r\n{title}"));
        }
        _ => {}
    }
    entry
}

/// A random list of entries whose titles are not canonical.
fn lossy_entries(rng: &mut Rng, count: usize, coverage: &mut Coverage) -> Vec<LoadedEntry> {
    (0..count).map(|_| lossy_entry(rng, coverage)).collect()
}

/// An entry that always carries a title and a duration.
///
/// Every format writes a marker for such an entry (`#EXTM3U`, `[playlist]`,
/// `<playlist>`), so a file built from these entries always has one.
fn marked_entry(rng: &mut Rng) -> LoadedEntry {
    let mut entry = LoadedEntry::new(MediaLocation::File(PathBuf::from(normalized(
        "C:/Music/Lists/marked.mp3",
    ))));
    entry.title = Some(random_text(rng, TITLE_CHARS, 1, 24));
    entry.duration_ms = Some(rng.below(1_000_000_000_000) as u64);
    entry
}

// ---------------------------------------------------------------------------
// what a round trip must produce
// ---------------------------------------------------------------------------

/// The title a round trip must yield.
///
/// Every reader trims a title, an empty title becomes no title at all, and
/// the line-based formats cannot carry a line break: there it becomes a
/// space. XSPF escapes a line break, so it keeps it (`CONTRACT_NOTES.md` §4).
fn expected_title(format: PlaylistFormat, title: Option<&str>) -> Option<String> {
    let title = title?.trim();
    if title.is_empty() {
        return None;
    }
    if format == PlaylistFormat::Xspf {
        return Some(title.to_owned());
    }
    Some(title.replace(['\r', '\n'], " ").trim().to_owned())
}

/// The entries a round trip must yield.
fn expected_after_round_trip(format: PlaylistFormat, entries: &[LoadedEntry]) -> Vec<LoadedEntry> {
    entries
        .iter()
        .map(|entry| LoadedEntry {
            location: entry.location.clone(),
            title: expected_title(format, entry.title.as_deref()),
            duration_ms: entry.duration_ms,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// the readers never panic
// ---------------------------------------------------------------------------

/// Asserts that every entry a reader invents has a location.
fn assert_sane_entries(entries: &[LoadedEntry], context: &str, format: PlaylistFormat) {
    for entry in entries {
        assert!(
            !entry.location.to_display_string().is_empty(),
            "{context}: {format} invented an entry without a location"
        );
    }
}

/// Asserts that every entry point answers `bytes` with entries or with an
/// error, and never with a panic.
fn assert_readers_are_total(bytes: &[u8], context: &str) {
    for format in SUPPORTED {
        match read_playlist_as(format, bytes, &base()) {
            Ok(entries) => assert_sane_entries(&entries, context, format),
            Err(PlaylistError::UnsupportedFormat(_)) => {}
            Err(other) => panic!("{context}: {format} failed with {other}"),
        }
    }
    for name in ["list.m3u", "list.m3u8", "list.pls", "list.xspf"] {
        let path = base().join(name);
        match read_playlist_path(bytes, &path) {
            Ok(entries) => assert_sane_entries(&entries, context, PlaylistFormat::M3u),
            Err(PlaylistError::UnsupportedFormat(_)) => {}
            Err(other) => panic!("{context}: {name} failed with {other}"),
        }
    }
    match read_playlist(bytes, &base()) {
        Ok(entries) => assert_sane_entries(&entries, context, PlaylistFormat::M3u),
        Err(PlaylistError::UnsupportedFormat(_)) => {}
        Err(other) => panic!("{context}: detection failed with {other}"),
    }
}

// ---------------------------------------------------------------------------
// R1 — the round trip
// ---------------------------------------------------------------------------

#[test]
fn a_written_playlist_reads_back_as_the_same_entries() {
    let mut rng = Rng::new(SEED);
    let mut coverage = Coverage::default();
    for round in 0..200 {
        let count = rng.below(24);
        let entries = random_entries(&mut rng, count, &mut coverage);
        for format in SUPPORTED {
            let bytes = write_playlist(format, &entries).expect("writing never fails");
            let back = read_playlist_path(&bytes, &path_of(format)).expect("reading never fails");
            assert_eq!(
                back,
                expected_after_round_trip(format, &entries),
                "round {round}, format {format}"
            );
        }
    }
    assert!(coverage.file_location && coverage.url_location);
    assert!(coverage.with_title && coverage.with_duration);
    assert!(coverage.without_metadata);
}

#[test]
fn a_round_trip_loses_exactly_what_the_contract_documents() {
    // The same property, but with titles that are padded, empty or carry a
    // line break: the round trip must lose exactly those and nothing else.
    let mut rng = Rng::new(SEED ^ 0x11);
    let mut coverage = Coverage::default();
    for round in 0..200 {
        let count = rng.below(24);
        let entries = lossy_entries(&mut rng, count, &mut coverage);
        for format in SUPPORTED {
            let bytes = write_playlist(format, &entries).expect("writing never fails");
            let back = read_playlist_path(&bytes, &path_of(format)).expect("reading never fails");
            assert_eq!(
                back,
                expected_after_round_trip(format, &entries),
                "round {round}, format {format}"
            );
        }
    }
    assert!(coverage.padded_title);
    assert!(coverage.empty_title);
    assert!(coverage.title_with_line_break);
}

// ---------------------------------------------------------------------------
// R2 — the detection
// ---------------------------------------------------------------------------

#[test]
fn a_written_playlist_is_detected_as_the_format_it_was_written_in() {
    let mut rng = Rng::new(SEED ^ 0x22);
    let mut coverage = Coverage::default();
    for round in 0..200 {
        let count = rng.below(24);
        let entries = random_entries(&mut rng, count, &mut coverage);
        for format in SUPPORTED {
            let bytes = write_playlist(format, &entries).expect("writing never fails");
            let path = path_of(format);
            // The file name tells the two M3U encodings apart.
            assert_eq!(
                detect(&bytes, Some(path.as_path())),
                Some(format),
                "round {round}, format {format}"
            );
            // The import reports the same format.
            let imported = import_playlist_path(&bytes, &path).expect("importing never fails");
            assert_eq!(imported.format, format, "round {round}, format {format}");
            // Without a file name the content decides. The two M3U encodings
            // are byte-identical, so both are read as M3U, and an empty file
            // is not a playlist at all.
            let expected: Option<PlaylistFormat> = if bytes.is_empty() {
                None
            } else if format == PlaylistFormat::M3u8 {
                Some(PlaylistFormat::M3u)
            } else {
                Some(format)
            };
            assert_eq!(
                detect(&bytes, None),
                expected,
                "round {round}, format {format}"
            );
        }
    }
    assert!(coverage.with_title && coverage.with_duration);
    assert!(coverage.without_metadata);
}

// ---------------------------------------------------------------------------
// R3 — a long comment in front of the file
// ---------------------------------------------------------------------------

/// A long comment in front of the file, in the comment syntax of the format.
fn preamble(format: PlaylistFormat, lines: usize) -> Vec<u8> {
    match format {
        PlaylistFormat::M3u | PlaylistFormat::M3u8 => {
            let mut out = Vec::new();
            for i in 0..lines {
                out.extend_from_slice(format!("# filler {i}\r\n").as_bytes());
            }
            out
        }
        PlaylistFormat::Pls => {
            let mut out = Vec::new();
            for i in 0..lines {
                out.extend_from_slice(format!("; filler {i}\r\n").as_bytes());
            }
            out
        }
        PlaylistFormat::Xspf => {
            let mut out = Vec::from("<!--".as_bytes());
            out.extend(std::iter::repeat_n(b'x', lines * 16));
            out.extend_from_slice(b"-->");
            out
        }
        PlaylistFormat::B4s => Vec::new(),
    }
}

#[test]
fn a_marker_behind_a_long_preamble_is_still_found() {
    // The marker search covers the whole file, not only its first kilobyte
    // (the wave132 fix). The entries carry a title and a duration, so every
    // format writes a marker.
    let mut rng = Rng::new(SEED ^ 0x33);
    for format in SUPPORTED {
        let entries: Vec<_> = (0..4).map(|_| marked_entry(&mut rng)).collect();
        let bytes = write_playlist(format, &entries).expect("writing never fails");
        assert!(sniff(&bytes).is_some(), "{format} must write a marker");

        let mut with_preamble = preamble(format, 400);
        with_preamble.extend_from_slice(&bytes);
        assert!(
            with_preamble.len() > 4096,
            "the preamble must reach beyond the first kilobyte"
        );
        assert_eq!(
            sniff(&with_preamble),
            sniff(&bytes),
            "the preamble must not hide the marker of a {format}"
        );
        assert!(
            sniff(&with_preamble).is_some(),
            "the marker of a {format} must be found behind the preamble"
        );
    }
}

#[test]
fn a_long_comment_in_front_of_the_file_does_not_hide_the_format() {
    let mut rng = Rng::new(SEED ^ 0x34);
    let mut coverage = Coverage::default();
    for round in 0..60 {
        let count = rng.below(20);
        let entries = random_entries(&mut rng, count, &mut coverage);
        for format in SUPPORTED {
            let bytes = write_playlist(format, &entries).expect("writing never fails");
            let mut with_preamble = preamble(format, 400);
            with_preamble.extend_from_slice(&bytes);
            assert!(
                with_preamble.len() > 4096,
                "the preamble must reach beyond the first kilobyte"
            );
            // The format is still recognized ...
            let path = path_of(format);
            assert_eq!(
                detect(&with_preamble, Some(path.as_path())),
                Some(format),
                "round {round}, format {format}"
            );
            // ... and the entries are still the same.
            let back = read_playlist_path(&with_preamble, &path).expect("reading never fails");
            assert_eq!(
                back,
                expected_after_round_trip(format, &entries),
                "round {round}, format {format}"
            );
        }
    }
    assert!(coverage.with_title && coverage.with_duration);
    assert!(coverage.without_metadata);
}

// ---------------------------------------------------------------------------
// M1 — malformed input never panics
// ---------------------------------------------------------------------------

#[test]
fn random_bytes_never_panic_a_reader() {
    let mut rng = Rng::new(SEED ^ 0x44);
    for round in 0..1500 {
        let len = rng.below(300);
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            bytes.push((rng.next_u64() >> 24) as u8);
        }
        assert_readers_are_total(&bytes, &format!("random bytes, round {round}"));
    }
}

#[test]
fn random_bytes_with_a_marker_in_them_never_panic_a_reader() {
    // Uniform bytes are mostly refused before a parser runs. Bytes that carry
    // a marker reach the parser, so they are generated here.
    let mut rng = Rng::new(SEED ^ 0x55);
    for round in 0..1500 {
        let len = 1 + rng.below(300);
        let mut bytes = Vec::with_capacity(len + MARKERS[0].len());
        for _ in 0..len {
            bytes.push(ALPHABET[rng.below(ALPHABET.len())]);
        }
        let marker = MARKERS[rng.below(MARKERS.len())];
        let offset = rng.below(bytes.len() + 1);
        bytes.splice(offset..offset, marker.bytes());
        assert_readers_are_total(&bytes, &format!("marked bytes, round {round}"));
    }
}

#[test]
fn a_truncated_playlist_never_panics_and_never_invents_entries() {
    let mut rng = Rng::new(SEED ^ 0x66);
    let mut coverage = Coverage::default();
    for round in 0..8 {
        let count = 1 + rng.below(4);
        let entries = random_entries(&mut rng, count, &mut coverage);
        for format in SUPPORTED {
            let bytes = write_playlist(format, &entries).expect("writing never fails");
            for cut in 0..=bytes.len() {
                let prefix = &bytes[..cut];
                let context = format!("truncated {format}, round {round}, cut {cut}");
                assert_readers_are_total(prefix, &context);
                // A prefix never yields more entries than the whole file.
                if let Ok(back) = read_playlist_as(format, prefix, &base()) {
                    assert!(
                        back.len() <= entries.len(),
                        "{context}: {} entries out of {}",
                        back.len(),
                        entries.len()
                    );
                }
            }
        }
    }
    assert!(coverage.file_location && coverage.url_location);
    assert!(coverage.with_title && coverage.with_duration);
}

// ---------------------------------------------------------------------------
// M2 — a contradicting format is refused
// ---------------------------------------------------------------------------

#[test]
fn a_marker_at_a_wrong_offset_is_never_misread_as_the_claimed_format() {
    let mut rng = Rng::new(SEED ^ 0x77);
    for round in 0..1500 {
        let len = 1 + rng.below(200);
        let mut bytes = Vec::with_capacity(len + MARKERS[0].len());
        for _ in 0..len {
            bytes.push(ALPHABET[rng.below(ALPHABET.len())]);
        }
        let marker = MARKERS[rng.below(MARKERS.len())];
        // Half of the time the marker is the first thing in the file: an XML
        // root is only recognized at the beginning of the document.
        let offset = if rng.below(2) == 0 {
            0
        } else {
            rng.below(bytes.len() + 1)
        };
        bytes.splice(offset..offset, marker.bytes());

        let context = format!("marker `{marker}` at offset {offset}, round {round}");
        assert_readers_are_total(&bytes, &context);

        // A file that carries another format's marker must not be read as the
        // claimed format.
        if let Some(found) = sniff(&bytes) {
            for format in SUPPORTED {
                let compatible =
                    found == format || (found.is_m3u_family() && format.is_m3u_family());
                if compatible {
                    continue;
                }
                let result = read_playlist_as(format, &bytes, &base());
                assert!(
                    matches!(result, Err(PlaylistError::UnsupportedFormat(_))),
                    "{context}: reading it as {format} must be refused, it is {found}"
                );
            }
        }
    }
}

#[test]
fn b4s_content_is_refused_by_every_supported_format() {
    // The B4S dialect is recognized but not implemented (CONTRACT_NOTES.md
    // §1). It must never be read as one of the other formats, not even when
    // the format is claimed explicitly.
    let b4s = b"<WinampXML><playlist><entry/></playlist></WinampXML>";
    for format in SUPPORTED {
        let result = read_playlist_as(format, b4s, &base());
        assert!(
            matches!(result, Err(PlaylistError::UnsupportedFormat(_))),
            "a B4S document must be refused as {format}"
        );
    }
    assert_eq!(sniff(b4s), Some(PlaylistFormat::B4s));
}

// ---------------------------------------------------------------------------
// W1 and W2 — writing is stable
// ---------------------------------------------------------------------------

#[test]
fn writing_the_parsed_entries_again_yields_the_same_bytes() {
    let mut rng = Rng::new(SEED ^ 0x88);
    let mut coverage = Coverage::default();
    for round in 0..200 {
        let count = rng.below(24);
        let entries = random_entries(&mut rng, count, &mut coverage);
        for format in SUPPORTED {
            let first = write_playlist(format, &entries).expect("writing never fails");
            let parsed = read_playlist_path(&first, &path_of(format)).expect("reading never fails");
            let second = write_playlist(format, &parsed).expect("writing never fails");
            assert_eq!(first, second, "round {round}, format {format}");
        }
    }
    assert!(coverage.file_location && coverage.url_location);
    assert!(coverage.with_title && coverage.with_duration);
    assert!(coverage.without_metadata);
}

#[test]
fn a_playlist_converges_after_one_write_read_write_round() {
    // A title that is not canonical (padded, empty or with a line break) is
    // normalized by the first round. From then on the bytes do not change.
    let mut rng = Rng::new(SEED ^ 0x99);
    let mut coverage = Coverage::default();
    for round in 0..200 {
        let count = rng.below(24);
        let entries = lossy_entries(&mut rng, count, &mut coverage);
        for format in SUPPORTED {
            let path = path_of(format);
            let first = write_playlist(format, &entries).expect("writing never fails");
            let parsed = read_playlist_path(&first, &path).expect("reading never fails");
            let second = write_playlist(format, &parsed).expect("writing never fails");
            let parsed_again = read_playlist_path(&second, &path).expect("reading never fails");
            let third = write_playlist(format, &parsed_again).expect("writing never fails");
            assert_eq!(second, third, "round {round}, format {format}");
        }
    }
    assert!(coverage.padded_title);
    assert!(coverage.empty_title);
    assert!(coverage.title_with_line_break);
}

// ---------------------------------------------------------------------------
// D1 — the readers are deterministic
// ---------------------------------------------------------------------------

#[test]
fn reading_the_same_bytes_twice_gives_the_same_result() {
    let mut rng = Rng::new(SEED ^ 0xAA);
    for round in 0..300 {
        let len = rng.below(200);
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            bytes.push(ALPHABET[rng.below(ALPHABET.len())]);
        }
        for format in SUPPORTED {
            let first = read_playlist_report_as(format, &bytes, &base());
            let second = read_playlist_report_as(format, &bytes, &base());
            match (first, second) {
                (Ok(a), Ok(b)) => assert_eq!(a, b, "round {round}, format {format}"),
                (Err(a), Err(b)) => assert_eq!(a.to_string(), b.to_string()),
                (a, b) => panic!("round {round}, format {format}: {a:?} vs {b:?}"),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Two documented instabilities
// ---------------------------------------------------------------------------

#[test]
fn an_entry_with_an_empty_title_loses_its_extinf_line_when_it_is_written_again() {
    // An entry with an empty title and no duration is written with an
    // `#EXTINF` line (`#EXTINF:-1,`), but the reader turns the empty title
    // into "no title" (CONTRACT_NOTES.md §4). The entry then has neither a
    // title nor a duration, so the second write drops the line. The file
    // converges after one round; it is not byte-identical to the first one.
    let location = normalized("C:/Music/Lists/a.mp3");
    let entries = vec![LoadedEntry {
        location: MediaLocation::File(PathBuf::from(&location)),
        title: Some(String::new()),
        duration_ms: None,
    }];
    let first = write_playlist(PlaylistFormat::M3u, &entries).expect("writing never fails");
    assert_eq!(
        first,
        format!("#EXTM3U\r\n#EXTINF:-1,\r\n{location}\r\n").as_bytes()
    );

    let parsed =
        read_playlist_path(&first, &path_of(PlaylistFormat::M3u)).expect("reading never fails");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].title, None);
    assert_eq!(parsed[0].duration_ms, None);

    let second = write_playlist(PlaylistFormat::M3u, &parsed).expect("writing never fails");
    assert_eq!(second, format!("{location}\r\n").as_bytes());
}

#[test]
fn a_relative_location_that_looks_like_a_pls_key_is_read_as_a_pls() {
    // A `FileN=` line and a `[playlist]` section are the PLS markers, and a
    // content marker wins over the file name (CONTRACT_NOTES.md §3). A
    // *relative* location that looks like one is therefore read as a PLS and
    // the entry comes back changed. The contract asks the caller to resolve
    // the locations before writing (`LoadedEntry::location` is documented as
    // resolved), so this is the documented behaviour of an input the caller
    // must not produce - pinned here so it cannot change silently.
    let entries = vec![LoadedEntry::new(MediaLocation::File(PathBuf::from(
        "File1=x.mp3",
    )))];
    let bytes = write_playlist(PlaylistFormat::M3u, &entries).expect("writing never fails");
    assert_eq!(bytes, b"File1=x.mp3\r\n");

    let path = path_of(PlaylistFormat::M3u);
    assert_eq!(
        detect(&bytes, Some(path.as_path())),
        Some(PlaylistFormat::Pls)
    );

    let back = read_playlist_path(&bytes, &path).expect("reading never fails");
    assert_eq!(back.len(), 1);
    assert_eq!(
        back[0].location,
        MediaLocation::File(PathBuf::from(normalized("C:/Music/Lists/x.mp3"))),
        "the `File1=` prefix is gone: the file was read as a PLS"
    );
}
