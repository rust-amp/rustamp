// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Property tests for the query layer.
//!
//! The tests in `lib.rs` pin one behaviour each. These pin the
//! *invariants* that have to hold for every combination of filter, sort and
//! window:
//!
//! * `total` counts the matches before the window, and the window does not
//!   change it;
//! * the sort order is a total order, so a query walked in pages returns
//!   every row exactly once and in the same order;
//! * the filters compose with AND: the result of two filters is a subset
//!   of both, and an empty filter is the identity;
//! * the free text matches exactly the rows whose title, artist or album
//!   contains the term;
//! * the path prefix is a literal byte prefix;
//! * `limit: 0` returns no rows and still reports the full total.
//!
//! The libraries are synthetic: the rows go straight into the index, so a
//! row can carry a missing title, a missing duration, a Unicode value or a
//! `%` in its name - things a WAV fixture cannot express. The generator is
//! seeded, so a failing case is reproducible from its seed alone.

use crate::testsupport::Rng;
use crate::{Library, Query, SortKey, TrackRow};
use std::cmp::Ordering;
use std::path::PathBuf;

/// The seeds the properties run over. Every seed builds another library.
const SEEDS: &[u64] = &[
    0x1234_5678_9abc_def0,
    0x0f1e_2d3c_4b5a_6978,
    0xdead_beef_cafe_f00d,
    0x00c0_ffee_0bad_f00d,
    0x7fff_ffff_ffff_ffff,
];

/// The number of rows a synthetic library holds.
const ROWS: usize = 32;

/// The number of random filters a property draws per library.
const FILTERS: usize = 10;

/// The folders the synthetic rows live in.
///
/// `music` and `music2` share a byte prefix without being one folder, so
/// a path prefix can be tested against a directory boundary.
const FOLDERS: &[&str] = &["music", "music2", "other"];

/// The titles the synthetic rows draw from.
///
/// The pool holds a missing value, a pair that differs only in case, a
/// Unicode pair and the three `LIKE` metacharacters.
const TITLES: &[Option<&str>] = &[
    None,
    Some("Alpha"),
    Some("alpha"),
    Some("Ärzte"),
    Some("ärzte"),
    Some("Beta"),
    Some("100% Pure"),
    Some("under_score"),
    Some("back\\slash"),
    Some("Ωmega"),
];

/// The artists the synthetic rows draw from.
const ARTISTS: &[Option<&str>] = &[
    None,
    Some("Artist A"),
    Some("artist a"),
    Some("Ärzte"),
    Some("The Band"),
];

/// The albums the synthetic rows draw from.
const ALBUMS: &[Option<&str>] = &[
    None,
    Some("Album 1"),
    Some("Album 2"),
    Some("Ärzte Live"),
    Some("Jazz ist anders"),
];

/// The genres the synthetic rows draw from.
const GENRES: &[Option<&str>] = &[None, Some("Rock"), Some("Pop"), Some("Jazz")];

/// The durations the synthetic rows draw from, in milliseconds.
const DURATIONS: &[Option<i64>] = &[None, Some(0), Some(500), Some(1000), Some(2000)];

/// The sort keys the properties run over.
const SORT_KEYS: &[SortKey] = &[
    SortKey::Id,
    SortKey::Title,
    SortKey::Artist,
    SortKey::Album,
    SortKey::Duration,
];

/// The free-text terms that need a case of their own: the metacharacters,
/// the whitespace-only term and the Unicode pair.
const TERMS: &[&str] = &[
    "",
    "   ",
    "  spaced  ",
    "%",
    "_",
    "\\",
    "100%",
    "a_b",
    "ÄRZTE",
    "ärzte",
    "Ωmega",
    "ΩMEGA",
    "Alpha",
    "alpha",
    "nothing at all",
];

/// A synthetic library: the index and the rows it holds.
struct Fixture {
    lib: Library,
    rows: Vec<TrackRow>,
}

