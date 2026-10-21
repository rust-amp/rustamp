//! Acceptance against the external MIT webamp base-2.91 archive, never bundled.
//! Set RUSTAMP_BASE291_SKIN to override the local fixture path. A missing fixture
//! is an error: this suite must not silently report skipped acceptance as green.
use rustamp_core::{command::SelectOp, playlist::EntryId};
use rustamp_gfx::{Image, Rect};
use rustamp_skin::{
    ArchiveSource, BuiltinDefaultSkin, CursorSlot, Sheet, Skin, SkinSource, Substitute, webamp,
};
use rustamp_ui::*;

fn archive() -> ArchiveSource {
    let path = std::env::var_os("RUSTAMP_BASE291_SKIN")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("RUSTAMP_TESTSKINS")
                .map(|root| std::path::PathBuf::from(root).join("base-2.91.wsz"))
        })
        .unwrap_or_else(|| "D:/code/rustamp-testskins/base-2.91.wsz".into());
    ArchiveSource::open(&path)
        .unwrap_or_else(|e| panic!("base-2.91 acceptance requires {}: {e}", path.display()))
}

fn skin() -> Skin {
    Skin::from_source(&mut archive(), &BuiltinDefaultSkin).unwrap()
}

fn region(frame: &Image, skin: &Skin, sheet: Sheet, src: Rect, dx: i32, dy: i32) {
    let raster = skin.sheet(sheet).unwrap();
    for y in 0..src.height as i32 {
        for x in 0..src.width as i32 {
            let sx = src.x + x;
            let sy = src.y + y;
            assert!(
                sx >= 0 && sy >= 0 && sx < raster.width() as i32 && sy < raster.height() as i32
            );
            let i = ((sy as u32 * raster.width() + sx as u32) * 4) as usize;
            assert_eq!(
                frame.pixel(dx + x, dy + y).unwrap(),
                raster.as_rgba()[i..i + 4],
                "{sheet:?} source {sx},{sy} at {},{}",
                dx + x,
                dy + y
            );
        }
    }
}

fn sprite(frame: &Image, skin: &Skin, name: &str, dx: i32, dy: i32) {
    let def = skin
        .sprite(name)
        .unwrap_or_else(|| panic!("missing sprite {name}"));
    region(
        frame,
        skin,
        def.sheet,
        Rect::new(def.x as i32, def.y as i32, def.width, def.height),
        dx,
        dy,
    );
}

/// Like [`region`], but leaves one rectangle of the destination alone.
///
/// The shaded main window carries its own analyser (S08 §1: 38 x 5 px). Where
/// that area sits in the windowshade is our own choice, so the pixels below
/// it are ours and not the reference's; everything around it still has to
/// match the sheet.
fn region_except(
    frame: &Image,
    skin: &Skin,
    sheet: Sheet,
    src: Rect,
    dx: i32,
    dy: i32,
    skip: Rect,
) {
    let raster = skin.sheet(sheet).unwrap();
    for y in 0..src.height as i32 {
        for x in 0..src.width as i32 {
            let sx = src.x + x;
            let sy = src.y + y;
            let at = Rect::new(dx + x, dy + y, 1, 1);
            if at.x < skip.right()
                && at.right() > skip.x
                && at.y < skip.bottom()
                && at.bottom() > skip.y
            {
                continue;
            }
            assert!(
                sx >= 0 && sy >= 0 && sx < raster.width() as i32 && sy < raster.height() as i32
            );
            let i = ((sy as u32 * raster.width() + sx as u32) * 4) as usize;
            assert_eq!(
                frame.pixel(dx + x, dy + y).unwrap(),
                raster.as_rgba()[i..i + 4],
                "{sheet:?} source {sx},{sy} at {},{}",
                dx + x,
                dy + y
            );
        }
    }
}

#[test]
fn base291_archive_native_sheets_metadata_and_tables() {
    let mut source = archive();
    assert_eq!(source.list().len(), 32);
    let skin = Skin::from_source(&mut source, &BuiltinDefaultSkin).unwrap();
    assert!(skin.warnings().is_empty(), "{:?}", skin.warnings());
    for (sheet, w, h, encoding) in [
        (Sheet::Main, 275, 116, 1),
        (Sheet::Titlebar, 344, 87, 0),
        (Sheet::Cbuttons, 136, 36, 1),
        (Sheet::Shufrep, 92, 85, 1),
        (Sheet::Posbar, 307, 10, 1),
        (Sheet::Volume, 68, 433, 0),
        (Sheet::Balance, 68, 433, 1),
        (Sheet::Monoster, 58, 24, 1),
        (Sheet::Playpaus, 42, 9, 0),
        (Sheet::Numbers, 99, 13, 1),
        (Sheet::Text, 155, 74, 1),
        (Sheet::Pledit, 280, 186, 0),
        (Sheet::EqMain, 275, 315, 1),
        (Sheet::EqEx, 275, 82, 0),
        (Sheet::Gen, 194, 109, 1),
        (Sheet::GenEx, 130, 75, 0),
        (Sheet::Mb, 234, 119, 0),
        (Sheet::Video, 234, 119, 0),
    ] {
        let file = source.read_any(sheet.stem(), &["bmp"]).unwrap();
        assert_eq!(
            u16::from_le_bytes(file.bytes[28..30].try_into().unwrap()),
            8
        );
        assert_eq!(
            u32::from_le_bytes(file.bytes[30..34].try_into().unwrap()),
            encoding
        );
        let image = skin.sheet(sheet).unwrap();
        assert_eq!((image.width(), image.height()), (w, h), "{sheet:?}");
        assert_eq!(image.raw(), file.bytes, "{sheet:?} must not be substituted");
        assert!(
            image
                .as_rgba()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[3] == 255)
        );
    }
    assert_eq!(skin.pledit().normal, [0, 255, 0]);
    assert_eq!(skin.pledit().current, [255; 3]);
    assert_eq!(skin.pledit().normal_bg, [0; 3]);
    assert_eq!(skin.pledit().selected_bg, [0, 0, 198]);
    assert_eq!(
        skin.pledit().font.as_ref().map(|font| font.family.as_str()),
        Some("Arial")
    );
    assert!(skin.region().is_empty()); // All base-skin polygons are comments.
    assert_eq!(skin.viscolors(), &webamp::DEFAULT_VISCOLORS);
    for name in [
        "normal", "close", "eqslid", "eqtitle", "posbar", "psize", "ptbar", "pvscroll", "titlebar",
        "volbal", "pnormal",
    ] {
        let file = source.read_any(name, &["cur"]).unwrap();
        let slot = CursorSlot::from_file_name(&format!("{name}.cur")).unwrap();
        assert_eq!(skin.cursor_slot(slot).unwrap().bytes, file.bytes);
    }
    for def in webamp::SPRITES.iter().filter(|d| d.sheet != Sheet::NumsEx) {
        let (image, resolved) = skin.sprite_image(def.name).unwrap();
        assert_eq!(resolved, def);
        assert!(
            def.x + def.width <= image.width() && def.y + def.height <= image.height(),
            "{} outside real sheet",
            def.name
        );
    }
    assert_eq!(skin.time_digit_sheet(), Sheet::Numbers);
    assert!(
        skin.substitutions()
            .iter()
            .any(|s| s.requested == "nums_ex.bmp" && s.with == Substitute::DefaultSkin)
    );
    assert_ne!(
        skin.sheet_raw(Sheet::NumsEx),
        skin.sheet_raw(Sheet::Numbers),
        "independent sheets must not be aliased"
    );
}

