// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Seeded property suite for the window renderers.
//!
//! Four renderers draw a frame in this crate: the playlist editor
//! ([`crate::playlist_window`]), the media library
//! ([`crate::library_render`]), the equalizer ([`crate::eq_render`]) and the
//! classic analyser ([`crate::visualization`]). Each of them documents a few
//! rules - "a row is drawn only inside the list band", "a row holds ink and
//! background and never a blend of the two", "the page holds whole rows",
//! "an empty list draws no rows", "the visible ids are the contiguous slice
//! the model reports" - and the example tests of today pin each of them for
//! **one** state.
//!
//! This module states the same rules over **random** content: a fixed list
//! of seeds, a fixed number of cases per seed, and a failure that names the
//! seed and the case that produced it. The generator is the small xorshift
//! the layout suite uses ([`crate::layout_props`]), so the crate takes no
//! dependency for it and a seed always yields the same cases.
//!
//! # The properties
//!
//! * **Nothing is drawn outside its band** - the playlist's rows, its
//!   selection and its drag insertion line stay in the list band, the
//!   library's rows stay in the rows band and its status text in the status
//!   band, the equalizer's response curve stays in the graph area and its
//!   sliders in their columns, and the analyser stays in its area.
//! * **The 1-bit ink rule** - a playlist row holds the skin's ink or the
//!   skin's background, never a blend of the two, at every font size and for
//!   every selection.
//! * **Row count and pitch agree** - the page holds whole rows only, and the
//!   model and the free functions agree on how many that is.
//! * **An empty state draws no rows** but still draws its chrome.
//! * **Scrolling never leaves a gap** - the visible ids are the contiguous
//!   slice the model reports, the renderer draws that slice's rows in the
//!   bands the model names, and every label lands in the row of its entry.
//! * **Nothing panics** - empty strings, very long titles and a zero-size
//!   rectangle are ordinary input, not a crash.
//!
//! # What the suite cannot claim
//!
//! The rules are the ones the renderers **document**. Where a spec leaves a
//! choice open, the property states the rule this implementation picked and
//! says so: the row pitch follows the font (S01 §4.2, provisional), the
//! graph curve is a polyline of the ten bands (own choice), and the library
//! window is a modern window with no spec at all (S20 is not started).

use std::path::PathBuf;

use rustamp_core::command::SelectOp;
use rustamp_core::playlist::EntryId;
use rustamp_gfx::{Image, Rect};
use rustamp_skin::{BuiltinDefaultSkin, Skin};
use rustamp_vis::analyser::{Geometry, SCOPE_COLUMNS};

use crate::eq_render::render_equalizer_window;
use crate::equalizer::{EqSlider, EqualizerLayout, EqualizerState};
use crate::library::{LibraryRow, LibraryView};
use crate::library_layout::LibraryLayout;
use crate::library_render::{CoverRef, LIBRARY_LINE_HEIGHT, LibraryChrome, render_library_window};
use crate::playlist::PlaylistWindow;
use crate::playlist_font::{MAX_FONT_SIZE, MIN_FONT_SIZE};
use crate::playlist_window::{
    LIST_BOTTOM, LIST_LEFT, LIST_PAGE_HEIGHT, LIST_RIGHT, LIST_TOP, in_list_area,
    render_playlist_window, row_pitch, row_rect, visible_rows,
};
use crate::shortcuts::Modifiers;
use crate::visualization::{VisInput, VisMode, VisState, draw_visualization};

/// The seeds every property runs over. Fixed on purpose: a failure names the
/// seed that produced it.
const SEEDS: [u64; 8] = [
    0x0000_0000_0000_0001,
    0x5EED_1234_ABCD_0001,
    0x0BAD_C0DE_0BAD_C0DE,
    0xDEAD_BEEF_0000_0001,
    0x0123_4567_89AB_CDEF,
    0x7FFF_FFFF_FFFF_FFFF,
    0xA5A5_5A5A_A5A5_5A5A,
    0x0F0F_0F0F_0F0F_0F0F,
];

/// How many random cases one seed produces.
const CASES: usize = 48;

/// A small deterministic generator (xorshift64*).
///
/// The suite must be reproducible and the crate takes no dependency for it,
/// so the generator lives here.
struct Rng(u64);