impl Fixture {
    /// A library with `count` rows, drawn from the pools by `seed`.
    fn new(seed: u64, count: usize) -> Self {
        let mut rng = Rng::new(seed);
        let lib = Library::in_memory().unwrap();
        for index in 0..count {
            let folder = rng.pick(FOLDERS);
            let path = format!("{folder}/track{index:03}.mp3");
            let title = rng.pick(TITLES).map(|value| value.to_string());
            let artist = rng.pick(ARTISTS).map(|value| value.to_string());
            let album = rng.pick(ALBUMS).map(|value| value.to_string());
            let genre = rng.pick(GENRES).map(|value| value.to_string());
            let duration = *rng.pick(DURATIONS);
            // The album artist is never searched, so it gets a value of its
            // own: a free text that finds it would be a bug.
            let album_artist = format!("AA-{index}");
            lib.conn
                .execute(
                    "INSERT INTO tracks (path, title, artist, album, album_artist, genre, \
                     duration_ms, mtime_ns, size, scanned_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 0, 0)",
                    rusqlite::params![path, title, artist, album, album_artist, genre, duration],
                )
                .unwrap();
        }
        // The rows as the index holds them, in the order of the row id.
        let rows = lib.query(&Query::default()).unwrap().rows;
        Fixture { lib, rows }
    }

    /// The ids of the rows `filter` matches, in the order of the query.
    fn ids(&self, filter: &Query) -> Vec<i64> {
        self.lib
            .query(filter)
            .unwrap()
            .rows
            .iter()
            .map(|row| row.id)
            .collect()
    }
}

/// The ids of `rows`, in their order.
fn ids_of(rows: &[TrackRow]) -> Vec<i64> {
    rows.iter().map(|row| row.id).collect()
}

/// Whether `row` satisfies `filter`, as the query layer documents it.
///
/// The field filters are exact and case-sensitive, the path prefix is a
/// literal byte prefix, and the free text is a case-insensitive substring
/// of the title, the artist or the album.
fn matches(row: &TrackRow, filter: &Query) -> bool {
    if let Some(prefix) = &filter.path_prefix
        && !row
            .path
            .to_string_lossy()
            .starts_with(prefix.to_string_lossy().as_ref())
    {
        return false;
    }
    if let Some(value) = &filter.artist
        && row.metadata.artist.as_deref() != Some(value.as_str())
    {
        return false;
    }
    if let Some(value) = &filter.album
        && row.metadata.album.as_deref() != Some(value.as_str())
    {
        return false;
    }
    if let Some(value) = &filter.title
        && row.metadata.title.as_deref() != Some(value.as_str())
    {
        return false;
    }
    if let Some(value) = &filter.genre
        && row.metadata.genre.as_deref() != Some(value.as_str())
    {
        return false;
    }
    if let Some(text) = &filter.text {
        let term = text.trim().to_lowercase();
        if !term.is_empty() && !contains_term(row, &term) {
            return false;
        }
    }
    true
}

/// Whether the title, the artist or the album of `row` contains the folded
/// `term`.
fn contains_term(row: &TrackRow, term: &str) -> bool {
    [
        row.metadata.title.as_deref(),
        row.metadata.artist.as_deref(),
        row.metadata.album.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.to_lowercase().contains(term))
}

/// The comparison the `ORDER BY` of the query layer performs.
///
/// A missing value sorts last in both directions, a text column compares
/// case-folded, and a tie always breaks by the ascending row id. The
/// comparison is total: two rows never compare equal.
fn compare(a: &TrackRow, b: &TrackRow, key: SortKey, descending: bool) -> Ordering {
    match key {
        SortKey::Id => flip(a.id.cmp(&b.id), descending),
        SortKey::Duration => {
            let missing = a
                .metadata
                .duration_ms
                .is_none()
                .cmp(&b.metadata.duration_ms.is_none());
            if missing != Ordering::Equal {
                return missing;
            }
            let order = flip(
                a.metadata.duration_ms.cmp(&b.metadata.duration_ms),
                descending,
            );
            if order != Ordering::Equal {
                return order;
            }
            a.id.cmp(&b.id)
        }
        SortKey::Title | SortKey::Artist | SortKey::Album => {
            let left = text_of(a, key);
            let right = text_of(b, key);
            let missing = left.is_none().cmp(&right.is_none());
            if missing != Ordering::Equal {
                return missing;
            }
            let order = flip(fold(left).cmp(&fold(right)), descending);
            if order != Ordering::Equal {
                return order;
            }
            a.id.cmp(&b.id)
        }
    }
}