#[test]
fn base291_main_focus_shade_scale_and_pressed_buttons() {
    let skin = skin();
    let layout = MainWindowLayout::CLASSIC;
    for shade in [false, true] {
        for focused in [false, true] {
            let state = MainWindowState {
                windowshade: shade,
                title_bar_focused: focused,
                time_visible: false,
                ..Default::default()
            };
            let frame = render_main_window(&state, &skin, &layout);
            assert_eq!(
                (frame.width(), frame.height()),
                (275, if shade { 14 } else { 116 })
            );
            // The whole title band is the skin's own title-bar cell: the
            // renderer paints nothing over it, neither the windowshade
            // position slider (the skin's shade cell carries the groove
            // itself) nor anything else. The shaded analyser (S08 §1) is our
            // own area and sits in the shaded band, so its rectangle is left
            // out there.
            let skip = if shade {
                layout.shade_visualization
            } else {
                Rect::new(0, 0, 0, 0)
            };
            region_except(
                &frame,
                &skin,
                Sheet::Titlebar,
                Rect::new(
                    27,
                    match (shade, focused) {
                        (false, true) => 0,
                        (false, false) => 15,
                        (true, true) => 29,
                        (true, false) => 42,
                    },
                    275,
                    14,
                ),
                0,
                0,
                skip,
            );
            let double = render_main_window(
                &MainWindowState {
                    double_size: true,
                    ..state
                },
                &skin,
                &layout,
            );
            assert_eq!(double, frame.scaled(2));
        }
    }
    for (button, name) in [
        (TransportButton::Previous, "MAIN_PREVIOUS_BUTTON"),
        (TransportButton::Play, "MAIN_PLAY_BUTTON"),
        (TransportButton::Pause, "MAIN_PAUSE_BUTTON"),
        (TransportButton::Stop, "MAIN_STOP_BUTTON"),
        (TransportButton::Next, "MAIN_NEXT_BUTTON"),
        (TransportButton::Eject, "MAIN_EJECT_BUTTON"),
    ] {
        for held in [false, true] {
            let state = MainWindowState {
                pressed: held.then_some(button),
                ..Default::default()
            };
            let frame = render_main_window(&state, &skin, &layout);
            let rect = match button {
                TransportButton::Previous => layout.previous,
                TransportButton::Play => layout.play,
                TransportButton::Pause => layout.pause,
                TransportButton::Stop => layout.stop,
                TransportButton::Next => layout.next,
                TransportButton::Eject => layout.eject,
            };
            sprite(
                &frame,
                &skin,
                &format!("{name}{}", if held { "_ACTIVE" } else { "" }),
                rect.x,
                rect.y,
            );
        }
    }
    for (button, name) in [
        (WindowButton::Menu, "MAIN_OPTIONS_BUTTON_DEPRESSED"),
        (WindowButton::Minimize, "MAIN_MINIMIZE_BUTTON_DEPRESSED"),
        (WindowButton::Shade, "MAIN_SHADE_BUTTON_DEPRESSED"),
        (WindowButton::Close, "MAIN_CLOSE_BUTTON_DEPRESSED"),
    ] {
        let state = MainWindowState {
            pressed_window_button: Some(button),
            ..Default::default()
        };
        let frame = render_main_window(&state, &skin, &layout);
        let rect = layout.window_button(button);
        sprite(&frame, &skin, name, rect.x, rect.y);
        let shade = render_main_window(
            &MainWindowState {
                windowshade: true,
                ..state
            },
            &skin,
            &layout,
        );
        sprite(
            &shade,
            &skin,
            if button == WindowButton::Shade {
                "MAIN_SHADE_BUTTON_SELECTED_DEPRESSED"
            } else {
                name
            },
            rect.x,
            rect.y,
        );
    }
}

/// The shaded main window against the measured reference capture.
///
/// The capture directory holds the approved client captures of Winamp 2.95
/// (skin base-2.91, 96 dpi, 1024x768 guest). `main-shade.reference.png` is
/// the shaded main window, 275 x 14. This renders the same state headlessly
/// (the same skin, the same size, the same focus) and compares it pixel by
/// pixel. No capture is taken here and no pixel is fabricated: the reference
/// is a file, the other side is this renderer's output.
///
/// The strip matches the reference pixel for pixel: the shaded analyser
/// (S08 §1) has nothing to show, so the area keeps the skin's own art -
/// exactly what the reference shows there (measured 2026-10-21).
#[test]
fn base291_shade_strip_matches_the_reference_capture() {
    let root = std::path::PathBuf::from(
        std::env::var_os("RUSTAMP_BASE291_CAPTURES")
            .unwrap_or_else(|| "D:/code/rustamp-testcaptures/base291".into()),
    );
    let reference_path = root.join("main-shade.reference.png");
    if !reference_path.is_file() {
        eprintln!(
            "SKIP base291_shade_strip_matches_the_reference_capture: the reference capture {} is not available",
            reference_path.display()
        );
        return;
    }
    let skin = skin();
    let layout = MainWindowLayout::CLASSIC;
    let state = MainWindowState {
        windowshade: true,
        title_bar_focused: true,
        ..Default::default()
    };
    let frame = render_main_window(&state, &skin, &layout);
    assert_eq!((frame.width(), frame.height()), (275, 14));
    let (width, height, rgba) = frame.into_raw();
    let ours = rustamp_testkit::RgbaImage::from_raw(width, height, rgba).unwrap();
    let reference = rustamp_testkit::RgbaImage::load_png(&reference_path).unwrap();
    let diff = rustamp_testkit::compare(&reference, &ours, 0);
    assert_eq!(diff.size_mismatch, None, "{}", diff.summary());
    assert_eq!(
        diff.mismatched_pixels,
        0,
        "{}: the shaded main window must match the reference pixel for pixel",
        diff.summary()
    );
}