impl Rng {
    /// A generator for `seed`. The state is never zero: xorshift would stay
    /// there forever.
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    /// The next 64 random bits.
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value in `0..bound` (`0` for an empty range).
    fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            0
        } else {
            self.next_u64() % bound
        }
    }

    /// A value in `low..=high`.
    fn range(&mut self, low: i32, high: i32) -> i32 {
        if high <= low {
            return low;
        }
        let span = u64::try_from(i64::from(high) - i64::from(low) + 1).unwrap_or(1);
        let offset = i64::try_from(self.below(span)).unwrap_or(0);
        i64::from(low)
            .checked_add(offset)
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(low)
    }

    /// A coin toss.
    fn coin(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The playlist colours the properties check against.
///
/// The four colours differ from one another and from the built-in palettes,
/// so a hardcoded colour cannot pass by accident.
const PLEDIT_TEXT: &[u8] =
    b"[Text]\nNormal=#13579B\nCurrent=#ECA864\nNormalBG=#192A3B\nSelectedBG=#4C5D6E\n";

/// The skin the properties render with.
///
/// The playlist colours are the ones above; every other sheet is the built-in
/// default's, so the equalizer and the analyser have their artwork.
fn skin() -> Skin {
    let mut source = BuiltinDefaultSkin::source();
    source.add("pledit.txt", PLEDIT_TEXT.to_vec());
    Skin::from_source(&mut source, &BuiltinDefaultSkin).expect("the built-in skin loads")
}

/// An opaque RGBA colour from an RGB triple.
fn opaque(rgb: [u8; 3]) -> [u8; 4] {
    [rgb[0], rgb[1], rgb[2], 255]
}

/// The alphabet the random labels are built from.
///
/// It is plain printable ASCII, and it is **lower case**: the classic
/// bitmap font has no upper-case glyphs (S01 §2.2), so a label of upper-case
/// letters would draw no ink on a host without a system font.
const ALPHABET: &str = "abcdefghijklmnopqrstuvwxyz0123456789 .:-_()";

/// A random label: empty, short or long.
fn random_label(rng: &mut Rng) -> String {
    let length = match rng.below(4) {
        0 => 0,
        1 => rng.below(12) as usize,
        2 => rng.below(60) as usize,
        _ => rng.below(200) as usize,
    };
    (0..length)
        .map(|_| {
            let index = rng.below(ALPHABET.len() as u64) as usize;
            ALPHABET.chars().nth(index).unwrap_or(' ')
        })
        .collect()
}

/// A playlist window with random content.
///
/// The list holds a random number of entries, the font a random size, the
/// view a random scroll offset, the selection a random set of entries and
/// the current entry a random one of them.
fn random_playlist(rng: &mut Rng) -> PlaylistWindow {
    let length = rng.range(0, 40) as usize;
    let mut window = PlaylistWindow::new();
    window.set_entries((0..length as u64).map(EntryId).collect());
    window.set_font_size_override(Some(
        rng.range(MIN_FONT_SIZE as i32, MAX_FONT_SIZE as i32) as u32
    ));
    window.scroll_to(rng.range(0, 60) as usize);
    if length > 0 {
        let mut selected = Vec::new();
        for index in 0..length {
            if rng.coin() {
                selected.push(EntryId(index as u64));
            }
        }
        window.apply_select(&SelectOp::Set(selected));
        if rng.coin() {
            window.set_current(Some(EntryId(rng.below(length as u64))));
        }
    }
    window
}

/// The labels of a playlist window: one per visible row.
fn random_labels(rng: &mut Rng, count: usize) -> Vec<String> {
    (0..count).map(|_| random_label(rng)).collect()
}

/// The pixels where two frames differ.
///
/// The frames are compared row by row over their raw bytes: the library's
/// frames are large, and the properties run over hundreds of cases.
fn differences(a: &Image, b: &Image) -> Vec<(i32, i32)> {
    let mut changed = Vec::new();
    let (a_width, a_height) = (a.width() as usize, a.height() as usize);
    let (b_width, b_height) = (b.width() as usize, b.height() as usize);
    let (a_bytes, b_bytes) = (a.as_rgba(), b.as_rgba());
    for y in 0..a_height.min(b_height) {
        let a_start = y * a_width * 4;
        let b_start = y * b_width * 4;
        let a_row = &a_bytes[a_start..a_start + a_width * 4];
        let b_row = &b_bytes[b_start..b_start + b_width * 4];
        for x in 0..a_width.min(b_width) {
            let pixel = x * 4;
            if a_row[pixel..pixel + 4] != b_row[pixel..pixel + 4] {
                changed.push((x as i32, y as i32));
            }
        }
    }
    changed
}

/// Whether `(x, y)` lies in one of the rectangles.
fn in_any(rects: &[Rect], x: i32, y: i32) -> bool {
    rects
        .iter()
        .any(|rect| x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom())
}

/// A library row with random display values.
fn random_row(rng: &mut Rng, id: i64) -> LibraryRow {
    LibraryRow {
        id,
        path: PathBuf::from(format!("C:/music/track{id}.flac")),
        title: random_label(rng),
        artist: random_label(rng),
        album: random_label(rng),
        duration_ms: match rng.below(3) {
            0 => None,
            1 => Some(rng.below(3_600_000)),
            _ => Some(215_000),
        },
    }
}

/// A random picture, as the artwork cache would hand it over.
///
/// The picture is opaque: a transparent cover would draw nothing, and the
/// properties must not depend on the luck of the draw.
fn random_cover(rng: &mut Rng) -> (Vec<u8>, u32, u32) {
    let width = 1 + rng.below(64) as u32;
    let height = 1 + rng.below(64) as u32;
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for _ in 0..(width as usize * height as usize) {
        pixels.push(rng.next_u64() as u8);
        pixels.push(rng.next_u64() as u8);
        pixels.push(rng.next_u64() as u8);
        pixels.push(255);
    }
    (pixels, width, height)
}

/// The pictures a library page hands to the renderer.
///
/// The renderer borrows the bytes, so the pictures own them and the covers
/// are borrowed from them.
type CoverPictures = Vec<Option<(Vec<u8>, u32, u32)>>;

/// The covers of the visible rows, borrowed from `pictures`.
fn borrow_covers(pictures: &CoverPictures) -> Vec<Option<CoverRef<'_>>> {
    pictures
        .iter()
        .map(|picture| {
            picture.as_ref().map(|(pixels, width, height)| CoverRef {
                width: *width,
                height: *height,
                pixels: pixels.as_slice(),
            })
        })
        .collect()
}

