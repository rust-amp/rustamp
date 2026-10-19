// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The equalizer window in its windowshade form (S01 §1/§3, S05 §1.2/§1.5).
//!
//! Everything here is headless: frames are painted into image buffers and the
//! behaviour model is driven with plain coordinates. No window, no presenter,
//! no device and no child process.
//!
//! What the specs settle, and what this file pins:
//!
//! * **The size.** S01 §1: the windowshade height is 14 px for the main
//!   window, the equalizer and the playlist editor, so the shaded equalizer
//!   is 275 x 14. The double-size form is the scaled shade, 550 x 28 (an own
//!   choice for the equalizer; S05 §2.3 measured the combination for the main
//!   window only).
//! * **The pixels.** S01 §3 (provisional): the shaded band comes from
//!   `eq_ex.bmp`, row 0 focused and row 15 unfocused. S05 §1.2 settles for
//!   the windowshade in general that it is "a single 14-px title-bar row: no
//!   body rows are drawn" and that the strip is **not** a crop of the
//!   unshaded title band.
//! * **The input.** The On/Auto/Presets buttons and the eleven sliders
//!   belong to the body, so a shaded equalizer neither draws nor answers
//!   them. S07 has no shade wording at all, so nothing in the audited specs
//!   shows sliders in this form: the shaded window answers its title band
//!   alone.

use rustamp_gfx::Image;
use rustamp_skin::{BuiltinDefaultSkin, BuiltinLookalikeSkin, Sheet, Skin};
use rustamp_ui::equalizer::{EqSlider, EqWidget, hit_test};
use rustamp_ui::{
    EqualizerLayout, EqualizerState, EqualizerWindow, WindowKind, eq_frame_size,
    render_equalizer_window, window_size,
};

/// The built-in skins, each with its own `eq_ex.bmp`.
fn skins() -> Vec<Skin> {
    let mut lookalike = BuiltinLookalikeSkin::source();
    let mut rust_theme = BuiltinDefaultSkin::source();
    vec![
        Skin::from_source(&mut lookalike, &BuiltinLookalikeSkin)
            .expect("the look-alike skin loads"),
        Skin::from_source(&mut rust_theme, &BuiltinDefaultSkin).expect("the rust theme loads"),
    ]
}

/// The `eq_ex.bmp` band a shaded equalizer draws (S01 §3, provisional).
fn shade_band(skin: &Skin, focused: bool) -> Image {
    let sheet = sheet_image(skin, Sheet::EqEx);
    let row = if focused { 0 } else { 15 };
    let mut band = Image::filled(275, 14, [0, 0, 0, 255]);
    band.blit_rect(&sheet, rustamp_gfx::Rect::new(0, row, 275, 14), 0, 0);
    band
}

/// A skin sheet as an image.
fn sheet_image(skin: &Skin, sheet: Sheet) -> Image {
    let raster = skin.sheet(sheet).expect("the sheet is present");
    Image::from_rgba(raster.width(), raster.height(), raster.as_rgba().to_vec()).expect("rgba")
}

/// S01 §1: the shaded equalizer is a 275 x 14 strip.
#[test]
fn the_shaded_equalizer_is_a_275_by_14_strip() {
    let state = EqualizerState {
        windowshade: true,
        ..EqualizerState::default()
    };
    assert_eq!(eq_frame_size(&state), (275, 14));
    assert_eq!(window_size(WindowKind::Equalizer, true, false), (275, 14));
    // The unshaded window keeps its S01 §1 size.
    assert_eq!(eq_frame_size(&EqualizerState::default()), (275, 116));
}

/// The double-size form of the shade is the scaled shade (550 x 28). S05 §2.3
/// measured that for the main window; for the equalizer it is an own choice,
/// kept so the docked size and the painted frame agree.
#[test]
fn the_double_sized_shade_is_the_scaled_shade() {
    for skin in skins() {
        let shade = render_equalizer_window(
            &EqualizerState {
                windowshade: true,
                ..EqualizerState::default()
            },
            &skin,
            &EqualizerLayout::CLASSIC,
        );
        let doubled = render_equalizer_window(
            &EqualizerState {
                windowshade: true,
                double_size: true,
                ..EqualizerState::default()
            },
            &skin,
            &EqualizerLayout::CLASSIC,
        );
        assert_eq!((doubled.width(), doubled.height()), (550, 28));
        assert_eq!(doubled, shade.scaled(2), "exact whole-frame pixel doubling");
    }
}

/// S01 §3 (provisional): the shaded frame is the `eq_ex.bmp` band, focused
/// row 0 and unfocused row 15.
#[test]
fn the_shaded_frame_is_the_skins_shade_band() {
    for skin in skins() {
        for focused in [true, false] {
            let state = EqualizerState {
                windowshade: true,
                focused,
                ..EqualizerState::default()
            };
            let frame = render_equalizer_window(&state, &skin, &EqualizerLayout::CLASSIC);
            assert_eq!((frame.width(), frame.height()), (275, 14));
            let band = shade_band(&skin, focused);
            for y in 0..14 {
                for x in 0..275 {
                    assert_eq!(
                        frame.pixel(x, y),
                        band.pixel(x, y),
                        "pixel {x},{y} (focused: {focused})"
                    );
                }
            }
        }
    }
}