/// The normal main window against the measured reference capture.
///
/// The capture directory holds the approved client capture of Winamp 2.95
/// (skin base-2.91, 96 dpi, 1024x768 guest). `main-active.reference.png` is
/// the focused main window with nothing loaded, 275 x 116. This renders the
/// same state headlessly (the same skin, the same size, the same focus, the
/// same empty playlist, the same volume) and compares it pixel by pixel. No
/// capture is taken here and no pixel is fabricated: the reference is a file,
/// the other side is this renderer's output.
///
/// The one documented difference is the **program name** in the marquee: the
/// reference shows Winamp's own name (`WINAMP 2.95`), ours shows ours
/// (`RUSTAMP`). The decoder indicator's idle cell is the second one. Every
/// other pixel must match the reference.
#[test]
fn base291_main_window_matches_the_reference_capture() {
    let root = std::path::PathBuf::from(
        std::env::var_os("RUSTAMP_BASE291_CAPTURES")
            .unwrap_or_else(|| "D:/code/rustamp-testcaptures/base291".into()),
    );
    if !root.join("main-active.reference.png").is_file() {
        eprintln!(
            "SKIP base291_main_window_matches_the_reference_capture: the reference captures in {} are not available",
            root.display()
        );
        return;
    }
    let skin = skin();
    let layout = MainWindowLayout::CLASSIC;
    let state = MainWindowState {
        title_bar_focused: true,
        // The capture was taken with the equalizer and the playlist open and
        // with the volume at 0.
        eq_open: true,
        playlist_open: true,
        volume: 0,
        // Nothing is loaded: the time display and the readouts are blank and
        // no position bar is drawn.
        transport: TransportState::Stopped,
        time_visible: false,
        track_loaded: false,
        seekable: false,
        kbps: None,
        khz: None,
        title: PROGRAM_NAME.to_string(),
        ..Default::default()
    };
    // The focused and the unfocused window at 1x, and the double-size one:
    // the double-size window is the exact integer 2x of the 1x frame
    // (S01 §1), so its differences are the same ones, four times as many.
    for (case, double_size, focused) in [
        ("main-active", false, true),
        ("main-inactive", false, false),
        ("main-double", true, true),
    ] {
        let frame = render_main_window(
            &MainWindowState {
                title_bar_focused: focused,
                double_size,
                // The latched clutter letter `D` is lit while the double-size
                // mode is on (S02 §2).
                clutter_d: double_size,
                ..state.clone()
            },
            &skin,
            &layout,
        );
        let scale: i32 = if double_size { 2 } else { 1 };
        assert_eq!(
            (frame.width(), frame.height()),
            (275 * scale as u32, 116 * scale as u32),
            "{case}"
        );
        let (width, height, rgba) = frame.into_raw();
        let ours = rustamp_testkit::RgbaImage::from_raw(width, height, rgba).unwrap();
        let reference =
            rustamp_testkit::RgbaImage::load_png(root.join(format!("{case}.reference.png")))
                .unwrap();
        let diff = rustamp_testkit::compare(&reference, &ours, 0);
        assert_eq!(diff.size_mismatch, None, "{case}: {}", diff.summary());

        // Every pixel that differs must lie in one of the two documented
        // rectangles: the marquee band (the program name) and the decoder
        // indicator's own cell (the idle decoder look, see the lane report).
        let band = layout.marquee;
        let decoder = layout.decoder_indicator;
        let mut in_band = 0u64;
        let mut in_decoder = 0u64;
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                if reference.pixel(x as u32, y as u32) == ours.pixel(x as u32, y as u32) {
                    continue;
                }
                if x >= band.x * scale
                    && x < (band.x + band.width as i32) * scale
                    && y >= band.y * scale
                    && y < (band.y + band.height as i32) * scale
                {
                    in_band += 1;
                    continue;
                }
                assert!(
                    x >= decoder.x * scale
                        && x < (decoder.x + decoder.width as i32) * scale
                        && y >= decoder.y * scale
                        && y < (decoder.y + decoder.height as i32) * scale,
                    "{case}: {x},{y} differs from the reference but is neither the marquee band nor the decoder cell: {}",
                    diff.summary()
                );
                in_decoder += 1;
            }
        }
        assert_eq!(
            in_band,
            120 * scale as u64 * scale as u64,
            "{case}: the marquee must differ in exactly the program-name glyphs: {}",
            diff.summary()
        );
        assert_eq!(
            in_decoder,
            12 * scale as u64 * scale as u64,
            "{case}: the decoder cell must differ in exactly the idle indicator: {}",
            diff.summary()
        );
        assert_eq!(
            diff.mismatched_pixels,
            in_band + in_decoder,
            "{case}: {}",
            diff.summary()
        );
        eprintln!(
            "{case}: {} differing pixels ({in_band} in the marquee band, {in_decoder} in the decoder cell)",
            diff.mismatched_pixels
        );
    }
}

