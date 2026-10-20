//! Property tests for the no-op rule of the tag writer.
//!
//! A write that would not change the tags the file has does not touch the
//! file: its bytes and its modification time stay the ones it had, and the
//! report says so ([`WriteReport::unchanged`]). The rule is subtle — a field
//! that is `None` means "keep", a value is compared as a reader sees it, a
//! second tag of the file does not count — and the example tests of
//! `tests/unchanged.rs` only show a handful of the cases.
//!
//! The requests here are generated from fixed seeds, so every failure can be
//! replayed: the seed and the case number are part of the message.
//!
//! # The rule the properties state
//!
//! * **A write is a no-op exactly when nothing would change.** The report
//!   says `unchanged` exactly when the request would leave the values a
//!   reader sees as they are. A no-op leaves the bytes and the modification
//!   time of the file alone; a change rewrites the file — and the values a
//!   reader sees are then not the ones the file had.
//! * **Naming a field decides.** A request that names a field with a value
//!   the file does not have is never a no-op, whatever the other fields say.
//! * **A removal of a present field is a change, of an absent one is not.**
//! * **A removal wins over a value.** A field that is in both the metadata
//!   and the removal list is removed, so the value of such a field cannot
//!   decide anything.
//! * **An `extra` entry the format cannot store is never a change.** It is
//!   reported ([`WriteReport::unwritten_extra`]), not written.
//! * **A number the format cannot carry is not a change on its own.** It
//!   cannot be put in the file, so it only matters when the file has a
//!   number the write would take with it.
//! * **A value that only a second tag holds is a change.** The comparison is
//!   made on the primary tag, the tag the writer writes.
//! * **No request panics**, whatever it names.
//!
//! # The model
//!
//! The properties compare the report with a model of the rule that is built
//! from the *values a reader sees* — the values [`read_tags`] returns — and
//! not from the items the tag holds. The model knows the two things the
//! format decides: the numbers the MP4 family cannot carry, and the `extra`
//! keys a format has no field for.
//!
//! The fixtures are copied into a temporary directory first: a test never
//! writes into the fixture tree.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rustamp_core::meta::TrackMetadata;
use rustamp_meta::{
    R128_TRACK_GAIN_KEY, REFERENCE_LOUDNESS_KEY, TagField, read_tags, write_tags_with_report,
};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// The generator
// ---------------------------------------------------------------------------

/// A seeded pseudo-random generator (SplitMix64).
///
/// The suite needs no dependency and no randomness at run time: every case
/// comes from a fixed seed, so a failure can be replayed.
struct Rng(u64);

impl Rng {
    /// A generator for the seed.
    fn new(seed: u64) -> Self {
        Rng(seed)
    }

