// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The path a template renders stays inside the target directory.
//!
//! The template language inserts tag values into a **path**, and a tag is
//! free text: it can be `..`, it can carry a separator or a drive letter,
//! and the template itself can name a directory above the target. The module
//! promises a path **relative to the target directory** (see
//! [`rustamp_library::RenderedPath::relative_path`]), so a path that would
//! leave it is refused, not handed to a caller that would write outside.
//!
//! These tests use the public surface of the crate only: a caller sees the
//! segments, the joined path and the plan, and the plan is what the
//! extraction writes.

use rustamp_core::meta::TrackMetadata;
use rustamp_library::{ExtractErrorReason, PathTemplate, RenderedPath, TrackRow};
use std::path::{Component, Path, PathBuf};

/// The directory an extraction writes into.
fn target() -> PathBuf {
    PathBuf::from(r"C:\target")
}

/// A row whose artist and title carry the given values.
fn row(id: i64, artist: Option<&str>, title: Option<&str>) -> TrackRow {
    TrackRow {
        id,
        path: PathBuf::from(format!(r"C:\music\{id}.mp3")),
        metadata: TrackMetadata {
            artist: artist.map(str::to_owned),
            title: title.map(str::to_owned),
            ..Default::default()
        },
        mtime_ns: None,
        size: None,
    }
}

/// Renders one row through `template`.
fn rendered(template: &str, artist: Option<&str>, title: Option<&str>) -> RenderedPath {
    PathTemplate::parse(template).render(&row(1, artist, title))
}

/// Whether `path` is one path inside `root`: relative, and every component
/// a plain name.
fn is_inside(root: &Path, path: &Path) -> bool {
    if path.is_absolute() {
        return false;
    }
    if !path
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return false;
    }
    let joined = root.join(path);
    joined.starts_with(root)
        && joined.components().count() == root.components().count() + path.components().count()
}

/// The tracks the plan refuses, as the reason of each.
fn reasons(plan: &rustamp_library::ExtractPlan) -> Vec<&ExtractErrorReason> {
    plan.errors.iter().map(|error| &error.reason).collect()
}

#[test]
fn a_value_that_names_the_directory_above_the_target_is_refused() {
    // The artist of a track is free text, and `..` is a legal name on disk.
    let path = rendered(r"Artist\Title", Some(".."), Some("Dancing Queen"));
    assert_eq!(path.segments, vec!["..", "Dancing Queen"]);
    assert_eq!(
        path.relative_path(),
        None,
        "a segment of `..` is not a path inside the target directory"
    );

    let plan =
        PathTemplate::parse(r"Artist\Title").plan(&[row(1, Some(".."), Some("Dancing Queen"))]);
    assert!(
        plan.files.is_empty(),
        "nothing may be written for that track"
    );
    assert_eq!(plan.errors.len(), 1);
    assert!(
        matches!(plan.errors[0].reason, ExtractErrorReason::UnsafePath { .. }),
        "the plan reports the path instead of handing it out: {:?}",
        plan.errors[0].reason
    );
    assert_eq!(
        plan.errors[0].to_string(),
        format!(
            "Error extracting track #1: the path is not one path inside the target directory: ..{}Dancing Queen",
            std::path::MAIN_SEPARATOR
        )
    );
}

#[test]
fn a_value_that_carries_a_separator_or_a_drive_is_refused() {
    // Every one of these leaves the target directory, or silently writes a
    // level deeper than the template names.
    let cases: &[(&str, &str)] = &[
        (r"..\..\evil", "above the target twice"),
        (r"a\b", "a separator inside one segment"),
        ("a/b", "a forward slash inside one segment"),
        (r"C:\Windows\evil", "an absolute path"),
        (r"\\server\share\evil", "a UNC path"),
        (r"\evil", "a rooted path"),
        ("/evil", "a rooted path with a forward slash"),
        (r"C:evil", "a drive-relative path"),
    ];
    for (value, what) in cases {
        let path = rendered("Artist", Some(value), None);
        assert_eq!(
            path.relative_path(),
            None,
            "{what}: {value:?} is not a path inside the target directory"
        );
        let plan = PathTemplate::parse("Artist").plan(&[row(1, Some(value), None)]);
        assert!(
            plan.files.is_empty(),
            "{what}: no file may be planned for {value:?}"
        );
        assert!(
            matches!(plan.errors[0].reason, ExtractErrorReason::UnsafePath { .. }),
            "{what}: {value:?} is reported: {:?}",
            plan.errors[0].reason
        );
    }
}