#[test]
fn base291_main_readouts_marquee_clutter_toggles_and_sliders() {
    let skin = skin();
    let layout = MainWindowLayout::CLASSIC;
    for (transport, name) in [
        (TransportState::Playing, "MAIN_PLAYING_INDICATOR"),
        (TransportState::Paused, "MAIN_PAUSED_INDICATOR"),
        (TransportState::Stopped, "MAIN_STOPPED_INDICATOR"),
    ] {
        for value in [0, 50, 100] {
            let state = MainWindowState {
                transport,
                volume: value,
                balance: value,
                position: value,
                // A loaded track: the position bar is drawn only then.
                track_loaded: true,
                title: "ABC".into(),
                kbps: Some(192),
                khz: Some(44),
                time_seconds: 12 * 60 + 34,
                time_remaining: true,
                shuffle: true,
                repeat: true,
                eq_open: true,
                playlist_open: true,
                clutter_o: true,
                clutter_a: true,
                clutter_i: true,
                clutter_d: true,
                clutter_v: true,
                ..Default::default()
            };
            let frame = render_main_window(&state, &skin, &layout);
            sprite(&frame, &skin, name, 26, 28);
            for (digit, rect) in [1, 2, 3, 4].into_iter().zip(layout.time_digits) {
                sprite(&frame, &skin, &format!("DIGIT_{digit}"), rect.x, rect.y);
            }
            sprite(&frame, &skin, "MINUS_SIGN", 38, 32);
            region(&frame, &skin, Sheet::Text, Rect::new(0, 0, 15, 6), 111, 27);
            region(&frame, &skin, Sheet::Text, Rect::new(5, 6, 5, 6), 111, 43);
            region(&frame, &skin, Sheet::Text, Rect::new(20, 6, 5, 6), 156, 43);
            for (letter, rect) in [
                ('O', layout.clutter_o),
                ('A', layout.clutter_a),
                ('I', layout.clutter_i),
                ('D', layout.clutter_d),
                ('V', layout.clutter_v),
            ] {
                sprite(
                    &frame,
                    &skin,
                    &format!("MAIN_CLUTTER_BAR_BUTTON_{letter}_SELECTED"),
                    rect.x,
                    rect.y,
                );
            }
            // Shuffle's last column overlaps Repeat at x210; compare its visible portion.
            region(
                &frame,
                &skin,
                Sheet::Shufrep,
                Rect::new(28, 30, 46, 15),
                164,
                89,
            );
            for (name, rect) in [
                ("MAIN_REPEAT_BUTTON_SELECTED", layout.repeat),
                ("MAIN_EQ_BUTTON_SELECTED", layout.eq_toggle),
                ("MAIN_PLAYLIST_BUTTON_SELECTED", layout.playlist_toggle),
            ] {
                sprite(&frame, &skin, name, rect.x, rect.y);
            }
            for (name, rect) in [
                ("MAIN_VOLUME_THUMB", layout.volume_thumb_at(value)),
                ("MAIN_BALANCE_THUMB", layout.balance_thumb_at(value)),
                (
                    "MAIN_POSITION_SLIDER_THUMB",
                    layout.position_thumb_at(value),
                ),
            ] {
                sprite(&frame, &skin, name, rect.x, rect.y);
            }
            let scrolled = render_main_window(
                &MainWindowState {
                    marquee_offset: 5,
                    ..state
                },
                &skin,
                &layout,
            );
            region(
                &scrolled,
                &skin,
                Sheet::Text,
                Rect::new(5, 0, 10, 6),
                111,
                27,
            );
        }
    }
}

#[test]
fn base291_equalizer_focus_shade_scale_buttons_thumbs_and_curve() {
    let skin = skin();
    let layout = EqualizerLayout::CLASSIC;
    for shade in [false, true] {
        for focused in [false, true] {
            let state = EqualizerState {
                windowshade: shade,
                focused,
                ..Default::default()
            };
            let frame = render_equalizer_window(&state, &skin, &layout);
            region(
                &frame,
                &skin,
                if shade { Sheet::EqEx } else { Sheet::EqMain },
                Rect::new(
                    0,
                    if shade {
                        if focused { 0 } else { 15 }
                    } else if focused {
                        134
                    } else {
                        149
                    },
                    275,
                    14,
                ),
                0,
                0,
            );
            assert_eq!(
                render_equalizer_window(
                    &EqualizerState {
                        double_size: true,
                        ..state
                    },
                    &skin,
                    &layout
                ),
                frame.scaled(2)
            );
        }
    }
    for engaged in [false, true] {
        for held in [false, true] {
            for widget in [EqWidget::On, EqWidget::Auto, EqWidget::Presets] {
                let state = EqualizerState {
                    enabled: engaged,
                    auto: engaged,
                    held: held.then_some(widget),
                    ..Default::default()
                };
                let frame = render_equalizer_window(&state, &skin, &layout);
                let (src, dx) = match widget {
                    EqWidget::On => (
                        Rect::new(
                            match (engaged, held) {
                                (false, false) => 10,
                                (true, false) => 69,
                                (false, true) => 128,
                                (true, true) => 187,
                            },
                            119,
                            25,
                            12,
                        ),
                        14,
                    ),
                    EqWidget::Auto => (
                        Rect::new(
                            match (engaged, held) {
                                (false, false) => 35,
                                (true, false) => 94,
                                (false, true) => 153,
                                (true, true) => 212,
                            },
                            119,
                            33,
                            12,
                        ),
                        39,
                    ),
                    _ => (Rect::new(224, if held { 176 } else { 164 }, 44, 12), 217),
                };
                region(&frame, &skin, Sheet::EqMain, src, dx, 18);
            }
        }
    }
    for value in [0, 31, 63] {
        for slider in EqSlider::ALL_BANDS {
            for held in [false, true] {
                let state = EqualizerState {
                    bands: [value; 10],
                    preamp: value,
                    held: held.then_some(EqWidget::Slider(slider)),
                    thumb_dragged: held,
                    ..Default::default()
                };
                let frame = render_equalizer_window(&state, &skin, &layout);
                let rect = EqualizerLayout::thumb_rect(slider, value);
                region(
                    &frame,
                    &skin,
                    Sheet::EqMain,
                    Rect::new(0, if held { 176 } else { 164 }, 11, 11),
                    rect.x,
                    rect.y,
                );
                // The track is one 14 x 63 sheet column, drawn one pixel left
                // of the thumb: the measured reference capture's column. The
                // thumb covers its middle, so that part is left alone.
                let track = EqualizerLayout::track_rect(slider);
                if value == 31 {
                    // The flat value's column is the measured one.
                    region_except(
                        &frame,
                        &skin,
                        Sheet::EqMain,
                        Rect::new(13, 229, 14, 63),
                        track.x,
                        track.y,
                        rect,
                    );
                }
                // The body colour is the sheet's colour scale: red at the
                // most boost value, green at the most cut one (S01 §3.1).
                let body = frame.pixel(rect.x + 5, 50).unwrap();
                let scale = skin.sheet(Sheet::EqMain).unwrap();
                let scale = rustamp_gfx::Image::from_rgba(
                    scale.width(),
                    scale.height(),
                    scale.as_rgba().to_vec(),
                )
                .unwrap();
                let expected = match value {
                    0 => scale.pixel(115, 294),
                    63 => scale.pixel(115, 294 + 18),
                    _ => continue,
                };
                assert_eq!(Some(body), expected, "the track body colour of {value}");
                // The curve's row: the flat value lies on the graph's centre
                // row, the ends on the outermost ones.
                let row = match value {
                    0 => 0,
                    31 => 9,
                    _ => 18,
                };
                region(
                    &frame,
                    &skin,
                    Sheet::EqMain,
                    Rect::new(115, 294 + row, 1, 1),
                    88,
                    17 + row,
                );
            }
        }
    }
}

