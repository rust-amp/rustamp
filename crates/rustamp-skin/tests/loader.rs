//! Skin loader integration tests: containers, case handling, fallbacks, text
//! files, corrupt input and the compositor hand-off.

mod support;

use rustamp_gfx::Image;
use rustamp_skin::{
    BuiltinDefaultSkin, MemorySource, PlaylistStyle, Sheet, Skin, SkinFallback, Substitute, webamp,
};
use support::*;

/// Loads the synthetic skin from a folder, a `.zip`, a `.wsz` and an in-memory
/// container and asserts that all four produce the same result.
#[test]
fn folder_zip_wsz_and_memory_load_identically() {
    let dir = tempfile::tempdir().unwrap();
    let entries = synthetic_entries();
    write_folder(dir.path(), &entries);

    let zip_path = dir.path().join("skin.zip");
    std::fs::write(&zip_path, zip_bytes(&entries)).unwrap();
    let wsz_path = dir.path().join("skin.wsz");
    std::fs::write(&wsz_path, zip_bytes(&entries)).unwrap();

    let folder_skin = Skin::load(dir.path()).unwrap();
    let zip_skin = Skin::load(&zip_path).unwrap();
    let wsz_skin = Skin::load(&wsz_path).unwrap();
    let mut memory = MemorySource::from_entries("memory", entries.clone());
    let memory_skin = Skin::from_source(&mut memory, &BuiltinDefaultSkin).unwrap();

    for sheet in Sheet::ALL {
        assert_eq!(
            folder_skin.sheet(*sheet),
            zip_skin.sheet(*sheet),
            "sheet {sheet:?} differs between folder and zip"
        );
        assert_eq!(
            zip_skin.sheet(*sheet),
            wsz_skin.sheet(*sheet),
            "sheet {sheet:?} differs between zip and wsz"
        );
        assert_eq!(
            wsz_skin.sheet(*sheet),
            memory_skin.sheet(*sheet),
            "sheet {sheet:?} differs between wsz and memory"
        );
    }

    assert_eq!(folder_skin.pledit(), zip_skin.pledit());
    assert_eq!(folder_skin.viscolors(), zip_skin.viscolors());
    assert_eq!(folder_skin.region(), zip_skin.region());
    assert_eq!(folder_skin.region(), wsz_skin.region());
    assert_eq!(folder_skin.region(), memory_skin.region());
    assert!(folder_skin.has_main_window_sprites());
    assert!(zip_skin.has_main_window_sprites());
    assert!(wsz_skin.has_main_window_sprites());
    assert!(memory_skin.has_main_window_sprites());
}

#[test]
fn entry_names_are_case_insensitive() {
    let dir = tempfile::tempdir().unwrap();

    // Only an upper-case entry: it must still be found.
    write_folder(
        dir.path(),
        &[("MAIN.BMP".to_string(), bmp24_solid(275, 116, [9, 8, 7]))],
    );
    let skin = Skin::load(dir.path()).unwrap();
    assert_eq!(
        skin.sheet(Sheet::Main).unwrap().pixel(0, 0),
        Some([9, 8, 7, 255])
    );

    // Mixed case in a subfolder.
    let dir2 = tempfile::tempdir().unwrap();
    write_folder(
        dir2.path(),
        &[("sub/MaIn.BmP".to_string(), bmp24_solid(275, 116, [1, 1, 1]))],
    );
    let skin2 = Skin::load(dir2.path()).unwrap();
    assert_eq!(
        skin2.sheet(Sheet::Main).unwrap().pixel(0, 0),
        Some([1, 1, 1, 255])
    );

    // Both spellings present: the last entry in container order wins.
    let mut entries = synthetic_entries();
    entries.retain(|(name, _)| !name.eq_ignore_ascii_case("main.bmp"));
    entries.push(("main.bmp".to_string(), bmp24_solid(275, 116, [0, 0, 0])));
    entries.push((
        "MAIN.BMP".to_string(),
        bmp24_solid(275, 116, [255, 255, 255]),
    ));
    let zip = dir.path().join("dup.zip");
    std::fs::write(&zip, zip_bytes(&entries)).unwrap();
    let skin3 = Skin::load(&zip).unwrap();
    assert_eq!(
        skin3.sheet(Sheet::Main).unwrap().pixel(0, 0),
        Some([255, 255, 255, 255])
    );
}