/// S01 §3: the shade renders the **shade** artwork, the unshaded window its
/// own. The shaded frame is the `eq_ex.bmp` band, the unshaded frame's title
/// band is the `eqmain.bmp` band, and the body is gone in the shade.
#[test]
fn the_shaded_frame_is_the_shade_artwork_and_not_the_unshaded_render() {
    for skin in skins() {
        let unshaded =
            render_equalizer_window(&EqualizerState::default(), &skin, &EqualizerLayout::CLASSIC);
        let shaded = render_equalizer_window(
            &EqualizerState {
                windowshade: true,
                ..EqualizerState::default()
            },
            &skin,
            &EqualizerLayout::CLASSIC,
        );
        assert_eq!((unshaded.width(), unshaded.height()), (275, 116));
        assert_eq!((shaded.width(), shaded.height()), (275, 14));

        // The unshaded title band is `eqmain.bmp` (S01 §3, round 2: focused
        // row 134).
        let eqmain = sheet_image(&skin, Sheet::EqMain);
        for y in 0..14 {
            for x in 0..275 {
                assert_eq!(
                    unshaded.pixel(x, y),
                    eqmain.pixel(x, 134 + y),
                    "unshaded pixel {x},{y}"
                );
            }
        }
        // The shaded band is `eq_ex.bmp` (S01 §3, provisional: focused row 0).
        let band = shade_band(&skin, true);
        for y in 0..14 {
            for x in 0..275 {
                assert_eq!(shaded.pixel(x, y), band.pixel(x, y), "shaded pixel {x},{y}");
            }
        }
        // The body of the unshaded window has no counterpart in the shade:
        // the shade is the 14-px strip alone (S05 §1.2).
        assert!(unshaded.pixel(0, 20).is_some());
        assert!(shaded.pixel(0, 20).is_none());
    }
}

/// The body controls are not drawn in the shade: the On button's footprint
/// of the shaded frame carries the band's own pixels, not the button sprite.
#[test]
fn the_shaded_frame_carries_no_body_controls() {
    for skin in skins() {
        let eqmain = skin
            .sheet(Sheet::EqMain)
            .expect("the equalizer sheet is present");
        let sheet = Image::from_rgba(eqmain.width(), eqmain.height(), eqmain.as_rgba().to_vec())
            .expect("rgba");
        let shaded = render_equalizer_window(
            &EqualizerState {
                windowshade: true,
                ..EqualizerState::default()
            },
            &skin,
            &EqualizerLayout::CLASSIC,
        );
        // The On button sits at (14,18) in the unshaded window, which is
        // outside the 14-px shade altogether.
        for (x, y) in [(14, 18), (39, 18), (217, 18), (22, 63), (79, 63)] {
            assert_eq!(shaded.pixel(x, y), None, "no body pixel at {x},{y}");
        }
        // ...and the band's own pixels are the ones of `eq_ex.bmp`.
        let band = shade_band(&skin, true);
        assert_eq!(shaded.pixel(0, 0), band.pixel(0, 0));
        assert_eq!(shaded.pixel(274, 13), band.pixel(274, 13));
        // The On sprite is not somewhere in the strip either.
        let on_sprite = sheet.pixel(10, 119);
        let mut found = false;
        for y in 0..14 {
            for x in 0..275 {
                if shaded.pixel(x, y) == on_sprite {
                    found = true;
                }
            }
        }
        assert!(!found, "the On button must not be drawn in the shade");
    }
}

/// S05 §1.2: the shaded equalizer answers its title band and nothing else.
#[test]
fn the_shaded_equalizer_answers_only_its_title_band() {
    let mut window = EqualizerWindow::new();
    window.set_windowshade(true);
    assert!(window.state().windowshade);

    // The band is the whole shaded window and it answers.
    assert_eq!(
        hit_test(&EqualizerLayout::CLASSIC, 137, 7, true),
        Some(EqWidget::TitleBar)
    );
    // The body controls do not.
    assert_eq!(hit_test(&EqualizerLayout::CLASSIC, 20, 24, true), None);
    assert_eq!(hit_test(&EqualizerLayout::CLASSIC, 55, 24, true), None);
    assert_eq!(hit_test(&EqualizerLayout::CLASSIC, 239, 24, true), None);
    for slider in EqSlider::ALL_BANDS {
        let rect = EqualizerLayout::slider_hit(slider);
        assert_eq!(
            hit_test(&EqualizerLayout::CLASSIC, rect.x, rect.y, true),
            None,
            "{slider:?} must not answer in the shade"
        );
    }

    // ...and no click changes a value.
    let preamp = window.state().preamp;
    let bands = window.state().bands;
    assert!(window.mouse_down(22, 95).is_empty());
    assert!(window.mouse_move(22, 40).is_empty());
    assert!(window.mouse_up(22, 40).is_empty());
    assert!(window.mouse_down(20, 24).is_empty());
    assert!(window.mouse_up(20, 24).is_empty());
    assert!(window.mouse_down(239, 24).is_empty());
    assert!(window.mouse_up(239, 24).is_empty());
    assert_eq!(window.state().preamp, preamp);
    assert_eq!(window.state().bands, bands);
    assert!(window.state().enabled);
    assert!(!window.state().auto);
    assert_eq!(window.take_menu_request(), None);
}

/// The unshaded path is unchanged: the same clicks still act.
#[test]
fn the_unshaded_equalizer_still_answers_its_body() {
    let mut window = EqualizerWindow::new();
    assert!(!window.state().windowshade);

    assert_eq!(
        hit_test(&EqualizerLayout::CLASSIC, 22, 63, false),
        Some(EqWidget::Slider(EqSlider::Preamp))
    );
    assert!(window.mouse_down(20, 24).is_empty());
    assert_eq!(window.state().held, Some(EqWidget::On));
    assert_eq!(window.mouse_up(20, 24).len(), 1);
    assert!(!window.state().enabled);

    // A press below the flat thumb jumps and commits (S07 §5/§6).
    assert_eq!(window.mouse_down(22, 95).len(), 1);
    assert!(window.state().preamp > rustamp_ui::equalizer::FLAT_VALUE);
}
