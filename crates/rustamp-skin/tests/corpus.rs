//! Corpus test: load every real classic skin from the skin corpus.
//!
//! The corpus lives outside the repository (test input only) and is located
//! through `RUSTAMP_SKIN_CORPUS`, defaulting to
//! `D:\code\rustamp-free-sources\skins-corpus`. When the directory does not
//! exist the test prints `SKIP` and passes, so it stays portable.

use std::path::PathBuf;

use rustamp_gfx::{Image, Rect};
use rustamp_skin::Skin;

fn corpus_dir() -> PathBuf {
    std::env::var_os("RUSTAMP_SKIN_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"D:\code\rustamp-free-sources\skins-corpus"))
}

fn corpus_files() -> Vec<PathBuf> {
    let dir = corpus_dir();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                            e.eq_ignore_ascii_case("wsz") || e.eq_ignore_ascii_case("zip")
                        })
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

#[test]
fn corpus_skins_load_without_panicking() {
    let dir = corpus_dir();
    if !dir.is_dir() {
        println!(
            "SKIP corpus: {} does not exist (set RUSTAMP_SKIN_CORPUS)",
            dir.display()
        );
        return;
    }

    let files = corpus_files();
    assert!(!files.is_empty(), "corpus directory has no .wsz/.zip files");

    let mut loaded = 0usize;
    let mut with_substitutions = 0usize;
    let mut decode_warnings = 0usize;

    for path in &files {
        let skin = match Skin::load(path) {
            Ok(skin) => skin,
            Err(error) => panic!("{}: failed to load: {error}", path.display()),
        };
        loaded += 1;

        // The main window must always have its sprites: either the skin
        // provides them or the default skin fills in.
        assert!(
            skin.has_main_window_sprites(),
            "{}: main-window sheets still missing after fallback: {:?}",
            path.display(),
            skin.missing_sheets()
        );

        // Cross-check the loader against the compositor: paint the main
        // background region into a fresh RGBA buffer and compare it with the
        // decoded sheet.
        let (sheet, def) = skin
            .sprite_image("MAIN_WINDOW_BACKGROUND")
            .unwrap_or_else(|| panic!("{}: no MAIN_WINDOW_BACKGROUND", path.display()));
        let source =
            Image::from_rgba(sheet.width(), sheet.height(), sheet.as_rgba().to_vec()).unwrap();
        let mut canvas = Image::new(def.width, def.height);
        canvas.blit_rect(
            &source,
            Rect::new(def.x as i32, def.y as i32, def.width, def.height),
            0,
            0,
        );
        for (dx, dy) in [
            (0u32, 0u32),
            (def.width / 2, def.height / 2),
            (def.width - 1, def.height - 1),
        ] {
            let sx = def.x + dx;
            let sy = def.y + dy;
            let expected = sheet.pixel(sx, sy).unwrap_or([0, 0, 0, 0]);
            assert_eq!(
                canvas.pixel(dx as i32, dy as i32),
                Some(expected),
                "{}: painted pixel ({dx},{dy}) mismatch",
                path.display()
            );
        }

        if !skin.substitutions().is_empty() {
            with_substitutions += 1;
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            println!("{name}:");
            for substitution in skin.substitutions() {
                println!("    {substitution}");
            }
        }
        for warning in skin.warnings() {
            decode_warnings += 1;
            println!("{}: decode warning: {warning}", path.display());
        }
    }

    println!(
        "corpus: {loaded} skins loaded, {with_substitutions} needed substitutions, \
         {decode_warnings} decode warnings"
    );
}