#[test]
fn raw_sheet_bytes_are_kept_alongside_the_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let entries = synthetic_entries();
    write_folder(dir.path(), &entries);
    let skin = Skin::load(dir.path()).unwrap();

    let raw = skin.sheet_raw(Sheet::Main).expect("main.bmp raw bytes");
    assert_eq!(&raw[0..2], b"BM");
    let original = &entries
        .iter()
        .find(|(name, _)| name == "main.bmp")
        .unwrap()
        .1;
    assert_eq!(raw, original.as_slice());

    // A generated default sheet has no raw source bytes.
    let mut entries = synthetic_entries();
    entries.retain(|(name, _)| name != "main.bmp");
    let dir2 = tempfile::tempdir().unwrap();
    write_folder(dir2.path(), &entries);
    let skin2 = Skin::load(dir2.path()).unwrap();
    assert!(skin2.sheet_raw(Sheet::Main).unwrap().is_empty());
}

#[test]
fn archive_path_escapes_are_not_readable() {
    let dir = tempfile::tempdir().unwrap();
    let entries = vec![
        ("../main.bmp".to_string(), bmp24_solid(275, 116, [1, 2, 3])),
        (
            "sub/../../titlebar.bmp".to_string(),
            bmp24_solid(344, 87, [4, 5, 6]),
        ),
    ];
    let zip = dir.path().join("escape.zip");
    std::fs::write(&zip, zip_bytes(&entries)).unwrap();

    let skin = Skin::load(&zip).unwrap();
    // Both escaping entries are ignored, so the default skin fills in.
    let default_main = BuiltinDefaultSkin.sheet_image(Sheet::Main).unwrap();
    assert_eq!(
        skin.sheet(Sheet::Main).unwrap().pixel(0, 0),
        default_main.pixel(0, 0)
    );
    assert_ne!(
        skin.sheet(Sheet::Main).unwrap().pixel(0, 0),
        Some(rustamp_skin::PLACEHOLDER_RGBA)
    );
    assert!(
        skin.substitutions()
            .iter()
            .any(|s| s.requested == "main.bmp" && s.with == Substitute::DefaultSkin)
    );
}

#[test]
fn nums_ex_falls_back_to_numbers_and_is_reported() {
    // Skin with numbers.bmp but no nums_ex.bmp.
    let mut entries = synthetic_entries();
    entries.retain(|(name, _)| !name.eq_ignore_ascii_case("numbers.bmp"));
    entries.push((
        "numbers.bmp".to_string(),
        bmp24_solid(99, 13, [200, 100, 50]),
    ));

    let dir = tempfile::tempdir().unwrap();
    write_folder(dir.path(), &entries);
    let skin = Skin::load(dir.path()).unwrap();

    let nums_ex = skin
        .sheet(Sheet::NumsEx)
        .expect("nums_ex should be substituted");
    assert_eq!(nums_ex.pixel(0, 0), Some([200, 100, 50, 255]));
    assert!(
        skin.substitutions().iter().any(|s| {
            s.requested == "nums_ex.bmp" && s.with == Substitute::Sheet(Sheet::Numbers)
        }),
        "expected a nums_ex -> numbers substitution, got {:?}",
        skin.substitutions()
    );

    // The reverse direction: only nums_ex.bmp present.
    let mut entries = synthetic_entries();
    entries.retain(|(name, _)| !name.eq_ignore_ascii_case("numbers.bmp"));
    entries.push(("nums_ex.bmp".to_string(), bmp24_solid(108, 13, [7, 7, 7])));
    let dir2 = tempfile::tempdir().unwrap();
    write_folder(dir2.path(), &entries);
    let skin2 = Skin::load(dir2.path()).unwrap();
    assert_eq!(
        skin2.sheet(Sheet::Numbers).unwrap().pixel(0, 0),
        Some([7, 7, 7, 255])
    );
    assert!(
        skin2.substitutions().iter().any(|s| {
            s.requested == "numbers.bmp" && s.with == Substitute::Sheet(Sheet::NumsEx)
        })
    );
}