    /// The next number.
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..bound`.
    fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }

    /// A number in `low..=high`.
    fn between(&mut self, low: u64, high: u64) -> u64 {
        low + self.below(high - low + 1)
    }

    /// One of the items.
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// The seeds every property runs with.
///
/// A failure names the seed, so it can be replayed.
const SEEDS: [u64; 3] = [0x5EED_0A01, 0x5EED_0A02, 0x5EED_0A03];

/// The cases per seed and format.
const CASES: usize = 12;

// ---------------------------------------------------------------------------
// The formats
// ---------------------------------------------------------------------------

/// The tag format the writer writes for a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagKind {
    /// ID3v2: MP3, AAC, WAV, AIFF.
    Id3v2,
    /// Vorbis comments: FLAC, Ogg Vorbis, Opus.
    Vorbis,
    /// The MP4 `ilst` atoms: M4A, ALAC.
    Ilst,
}

/// A file the writer supports.
struct Format {
    /// The name in the failure messages.
    name: &'static str,
    /// The tag the writer writes for the file.
    kind: TagKind,
    /// The fixture the file is copied from.
    fixture: &'static str,
}

/// The formats the suite runs over.
const FORMATS: &[Format] = &[
    Format {
        name: "mp3",
        kind: TagKind::Id3v2,
        fixture: "sine.mp3",
    },
    Format {
        name: "flac",
        kind: TagKind::Vorbis,
        fixture: "sine.flac",
    },
    Format {
        name: "ogg",
        kind: TagKind::Vorbis,
        fixture: "sine.ogg",
    },
    Format {
        name: "opus",
        kind: TagKind::Vorbis,
        fixture: "sine.opus",
    },
    Format {
        name: "m4a",
        kind: TagKind::Ilst,
        fixture: "sine_aac.m4a",
    },
    Format {
        name: "alac",
        kind: TagKind::Ilst,
        fixture: "sine_alac.m4a",
    },
];

// ---------------------------------------------------------------------------
// The files
// ---------------------------------------------------------------------------

/// A fixture of the decoder crate.
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

// ---------------------------------------------------------------------------
// The cases
// ---------------------------------------------------------------------------

/// A request: the metadata to write and the fields to remove.
#[derive(Debug)]
struct Request {
    /// The metadata of the request.
    metadata: TrackMetadata,
    /// The fields the request removes.
    removals: Vec<TagField>,
}

/// A random text: plain ASCII, or characters from outside it.
fn random_word(rng: &mut Rng) -> String {
    const ASCII: &[&str] = &["title", "Song", "track 1", "a-b_c", "0123", "MiXeD"];
    const UNICODE: &[&str] = &["Ünïcødé", "日本語", "🎵", "Ω≈ç√∫", "Ævar Þór", "naïve"];

    let mut word = if rng.below(100) < 40 {
        rng.pick(UNICODE).to_string()
    } else {
        rng.pick(ASCII).to_string()
    };
    if rng.below(100) < 50 {
        word.push('-');
        word.push_str(&rng.next_u64().to_string());
    }
    word
}

/// A text value for a request.
///
/// The value is sometimes the one the file has (the request then decides
/// nothing), sometimes empty, sometimes padded, sometimes a new value — and
/// sometimes the field is not named at all, which means "keep".
///
/// A request that repeats the values of the file names the current value of
/// every field it names: such a request changes nothing.
fn random_text(rng: &mut Rng, current: Option<&str>, replay: bool) -> Option<String> {
    if replay {
        return current.map(str::to_owned);
    }
    match rng.below(100) {
        0..=24 => None,
        25..=34 => Some(String::new()),
        35..=44 => Some("   ".to_owned()),
        45..=59 => current.map(str::to_owned),
        60..=74 => Some(format!("  {}  ", random_word(rng))),
        _ => Some(random_word(rng)),
    }
}

/// A year for a request.
fn random_year(rng: &mut Rng, current: Option<u32>, replay: bool) -> Option<u32> {
    if replay {
        return current;
    }
    match rng.below(100) {
        0..=24 => None,
        25..=39 => current,
        40..=49 => Some(0),
        50..=59 => Some(9999),
        60..=74 => Some(current.map_or(1, |year| year.wrapping_add(1))),
        _ => Some(rng.between(1000, 9999) as u32),
    }
}

/// A number for a request.
///
/// The edges are the numbers the MP4 family cannot carry: a zero and a
/// number above 65535.
fn random_number(rng: &mut Rng, current: Option<u32>, replay: bool) -> Option<u32> {
    if replay {
        return current;
    }
    match rng.below(100) {
        0..=24 => None,
        25..=39 => current,
        40..=49 => Some(0),
        50..=59 => Some(u32::MAX),
        60..=69 => Some(65536),
        70..=84 => Some(current.map_or(1, |number| number.wrapping_add(1))),
        _ => Some(rng.between(1, 1000) as u32),
    }
}

/// The `extra` keys the suite writes.
const EXTRA_KEYS: &[&str] = &[
    "REPLAYGAIN_TRACK_GAIN",
    R128_TRACK_GAIN_KEY,
    "DISCTOTAL",
    "CUSTOM_UNKNOWN_KEY",
    REFERENCE_LOUDNESS_KEY,
];

/// The value of an `extra` key.
///
/// The values are the ones a tagger writes: the reader parses some of them,
/// so they cannot be arbitrary text.
fn extra_value(key: &str) -> &'static str {
    match key {
        "REPLAYGAIN_TRACK_GAIN" => "-3.50 dB",
        R128_TRACK_GAIN_KEY => "-768",
        "DISCTOTAL" => "12",
        _ => "custom value",
    }
}

/// Whether the format can carry the `extra` key.
fn carries_extra(kind: TagKind, key: &str) -> bool {
    match key {
        "REPLAYGAIN_TRACK_GAIN" => true,
        R128_TRACK_GAIN_KEY => matches!(kind, TagKind::Id3v2 | TagKind::Vorbis),
        "DISCTOTAL" => matches!(kind, TagKind::Vorbis),
        _ => false,
    }
}

/// Every removal the writer supports.
const REMOVALS: &[TagField] = &[
    TagField::Title,
    TagField::Artist,
    TagField::Album,
    TagField::AlbumArtist,
    TagField::Year,
    TagField::Genre,
    TagField::Comment,
    TagField::Track,
    TagField::Disc,
];

/// A request that is built from the values the file has.
///
/// The request names some of the fields, removes some of them and leaves the
/// rest alone. The cases reach the edges of the rule: a value the file
/// already has, an empty value, a value outside ASCII, a number the MP4
/// family cannot carry and a key no format has a field for.
fn random_request(rng: &mut Rng, before: &TrackMetadata) -> Request {
    // A third of the requests repeat the values the file has: they are the
    // cases in which the writer must not touch the file at all.
    let replay = rng.below(100) < 30;
    let mut metadata = TrackMetadata {
        title: random_text(rng, before.title.as_deref(), replay),
        artist: random_text(rng, before.artist.as_deref(), replay),
        album: random_text(rng, before.album.as_deref(), replay),
        album_artist: random_text(rng, before.album_artist.as_deref(), replay),
        genre: random_text(rng, before.genre.as_deref(), replay),
        comment: random_text(rng, before.comment.as_deref(), replay),
        year: random_year(rng, before.year, replay),
        track_no: random_number(rng, before.track_no, replay),
        track_total: random_number(rng, before.track_total, replay),
        disc_no: random_number(rng, before.disc_no, replay),
        ..TrackMetadata::default()
    };
    for key in EXTRA_KEYS {
        let current = before.extra.get(*key).map(String::as_str);
        let value = if replay {
            current.map(str::to_owned)
        } else {
            match rng.below(100) {
                0..=29 => None,
                30..=49 => current.map(str::to_owned),
                _ => Some(extra_value(key).to_owned()),
            }
        };
        if let Some(value) = value {
            metadata.extra.insert((*key).to_owned(), value);
        }
    }

    let mut removals = Vec::new();
    if !replay {
        for field in REMOVALS {
            if rng.below(100) < 15 {
                removals.push(*field);
            }
        }
    }
    Request { metadata, removals }
}

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

/// The value a reader sees for a text value.
///
/// A value of blanks is no value, exactly as the reader sees it.
fn normalized(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// Whether the format can carry the number.
///
/// The MP4 `trkn` and `disk` atoms hold a pair of 16-bit numbers, and a zero
/// is "no number at all": the MP4 family carries neither a zero nor a number
/// above 65535. The other formats store the number as text.
fn carries_number(kind: TagKind, number: u32) -> bool {
    match kind {
        TagKind::Id3v2 | TagKind::Vorbis => true,
        TagKind::Ilst => (1..=u32::from(u16::MAX)).contains(&number),
    }
}

/// Whether the field holds a value the reader sees.
fn present(before: &TrackMetadata, field: TagField) -> bool {
    match field {
        TagField::Title => before.title.is_some(),
        TagField::Artist => before.artist.is_some(),
        TagField::Album => before.album.is_some(),
        TagField::AlbumArtist => before.album_artist.is_some(),
        TagField::Year => before.year.is_some(),
        TagField::Genre => before.genre.is_some(),
        TagField::Comment => before.comment.is_some(),
        TagField::Track => before.track_no.is_some() || before.track_total.is_some(),
        TagField::Disc => before.disc_no.is_some(),
    }
}

/// Whether the number changes the value the file has.
///
/// A number the format can carry is the value the file has afterwards. A
/// number it cannot carry is not in the file at all: it only matters when the
/// file has a number the write would take with it.
fn number_changed(kind: TagKind, current: Option<u32>, requested: u32) -> bool {
    if carries_number(kind, requested) {
        current != Some(requested)
    } else {
        current.is_some()
    }
}

/// Whether the request would change the values the file has.
///
/// The model is the rule of the `unchanged` module, written down over the
/// values a reader sees:
///
/// * a field the request does not name keeps the value the file has, so it
///   cannot decide anything;
/// * a field of the removal list is removed: the request wins over a value,
///   so the value of such a field cannot decide anything either;
/// * a removal of a field the file has a value for is a change;
/// * a value the format cannot carry is not in the file, so it only matters
///   when the file has a value the write would take with it;
/// * an `extra` entry the format cannot store is not a change: it cannot be
///   in the file and cannot be put there.
fn model_changed(
    format: &Format,
    before: &TrackMetadata,
    metadata: &TrackMetadata,
    removals: &[TagField],
) -> bool {
    // The removals: a removal of a field the file holds is a change.
    for field in removals {
        if present(before, *field) {
            return true;
        }
    }

    // The named text fields. A field of the removal list is not compared
    // here: the removal wins over a value.
    if !removals.contains(&TagField::Title)
        && text_changed(before.title.as_deref(), metadata.title.as_deref())
    {
        return true;
    }
    if !removals.contains(&TagField::Artist)
        && text_changed(before.artist.as_deref(), metadata.artist.as_deref())
    {
        return true;
    }
    if !removals.contains(&TagField::Album)
        && text_changed(before.album.as_deref(), metadata.album.as_deref())
    {
        return true;
    }
    if !removals.contains(&TagField::AlbumArtist)
        && text_changed(
            before.album_artist.as_deref(),
            metadata.album_artist.as_deref(),
        )
    {
        return true;
    }
    if !removals.contains(&TagField::Genre)
        && text_changed(before.genre.as_deref(), metadata.genre.as_deref())
    {
        return true;
    }
    if !removals.contains(&TagField::Comment)
        && text_changed(before.comment.as_deref(), metadata.comment.as_deref())
    {
        return true;
    }

    // The named numbers.
    if let Some(year) = metadata.year
        && before.year != Some(year)
    {
        return true;
    }
    if !removals.contains(&TagField::Track) {
        if let Some(number) = metadata.track_no
            && number_changed(format.kind, before.track_no, number)
        {
            return true;
        }
        if let Some(number) = metadata.track_total
            && number_changed(format.kind, before.track_total, number)
        {
            return true;
        }
    }
    if !removals.contains(&TagField::Disc)
        && let Some(number) = metadata.disc_no
        && number_changed(format.kind, before.disc_no, number)
    {
        return true;
    }

    // The `extra` entries the format can store.
    for (key, value) in &metadata.extra {
        if !carries_extra(format.kind, key) {
            continue;
        }
        let current = before.extra.get(key).map(String::as_str);
        if normalized(value) != current.and_then(normalized) {
            return true;
        }
    }

    false
}

/// Whether the requested text value differs from the one the file has.
///
/// A request that does not name the field means "keep", so the field cannot
/// decide anything.
fn text_changed(current: Option<&str>, requested: Option<&str>) -> bool {
    match requested {
        None => false,
        Some(value) => normalized(value) != current.and_then(normalized),
    }
}

// ---------------------------------------------------------------------------
// The fields
// ---------------------------------------------------------------------------

/// The fields the metadata can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Year,
    Genre,
    Comment,
    TrackNo,
    TrackTotal,
    DiscNo,
}

/// Every field, in a fixed order.
const FIELDS: &[Field] = &[
    Field::Title,
    Field::Artist,
    Field::Album,
    Field::AlbumArtist,
    Field::Year,
    Field::Genre,
    Field::Comment,
    Field::TrackNo,
    Field::TrackTotal,
    Field::DiscNo,
];

impl Field {
    /// The removal that takes the field away.
    ///
    /// The track number and the total are one field: half the formats keep
    /// the two in one place.
    fn removal(self) -> TagField {
        match self {
            Self::Title => TagField::Title,
            Self::Artist => TagField::Artist,
            Self::Album => TagField::Album,
            Self::AlbumArtist => TagField::AlbumArtist,
            Self::Year => TagField::Year,
            Self::Genre => TagField::Genre,
            Self::Comment => TagField::Comment,
            Self::TrackNo | Self::TrackTotal => TagField::Track,
            Self::DiscNo => TagField::Disc,
        }
    }

    /// The value the reader sees for the field.
    fn value(self, metadata: &TrackMetadata) -> Option<String> {
        match self {
            Self::Title => metadata.title.clone(),
            Self::Artist => metadata.artist.clone(),
            Self::Album => metadata.album.clone(),
            Self::AlbumArtist => metadata.album_artist.clone(),
            Self::Year => metadata.year.map(|year| year.to_string()),
            Self::Genre => metadata.genre.clone(),
            Self::Comment => metadata.comment.clone(),
            Self::TrackNo => metadata.track_no.map(|number| number.to_string()),
            Self::TrackTotal => metadata.track_total.map(|number| number.to_string()),
            Self::DiscNo => metadata.disc_no.map(|number| number.to_string()),
        }
    }
}

/// A metadata that names exactly one field with a new value.
///
/// The value is one the file cannot have: the numbers of [`full_metadata`]
/// stay below 1000, the years below 9000, and the texts carry a number the
/// words of the file do not have.
fn patch(field: Field, rng: &mut Rng) -> TrackMetadata {
    let mut metadata = TrackMetadata::default();
    set_field(&mut metadata, field, Some(random_value(field, rng)));
    metadata
}

/// The value of a field.
fn random_value(field: Field, rng: &mut Rng) -> String {
    match field {
        Field::Year => rng.between(9001, 9999).to_string(),
        Field::TrackNo | Field::TrackTotal | Field::DiscNo => rng.between(1001, 2000).to_string(),
        _ => format!("A Patched Value {}", rng.next_u64()),
    }
}

/// Sets the value of a field of a metadata.
fn set_field(metadata: &mut TrackMetadata, field: Field, value: Option<String>) {
    match field {
        Field::Title => metadata.title = value,
        Field::Artist => metadata.artist = value,
        Field::Album => metadata.album = value,
        Field::AlbumArtist => metadata.album_artist = value,
        Field::Year => metadata.year = value.and_then(|value| value.parse().ok()),
        Field::Genre => metadata.genre = value,
        Field::Comment => metadata.comment = value,
        Field::TrackNo => metadata.track_no = value.and_then(|value| value.parse().ok()),
        Field::TrackTotal => metadata.track_total = value.and_then(|value| value.parse().ok()),
        Field::DiscNo => metadata.disc_no = value.and_then(|value| value.parse().ok()),
    }
}

// ---------------------------------------------------------------------------
// The properties
// ---------------------------------------------------------------------------

/// How the cases of the main property reached the edges of the rule.
#[derive(Debug, Default)]
struct Counters {
    /// A request that changes nothing.
    no_op: usize,
    /// A request that changes something.
    change: usize,
    /// A request that names a field with the value the file has.
    same_value: usize,
    /// A request that names a field with an empty value.
    empty_value: usize,
    /// A request that names a field with a value outside ASCII.
    unicode: usize,
    /// A request that names a number the MP4 family cannot carry.
    uncarryable_number: usize,
    /// A request that names a key no format has a field for.
    uncarryable_extra: usize,
    /// A request that removes a field the file holds.
    removal_of_a_present_field: usize,
    /// A request that removes a field the file does not hold.
    removal_of_an_absent_field: usize,
    /// A request that names a field and removes it at the same time.
    both: usize,
}

#[test]
fn a_write_is_a_no_op_exactly_when_nothing_would_change() {
    let mut counters = Counters::default();

    for format in FORMATS {
        for seed in SEEDS {
            let dir = TempDir::new().unwrap();
            let path = copy_fixture(&dir, format.fixture);

            let mut rng = Rng::new(seed);
            for case in 0..CASES {
                // The values the file has now, and the request built from them.
                let before = read_tags(&path).unwrap();
                let request = random_request(&mut rng, &before);
                count(&mut counters, format, &before, &request);

                let bytes = bytes_of(&path);
                let mtime = mtime_of(&path);

                let report =
                    write_tags_with_report(&path, &request.metadata, &request.removals).unwrap();

                let changed = model_changed(format, &before, &request.metadata, &request.removals);
                assert_eq!(
                    report.unchanged, !changed,
                    "{}: seed {seed:#x}, case {case}: the report says {1}, the model says the request changes {changed}\nbefore: {before:?}\nrequest: {request:?}",
                    format.name, report.unchanged
                );

                if report.unchanged {
                    counters.no_op += 1;
                    // The file is not touched at all.
                    assert_eq!(
                        bytes_of(&path),
                        bytes,
                        "{}: seed {seed:#x}, case {case}: the file was rewritten",
                        format.name
                    );
                    assert_eq!(
                        mtime_of(&path),
                        mtime,
                        "{}: seed {seed:#x}, case {case}: the file was touched",
                        format.name
                    );
                    assert_eq!(
                        read_tags(&path).unwrap(),
                        before,
                        "{}: seed {seed:#x}, case {case}: the values changed",
                        format.name
                    );
                } else {
                    counters.change += 1;
                    // A change is a change: the file is written.
                    assert!(
                        bytes_of(&path) != bytes || mtime_of(&path) != mtime,
                        "{}: seed {seed:#x}, case {case}: the file was not written",
                        format.name
                    );
                    // And it is a change a reader sees: the values are not
                    // the ones the file had.
                    assert_ne!(
                        read_tags(&path).unwrap(),
                        before,
                        "{}: seed {seed:#x}, case {case}: the file was written, but the values did not change\nbefore: {before:?}\nrequest: {request:?}",
                        format.name
                    );
                }
            }
        }
    }

    // The properties are only worth something when the cases reach the
    // edges. A generator that stopped there would make every property
    // vacuous.
    for (what, count) in [
        ("a request that changes nothing", counters.no_op),
        ("a request that changes something", counters.change),
        ("a value the file already has", counters.same_value),
        ("an empty value", counters.empty_value),
        ("a value outside ASCII", counters.unicode),
        (
            "a number the MP4 family cannot carry",
            counters.uncarryable_number,
        ),
        (
            "a key no format has a field for",
            counters.uncarryable_extra,
        ),
        (
            "a removal of a present field",
            counters.removal_of_a_present_field,
        ),
        (
            "a removal of an absent field",
            counters.removal_of_an_absent_field,
        ),
        ("a field that is named and removed", counters.both),
    ] {
        assert!(count > 0, "no case has {what}");
    }
}

/// Counts what the cases of the main property reached.
fn count(counters: &mut Counters, format: &Format, before: &TrackMetadata, request: &Request) {
    for key in request.metadata.extra.keys() {
        if !carries_extra(format.kind, key) {
            counters.uncarryable_extra += 1;
        }
    }
    for number in [
        request.metadata.track_no,
        request.metadata.track_total,
        request.metadata.disc_no,
    ] {
        if let Some(number) = number
            && !carries_number(format.kind, number)
        {
            counters.uncarryable_number += 1;
        }
    }
    for value in [
        &request.metadata.title,
        &request.metadata.artist,
        &request.metadata.album,
        &request.metadata.album_artist,
        &request.metadata.genre,
        &request.metadata.comment,
    ] {
        match value {
            Some(value) if value.is_empty() => counters.empty_value += 1,
            Some(value) if !value.is_ascii() => counters.unicode += 1,
            _ => {}
        }
    }
    for field in FIELDS {
        let Some(value) = (*field).value(&request.metadata) else {
            continue;
        };
        if Some(value) == (*field).value(before) {
            counters.same_value += 1;
        }
    }
    for removal in &request.removals {
        if present(before, *removal) {
            counters.removal_of_a_present_field += 1;
        } else {
            counters.removal_of_an_absent_field += 1;
        }
        for field in fields_of(removal) {
            if field.value(&request.metadata).is_some() {
                counters.both += 1;
            }
        }
    }
}

/// The fields a removal takes with it.
///
/// A field is a whole field: removing the track takes the track number and
/// the total number of tracks with it.
fn fields_of(removal: &TagField) -> &'static [Field] {
    match removal {
        TagField::Title => &[Field::Title],
        TagField::Artist => &[Field::Artist],
        TagField::Album => &[Field::Album],
        TagField::AlbumArtist => &[Field::AlbumArtist],
        TagField::Year => &[Field::Year],
        TagField::Genre => &[Field::Genre],
        TagField::Comment => &[Field::Comment],
        TagField::Track => &[Field::TrackNo, Field::TrackTotal],
        TagField::Disc => &[Field::DiscNo],
    }
}

#[test]
fn naming_a_field_with_a_new_value_is_never_a_no_op() {
    for format in FORMATS {
        for seed in SEEDS {
            let dir = TempDir::new().unwrap();
            let path = copy_fixture(&dir, format.fixture);

            // A file with a value in every field the removal list knows, so
            // that a removal of another field has something to remove.
            let mut rng = Rng::new(seed);
            let full = full_metadata(&mut rng);
            write_tags_with_report(&path, &full, &[]).unwrap();

            for field in FIELDS {
                let mut rng = Rng::new(seed ^ 0x9E37_79B9);
                let request = patch(*field, &mut rng);
                // The other fields say what they like: the value of the file
                // for the fields the request does not name.
                let mut metadata = request.clone();
                for other in [Field::Album, Field::Genre, Field::Comment] {
                    if other == *field {
                        continue;
                    }
                    set_field(&mut metadata, other, before_value(&path, other));
                }

                let report = write_tags_with_report(&path, &metadata, &[]).unwrap();
                assert!(
                    !report.unchanged,
                    "{}: seed {seed:#x}: {:?} was a no-op: {report:?}",
                    format.name, field
                );
                // And the value is in the file.
                let after = read_tags(&path).unwrap();
                assert_eq!(
                    (*field).value(&after),
                    (*field).value(&metadata),
                    "{}: seed {seed:#x}: {:?} did not take the new value",
                    format.name,
                    field
                );
            }
        }
    }
}

/// A metadata with every field filled.
fn full_metadata(rng: &mut Rng) -> TrackMetadata {
    let mut metadata = TrackMetadata {
        title: Some(random_word(rng)),
        artist: Some(random_word(rng)),
        album: Some(random_word(rng)),
        album_artist: Some(random_word(rng)),
        genre: Some(random_word(rng)),
        comment: Some(random_word(rng)),
        year: Some(rng.between(1000, 9000) as u32),
        track_no: Some(rng.between(1, 1000) as u32),
        track_total: Some(rng.between(1, 1000) as u32),
        disc_no: Some(rng.between(1, 1000) as u32),
        ..TrackMetadata::default()
    };
    metadata
        .extra
        .insert("REPLAYGAIN_TRACK_GAIN".into(), "-3.50 dB".into());
    metadata
}

/// The value the file has for the field.
fn before_value(path: &Path, field: Field) -> Option<String> {
    field.value(&read_tags(path).unwrap())
}

#[test]
fn a_removal_of_a_present_field_is_a_change_and_of_an_absent_one_is_not() {
    for format in FORMATS {
        for seed in SEEDS {
            let dir = TempDir::new().unwrap();
            let path = copy_fixture(&dir, format.fixture);

            let mut rng = Rng::new(seed);
            let full = full_metadata(&mut rng);
            write_tags_with_report(&path, &full, &[]).unwrap();

            for removal in REMOVALS {
                // The field is in the file: the removal takes it away.
                let report =
                    write_tags_with_report(&path, &TrackMetadata::default(), &[*removal]).unwrap();
                assert!(
                    !report.unchanged,
                    "{}: seed {seed:#x}: the removal of {removal:?} was a no-op: {report:?}",
                    format.name
                );

                // The field is gone: the same removal changes nothing.
                let bytes = bytes_of(&path);
                let mtime = mtime_of(&path);
                let report =
                    write_tags_with_report(&path, &TrackMetadata::default(), &[*removal]).unwrap();
                assert!(
                    report.unchanged,
                    "{}: seed {seed:#x}: the removal of {removal:?} was a change: {report:?}",
                    format.name
                );
                assert_eq!(
                    bytes_of(&path),
                    bytes,
                    "{}: seed {seed:#x}: the file was rewritten",
                    format.name
                );
                assert_eq!(
                    mtime_of(&path),
                    mtime,
                    "{}: seed {seed:#x}: the file was touched",
                    format.name
                );
            }
        }
    }
}

#[test]
fn an_extra_the_format_cannot_store_is_never_a_change() {
    for format in FORMATS {
        for seed in SEEDS {
            let dir = TempDir::new().unwrap();
            let path = copy_fixture(&dir, format.fixture);

            let mut rng = Rng::new(seed);
            let full = full_metadata(&mut rng);
            write_tags_with_report(&path, &full, &[]).unwrap();

            let bytes = bytes_of(&path);
            let mtime = mtime_of(&path);

            // A key no format has a field for: the key cannot be in the
            // file and cannot be put there.
            let mut metadata = TrackMetadata::default();
            metadata
                .extra
                .insert(REFERENCE_LOUDNESS_KEY.into(), "89 dB SPL".into());
            metadata
                .extra
                .insert("CUSTOM_UNKNOWN_KEY".into(), "custom value".into());

            let report = write_tags_with_report(&path, &metadata, &[]).unwrap();
            // The report names the refused keys in the order of the map: the
            // keys are sorted by name.
            assert_eq!(
                report.unwritten_extra,
                vec![
                    "CUSTOM_UNKNOWN_KEY".to_owned(),
                    REFERENCE_LOUDNESS_KEY.to_owned()
                ],
                "{}: seed {seed:#x}: the report names the wrong entries: {report:?}",
                format.name
            );
            // The refused keys are reported, but they are not a change: the
            // file holds every value the request names.
            assert!(
                report.unchanged,
                "{}: seed {seed:#x}: {report:?}",
                format.name
            );
            assert_eq!(
                bytes_of(&path),
                bytes,
                "{}: seed {seed:#x}: the file was rewritten",
                format.name
            );
            assert_eq!(
                mtime_of(&path),
                mtime,
                "{}: seed {seed:#x}: the file was touched",
                format.name
            );
        }
    }
}

/// A 128-byte ID3v1.1 tag.
fn id3v1(title: &str, artist: &str, album: &str, year: &str) -> Vec<u8> {
    let mut tag = vec![0u8; 128];
    tag[0..3].copy_from_slice(b"TAG");
    let mut put = |at: usize, len: usize, text: &str| {
        let bytes = text.as_bytes();
        let len = bytes.len().min(len);
        tag[at..at + len].copy_from_slice(&bytes[..len]);
    };
    put(3, 30, title);
    put(33, 30, artist);
    put(63, 30, album);
    put(93, 4, year);
    tag[127] = 255; // the genre: none
    tag
}

#[test]
fn a_value_that_only_the_id3v1_tag_holds_is_a_change() {
    const TITLE: &str = "Only In The V1 Tag";
    const ARTIST: &str = "The V1 Artist";
    const ALBUM: &str = "The V1 Album";
    const YEAR: &str = "1994";

    for seed in SEEDS {
        let dir = TempDir::new().unwrap();
        let path = copy_fixture(&dir, "sine.mp3");

        // The ID3v2 tag loses its values, and an ID3v1 tag takes them over:
        // they are now only in the second tag of the file.
        write_tags_with_report(
            &path,
            &TrackMetadata::default(),
            &[
                TagField::Title,
                TagField::Artist,
                TagField::Album,
                TagField::Year,
            ],
        )
        .unwrap();
        let mut bytes = bytes_of(&path);
        bytes.extend_from_slice(&id3v1(TITLE, ARTIST, ALBUM, YEAR));
        std::fs::write(&path, &bytes).unwrap();

        // The reader sees the values — they come from the ID3v1 tag.
        let before = read_tags(&path).unwrap();
        assert_eq!(before.title.as_deref(), Some(TITLE), "seed {seed:#x}");
        assert_eq!(before.artist.as_deref(), Some(ARTIST), "seed {seed:#x}");
        assert_eq!(before.album.as_deref(), Some(ALBUM), "seed {seed:#x}");
        assert_eq!(before.year, Some(1994), "seed {seed:#x}");

        // A request for one of them is a change: the write puts the value
        // into the tag the reader prefers.
        for (field, value) in [
            (Field::Title, TITLE.to_owned()),
            (Field::Artist, ARTIST.to_owned()),
            (Field::Album, ALBUM.to_owned()),
            (Field::Year, YEAR.to_owned()),
        ] {
            let mut metadata = TrackMetadata::default();
            match field {
                Field::Title => metadata.title = Some(value),
                Field::Artist => metadata.artist = Some(value),
                Field::Album => metadata.album = Some(value),
                Field::Year => metadata.year = Some(value.parse().unwrap()),
                _ => unreachable!(),
            }
            let report = write_tags_with_report(&path, &metadata, &[]).unwrap();
            assert!(
                !report.unchanged,
                "seed {seed:#x}: the value of {field:?} was a no-op: {report:?}"
            );
            // And now that the primary tag holds the value, the same request
            // changes nothing.
            let bytes = bytes_of(&path);
            let mtime = mtime_of(&path);
            let report = write_tags_with_report(&path, &metadata, &[]).unwrap();
            assert!(
                report.unchanged,
                "seed {seed:#x}: the value of {field:?} was a change: {report:?}"
            );
            assert_eq!(
                bytes_of(&path),
                bytes,
                "seed {seed:#x}: the file was rewritten"
            );
            assert_eq!(
                mtime_of(&path),
                mtime,
                "seed {seed:#x}: the file was touched"
            );
        }
    }
}

#[test]
fn a_removal_wins_over_the_value_of_the_same_field() {
    for format in FORMATS {
        for seed in SEEDS {
            let dir = TempDir::new().unwrap();
            let path = copy_fixture(&dir, format.fixture);

            // A title in the file.
            let titled = TrackMetadata {
                title: Some("A Title".into()),
                ..TrackMetadata::default()
            };
            write_tags_with_report(&path, &titled, &[]).unwrap();

            // A request that names the title and removes it: the removal
            // wins, so the title is taken out of the file.
            let report = write_tags_with_report(&path, &titled, &[TagField::Title]).unwrap();
            assert!(
                !report.unchanged,
                "{}: seed {seed:#x}: the removal of the title was a no-op: {report:?}",
                format.name
            );
            assert_eq!(
                read_tags(&path).unwrap().title,
                None,
                "{}: seed {seed:#x}: the title survived",
                format.name
            );

            // The same request now: the file does not have the title and
            // will not have it, so the value cannot make it a change. The
            // file is not touched.
            let bytes = bytes_of(&path);
            let mtime = mtime_of(&path);
            let report = write_tags_with_report(&path, &titled, &[TagField::Title]).unwrap();
            assert!(
                report.unchanged,
                "{}: seed {seed:#x}: the value of a removed field made it a change: {report:?}",
                format.name
            );
            assert_eq!(
                bytes_of(&path),
                bytes,
                "{}: seed {seed:#x}: the file was rewritten",
                format.name
            );
            assert_eq!(
                mtime_of(&path),
                mtime,
                "{}: seed {seed:#x}: the file was touched",
                format.name
            );
        }
    }
}

#[test]
fn a_number_the_format_cannot_store_is_not_a_change_by_itself() {
    // The MP4 `trkn` and `disk` atoms hold a pair of 16-bit numbers, and a
    // zero is "no number at all": the MP4 family carries neither a zero nor
    // a number above 65535. Such a number cannot be put in the file, so it
    // only matters when the file has a number the write would take with it.
    for name in ["sine_aac.m4a", "sine_alac.m4a"] {
        let dir = TempDir::new().unwrap();
        let path = copy_fixture(&dir, name);

        // A file without a number: the number cannot be put in the file and
        // the file has none to lose, so nothing changes.
        for number in [0u32, 70000, u32::MAX] {
            for field in [Field::TrackNo, Field::DiscNo] {
                let mut metadata = TrackMetadata::default();
                set_field(&mut metadata, field, Some(number.to_string()));

                let bytes = bytes_of(&path);
                let mtime = mtime_of(&path);
                let report = write_tags_with_report(&path, &metadata, &[]).unwrap();
                assert_eq!(
                    report.unstoreable,
                    vec![field.removal()],
                    "{name}: the number {number} of {field:?} was not reported"
                );
                assert!(
                    report.unchanged,
                    "{name}: the number {number} of {field:?} was a change: {report:?}"
                );
                assert_eq!(
                    bytes_of(&path),
                    bytes,
                    "{name}: the number {number} of {field:?} rewrote the file"
                );
                assert_eq!(
                    mtime_of(&path),
                    mtime,
                    "{name}: the number {number} of {field:?} touched the file"
                );
            }
        }

        // A file with a number: the write takes the number with it, so the
        // request is a change.
        let numbered = TrackMetadata {
            track_no: Some(5),
            ..TrackMetadata::default()
        };
        write_tags_with_report(&path, &numbered, &[]).unwrap();

        let uncarryable = TrackMetadata {
            track_no: Some(70000),
            ..TrackMetadata::default()
        };
        let report = write_tags_with_report(&path, &uncarryable, &[]).unwrap();
        assert!(
            !report.unchanged,
            "{name}: the number that took the old one with it was a no-op: {report:?}"
        );
        assert_eq!(read_tags(&path).unwrap().track_no, None, "{name}");
    }
}

/// Adds the description atom of the MP4 family to the file at `path`.
///
/// The atom is the description of the file, and the reader takes it as the
/// comment when the file has no comment of its own.
fn plant_mp4_description(path: &Path, value: &str) {
    use lofty::config::{ParseOptions, WriteOptions};
    use lofty::file::AudioFile;
    use lofty::mp4::{Atom, AtomData, AtomIdent, Mp4File};
    use lofty::tag::TagExt;
    use std::io::Seek;

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open the MP4 file");
    let mp4 = Mp4File::read_from(&mut file, ParseOptions::new().read_properties(false))
        .expect("read the MP4 file");
    let mut ilst = mp4.ilst().cloned().unwrap_or_default();
    ilst.insert(Atom::new(
        AtomIdent::Fourcc(*b"desc"),
        AtomData::UTF8(value.into()),
    ));
    file.rewind().expect("rewind");
    ilst.save_to(&mut file, WriteOptions::default())
        .expect("write the atom");
}

#[test]
fn an_empty_comment_takes_the_description_with_it() {
    // The reader takes a description as the comment when a file has no
    // comment of its own, so a comment that is empty has to take the
    // description with it: the file must not show a comment the caller has
    // asked away.
    let dir = TempDir::new().unwrap();

    // The Vorbis family: the description is a comment of its own.
    let path = copy_fixture(&dir, "sine.flac");
    assert_eq!(
        read_tags(&path).unwrap().comment.as_deref(),
        Some("hello"),
        "the fixture has no description"
    );
    let empty = TrackMetadata {
        comment: Some(String::new()),
        ..TrackMetadata::default()
    };
    let report = write_tags_with_report(&path, &empty, &[]).unwrap();
    assert!(!report.unchanged, "flac: {report:?}");
    assert_eq!(
        read_tags(&path).unwrap().comment,
        None,
        "flac: the description is still the comment"
    );

    // The MP4 family: the `desc` atom is the description of the file.
    let path = copy_fixture(&dir, "sine_aac.m4a");
    plant_mp4_description(&path, "The Description");
    assert_eq!(
        read_tags(&path).unwrap().comment.as_deref(),
        Some("The Description"),
        "m4a: the description is not the comment"
    );
    let empty = TrackMetadata {
        comment: Some(String::new()),
        ..TrackMetadata::default()
    };
    let report = write_tags_with_report(&path, &empty, &[]).unwrap();
    assert!(!report.unchanged, "m4a: {report:?}");
    assert_eq!(
        read_tags(&path).unwrap().comment,
        None,
        "m4a: the description is still the comment"
    );
}

#[test]
fn a_removal_of_a_field_the_reader_cannot_see_is_not_a_change() {
    for format in FORMATS {
        let dir = TempDir::new().unwrap();
        let path = copy_fixture(&dir, format.fixture);

        // A title in the file.
        let titled = TrackMetadata {
            title: Some("A Title".into()),
            ..TrackMetadata::default()
        };
        write_tags_with_report(&path, &titled, &[]).unwrap();

        // A title of zero length: the reader sees no title, but the file
        // still carries the frame of the title.
        let empty = TrackMetadata {
            title: Some(String::new()),
            ..TrackMetadata::default()
        };
        let report = write_tags_with_report(&path, &empty, &[]).unwrap();
        assert!(!report.unchanged, "{}: {report:?}", format.name);
        assert_eq!(read_tags(&path).unwrap().title, None, "{}", format.name);

        // The removal of the title changes nothing the reader can see: the
        // file is not touched.
        let bytes = bytes_of(&path);
        let mtime = mtime_of(&path);
        let report =
            write_tags_with_report(&path, &TrackMetadata::default(), &[TagField::Title]).unwrap();
        assert!(
            report.unchanged,
            "{}: the removal of a title the reader cannot see was a change: {report:?}",
            format.name
        );
        assert_eq!(
            bytes_of(&path),
            bytes,
            "{}: the file was rewritten",
            format.name
        );
        assert_eq!(
            mtime_of(&path),
            mtime,
            "{}: the file was touched",
            format.name
        );
    }
}

#[test]
fn no_request_panics() {
    for format in FORMATS {
        let dir = TempDir::new().unwrap();
        let path = copy_fixture(&dir, format.fixture);

        // The empty request: nothing to write, nothing to remove.
        let report = write_tags_with_report(&path, &TrackMetadata::default(), &[]).unwrap();
        assert!(report.unchanged, "{}: {report:?}", format.name);

        // A value outside ASCII.
        let unicode = TrackMetadata {
            title: Some("日本語 🎵 Ω≈ç√∫".into()),
            artist: Some("Ævar Þór".into()),
            ..TrackMetadata::default()
        };
        let report = write_tags_with_report(&path, &unicode, &[]).unwrap();
        assert!(!report.unchanged, "{}: {report:?}", format.name);
        assert_eq!(
            read_tags(&path).unwrap().title.as_deref(),
            Some("日本語 🎵 Ω≈ç√∫"),
            "{}",
            format.name
        );

        // A value of zero length.
        let empty = TrackMetadata {
            title: Some(String::new()),
            ..TrackMetadata::default()
        };
        let report = write_tags_with_report(&path, &empty, &[]).unwrap();
        assert!(!report.unchanged, "{}: {report:?}", format.name);

        // A field the request names and removes at the same time: the
        // removal wins. The title is already gone, so nothing changes.
        let both = TrackMetadata {
            title: Some("A Title".into()),
            ..TrackMetadata::default()
        };
        let report = write_tags_with_report(&path, &both, &[TagField::Title]).unwrap();
        assert!(report.unchanged, "{}: {report:?}", format.name);
        assert_eq!(
            read_tags(&path).unwrap().title,
            None,
            "{}: the title survived",
            format.name
        );

        // The empty removals with the empty metadata.
        let report = write_tags_with_report(&path, &TrackMetadata::default(), &[]).unwrap();
        assert!(report.unchanged, "{}: {report:?}", format.name);
    }
}
