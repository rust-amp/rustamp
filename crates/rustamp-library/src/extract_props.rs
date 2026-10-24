// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Property tests for the path template.
//!
//! The tests in `extract.rs` pin one behaviour each. These pin the
//! *invariants* that have to hold for every template and every row:
//!
//! * rendering never panics: a template of any shape and a row of any shape
//!   render to a path;
//! * a field the row does not carry renders as an empty text and is reported
//!   exactly once, whatever the spelling of the token was;
//! * a row without any field renders the literal text of the template and
//!   nothing else;
//! * the plan adds up: the files and the errors cover the selection, every
//!   planned file has a file name, and no two planned files share a path;
//! * the counts of the plan are the counts of its lists.
//!
//! The templates and the rows are synthetic: the generator is seeded, so a
//! failing case is reproducible from its seed alone.

use crate::testsupport::Rng;
use crate::{
    ExtractErrorReason, PathTemplate, TemplateField, TemplatePart, TemplateToken, TrackRow,
};
use rustamp_core::meta::TrackMetadata;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// The seeds the properties run over. Every seed builds another set of
/// templates and rows.
const SEEDS: &[u64] = &[
    0x1234_5678_9abc_def0,
    0x0f1e_2d3c_4b5a_6978,
    0xdead_beef_cafe_f00d,
    0x00c0_ffee_0bad_f00d,
    0x7fff_ffff_ffff_ffff,
];

/// The number of templates a seed builds.
const TEMPLATES: usize = 24;

/// The number of rows a seed builds.
const ROWS: usize = 12;

/// The pieces a random template is built from.
///
/// The pool holds every token of the language, the words that are no token,
/// the directory separator and some text that is neither.
const PIECES: &[&str] = &[
    "Artist", "ARTIST", "artist", "Album", "ALBUM", "album", "Title", "TITLE", "title", "Genre",
    "GENRE", "genre", "year", "Year", "YEAR", "#", "##", "###", "####", "track", "ntracks",
    "Hello", " - ", "\\", "mp3", "", "  ", "Ärzte",
];

/// The values a random field of a random row holds.
const VALUES: &[&str] = &[
    "Abba Gold",
    "ABBA GOLD",
    "abba gold",
    "Ärzte",
    "100% Pure",
    "with - dash",
    "",
];

/// A random template.
fn random_template(rng: &mut Rng) -> String {
    let mut template = String::new();
    for _ in 0..rng.below(8) + 1 {
        template.push_str(rng.pick(PIECES));
    }
    template
}

/// A random row: every field is there or it is not.
fn random_row(rng: &mut Rng, id: i64) -> TrackRow {
    let text = |rng: &mut Rng| {
        if rng.flip() {
            Some((*rng.pick(VALUES)).to_owned())
        } else {
            None
        }
    };
    let number = |rng: &mut Rng| {
        if rng.flip() {
            Some(rng.below(1000) as u32)
        } else {
            None
        }
    };
    TrackRow {
        id,
        path: PathBuf::from(format!(r"C:\music\{id}.mp3")),
        metadata: TrackMetadata {
            title: text(rng),
            artist: text(rng),
            album: text(rng),
            genre: text(rng),
            year: number(rng),
            track_no: number(rng),
            ..Default::default()
        },
        mtime_ns: None,
        size: None,
    }
}

/// A row without a single field.
fn empty_row() -> TrackRow {
    TrackRow {
        id: 0,
        path: PathBuf::from(r"C:\music\0.mp3"),
        metadata: TrackMetadata::default(),
        mtime_ns: None,
        size: None,
    }
}

/// Whether the row carries the value the token asks for.
fn carries(row: &TrackRow, token: TemplateToken) -> bool {
    match token {
        TemplateToken::Artist(_) => row.metadata.artist.is_some(),
        TemplateToken::Album(_) => row.metadata.album.is_some(),
        TemplateToken::Title(_) => row.metadata.title.is_some(),
        TemplateToken::Genre(_) => row.metadata.genre.is_some(),
        TemplateToken::Year => row.metadata.year.is_some(),
        TemplateToken::TrackNumber { .. } => row.metadata.track_no.is_some(),
    }
}