#[test]
fn missing_main_sheet_uses_default_skin() {
    let mut entries = synthetic_entries();
    entries.retain(|(name, _)| !name.eq_ignore_ascii_case("main.bmp"));
    let dir = tempfile::tempdir().unwrap();
    write_folder(dir.path(), &entries);

    let skin = Skin::load(dir.path()).unwrap();
    let main = skin
        .sheet(Sheet::Main)
        .expect("default skin must supply main.bmp");
    let default_main = BuiltinDefaultSkin.sheet_image(Sheet::Main).unwrap();
    assert_eq!(main.pixel(0, 0), default_main.pixel(0, 0));
    assert_ne!(main.pixel(0, 0), Some(rustamp_skin::PLACEHOLDER_RGBA));
    assert!(skin.has_main_window_sprites());
    assert!(
        skin.substitutions()
            .iter()
            .any(|s| s.requested == "main.bmp" && s.with == Substitute::DefaultSkin),
        "expected a default-skin substitution, got {:?}",
        skin.substitutions()
    );

    // An unknown sprite name is not invented.
    assert!(skin.sprite("NOT_A_REAL_SPRITE").is_none());
}

#[test]
fn text_files_use_documented_defaults_when_absent() {
    let mut entries = synthetic_entries();
    entries.retain(|(name, _)| {
        !name.eq_ignore_ascii_case("pledit.txt")
            && !name.eq_ignore_ascii_case("viscolor.txt")
            && !name.eq_ignore_ascii_case("region.txt")
    });
    let dir = tempfile::tempdir().unwrap();
    write_folder(dir.path(), &entries);

    let skin = Skin::load(dir.path()).unwrap();
    assert_eq!(skin.pledit(), &PlaylistStyle::default());
    assert_eq!(skin.viscolors(), &webamp::DEFAULT_VISCOLORS);
    assert!(skin.region().is_empty());
    for requested in ["pledit.txt", "viscolor.txt", "region.txt"] {
        assert!(
            skin.substitutions()
                .iter()
                .any(|s| s.requested == requested && s.with == Substitute::Defaults),
            "missing substitution report for {requested}"
        );
    }
}

#[test]
fn text_files_are_parsed_when_present() {
    let dir = tempfile::tempdir().unwrap();
    write_folder(dir.path(), &synthetic_entries());
    let skin = Skin::load(dir.path()).unwrap();

    let pledit = skin.pledit();
    assert_eq!(pledit.normal, [0x11, 0x22, 0x33]);
    assert_eq!(pledit.current, [0x44, 0x55, 0x66]);
    assert_eq!(pledit.normal_bg, [0x77, 0x88, 0x99]);
    assert_eq!(pledit.selected_bg, [0xAA, 0xBB, 0xCC]);
    assert_eq!(pledit.font.as_deref(), Some("Comic Sans MS"));

    assert_eq!(skin.viscolors()[0], [10, 20, 30]);
    assert_eq!(skin.viscolors()[1], [40, 50, 60]);
    assert_eq!(skin.viscolors()[2], webamp::DEFAULT_VISCOLORS[2]);

    assert_eq!(skin.region().normal().len(), 1);
    assert_eq!(
        skin.region().normal()[0],
        vec![(0, 0), (275, 0), (275, 116), (0, 116)]
    );
    assert_eq!(skin.region().window_shade().len(), 1);
}

#[test]
fn zero_byte_text_files_use_defaults() {
    let mut entries = synthetic_entries();
    entries.retain(|(name, _)| {
        !name.eq_ignore_ascii_case("pledit.txt") && !name.eq_ignore_ascii_case("viscolor.txt")
    });
    entries.push(("pledit.txt".to_string(), Vec::new()));
    entries.push(("viscolor.txt".to_string(), Vec::new()));
    let dir = tempfile::tempdir().unwrap();
    write_folder(dir.path(), &entries);

    let skin = Skin::load(dir.path()).unwrap();
    assert_eq!(skin.pledit(), &PlaylistStyle::default());
    assert_eq!(skin.viscolors(), &webamp::DEFAULT_VISCOLORS);
}