#[test]
fn the_template_itself_cannot_leave_the_target_directory() {
    // No tag is involved here: the template names the path.
    let templates: &[&str] = &[
        r"..\Title",
        r".\Title",
        r"C:\Artist\Title",
        r"Artist\\Title",
        r"\Artist",
        r"Artist\..\Title",
    ];
    for template in templates {
        let plan = PathTemplate::parse(template).plan(&[row(1, Some("Abba"), Some("Song"))]);
        assert!(
            plan.files.is_empty(),
            "{template:?} names a path outside the target directory: {:?}",
            plan.files
        );
        assert!(
            matches!(
                plan.errors.as_slice(),
                [error] if matches!(error.reason, ExtractErrorReason::UnsafePath { .. })
            ),
            "{template:?} is reported: {:?}",
            reasons(&plan)
        );
    }
}

#[test]
fn the_segments_of_a_rendered_path_are_one_component_each() {
    // A segment with a separator used to grow a level and an empty segment
    // used to disappear, so the joined path and the segments disagreed.
    for (template, artist, title) in [
        (r"Artist\Title", Some("a/b"), Some("c")),
        (r"Artist\\Title", Some("a"), Some("x")),
        (r"\Artist", Some("a"), None),
    ] {
        let path = rendered(template, artist, title);
        let joined = path.relative_path();
        assert!(
            joined.is_none()
                || joined
                    .as_ref()
                    .is_some_and(|p| p.components().count() == path.segments.len()),
            "{template:?}: the joined path has the levels of the segments: segments={:?} joined={:?}",
            path.segments,
            joined
        );
    }
}

#[test]
fn no_two_planned_files_write_to_the_same_path() {
    // The duplicate check compares the segments, so a segment that used to
    // collapse made two different segment lists join to one file: the second
    // track silently wrote over the first.
    let template = PathTemplate::parse(r"Artist\Title");
    let rows = vec![
        row(1, Some("safe"), Some("one")),
        row(2, Some("a/b"), Some("c")),
        row(3, Some("a"), Some("b/c")),
        row(4, Some("safe"), Some("two")),
    ];
    let plan = template.plan(&rows);
    assert_eq!(
        plan.counts().failed,
        2,
        "both tracks name a path that is not one path inside the target directory: {:?}",
        reasons(&plan)
    );
    assert_eq!(
        plan.counts().extracted,
        2,
        "the other two tracks are written"
    );

    let mut written: Vec<PathBuf> = Vec::new();
    for file in &plan.files {
        let path = file
            .path
            .relative_path()
            .expect("a planned file is inside the target");
        assert!(
            written.iter().all(|other| other != &path),
            "two planned files write to {}",
            path.display()
        );
        written.push(path);
    }
}

#[test]
fn every_planned_path_is_one_path_inside_the_target_directory() {
    // The values of a tag are free text, and the template is written by the
    // user. Whatever the two are, a planned file is written inside the target
    // directory and nowhere else.
    let templates = [
        r"Artist\Title",
        r"Artist\\Title",
        r"..\Title",
        r".\Artist\Title",
        "Artist",
        r"C:\Artist\Title",
        r"## - Artist - Title",
        r"Album\Artist\Title",
    ];
    let values = [
        "..",
        r"..\..",
        ".",
        "a/b",
        r"a\b",
        r"C:\Windows\evil",
        r"\\server\share\evil",
        r"\evil",
        "Song ",
        "CON",
        "Ärzte",
        "e\u{301}",
        "",
    ];
    let mut rows = Vec::new();
    let mut id = 0;
    for artist in values {
        for title in values {
            id += 1;
            rows.push(row(id, Some(artist), Some(title)));
        }
    }
    for template in templates {
        let plan = PathTemplate::parse(template).plan(&rows);
        assert_eq!(
            plan.files.len() + plan.errors.len(),
            rows.len(),
            "{template:?}: every track is a file or an error"
        );
        for file in &plan.files {
            let path = file
                .path
                .relative_path()
                .unwrap_or_else(|| panic!("{template:?}: a planned file is inside the target"));
            assert!(
                is_inside(&target(), &path),
                "{template:?} with the segments {:?} writes outside the target directory: {}",
                file.path.segments,
                path.display()
            );
        }
    }
}