#[test]
fn base291_playlist_focus_buttons_scroll_and_skin_colours() {
    let skin = skin();
    let mut window = PlaylistWindow::new();
    window.set_entries((0..60).map(EntryId).collect());
    window.apply_select(&SelectOp::Set(vec![EntryId(0)]));
    for focused in [false, true] {
        for offset in [0, 28, 56] {
            window.scroll_to(offset);
            let frame = render_playlist_window(&skin, &window, &[], focused);
            assert_eq!((frame.width(), frame.height()), (275, 116));
            let y = if focused { 0 } else { 21 };
            region(&frame, &skin, Sheet::Pledit, Rect::new(0, y, 25, 20), 0, 0);
            region(
                &frame,
                &skin,
                Sheet::Pledit,
                Rect::new(26, y, 100, 20),
                87,
                0,
            );
            for (dx, width) in [
                (25, 12),
                (37, 25),
                (62, 25),
                (187, 13),
                (200, 25),
                (225, 25),
            ] {
                region(
                    &frame,
                    &skin,
                    Sheet::Pledit,
                    Rect::new(127, y, width, 20),
                    dx,
                    0,
                );
            }
            region(
                &frame,
                &skin,
                Sheet::Pledit,
                SCROLL_HANDLE_SRC,
                260,
                scroll_handle_y(offset, 60, ROW_PITCH),
            );
            region(&frame, &skin, Sheet::Pledit, SCROLL_ARROWS_SRC, 260, 78);
            for button in PlaylistButton::ALL {
                let r = button_rect(*button);
                region(
                    &frame,
                    &skin,
                    Sheet::Pledit,
                    Rect::new(
                        if *button == PlaylistButton::ListOpts {
                            229
                        } else {
                            r.x
                        },
                        86,
                        r.width,
                        r.height,
                    ),
                    r.x,
                    r.y,
                );
            }
            if offset == 0 {
                assert_eq!(frame.pixel(12, 23), Some([0, 0, 198, 255]));
            }
        }
    }
}

#[test]
fn base291_missing_sheet_is_local_documented_fallback() {
    struct WithoutTitle(ArchiveSource);
    impl SkinSource for WithoutTitle {
        fn read_any(&mut self, base: &str, exts: &[&str]) -> Option<rustamp_skin::SkinFile> {
            if base == "titlebar" {
                None
            } else {
                self.0.read_any(base, exts)
            }
        }
        fn list(&self) -> Vec<String> {
            self.0
                .list()
                .into_iter()
                .filter(|s| !s.eq_ignore_ascii_case("titlebar.bmp"))
                .collect()
        }
        fn describe(&self) -> String {
            "base-2.91 without titlebar".into()
        }
    }
    let full = skin();
    let missing = Skin::from_source(&mut WithoutTitle(archive()), &BuiltinDefaultSkin).unwrap();
    assert!(
        missing
            .substitutions()
            .iter()
            .any(|s| s.requested == "titlebar.bmp" && s.with == Substitute::DefaultSkin)
    );
    for sheet in Sheet::ALL.iter().filter(|s| **s != Sheet::Titlebar) {
        assert_eq!(full.sheet_raw(*sheet), missing.sheet_raw(*sheet));
    }
    let frame = render_main_window(
        &MainWindowState::default(),
        &missing,
        &MainWindowLayout::CLASSIC,
    );
    sprite(&frame, &missing, "MAIN_TITLE_BAR_SELECTED", 0, 0);
    sprite(&frame, &full, "MAIN_PLAY_BUTTON", 39, 88);
    assert!(missing.warnings().is_empty());
}

#[test]
fn base291_shipped_extended_digits_are_selected_without_sheet_aliases() {
    struct WithExtended {
        base: ArchiveSource,
        digits: rustamp_skin::SkinFile,
        omit_numbers: bool,
    }
    impl SkinSource for WithExtended {
        fn read_any(&mut self, base: &str, exts: &[&str]) -> Option<rustamp_skin::SkinFile> {
            if base == "nums_ex" {
                Some(self.digits.clone())
            } else if base == "numbers" && self.omit_numbers {
                None
            } else {
                self.base.read_any(base, exts)
            }
        }
        fn list(&self) -> Vec<String> {
            let mut names = self.base.list();
            names.push("nums_ex.bmp".into());
            names
        }
        fn describe(&self) -> String {
            "base-2.91 with independent extended digits".into()
        }
    }
    let original = skin();
    let digits = BuiltinDefaultSkin::source()
        .read_any("nums_ex", &["bmp"])
        .unwrap();
    for omit_numbers in [false, true] {
        let skin = Skin::from_source(
            &mut WithExtended {
                base: archive(),
                digits: digits.clone(),
                omit_numbers,
            },
            &BuiltinDefaultSkin,
        )
        .unwrap();
        assert_eq!(skin.time_digit_sheet(), Sheet::NumsEx);
        assert_eq!(skin.sheet_raw(Sheet::NumsEx), Some(digits.bytes.as_slice()));
        if !omit_numbers {
            assert_eq!(
                skin.sheet_raw(Sheet::Numbers),
                original.sheet_raw(Sheet::Numbers)
            );
        }
        assert_ne!(
            skin.sheet_raw(Sheet::Numbers),
            skin.sheet_raw(Sheet::NumsEx)
        );
        let frame = render_main_window(
            &MainWindowState {
                time_seconds: 12 * 60 + 34,
                ..Default::default()
            },
            &skin,
            &MainWindowLayout::CLASSIC,
        );
        for (digit, rect) in [1, 2, 3, 4]
            .into_iter()
            .zip(MainWindowLayout::CLASSIC.time_digits)
        {
            sprite(&frame, &skin, &format!("DIGIT_{digit}_EX"), rect.x, rect.y);
        }
    }
}