#[test]
fn corrupt_inputs_never_panic() {
    // A property-style loop over mutations of a valid skin archive.
    let base = zip_bytes(&synthetic_entries());
    let dir = tempfile::tempdir().unwrap();

    let mut cases: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"not a zip".to_vec(),
        vec![0u8; 64],
        b"BM".to_vec(),
    ];
    // Truncations at a spread of offsets.
    for len in [1, 16, 100, base.len() / 2, base.len() - 1] {
        if len <= base.len() {
            cases.push(base[..len].to_vec());
        }
    }
    // Single-byte mutations at a spread of offsets.
    for i in (0..base.len()).step_by(base.len() / 32 + 1) {
        let mut mutated = base.clone();
        mutated[i] ^= 0xFF;
        cases.push(mutated);
    }

    for (i, bytes) in cases.iter().enumerate() {
        let path = dir.path().join(format!("case{i}.wsz"));
        std::fs::write(&path, bytes).unwrap();
        // The result may be Ok or Err; it must never panic.
        let _ = Skin::load(&path);
    }

    // Corrupt entries inside an otherwise valid archive.
    let mut entries = synthetic_entries();
    entries.retain(|(name, _)| {
        !name.eq_ignore_ascii_case("main.bmp") && !name.eq_ignore_ascii_case("pledit.txt")
    });
    entries.push(("main.bmp".to_string(), b"BM\x00\x00 garbage".to_vec()));
    entries.push(("pledit.txt".to_string(), Vec::new()));
    let path = dir.path().join("corrupt_entries.wsz");
    std::fs::write(&path, zip_bytes(&entries)).unwrap();
    let skin = Skin::load(&path).expect("a corrupt entry must fall back, not fail");
    let default_main = BuiltinDefaultSkin.sheet_image(Sheet::Main).unwrap();
    assert_eq!(
        skin.sheet(Sheet::Main).unwrap().pixel(0, 0),
        default_main.pixel(0, 0)
    );
    assert_ne!(
        skin.sheet(Sheet::Main).unwrap().pixel(0, 0),
        Some(rustamp_skin::PLACEHOLDER_RGBA)
    );
    assert_eq!(skin.pledit(), &PlaylistStyle::default());
}

/// Paints a sprite region of the synthetic skin into an RGBA buffer with the
/// compositor and compares the pixels byte for byte.
#[test]
fn paints_sheet_region_into_rgba_buffer() {
    let dir = tempfile::tempdir().unwrap();
    write_folder(dir.path(), &synthetic_entries());
    let skin = Skin::load(dir.path()).unwrap();

    let def = skin.sprite("MAIN_WINDOW_BACKGROUND").unwrap();
    assert_eq!(def.sheet, Sheet::Main);
    let (sheet, _) = skin.sprite_image("MAIN_WINDOW_BACKGROUND").unwrap();
    assert_eq!((sheet.width(), sheet.height()), (275, 116));

    let source = Image::from_rgba(sheet.width(), sheet.height(), sheet.as_rgba().to_vec()).unwrap();
    let mut canvas = Image::filled(10, 10, [0, 0, 0, 0]);
    // Paint a 10x10 region from the sheet's top-left corner.
    canvas.blit_rect(&source, rustamp_gfx::Rect::new(0, 0, 10, 10), 0, 0);

    for y in 0..10u32 {
        for x in 0..10u32 {
            let expected = [x as u8, y as u8, (x ^ y) as u8, 255];
            assert_eq!(
                canvas.pixel(x as i32, y as i32),
                Some(expected),
                "at ({x},{y})"
            );
        }
    }

    // 2x scaling of the painted region is pixel-exact.
    let doubled = canvas.scaled(2);
    assert_eq!(doubled.width(), 20);
    assert_eq!(doubled.pixel(2, 2), Some([1, 1, 0, 255]));
    assert_eq!(doubled.pixel(19, 19), Some([9, 9, 0, 255]));
}