/// A random analyser state.
fn random_vis_state(rng: &mut Rng) -> VisState {
    let mut state = VisState {
        mode: match rng.below(4) {
            0 => VisMode::Spectrum,
            1 => VisMode::Scope,
            2 => VisMode::ScopeDots,
            _ => VisMode::Off,
        },
        input: match rng.below(3) {
            0 => VisInput::Missing,
            1 => VisInput::Fresh,
            _ => VisInput::Stale,
        },
        ..VisState::default()
    };
    for band in 0..state.bars.len() {
        state.bars[band] = rng.below(24) as u8;
        state.peaks[band] = rng.below(24) as u8;
    }
    for column in 0..SCOPE_COLUMNS {
        state.scope[column] = rng.below(24) as u8;
    }
    state
}

// ---------------------------------------------------------------------------
// The playlist editor
// ---------------------------------------------------------------------------

/// The playlist's content stays inside the list band (S06 §2).
///
/// The chrome of the window - the title bar, the side rails, the bottom bar
/// and the scrollbar - does not depend on the rows, so a frame with labels
/// and a frame without them differ in the list band and nowhere else. The
/// same holds for the selection and for the current entry's ink.
#[test]
fn the_playlist_content_stays_inside_the_list_band() {
    let skin = skin();
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let window = random_playlist(&mut rng);
            let labels = random_labels(&mut rng, window.visible_ids().len());
            let empty = vec![String::new(); labels.len()];
            let with = render_playlist_window(&skin, &window, &labels, true);
            let without = render_playlist_window(&skin, &window, &empty, true);
            for (x, y) in differences(&with, &without) {
                assert!(
                    in_list_area(x, y),
                    "seed {seed:#x}, case {case}: the row content reached {x},{y}, outside the list band"
                );
            }

            // The selection and the current entry's ink stay in the band too.
            let mut unselected = window.clone();
            unselected.apply_select(&SelectOp::None);
            let plain = render_playlist_window(&skin, &unselected, &labels, true);
            for (x, y) in differences(&with, &plain) {
                assert!(
                    in_list_area(x, y),
                    "seed {seed:#x}, case {case}: the selection reached {x},{y}, outside the list band"
                );
            }
        }
    }
}