#[test]
fn base291_held_toggles_and_slider_sprites_follow_real_mouse_state() {
    let skin = skin();
    let layout = MainWindowLayout::CLASSIC;
    for (widget, name, rect) in [
        (
            Widget::Shuffle,
            "MAIN_SHUFFLE_BUTTON_DEPRESSED",
            layout.shuffle,
        ),
        (
            Widget::Repeat,
            "MAIN_REPEAT_BUTTON_DEPRESSED",
            layout.repeat,
        ),
        (
            Widget::EqToggle,
            "MAIN_EQ_BUTTON_DEPRESSED",
            layout.eq_toggle,
        ),
        (
            Widget::PlaylistToggle,
            "MAIN_PLAYLIST_BUTTON_DEPRESSED",
            layout.playlist_toggle,
        ),
        (
            Widget::VolumeSlider,
            "MAIN_VOLUME_THUMB_SELECTED",
            layout.volume_track,
        ),
        (
            Widget::BalanceSlider,
            "MAIN_BALANCE_THUMB_ACTIVE",
            layout.balance_track,
        ),
    ] {
        let mut machine = MainWindow::new();
        let x = rect.x + rect.width as i32 / 2;
        let y = rect.y + rect.height as i32 / 2;
        machine.mouse_down(x, y, 0);
        assert_eq!(machine.state().pressed_widget, Some(widget));
        let frame = render_main_window(machine.state(), &skin, &layout);
        let dst = match widget {
            Widget::VolumeSlider => layout.volume_thumb_at(machine.state().volume),
            Widget::BalanceSlider => layout.balance_thumb_at(machine.state().balance),
            _ => rect,
        };
        // Shuffle's last column is shared with Repeat.
        if widget == Widget::Shuffle {
            region(
                &frame,
                &skin,
                Sheet::Shufrep,
                Rect::new(28, 15, 46, 15),
                rect.x,
                rect.y,
            );
        } else {
            sprite(&frame, &skin, name, dst.x, dst.y);
        }
        machine.mouse_up(x, y, 1);
        assert_eq!(machine.state().pressed_widget, None);
    }
    for engaged in [false, true] {
        for widget in [
            Widget::Shuffle,
            Widget::Repeat,
            Widget::EqToggle,
            Widget::PlaylistToggle,
        ] {
            let state = MainWindowState {
                shuffle: engaged,
                repeat: engaged,
                eq_open: engaged,
                playlist_open: engaged,
                pressed_widget: Some(widget),
                ..Default::default()
            };
            let frame = render_main_window(&state, &skin, &layout);
            let (x, src_x, y, width, height) = match widget {
                Widget::Shuffle => (164, 28, if engaged { 45 } else { 15 }, 46, 15),
                Widget::Repeat => (210, 0, if engaged { 45 } else { 15 }, 28, 15),
                Widget::EqToggle => (219, 46, if engaged { 73 } else { 61 }, 23, 12),
                _ => (242, 69, if engaged { 73 } else { 61 }, 23, 12),
            };
            region(
                &frame,
                &skin,
                Sheet::Shufrep,
                Rect::new(src_x, y, width, height),
                x,
                if height == 15 { 89 } else { 58 },
            );
        }
    }
}

#[test]
fn base291_playlist_shade_and_held_title_controls_use_pledit() {
    let skin = skin();
    for shade in [false, true] {
        for focused in [false, true] {
            let mut window = PlaylistWindow::new();
            window.windowshade = shade;
            let frame = render_playlist_window(&skin, &window, &[], focused);
            assert_eq!(
                (frame.width(), frame.height()),
                (275, if shade { 14 } else { 116 })
            );
            if shade {
                // S01 §4 / measured: the left corner in both focus states, the
                // borderless body tile, then the right corner from the focused
                // or the unfocused row.
                region(&frame, &skin, Sheet::Pledit, SHADE_CORNER_SRC, 0, 0);
                for x in (SHADE_BODY_SRC.width as i32..SHADE_RIGHT_X)
                    .step_by(SHADE_BODY_SRC.width as usize)
                {
                    // The last body tile runs under the right corner, which is
                    // drawn after it; only its visible columns are asserted.
                    let width = (SHADE_RIGHT_X - x).min(SHADE_BODY_SRC.width as i32) as u32;
                    region(
                        &frame,
                        &skin,
                        Sheet::Pledit,
                        Rect::new(
                            SHADE_BODY_SRC.x,
                            SHADE_BODY_SRC.y,
                            width,
                            SHADE_BODY_SRC.height,
                        ),
                        x,
                        0,
                    );
                }
                region(
                    &frame,
                    &skin,
                    Sheet::Pledit,
                    if focused {
                        SHADE_RIGHT_FOCUSED_SRC
                    } else {
                        SHADE_RIGHT_SRC
                    },
                    SHADE_RIGHT_X,
                    0,
                );
            }
            // S01 §1: playlist ignores the player's double-size mode.
            assert_eq!(
                window_size(WindowKind::Playlist, shade, false),
                window_size(WindowKind::Playlist, shade, true)
            );
            for button in [PlaylistTitleButton::Close, PlaylistTitleButton::Shade] {
                window.pressed_title_button = Some(button);
                let frame = render_playlist_window(&skin, &window, &[], focused);
                let (name, rect) = match button {
                    PlaylistTitleButton::Close => ("PLAYLIST_CLOSE_SELECTED", CLOSE_BUTTON_RECT),
                    PlaylistTitleButton::Shade => (
                        if shade {
                            "PLAYLIST_EXPAND_SELECTED"
                        } else {
                            "PLAYLIST_COLLAPSE_SELECTED"
                        },
                        SHADE_BUTTON_RECT,
                    ),
                };
                sprite(&frame, &skin, name, rect.x, rect.y);
            }
        }
    }
}

fn memory_skin(font: &str, replace_text: bool) -> Skin {
    let mut original = archive();
    let mut memory = rustamp_skin::MemorySource::new("base-2.91 font control");
    for name in original.list() {
        let (stem, ext) = name.rsplit_once('.').unwrap();
        if replace_text && stem.eq_ignore_ascii_case("text") {
            continue;
        }
        let file = original.read_any(stem, &[ext]).unwrap();
        memory.add(name, file.bytes);
    }
    memory.add("pledit.txt",format!("[Text]\nNormal=#00FF00\nCurrent=#FFFFFF\nNormalBG=#000000\nSelectedBG=#0000C6\nFont={font}\n").into_bytes());
    Skin::from_source(&mut memory, &BuiltinDefaultSkin).unwrap()
}

/// Asserts that a playlist row holds nothing but the skin's ink and
/// background colours, and that both of them reach the row.
fn assert_row_is_pure(frame: &Image, i: usize, pitch: i32, ink: [u8; 3], bg: [u8; 3]) {
    let row = row_rect(i, pitch).unwrap();
    let ink = [ink[0], ink[1], ink[2], 255];
    let bg = [bg[0], bg[1], bg[2], 255];
    let mut has_ink = false;
    let mut has_bg = false;
    for y in row.y..row.bottom() {
        for x in row.x..row.right() {
            let p = frame.pixel(x, y).unwrap();
            assert!(p == ink || p == bg, "row {i}: off-palette pixel {p:?}");
            has_bg |= p == bg;
            has_ink |= p == ink;
        }
    }
    assert!(
        has_bg && has_ink,
        "skin text and background colours must both reach row {i}"
    );
}