/// The order of the comparison, reversed when the query sorts descending.
fn flip(order: Ordering, descending: bool) -> Ordering {
    if descending { order.reverse() } else { order }
}

/// The text of the sorted column.
fn text_of(row: &TrackRow, key: SortKey) -> Option<&str> {
    match key {
        SortKey::Title => row.metadata.title.as_deref(),
        SortKey::Artist => row.metadata.artist.as_deref(),
        SortKey::Album => row.metadata.album.as_deref(),
        SortKey::Id | SortKey::Duration => None,
    }
}

/// The folded text of the sorted column: the folding is the one the
/// `rustamp_lower` SQL function applies.
fn fold(text: Option<&str>) -> String {
    text.unwrap_or_default().to_lowercase()
}

/// The ids of the rows that satisfy `filter`, in the order the query layer
/// promises.
fn expected(rows: &[TrackRow], filter: &Query) -> Vec<i64> {
    let mut matching: Vec<&TrackRow> = rows.iter().filter(|row| matches(row, filter)).collect();
    matching.sort_by(|a, b| compare(a, b, filter.sort, filter.descending));
    matching.into_iter().map(|row| row.id).collect()
}

/// A random filter, drawn from the values the fixture holds.
///
/// The values come from the same pools as the rows, so a filter matches
/// something now and then.
fn random_filter(rng: &mut Rng, rows: &[TrackRow]) -> Query {
    Query {
        path_prefix: random_prefix(rng, rows),
        artist: random_value(rng, ARTISTS),
        album: random_value(rng, ALBUMS),
        title: random_value(rng, TITLES),
        genre: random_value(rng, GENRES),
        text: random_text(rng),
        sort: *rng.pick(SORT_KEYS),
        descending: rng.flip(),
        limit: None,
        offset: 0,
    }
}

/// A random value of a field filter, or no filter at all.
fn random_value(rng: &mut Rng, pool: &[Option<&str>]) -> Option<String> {
    rng.pick(pool).map(|value| value.to_string())
}

/// A random path prefix: a folder, a cut from a path of the fixture or a
/// spelling that differs in case.
fn random_prefix(rng: &mut Rng, rows: &[TrackRow]) -> Option<PathBuf> {
    match rng.below(6) {
        0 => None,
        1 => Some(PathBuf::from("music/")),
        2 => Some(PathBuf::from("other/")),
        3 => Some(PathBuf::from("Music/")),
        4 => Some(PathBuf::from("MUSIC/")),
        _ => {
            // A prefix cut from a path of the fixture.
            let path = rng.pick(rows).path.to_string_lossy().to_string();
            let chars: Vec<char> = path.chars().collect();
            let cut = rng.below(chars.len() + 1);
            Some(PathBuf::from(chars[..cut].iter().collect::<String>()))
        }
    }
}

/// A random free-text term, or no free text at all.
fn random_text(rng: &mut Rng) -> Option<String> {
    if rng.flip() {
        None
    } else {
        Some(rng.pick(TERMS).to_string())
    }
}

/// A query that matches exactly the rows that match both `first` and
/// `second`.
///
/// The two free texts cannot live in one query, so a pair with two
/// different texts has no combined query.
fn combine(first: &Query, second: &Query) -> Option<Query> {
    let text = match (&first.text, &second.text) {
        (None, None) => None,
        (Some(text), None) | (None, Some(text)) => Some(text.clone()),
        (Some(one), Some(other)) if one == other => Some(one.clone()),
        _ => return None,
    };
    Some(Query {
        path_prefix: combine_value(&first.path_prefix, &second.path_prefix, never_path()),
        artist: combine_value(&first.artist, &second.artist, never_text()),
        album: combine_value(&first.album, &second.album, never_text()),
        title: combine_value(&first.title, &second.title, never_text()),
        genre: combine_value(&first.genre, &second.genre, never_text()),
        text,
        sort: SortKey::Id,
        descending: false,
        limit: None,
        offset: 0,
    })
}