/// The drag insertion line stays inside the list band.
///
/// The line is the window's own one-pixel rule (S06 §8 Q6 leaves its shape
/// open), so it must not reach into the side rails or into the scrollbar.
#[test]
fn the_drag_insertion_line_stays_inside_the_list_band() {
    let skin = skin();
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let mut window = random_playlist(&mut rng);
            let pitch = window.row_pitch();
            let visible = window.visible_ids();
            if visible.is_empty() {
                // The empty space below the last entry arms no drag.
                continue;
            }
            let row = rng.below(visible.len() as u64) as i32;
            let press_y = LIST_TOP + row * pitch + pitch / 2;
            window.mouse_down(LIST_LEFT + 10, press_y, Modifiers::NONE);
            let selected = render_playlist_window(&skin, &window, &[], true);
            // Four pixels of motion arm the drag; the lower half of the row
            // asks for the insertion after it.
            window.mouse_move(LIST_LEFT + 20, press_y + pitch / 2 + 1);
            let dragging = render_playlist_window(&skin, &window, &[], true);
            let changed = differences(&selected, &dragging);
            assert!(
                !changed.is_empty(),
                "seed {seed:#x}, case {case}: the armed drag drew no insertion line"
            );
            for (x, y) in &changed {
                assert!(
                    in_list_area(*x, *y),
                    "seed {seed:#x}, case {case}: the insertion line reached {x},{y}, outside the list band"
                );
            }
            // It is one row of pixels, not a block.
            assert!(
                changed.iter().all(|(_, y)| *y == changed[0].1),
                "seed {seed:#x}, case {case}: the insertion line is more than one pixel high"
            );
        }
    }
}

/// The 1-bit ink rule: a playlist row holds the skin's ink or the skin's
/// background, never a blend of the two.
///
/// The rule is the renderer's own (S17 §5): the system font is rasterized
/// into a 1-bit mask, so a row can only carry the palette's colours. It must
/// hold at every font size and for every selection, and the selected row's
/// background is the skin's `SelectedBG` - not a colour of ours.
#[test]
fn a_playlist_row_holds_only_ink_and_background() {
    let skin = skin();
    let style = skin.pledit();
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let mut window = random_playlist(&mut rng);
            let size = rng.range(MIN_FONT_SIZE as i32, MAX_FONT_SIZE as i32) as u32;
            window.set_font_size_override(Some(size));
            let labels = random_labels(&mut rng, window.visible_ids().len());
            let frame = render_playlist_window(&skin, &window, &labels, true);
            let pitch = window.row_pitch();
            let selected = window.selected_ids();
            let current = window.current();
            for (index, id) in window.visible_ids().iter().enumerate() {
                let rect = row_rect(index, pitch).expect("a visible row");
                let background = opaque(if selected.contains(id) {
                    style.selected_bg
                } else {
                    style.normal_bg
                });
                let ink = opaque(if current == Some(*id) {
                    style.current
                } else {
                    style.normal
                });
                let mut ink_count = 0;
                for y in rect.y..rect.bottom() {
                    for x in rect.x..rect.right() {
                        let pixel = frame.pixel(x, y).expect("the pixel is inside the frame");
                        assert!(
                            pixel == background || pixel == ink,
                            "seed {seed:#x}, case {case}, size {size}, row {index}: off-palette pixel {pixel:?}"
                        );
                        ink_count += usize::from(pixel == ink);
                    }
                }
                if !labels.get(index).is_some_and(String::is_empty) {
                    assert!(
                        ink_count > 0,
                        "seed {seed:#x}, case {case}, size {size}, row {index}: the label drew no ink"
                    );
                }
            }
        }
    }
}

/// The page holds whole rows only, and the model agrees with the free
/// functions on how many that is.
///
/// The page is the settled four rows of 13 px (S01 §4.2); the row pitch
/// follows the font, so the number of rows in the page is recomputed. A row
/// is never clipped and never reaches past the page.
#[test]
fn the_page_holds_whole_rows_at_every_font_size() {
    for size in MIN_FONT_SIZE..=MAX_FONT_SIZE {
        let pitch = row_pitch(size);
        let rows = visible_rows(pitch);
        assert!(
            rows * pitch as usize <= LIST_PAGE_HEIGHT as usize,
            "size {size}: the page holds {rows} rows of {pitch} px, more than its {LIST_PAGE_HEIGHT} px"
        );
        assert!(
            row_rect(rows, pitch).is_none(),
            "size {size}: a row past the page would still be drawn"
        );
        for row in 0..rows {
            let rect = row_rect(row, pitch).expect("a visible row");
            assert!(
                rect.bottom() <= LIST_TOP + LIST_PAGE_HEIGHT,
                "size {size}, row {row}: the row reaches past the page"
            );
            assert!(
                rect.bottom() <= LIST_BOTTOM,
                "size {size}, row {row}: the row reaches past the list band"
            );
        }
        // The model owns the size and agrees with the free functions.
        let mut window = PlaylistWindow::new();
        window.set_font_size_override(Some(size));
        assert_eq!(window.row_pitch(), pitch, "size {size}");
        assert_eq!(window.visible_rows(), rows, "size {size}");
    }
}