#[test]
fn base291_playlist_font_is_the_requested_system_family_not_text_bmp() {
    let skin = skin();
    let different_bitmap = memory_skin("Arial", true);
    let courier = memory_skin("Courier New", false);
    let mut window = PlaylistWindow::new();
    window.set_entries(vec![EntryId(1), EntryId(2)]);
    window.set_current(Some(EntryId(1)));
    window.apply_select(&SelectOp::Set(vec![EntryId(2)]));
    let labels = vec!["1. Mixed Case WWW iii".into(), "2. Track Title".into()];
    let render = |skin: &Skin| render_playlist_window(skin, &window, &labels, true);
    let frame = render(&skin);
    assert_eq!(
        frame,
        render(&different_bitmap),
        "the playlist must not depend on text.bmp glyph shapes"
    );
    assert_ne!(
        frame,
        render(&courier),
        "Font= must change actual glyph shapes"
    );
    for (i, ink, bg) in [(0, [255u8; 3], [0u8; 3]), (1, [0, 255, 0], [0, 0, 198])] {
        assert_row_is_pure(&frame, i, ROW_PITCH, ink, bg);
    }
}

/// S17 §5 settles the family name of the `Font=` line; the size and the
/// style flags are read from the same line (see `rustamp_skin::pledit`).
#[test]
fn base291_playlist_font_size_comes_from_the_pledit_line() {
    let mut window = PlaylistWindow::new();
    window.set_entries(vec![EntryId(1), EntryId(2)]);
    let labels = vec!["1. Mixed Case WWW iii".into(), "2. Track Title".into()];

    // The size the skin asks for is the size the editor draws with: the app
    // reads it with `skin_font_size` and pushes it into the model, which owns
    // the runtime size (S03 §2 `40335` / `40336`).
    let plain_skin = memory_skin("Courier New", false);
    window.set_skin_font_size(skin_font_size(&plain_skin));
    let plain = render_playlist_window(&plain_skin, &window, &labels, true);
    let smaller_skin = memory_skin("Courier New,10", false);
    window.set_skin_font_size(skin_font_size(&smaller_skin));
    let smaller = render_playlist_window(&smaller_skin, &window, &labels, true);
    assert_ne!(
        plain, smaller,
        "the size in the Font= line must change the row"
    );
    // The 1-bit contract survives the new font: a row still holds the
    // skin's ink and its background, nothing blended.
    assert_row_is_pure(&plain, 0, ROW_PITCH, [0x00, 0xFF, 0x00], [0, 0, 0]);
    assert_row_is_pure(&smaller, 0, 10, [0x00, 0xFF, 0x00], [0, 0, 0]);
}

/// A family the host does not have must not change the picture: the
/// fallback chain ends at the default family.
#[test]
fn base291_playlist_font_falls_back_when_the_family_is_missing() {
    let mut window = PlaylistWindow::new();
    window.set_entries(vec![EntryId(1)]);
    let labels = vec!["1. Mixed Case WWW iii".into()];
    let render = |skin: &Skin| render_playlist_window(skin, &window, &labels, true);
    assert_eq!(
        render(&memory_skin("Rustamp missing family", false)),
        render(&memory_skin("Arial", false)),
        "a family the host does not have must fall back to the default one"
    );
}

/// The playlist editor against the measured reference captures.
///
/// The capture directory holds the approved client captures of Winamp 2.95
/// (skin base-2.91, 96 dpi, 1024x768 guest): `playlist-active` and
/// `playlist-inactive` are the 275 x 116 window with the focused and the
/// unfocused title band, `playlist-shade` is the 275 x 14 windowshade band in
/// the **unfocused** variant (its right corner carries the dim window
/// buttons, `PLEDIT.BMP` row 57, where the focused band takes row 42). All
/// three show an empty list. This renders the same states headlessly and
/// compares them pixel by pixel. No capture is taken here and no pixel is
/// fabricated: the references are files, the other side is this renderer.
///
/// Two rectangles in the references are **text the reference draws and the
/// skin does not carry**: it is painted by GDI in (0,226,0) with a 6 px
/// system font, six pixels high, at (6,4)-(48,9) in the shade band (the
/// window's label) and at (132,88)-(175,93) in the bottom bar. Our renderer
/// draws no such text: the string of the bottom-bar line is unread and the
/// raster is GDI's hinting, which the notes already record as its own case for
/// the shaded equalizer's label. So those two rectangles are named and
/// excluded, and the assertion is the sharp one: **every** pixel that differs
/// is reference text ink, and every one of the reference's ink pixels differs.
/// Nothing else may differ, which is what pins the title cells, the row band,
/// the five buttons, the scrollbar (its 8 x 18 handle cell) and the shade
/// band's corner, body and right cells.
#[test]
fn base291_playlist_matches_the_reference_captures() {
    let root = std::path::PathBuf::from(
        std::env::var_os("RUSTAMP_BASE291_CAPTURES")
            .unwrap_or_else(|| "D:/code/rustamp-testcaptures/base291".into()),
    );
    if !root.join("playlist-active.reference.png").is_file() {
        eprintln!(
            "SKIP base291_playlist_matches_the_reference_captures: the reference captures in {} are not available",
            root.display()
        );
        return;
    }
    /// The reference's system-font ink (GDI text the skin does not carry).
    const TEXT_INK: [u8; 4] = [0, 226, 0, 255];
    let skin = skin();
    let mut window = PlaylistWindow::new();
    for (case, shade, focused, text) in [
        ("playlist-active", false, true, Rect::new(132, 88, 44, 6)),
        ("playlist-inactive", false, false, Rect::new(132, 88, 44, 6)),
        ("playlist-shade", true, false, Rect::new(6, 4, 43, 6)),
    ] {
        window.windowshade = shade;
        let frame = render_playlist_window(&skin, &window, &[], focused);
        let (width, height, rgba) = frame.into_raw();
        let ours = rustamp_testkit::RgbaImage::from_raw(width, height, rgba).unwrap();
        let reference =
            rustamp_testkit::RgbaImage::load_png(root.join(format!("{case}.reference.png")))
                .unwrap();
        let diff = rustamp_testkit::compare(&reference, &ours, 0);
        assert_eq!(diff.size_mismatch, None, "{case}: {}", diff.summary());
        let mut ink = 0;
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                let wanted = reference.pixel(x as u32, y as u32);
                let ink_here = wanted == TEXT_INK
                    && x >= text.x
                    && x < text.right()
                    && y >= text.y
                    && y < text.bottom();
                if ink_here {
                    ink += 1;
                }
                if ours.pixel(x as u32, y as u32) == wanted {
                    assert!(!ink_here, "{case}: {x},{y} must differ from the reference");
                } else {
                    assert!(
                        ink_here,
                        "{case}: {x},{y} differs from the reference outside the reference's system-font text: {}",
                        diff.summary()
                    );
                }
            }
        }
        assert!(
            ink > 0,
            "{case}: the reference's text ink at {text:?} was not found"
        );
        assert_eq!(
            diff.mismatched_pixels,
            u64::try_from(ink).unwrap(),
            "{case}: only the reference's system-font text may differ: {}",
            diff.summary()
        );
    }
}