/// The value both filters ask for.
///
/// Two different values cannot be satisfied at once: the combined query
/// then asks for a value no row carries, and the result is empty.
fn combine_value<T: PartialEq + Clone>(
    first: &Option<T>,
    second: &Option<T>,
    never: T,
) -> Option<T> {
    match (first, second) {
        (Some(one), Some(other)) if one != other => Some(never),
        (Some(one), _) => Some(one.clone()),
        (_, Some(other)) => Some(other.clone()),
        (None, None) => None,
    }
}

/// A value no row of the fixture carries.
fn never_text() -> String {
    "\u{0}never".to_string()
}

/// A path prefix no row of the fixture carries.
fn never_path() -> PathBuf {
    PathBuf::from("\u{0}never")
}

/// The free-text terms: the ones that need a case of their own and cuts
/// from the values the rows hold.
fn terms(rng: &mut Rng) -> Vec<String> {
    let mut terms: Vec<String> = TERMS.iter().map(|term| term.to_string()).collect();
    let values = pool_values();
    for _ in 0..TERMS.len() {
        let value = rng.pick(&values);
        let chars: Vec<char> = value.chars().collect();
        let start = rng.below(chars.len());
        let end = start + rng.below(chars.len() - start + 1);
        terms.push(chars[start..end].iter().collect());
    }
    terms
}

/// Every value the title, artist and album pools hold.
fn pool_values() -> Vec<&'static str> {
    [TITLES, ARTISTS, ALBUMS]
        .into_iter()
        .flatten()
        .flatten()
        .copied()
        .collect()
}

/// `total` counts the matches before the window, and the window does not
/// change it.
#[test]
fn total_counts_the_matches_before_the_window() {
    for &seed in SEEDS {
        let fixture = Fixture::new(seed, ROWS);
        let mut rng = Rng::new(seed ^ 0x1111_2222);
        for _ in 0..FILTERS {
            let filter = random_filter(&mut rng, &fixture.rows);
            let all = expected(&fixture.rows, &filter);
            let page = fixture.lib.query(&filter).unwrap();
            assert_eq!(
                page.total,
                all.len(),
                "total is the number of matches (seed {seed})"
            );
            assert_eq!(
                ids_of(&page.rows),
                all,
                "the unpaged query returns every match (seed {seed})"
            );

            for (limit, offset) in [
                (None, 0),
                (None, 1),
                (None, 5),
                (Some(0), 0),
                (Some(1), 0),
                (Some(3), 2),
                (Some(7), 5),
                (Some(5), 100),
                (Some(100), 0),
            ] {
                let paged = fixture
                    .lib
                    .query(&Query {
                        limit,
                        offset,
                        ..filter.clone()
                    })
                    .unwrap();
                assert_eq!(
                    paged.total,
                    all.len(),
                    "the window does not change the total (limit {limit:?}, offset {offset})"
                );
                let start = usize::min(offset as usize, all.len());
                let end = match limit {
                    Some(limit) => usize::min(start + limit as usize, all.len()),
                    None => all.len(),
                };
                assert_eq!(
                    ids_of(&paged.rows),
                    all[start..end],
                    "the window is the matching slice (limit {limit:?}, offset {offset})"
                );
            }
        }
    }
}