/// An empty playlist draws no rows, and it still draws its chrome.
///
/// The row area of an empty list is the list background, pixel for pixel;
/// the rest of the window carries the chrome, and that chrome is the same
/// chrome a populated window draws.
#[test]
fn an_empty_playlist_draws_no_rows_but_its_chrome() {
    let skin = skin();
    let style = skin.pledit();
    let background = opaque(style.normal_bg);

    let empty = PlaylistWindow::new();
    let frame = render_playlist_window(&skin, &empty, &[], true);
    for y in LIST_TOP..LIST_TOP + LIST_PAGE_HEIGHT {
        for x in LIST_LEFT..LIST_RIGHT {
            assert_eq!(
                frame.pixel(x, y),
                Some(background),
                "the empty list drew something at {x},{y}"
            );
        }
    }

    // The chrome is there: each of its bands carries the skin's art, so the
    // empty window is not the bare list background.
    for (name, band) in [
        ("the title bar", Rect::new(0, 0, 275, 20)),
        ("the left rail", Rect::new(0, 20, 12, 58)),
        ("the right rail", Rect::new(255, 20, 20, 58)),
        ("the bottom bar", Rect::new(0, 78, 275, 38)),
        ("the scrollbar", Rect::new(260, 20, 8, 58)),
    ] {
        let painted = (band.y..band.bottom())
            .flat_map(|y| (band.x..band.right()).map(move |x| (x, y)))
            .any(|(x, y)| frame.pixel(x, y) != Some(background));
        assert!(painted, "the empty playlist drew no {name}");
    }

    // And the chrome does not depend on the content: a window with rows
    // draws the same chrome outside the list band.
    let mut full = PlaylistWindow::new();
    full.set_entries((0..4u64).map(EntryId).collect());
    let full_frame = render_playlist_window(&skin, &full, &[], true);
    for y in 0..frame.height() as i32 {
        for x in 0..frame.width() as i32 {
            if !in_list_area(x, y) {
                assert_eq!(
                    frame.pixel(x, y),
                    full_frame.pixel(x, y),
                    "the empty window's chrome differs at {x},{y}"
                );
            }
        }
    }
}

/// The playlist draws every label in the row of its own entry.
///
/// The renderer takes the labels of the visible entries in the order of the
/// model's slice, so the row of visible entry `i` shows label `i`. Swapping
/// two labels must swap the two rows and leave every other row alone.
#[test]
fn the_playlist_draws_every_label_in_its_own_row() {
    let skin = skin();
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let mut window = PlaylistWindow::new();
            window.set_entries((0..8u64).map(EntryId).collect());
            window.set_font_size_override(Some(
                rng.range(MIN_FONT_SIZE as i32, MAX_FONT_SIZE as i32) as u32,
            ));
            window.scroll_to(rng.range(0, 8) as usize);
            let visible = window.visible_ids();
            if visible.len() < 2 {
                continue;
            }
            let labels = random_labels(&mut rng, visible.len());
            let mut swapped = labels.clone();
            swapped.swap(0, 1);
            let plain = render_playlist_window(&skin, &window, &labels, true);
            let mixed = render_playlist_window(&skin, &window, &swapped, true);
            let pitch = window.row_pitch();
            let bands = |frame: &Image| {
                (0..visible.len())
                    .map(|row| {
                        let rect = row_rect(row, pitch).expect("a visible row");
                        (rect.y..rect.bottom())
                            .flat_map(|y| (rect.x..rect.right()).map(move |x| (x, y)))
                            .map(|(x, y)| frame.pixel(x, y))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            };
            let plain_bands = bands(&plain);
            let mixed_bands = bands(&mixed);
            // The two swapped labels follow their rows.
            assert_eq!(
                plain_bands[0], mixed_bands[1],
                "seed {seed:#x}, case {case}: the swapped label did not reach row 1"
            );
            assert_eq!(
                plain_bands[1], mixed_bands[0],
                "seed {seed:#x}, case {case}: the swapped label did not reach row 0"
            );
            // Every other row is untouched.
            for row in 2..visible.len() {
                assert_eq!(
                    plain_bands[row], mixed_bands[row],
                    "seed {seed:#x}, case {case}: row {row} changed with another row's label"
                );
            }
        }
    }
}