/// The equalizer against the measured reference captures.
///
/// The capture directory holds the approved client captures of Winamp 2.95
/// (skin base-2.91, 96 dpi, 1024x768 guest): `eq-active.reference.png` is the
/// equalizer while it holds the focus, `eq-inactive.reference.png` the same
/// window while the main window holds the focus and `eq-double.reference.png`
/// the window at double size. All three show a flat equalizer with the EQ
/// engaged and the AUTO button engaged. This renders the same states
/// headlessly (the same skin, the same sizes, the same focus) and compares
/// them pixel by pixel. No capture is taken here and no pixel is fabricated:
/// the references are files, the other side is this renderer's output.
#[test]
fn base291_equalizer_matches_the_reference_captures() {
    let root = std::path::PathBuf::from(
        std::env::var_os("RUSTAMP_BASE291_CAPTURES")
            .unwrap_or_else(|| "D:/code/rustamp-testcaptures/base291".into()),
    );
    if !root.join("eq-active.reference.png").is_file() {
        eprintln!(
            "SKIP base291_equalizer_matches_the_reference_captures: the reference captures in {} are not available",
            root.display()
        );
        return;
    }
    let skin = skin();
    let layout = EqualizerLayout::CLASSIC;
    for (case, focused, double_size) in [
        ("eq-active", true, false),
        ("eq-inactive", false, false),
        ("eq-double", true, true),
    ] {
        let state = EqualizerState {
            focused,
            double_size,
            enabled: true,
            auto: true,
            ..Default::default()
        };
        let frame = render_equalizer_window(&state, &skin, &layout);
        let (width, height, rgba) = frame.into_raw();
        let ours = rustamp_testkit::RgbaImage::from_raw(width, height, rgba).unwrap();
        let reference =
            rustamp_testkit::RgbaImage::load_png(root.join(format!("{case}.reference.png")))
                .unwrap();
        let diff = rustamp_testkit::compare(&reference, &ours, 0);
        assert_eq!(diff.size_mismatch, None, "{case}: {}", diff.summary());
        assert_eq!(
            diff.mismatched_pixels,
            0,
            "{case}: the equalizer must match the reference capture pixel for pixel: {}",
            diff.summary()
        );
    }
}

/// Requires real VM screenshots captured by the interactive-session runner.
/// The capture directory holds paired <case>.presented.png (Rustamp client
/// capture) and <case>.reference.png (approved reference client capture).
/// No screenshots or reference pixels are fabricated by this test.
#[test]
#[ignore = "VM-only: requires actual interactive-session client captures and approved reference captures"]
fn base291_vm_presented_pixels_match_reference_captures() {
    assert!(
        std::path::Path::new(r"C:\rustamp\code\Rustamp").is_dir(),
        "VM-only: the runner's staged guest tree is required; no host override"
    );
    let root = std::path::PathBuf::from(
        std::env::var_os("RUSTAMP_BASE291_VM_CAPTURES").expect("VM capture directory required"),
    );
    assert!(
        root.join("session-proof.json").is_file(),
        "interactive-session proof must accompany captures"
    );
    for case in [
        "main-active",
        "main-inactive",
        "main-shade",
        "main-double",
        "eq-active",
        "eq-inactive",
        "eq-shade",
        "eq-double",
        "playlist-active",
        "playlist-inactive",
        "playlist-shade",
    ] {
        let presented =
            rustamp_testkit::RgbaImage::load_png(root.join(format!("{case}.presented.png")))
                .unwrap();
        let reference =
            rustamp_testkit::RgbaImage::load_png(root.join(format!("{case}.reference.png")))
                .unwrap();
        let diff = rustamp_testkit::compare(&reference, &presented, 0);
        assert!(diff.is_match(), "{case}: {}", diff.summary());
    }
}

#[test]
fn base291_slider_frames_blank_readouts_and_visualization_palette() {
    use rustamp_ui::visualization::{SPECTRUM_BANDS, VisInput, VisMode, VisState, slots};
    let skin = skin();
    let layout = MainWindowLayout::CLASSIC;
    for (value, volume_frame, balance_frame) in [(0, 0, 27), (50, 13, 0), (100, 27, 27)] {
        let state = MainWindowState {
            volume: value,
            balance: value,
            time_visible: false,
            ..Default::default()
        };
        let frame = render_main_window(&state, &skin, &layout);
        region(
            &frame,
            &skin,
            Sheet::Volume,
            Rect::new(0, volume_frame * 15, 68, 1),
            107,
            57,
        );
        region(
            &frame,
            &skin,
            Sheet::Balance,
            Rect::new(9, balance_frame * 15, 38, 1),
            177,
            57,
        );
        for rect in layout.time_digits {
            region(&frame, &skin, Sheet::Main, rect, rect.x, rect.y);
        }
        for rect in [layout.kbps, layout.khz] {
            region(&frame, &skin, Sheet::Main, rect, rect.x, rect.y);
        }
        assert!(
            !frame
                .as_rgba()
                .as_chunks::<4>()
                .0
                .contains(&[0, 198, 255, 255]),
            "unblitted atlas filler must not leak into the main window"
        );
    }
    let rgba = |slot: usize| {
        let [r, g, b] = skin.viscolors()[slot];
        [r, g, b, 255]
    };
    for mode in [
        VisMode::Spectrum,
        VisMode::Scope,
        VisMode::ScopeDots,
        VisMode::Off,
    ] {
        let state = MainWindowState {
            visualization: VisState {
                mode,
                bars: [16; SPECTRUM_BANDS],
                scope: [8; 76],
                input: VisInput::Fresh,
                ..Default::default()
            },
            ..Default::default()
        };
        let frame = render_main_window(&state, &skin, &layout);
        let area = layout.visualization;
        match mode {
            VisMode::Spectrum => {
                // S17 §6: slot 2 at the top of a bar, slot 17 at the bottom.
                for row in 0..16 {
                    assert_eq!(
                        frame.pixel(area.x, area.y + 15 - row),
                        Some(rgba(slots::SPECTRUM_BOTTOM - row as usize))
                    );
                }
            }
            VisMode::Scope | VisMode::ScopeDots => {
                // The scope sits on the centre line, which wears the
                // brightest of the five oscilloscope colours.
                assert_eq!(
                    frame.pixel(area.x, area.y + 8),
                    Some(rgba(slots::OSCILLOSCOPE_FIRST))
                )
            }
            VisMode::Off => assert_eq!(frame.pixel(area.x, area.y), Some(rgba(slots::BACKGROUND))),
        }
    }
}