/// Walking a query in pages returns the unpaged result: no row is
/// skipped, none is returned twice, and the order is the same.
#[test]
fn paging_walks_the_result_without_gap_or_repeat() {
    for &seed in SEEDS {
        let fixture = Fixture::new(seed, ROWS);
        let mut rng = Rng::new(seed ^ 0x3333_4444);
        for _ in 0..FILTERS {
            let filter = random_filter(&mut rng, &fixture.rows);
            let page_size = 1 + rng.below(5) as u32;
            let unpaged = fixture.ids(&filter);
            assert_eq!(
                unpaged,
                expected(&fixture.rows, &filter),
                "the index and the model agree on the order"
            );

            let mut walked: Vec<i64> = Vec::new();
            let mut offset = 0u32;
            let mut pages = 0usize;
            loop {
                let page = fixture
                    .lib
                    .query(&Query {
                        limit: Some(page_size),
                        offset,
                        ..filter.clone()
                    })
                    .unwrap();
                assert_eq!(
                    page.total,
                    unpaged.len(),
                    "every page reports the same total"
                );
                walked.extend(ids_of(&page.rows));
                pages += 1;
                if page.rows.len() < page_size as usize {
                    break;
                }
                offset += page_size;
                assert!(pages <= unpaged.len() + 2, "the walk has to end");
            }

            assert_eq!(
                walked, unpaged,
                "the pages together are the unpaged result (page size {page_size})"
            );
            let mut unique = walked.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(unique.len(), walked.len(), "no row is returned twice");
        }
    }
}

/// The sort order is a total order: no two rows compare equal, the order
/// is the promised one, and it does not change between two queries.
#[test]
fn the_sort_order_is_a_total_order() {
    for &seed in SEEDS {
        let fixture = Fixture::new(seed, ROWS);
        for key in SORT_KEYS {
            for descending in [false, true] {
                let filter = Query {
                    sort: *key,
                    descending,
                    ..Default::default()
                };
                let page = fixture.lib.query(&filter).unwrap();
                assert_eq!(
                    ids_of(&page.rows),
                    expected(&fixture.rows, &filter),
                    "the rows come in the promised order"
                );
                assert_eq!(
                    ids_of(&fixture.lib.query(&filter).unwrap().rows),
                    ids_of(&page.rows),
                    "the same query returns the same rows"
                );
                for pair in page.rows.windows(2) {
                    assert_eq!(
                        compare(&pair[0], &pair[1], *key, descending),
                        Ordering::Less,
                        "no two rows compare equal"
                    );
                }
            }
        }
    }
}

/// The filters compose with AND: the result of two filters is a subset of
/// both, and the empty filter is the identity.
#[test]
fn filters_compose_with_and() {
    for &seed in SEEDS {
        let fixture = Fixture::new(seed, ROWS);
        let mut rng = Rng::new(seed ^ 0x5555_6666);
        let everything = fixture.ids(&Query::default());
        assert_eq!(
            everything.len(),
            fixture.rows.len(),
            "the empty filter is the identity"
        );

        for _ in 0..FILTERS {
            let first = random_filter(&mut rng, &fixture.rows);
            let second = random_filter(&mut rng, &fixture.rows);
            let first_ids = fixture.ids(&first);
            let second_ids = fixture.ids(&second);

            // The rows that satisfy both filters.
            let both: Vec<i64> = fixture
                .rows
                .iter()
                .filter(|row| matches(row, &first) && matches(row, &second))
                .map(|row| row.id)
                .collect();

            // One query carries both filters, unless the two texts differ.
            if let Some(combined) = combine(&first, &second) {
                assert_eq!(
                    fixture.ids(&combined),
                    both,
                    "the combined filter matches exactly the rows that satisfy both"
                );
            }

            // The two results meet in the rows that satisfy both.
            let mut intersection: Vec<i64> = first_ids
                .iter()
                .copied()
                .filter(|id| second_ids.contains(id))
                .collect();
            intersection.sort_unstable();
            let mut both_sorted = both.clone();
            both_sorted.sort_unstable();
            assert_eq!(
                intersection, both_sorted,
                "the two results meet in the rows that satisfy both"
            );

            // The result of both is a subset of each.
            for id in both {
                assert!(
                    first_ids.contains(&id) && second_ids.contains(&id),
                    "the result of both filters is a subset of each"
                );
            }
        }
    }
}