/// Scrolling never leaves a gap: the visible ids are the contiguous slice
/// the model reports.
///
/// The offset is clamped so that the last screenful is full, and the slice
/// is the entries from the offset, without a hole in it.
#[test]
fn scrolling_never_leaves_a_gap_in_the_visible_ids() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let length = rng.range(0, 80) as usize;
            let offset = rng.range(0, 100) as usize;
            let size = rng.range(MIN_FONT_SIZE as i32, MAX_FONT_SIZE as i32) as u32;
            let mut window = PlaylistWindow::new();
            window.set_entries((0..length as u64).map(EntryId).collect());
            window.set_font_size_override(Some(size));
            window.scroll_to(offset);

            let first = window.first_visible();
            let rows = window.visible_rows();
            let expected: Vec<EntryId> = (first..(first + rows).min(length))
                .map(|index| EntryId(index as u64))
                .collect();
            assert_eq!(
                window.visible_ids(),
                expected,
                "seed {seed:#x}, case {case}: the visible ids are not the contiguous slice"
            );
            // The offset is inside the list: the view never runs past its
            // end, and the last screenful is full (S01 §4.2: the bottom view
            // of the 60-entry list is rows 57-60).
            assert!(
                first <= window.max_first_visible(),
                "seed {seed:#x}, case {case}: the offset is past the list"
            );
            if length > rows {
                assert_eq!(
                    window.max_first_visible() + rows,
                    length,
                    "seed {seed:#x}, case {case}: the last screenful is not full"
                );
            }
        }
    }
}

/// The renderer draws the model's slice: the entry at visible row `k` is
/// drawn in row band `k`.
///
/// The selection is the witness: the selected row carries the skin's
/// `SelectedBG` and no other row does, so the band that lights up is the
/// band the model's own index names.
#[test]
fn the_playlist_draws_the_model_s_visible_slice() {
    let skin = skin();
    let style = skin.pledit();
    let selected_bg = opaque(style.selected_bg);
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let length = rng.range(1, 40) as usize;
            let offset = rng.range(0, 60) as usize;
            let size = rng.range(MIN_FONT_SIZE as i32, MAX_FONT_SIZE as i32) as u32;
            let mut window = PlaylistWindow::new();
            window.set_entries((0..length as u64).map(EntryId).collect());
            window.set_font_size_override(Some(size));
            window.scroll_to(offset);
            let visible = window.visible_ids();
            if visible.is_empty() {
                continue;
            }
            let chosen = rng.below(visible.len() as u64) as usize;
            window.apply_select(&SelectOp::Set(vec![visible[chosen]]));
            let labels = random_labels(&mut rng, visible.len());
            let frame = render_playlist_window(&skin, &window, &labels, true);
            let pitch = window.row_pitch();
            for (index, rect) in (0..window.visible_rows())
                .map(|index| (index, row_rect(index, pitch).expect("a visible row")))
                .collect::<Vec<_>>()
            {
                let mut selected_pixels = 0;
                for y in rect.y..rect.bottom() {
                    for x in rect.x..rect.right() {
                        selected_pixels += usize::from(frame.pixel(x, y) == Some(selected_bg));
                    }
                }
                if index == chosen {
                    assert!(
                        selected_pixels > 0,
                        "seed {seed:#x}, case {case}: the selected row {index} is not highlighted"
                    );
                } else {
                    assert_eq!(
                        selected_pixels, 0,
                        "seed {seed:#x}, case {case}: row {index} is highlighted although row {chosen} is the selected one"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The media library
// ---------------------------------------------------------------------------

/// The library's rows stay in the rows band.
///
/// The rows are the only thing the loaded page changes: the status line
/// names the scroll and the total, the scrollbar names the same, and the
/// header and the search field do not look at the rows at all.
#[test]
fn the_library_rows_stay_inside_the_rows_band() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let (view, viewport, frame_size, layout, pictures) = random_library(&mut rng);
            let covers = borrow_covers(&pictures);
            let chrome = LibraryChrome::new(&view, viewport);
            let mut with = Image::filled(frame_size.0, frame_size.1, [0, 0, 0, 255]);
            render_library_window(&mut with, &view, &layout, viewport, false, &covers);

            // The same view without the loaded page: only the rows differ.
            let mut empty = view.clone();
            empty.apply_page(Vec::new(), view.total(), view.offset());
            let mut without = Image::filled(frame_size.0, frame_size.1, [0, 0, 0, 255]);
            render_library_window(&mut without, &empty, &layout, viewport, false, &[]);

            let changed = differences(&with, &without);
            if !view.visible_rows().is_empty() && chrome.rows.height >= LIBRARY_LINE_HEIGHT {
                assert!(
                    !changed.is_empty(),
                    "seed {seed:#x}, case {case}: the loaded page drew nothing at all"
                );
            }
            for (x, y) in changed {
                assert!(
                    chrome.rows.x <= x
                        && x < chrome.rows.right()
                        && chrome.rows.y <= y
                        && y < chrome.rows.bottom(),
                    "seed {seed:#x}, case {case}: a row reached {x},{y}, outside the rows band"
                );
            }
        }
    }
}

/// The library's status text stays in the status band.
///
/// The scan's status replaces the visible range (the view's own rule), so
/// the two frames differ in the status band and nowhere else.
#[test]
fn the_library_status_stays_inside_the_status_band() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let (view, viewport, frame_size, layout, pictures) = random_library(&mut rng);
            let covers = borrow_covers(&pictures);
            let chrome = LibraryChrome::new(&view, viewport);
            let mut with = Image::filled(frame_size.0, frame_size.1, [0, 0, 0, 255]);
            render_library_window(&mut with, &view, &layout, viewport, false, &covers);

            let mut scanning = view.clone();
            scanning.set_scan_status(&format!("scanning: {}", random_label(&mut rng)));
            let mut without = Image::filled(frame_size.0, frame_size.1, [0, 0, 0, 255]);
            render_library_window(&mut without, &scanning, &layout, viewport, false, &covers);

            let changed = differences(&with, &without);
            if chrome.status.height >= LIBRARY_LINE_HEIGHT {
                assert!(
                    !changed.is_empty(),
                    "seed {seed:#x}, case {case}: the scan's status changed nothing"
                );
            }
            for (x, y) in changed {
                assert!(
                    chrome.status.x <= x
                        && x < chrome.status.right()
                        && chrome.status.y <= y
                        && y < chrome.status.bottom(),
                    "seed {seed:#x}, case {case}: the status text reached {x},{y}, outside the status band"
                );
            }
        }
    }
}