/// The fields the row does not carry, in the order the template asks for
/// them.
fn expected_missing(template: &PathTemplate, row: &TrackRow) -> Vec<TemplateField> {
    let mut missing: Vec<TemplateField> = Vec::new();
    for segment in template.segments() {
        for part in segment.parts() {
            let TemplatePart::Token(token) = part else {
                continue;
            };
            let field = token.field();
            if !carries(row, *token) && !missing.contains(&field) {
                missing.push(field);
            }
        }
    }
    missing
}

/// The literal text of every segment, with the tokens left out.
fn literal_text(template: &PathTemplate) -> Vec<String> {
    template
        .segments()
        .iter()
        .map(|segment| {
            segment
                .parts()
                .iter()
                .filter_map(|part| match part {
                    TemplatePart::Literal(literal) => Some(literal.as_str()),
                    TemplatePart::Token(_) => None,
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .collect()
}

#[test]
fn rendering_never_panics_and_reports_every_missing_field() {
    for seed in SEEDS {
        let mut rng = Rng::new(*seed);
        for _ in 0..TEMPLATES {
            let template = PathTemplate::parse(&random_template(&mut rng));
            for id in 0..ROWS as i64 {
                let row = random_row(&mut rng, id);
                let path = template.render(&row);
                assert_eq!(
                    path.missing,
                    expected_missing(&template, &row),
                    "seed {seed:#x}: the reported fields are the ones the row lacks"
                );
            }
        }
    }
}

#[test]
fn a_row_without_any_field_renders_only_the_literal_text() {
    for seed in SEEDS {
        let mut rng = Rng::new(*seed);
        for _ in 0..TEMPLATES {
            let template = PathTemplate::parse(&random_template(&mut rng));
            let row = empty_row();
            let path = template.render(&row);
            assert_eq!(
                path.segments,
                literal_text(&template),
                "seed {seed:#x}: a missing value renders as an empty text"
            );
            assert_eq!(
                path.missing,
                expected_missing(&template, &row),
                "seed {seed:#x}: every field the template asks for is reported"
            );
        }
    }
}

#[test]
fn the_plan_adds_up_and_never_hands_a_path_out_twice() {
    for seed in SEEDS {
        let mut rng = Rng::new(*seed);
        for _ in 0..TEMPLATES {
            let template = PathTemplate::parse(&random_template(&mut rng));
            let rows: Vec<TrackRow> = (0..ROWS as i64)
                .map(|id| random_row(&mut rng, id))
                .collect();
            let plan = template.plan(&rows);

            assert_eq!(
                plan.files.len() + plan.errors.len(),
                rows.len(),
                "seed {seed:#x}: every track of the selection is either a file or an error"
            );

            let mut paths: BTreeSet<Vec<String>> = BTreeSet::new();
            for file in &plan.files {
                assert!(
                    !file.path.file_name().is_empty(),
                    "seed {seed:#x}: a planned file has a file name"
                );
                assert!(
                    paths.insert(file.path.segments.clone()),
                    "seed {seed:#x}: no two planned files share a path"
                );
            }

            for error in &plan.errors {
                assert!(
                    matches!(
                        error.reason,
                        ExtractErrorReason::EmptyPath
                            | ExtractErrorReason::DuplicatePath { .. }
                            | ExtractErrorReason::UnsafePath { .. }
                    ),
                    "seed {seed:#x}: an error names its reason"
                );
                assert!(
                    error.number >= 1 && error.number <= rows.len(),
                    "seed {seed:#x}: the number of a failed track is in the selection"
                );
            }

            let counts = plan.counts();
            assert_eq!(counts.extracted, plan.files.len());
            assert_eq!(counts.failed, plan.errors.len());
        }
    }
}