/// The free text matches exactly the rows whose title, artist or album
/// contains the term.
#[test]
fn the_free_text_matches_the_contained_rows() {
    for &seed in SEEDS {
        let fixture = Fixture::new(seed, ROWS);
        let mut rng = Rng::new(seed ^ 0x7777_8888);
        for term in terms(&mut rng) {
            let filter = Query {
                text: Some(term.clone()),
                ..Default::default()
            };
            let expected = expected(&fixture.rows, &filter);
            let page = fixture.lib.query(&filter).unwrap();
            assert_eq!(
                ids_of(&page.rows),
                expected,
                "the free text matches exactly the rows that contain it (term {term:?})"
            );
            assert_eq!(
                page.total,
                expected.len(),
                "the total counts the rows that contain the term (term {term:?})"
            );
        }

        // The album artist is not searched: the free text covers the
        // title, the artist and the album only.
        for row in &fixture.rows {
            let Some(album_artist) = &row.metadata.album_artist else {
                continue;
            };
            let page = fixture
                .lib
                .query(&Query {
                    text: Some(album_artist.clone()),
                    ..Default::default()
                })
                .unwrap();
            assert!(
                page.rows.is_empty(),
                "the album artist is not searched (term {album_artist:?})"
            );
        }
    }
}

/// The path prefix is a literal byte prefix: a spelling that differs in
/// case matches nothing, and a prefix that is not a directory boundary
/// still matches.
#[test]
fn the_path_prefix_is_a_literal_byte_prefix() {
    let fixture = Fixture::new(SEEDS[0], ROWS);

    // A spelling that differs in case matches nothing: the prefix is a
    // byte prefix, not a case-insensitive one.
    for spelling in ["Music/", "MUSIC/", "Other/"] {
        let page = fixture
            .lib
            .query(&Query {
                path_prefix: Some(PathBuf::from(spelling)),
                ..Default::default()
            })
            .unwrap();
        assert!(
            page.rows.is_empty(),
            "the prefix is a byte prefix, so {spelling:?} matches nothing"
        );
        assert_eq!(page.total, 0);
    }

    // The exact spelling matches the rows under it.
    for folder in FOLDERS {
        let prefix = format!("{folder}/");
        let page = fixture
            .lib
            .query(&Query {
                path_prefix: Some(PathBuf::from(&prefix)),
                ..Default::default()
            })
            .unwrap();
        let expected = fixture
            .rows
            .iter()
            .filter(|row| row.path.to_string_lossy().starts_with(&prefix))
            .count();
        assert_eq!(
            page.total, expected,
            "the prefix {prefix:?} matches its rows"
        );
    }

    // A prefix that is not a directory boundary still matches: `music`
    // covers `music/` and `music2/`.
    let page = fixture
        .lib
        .query(&Query {
            path_prefix: Some(PathBuf::from("music")),
            ..Default::default()
        })
        .unwrap();
    let expected = fixture
        .rows
        .iter()
        .filter(|row| row.path.to_string_lossy().starts_with("music"))
        .count();
    assert_eq!(
        page.total, expected,
        "the prefix is not a directory boundary"
    );
    assert!(
        expected
            > fixture
                .rows
                .iter()
                .filter(|row| row.path.to_string_lossy().starts_with("music/"))
                .count(),
        "the prefix covers the sibling folder as well"
    );

    // An empty prefix is every row.
    let page = fixture
        .lib
        .query(&Query {
            path_prefix: Some(PathBuf::new()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(page.total, fixture.rows.len());
}

/// A limit of zero returns no rows and still reports the full total.
#[test]
fn a_limit_of_zero_returns_no_rows_and_the_full_total() {
    for &seed in SEEDS {
        let fixture = Fixture::new(seed, ROWS);
        let mut rng = Rng::new(seed ^ 0x9999_aaaa);
        for _ in 0..FILTERS {
            let filter = random_filter(&mut rng, &fixture.rows);
            let page = fixture
                .lib
                .query(&Query {
                    limit: Some(0),
                    ..filter.clone()
                })
                .unwrap();
            assert!(page.rows.is_empty(), "a limit of zero returns no rows");
            assert_eq!(
                page.total,
                expected(&fixture.rows, &filter).len(),
                "a limit of zero still reports the full total"
            );
        }
    }
}