/// A random library page: the view, its viewport, the frame it is drawn
/// into, the layout and the pictures of its covers.
fn random_library(rng: &mut Rng) -> (LibraryView, Rect, (u32, u32), LibraryLayout, CoverPictures) {
    let viewport_rows = rng.range(1, 12) as usize;
    let total = rng.range(0, 200) as usize;
    let loaded = rng.range(0, 40) as usize;
    let offset = rng.range(0, 200) as usize;

    let mut view = LibraryView::new(viewport_rows);
    let rows: Vec<LibraryRow> = (0..loaded)
        .map(|index| random_row(rng, index as i64))
        .collect();
    view.apply_page(rows, total, offset);
    view.scroll_to(rng.range(0, 200) as usize);
    if !view.rows().is_empty() && rng.coin() {
        let index = rng.below(view.rows().len() as u64) as usize;
        view.select_index(index);
    }
    if rng.coin() {
        view.set_search(&random_label(rng));
    }

    // The viewport is narrow enough for the columns to be clipped and short
    // enough for the bands to run out of room.
    let width = 80 + rng.below(320) as u32;
    let height = 20 + rng.below(200) as u32;
    let viewport = Rect::new(rng.range(0, 20), rng.range(0, 20), width, height);
    let frame_size = (
        viewport.right().max(0) as u32 + 8,
        viewport.bottom().max(0) as u32 + 8,
    );

    let chrome = LibraryChrome::new(&view, viewport);
    let layout = LibraryLayout::default().resized_to(chrome.text_cells());

    // The pictures of the visible rows: the first one always has a cover, so
    // a page with visible rows draws something; the others may have none.
    let mut pictures: CoverPictures = Vec::new();
    for index in 0..view.visible_rows().len() {
        if index == 0 || rng.coin() {
            pictures.push(Some(random_cover(rng)));
        } else {
            pictures.push(None);
        }
    }
    (view, viewport, frame_size, layout, pictures)
}

// ---------------------------------------------------------------------------
// The equalizer
// ---------------------------------------------------------------------------

/// The equalizer's response curve stays in the graph area, and its sliders
/// in their columns.
///
/// The curve is drawn from the ten band values and the preamp strip from the
/// preamp; both are the equalizer's own drawing, and the sliders' thumbs and
/// bodies follow the same values. Nothing else of the frame depends on them.
#[test]
fn the_equalizer_draws_nothing_outside_the_graph_and_the_sliders() {
    let skin = skin();
    let layout = EqualizerLayout::CLASSIC;
    let mut allowed = vec![layout.graph];
    for slider in EqSlider::ALL_BANDS {
        allowed.push(EqualizerLayout::track_rect(slider));
    }
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let mut state = EqualizerState::default();
            for band in 0..state.bands.len() {
                state.bands[band] = rng.below(64) as u8;
            }
            state.preamp = rng.below(64) as u8;
            let flat = render_equalizer_window(&state, &skin, &layout);

            let mut other = state.clone();
            for band in 0..other.bands.len() {
                other.bands[band] = rng.below(64) as u8;
            }
            other.preamp = rng.below(64) as u8;
            let drawn = render_equalizer_window(&other, &skin, &layout);

            for (x, y) in differences(&flat, &drawn) {
                assert!(
                    in_any(&allowed, x, y),
                    "seed {seed:#x}, case {case}: the equalizer drew at {x},{y}, outside the graph and the sliders"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The analyser
// ---------------------------------------------------------------------------

/// The analyser stays inside its area (S08 §1).
///
/// The area is the one the main window has: 76 x 16 px with ten bands, or
/// 38 x 5 px with five in the shaded window. Nothing of it may reach out of
/// the area, not even for a bar that is taller than the area.
#[test]
fn the_analyser_stays_inside_its_area() {
    let skin = skin();
    let colors = skin.viscolors();
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let geometry = if rng.coin() {
                Geometry::Normal
            } else {
                Geometry::Shaded
            };
            let state = random_vis_state(&mut rng);
            let area = Rect::new(
                rng.range(-20, 60),
                rng.range(-20, 60),
                rng.range(0, 120) as u32,
                rng.range(0, 40) as u32,
            );
            let sentinel = [7, 11, 13, 255];
            let width = 160 + rng.below(40) as u32;
            let height = 80 + rng.below(40) as u32;
            let mut canvas = Image::filled(width, height, sentinel);
            draw_visualization(&mut canvas, area, geometry, &state, colors);
            for y in 0..height as i32 {
                for x in 0..width as i32 {
                    let inside =
                        x >= area.x && x < area.right() && y >= area.y && y < area.bottom();
                    if !inside {
                        assert_eq!(
                            canvas.pixel(x, y),
                            Some(sentinel),
                            "seed {seed:#x}, case {case}: the analyser drew at {x},{y}, outside its area"
                        );
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Nothing panics
// ---------------------------------------------------------------------------

/// Nothing panics: an empty list, an empty title, a very long title and a
/// zero-size rectangle are ordinary input.
#[test]
fn no_render_panics_for_any_content() {
    let skin = skin();

    // The playlist: an empty list, an empty label and a very long one.
    let mut window = PlaylistWindow::new();
    window.set_entries(vec![EntryId(0)]);
    let labels = vec![String::new(), "x".repeat(20_000), " ".repeat(5_000)];
    let _ = render_playlist_window(&skin, &window, &labels, true);
    let _ = render_playlist_window(&skin, &window, &[], true);
    let mut empty = PlaylistWindow::new();
    let _ = render_playlist_window(&skin, &empty, &[], true);
    for size in MIN_FONT_SIZE..=MAX_FONT_SIZE {
        empty.set_font_size_override(Some(size));
        let _ = render_playlist_window(&skin, &empty, &[], true);
    }

    // The library: a zero-size viewport, a viewport outside the frame and
    // rows with empty and very long values.
    let mut view = LibraryView::new(4);
    let rows = vec![
        LibraryRow {
            id: 1,
            path: PathBuf::new(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            duration_ms: None,
        },
        LibraryRow {
            id: 2,
            path: PathBuf::new(),
            title: "y".repeat(20_000),
            artist: "z".repeat(5_000),
            album: String::new(),
            duration_ms: Some(u64::MAX),
        },
    ];
    view.apply_page(rows, 9_999, 0);
    let layout = LibraryLayout::default();
    for viewport in [
        Rect::new(0, 0, 0, 0),
        Rect::new(0, 0, 1, 1),
        Rect::new(-10, -10, 40, 40),
        Rect::new(0, 0, 4_000, 2_000),
    ] {
        let mut frame = Image::filled(64, 64, [0, 0, 0, 255]);
        render_library_window(&mut frame, &view, &layout, viewport, true, &[]);
    }

    // The equalizer: the extreme band values.
    let lowest = EqualizerState {
        bands: [0; 10],
        preamp: u8::MAX,
        ..EqualizerState::default()
    };
    let _ = render_equalizer_window(&lowest, &skin, &EqualizerLayout::CLASSIC);
    let highest = EqualizerState {
        bands: [u8::MAX; 10],
        preamp: 0,
        ..EqualizerState::default()
    };
    let _ = render_equalizer_window(&highest, &skin, &EqualizerLayout::CLASSIC);

    // The analyser: a zero-size area and a state full of extreme values.
    let state = VisState {
        mode: VisMode::Spectrum,
        input: VisInput::Fresh,
        bars: [u8::MAX; 10],
        peaks: [u8::MAX; 10],
        scope: [u8::MAX; SCOPE_COLUMNS],
        ..VisState::default()
    };
    let mut canvas = Image::filled(32, 32, [0, 0, 0, 255]);
    draw_visualization(
        &mut canvas,
        Rect::new(0, 0, 0, 0),
        Geometry::Normal,
        &state,
        skin.viscolors(),
    );
}
