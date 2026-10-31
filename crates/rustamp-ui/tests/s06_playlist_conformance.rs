//! S06 conformance: the playlist editor's settled facts, executable.
//!
//! Every case is named after the section of `specs/S06-playlist.md` it pins,
//! and every expected value is a number or a point the spec states: the
//! settled click points, the settled list band, the page height, the settled
//! arrow points, the dialog titles and the removal's after-state.
//!
//! # What the pixel counts mean here
//!
//! S06 quotes whole-window differing-pixel counts from the sandbox runs:
//! **2918 px** for the single click, **12203 px** / **13591 px** for SEL,
//! **1501 px** per arrow step, **3475 px** for the removal and **2162 px**
//! for the removal's after-state. Those counts are Winamp 2.95's own
//! rasterizer on its own skin and font, so a headless Rustamp frame cannot
//! reproduce them - the reference's glyphs are not ours. What a Rustamp frame
//! *can* pin is the geometry those counts' bounding boxes name (the list band
//! (12,23)-(254,93), the row band inside it, the arrow column) and the
//! behaviour the counts measured (one row selected, all visible rows
//! selected, one page per arrow step, the removal and its after-state).
//!
//! Each case below quotes the spec's count in its own documentation, so a
//! reviewer can match every assertion against the spec line by line. The
//! counts themselves are evidence, not expectations: a case that asserted
//! "2918 px" would fail on our rasterizer without the reference being
//! wrong.
//!
//! The cases are the model and the renderer's contract. The app seam - the
//! dialogs the window opens, the MISC popup and the removal's way into the
//! engine queue - is in `rustamp-app`'s `s06_playlist_conformance.rs`.

use rustamp_core::command::{Command, OpenMediaKind, SelectOp};
use rustamp_core::playlist::EntryId;
use rustamp_gfx::{Image, Rect};
use rustamp_platform::PopupRow;
use rustamp_skin::{BuiltinDefaultSkin, MemorySource, Skin};
use rustamp_ui::{
    LIST_BOTTOM, LIST_LEFT, LIST_RIGHT, LIST_SELECT_POINT, LIST_TOP, Modifiers,
    PLAYLIST_WINDOW_SIZE, PlaylistButton, PlaylistDialog, PlaylistTitleButton, PlaylistWindow,
    ROW_PITCH, ScrollDirection, VISIBLE_ROWS, has_window_menu_button, hit_test_button,
    hit_test_list_row, hit_test_scroll, hit_test_title_bar, in_list_area, list_area,
    render_playlist_window, row_rect, scroll_offset_for_handle_y, visible_rows,
};

/// The base skin's own selection colour (S06 §2: the highlight is (0,0,198)).
const SELECTED_BG: [u8; 4] = [0, 0, 198, 255];

/// The conformance skin's list background (the skin's `NormalBG`).
const NORMAL_BG: [u8; 4] = [0, 0, 0, 255];

/// The skin the render cases draw with.
///
/// The `pledit.txt` is the base skin's own palette (S06 §2's highlight colour
/// (0,0,198) is the base skin's `SelectedBG`), so the drawn band is the
/// colour the spec names. No `Font=` line: the rows keep the default 13 px
/// pitch (S01 §4.2), which is the pitch the four-row page is measured at.
fn conformance_skin() -> Skin {
    let mut source = MemorySource::new("s06-conformance");
    source.add(
        "pledit.txt",
        b"[Text]\nNormal=#00FF00\nCurrent=#FFFFFF\nNormalBG=#000000\nSelectedBG=#0000C6\n".to_vec(),
    );
    Skin::from_source(&mut source, &BuiltinDefaultSkin).expect("the conformance skin loads")
}

/// A playlist window holding `n` entries, the ids `0..n`.
fn window_with(n: u64) -> PlaylistWindow {
    let mut window = PlaylistWindow::new();
    window.set_entries((0..n).map(EntryId).collect());
    window
}

/// The pixels where two frames differ, in reading order.
fn differing_pixels(before: &Image, after: &Image) -> Vec<(i32, i32)> {
    let mut differing = Vec::new();
    for y in 0..before.height() as i32 {
        for x in 0..before.width() as i32 {
            if before.pixel(x, y) != after.pixel(x, y) {
                differing.push((x, y));
            }
        }
    }
    differing
}

/// The pixels of a rectangle, in reading order.
fn rect_pixels(rect: Rect) -> Vec<(i32, i32)> {
    let mut pixels = Vec::new();
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            pixels.push((x, y));
        }
    }
    pixels
}

// ---------------------------------------------------------------------------
// §1 - the title bar carries no window-menu button
// ---------------------------------------------------------------------------

/// S06 §1: a posted left click at (6,3) was delivered (step 4's pre-flight
/// `hit=(6,3) hitIsTarget=True`) and no `#32768` popup existed afterwards
/// (`pe_tb_mm_popup` / `_twin` exit 3), so the playlist title bar carries no
/// window-menu button - matching the S04 finding on the neighbouring windows.
///
/// The qualification the spec carries (one R1 timing) does not change the
/// claim: the button does not exist, so nothing in the UI may offer one.
#[test]
fn s06_section_1_the_title_bar_carries_no_window_menu_button() {
    assert!(
        !has_window_menu_button(),
        "S06 §1 settles that the playlist title bar has no window-menu button"
    );
    // (6,3) is where the window-menu button would sit: nothing answers it.
    assert_eq!(hit_test_title_bar(6, 3), None);
    // Outside the 20 px title band nothing hits.
    assert_eq!(hit_test_title_bar(6, 20), None);
    // The bar keeps its own buttons (S01 §4, provisional rects).
    assert_eq!(hit_test_title_bar(264, 3), Some(PlaylistTitleButton::Close));
    assert_eq!(hit_test_title_bar(254, 3), Some(PlaylistTitleButton::Shade));
}

// ---------------------------------------------------------------------------
// §2 - the list band, the single click and SEL
// ---------------------------------------------------------------------------

/// S06 §2's geometry: the changed region of the single click is the band
/// **(12,23)-(254,93)**, and the page holds **four rows** at the default
/// pitch (S01 §4.2: "the captured list shows four rows at 13 px pitch").
///
/// The band is 242 x 70 px; the four rows take 52 px of it, and the rest of
/// the band below the fourth row is list background (the bottom bar is drawn
/// over its lower part, S01 §4).
#[test]
fn s06_section_2_the_list_band_is_the_settled_rect_and_the_page_is_four_rows() {
    // The band's corners, as the spec's bbox names them.
    assert_eq!((LIST_LEFT, LIST_TOP), (12, 23));
    assert_eq!((LIST_RIGHT, LIST_BOTTOM), (254, 93));
    assert_eq!(list_area(), Rect::new(12, 23, 242, 70));
    // The client the band lives in (S06 scope; S01 §1).
    assert_eq!(PLAYLIST_WINDOW_SIZE, (275, 116));
    // The row pitch and the page it makes.
    assert_eq!(ROW_PITCH, 13);
    assert_eq!(visible_rows(ROW_PITCH), 4);
    assert_eq!(VISIBLE_ROWS, 4);
    // The four row bands of the page, top to bottom.
    // Winamp 2.95 selects y 23..=34, 35..=47, 48..=60 (measured 2026-10-31): the
    // band cuts the first row.
    assert_eq!(row_rect(0, ROW_PITCH), Some(Rect::new(12, 23, 242, 12)));
    for (row, top) in [(1, 35), (2, 48), (3, 61)] {
        assert_eq!(row_rect(row, ROW_PITCH), Some(Rect::new(12, top, 242, 13)));
    }
    // The fifth row is not on the page: below the fourth row is background.
    assert_eq!(row_rect(4, ROW_PITCH), None);
    // Every row band lies inside the settled band.
    for row in 0..visible_rows(ROW_PITCH) {
        let rect = row_rect(row, ROW_PITCH).expect("the visible row");
        assert!(rect.x >= LIST_LEFT && rect.right() <= LIST_RIGHT, "{row}");
        assert!(rect.y >= LIST_TOP && rect.bottom() <= LIST_BOTTOM, "{row}");
    }
}

/// S06 §2: a single posted click at **(60,28)** selects the first visible
/// row (**2918 px**, bbox (12,23)-(254,93); reproduced three times).
///
/// Exactly one row is selected, and the click leaves the row order alone
/// (S06 §2: "Row order is unchanged by the single click").
#[test]
fn s06_section_2_the_settled_click_selects_exactly_one_row() {
    // The settled click point and the row it lands on.
    assert_eq!(LIST_SELECT_POINT, (60, 28));
    assert_eq!(hit_test_list_row(60, 28, ROW_PITCH), Some(0));

    // The reference's list state: 60 entries, the view on the bottom page
    // (S01 §4.2: the bottom page of the 60-entry list shows rows 57-60).
    let mut window = window_with(60);
    window.scroll_to(56);
    assert_eq!(
        window.visible_ids(),
        vec![EntryId(56), EntryId(57), EntryId(58), EntryId(59)]
    );

    assert_eq!(
        window.mouse_down(60, 28, Modifiers::NONE),
        Some(Command::Select(SelectOp::Set(vec![EntryId(56)])))
    );
    // Exactly one row: the first visible one.
    assert_eq!(window.selected_ids(), vec![EntryId(56)]);
    assert_eq!(window.selected_ids().len(), 1);
    // The click changed nothing else: the row order stands (S06 §2).
    assert_eq!(window.len(), 60);
    assert_eq!(
        window.visible_ids(),
        vec![EntryId(56), EntryId(57), EntryId(58), EntryId(59)]
    );
}

/// S06 §2's drawn band: the single click paints **one row band** at
/// x ≈ 12-254, y ≈ 23-34 inside the settled band (the spec's bbox), in the
/// skin's own selection colour (0,0,198).
///
/// The changed pixels are exactly the first row's band: the click reaches
/// one row of the list area and nothing else.
#[test]
fn s06_section_2_the_click_paints_one_row_band_inside_the_settled_rect() {
    let skin = conformance_skin();
    let mut window = window_with(60);
    window.scroll_to(56);
    let before = render_playlist_window(&skin, &window, &[], true);
    window.mouse_down(60, 28, Modifiers::NONE);
    let after = render_playlist_window(&skin, &window, &[], true);

    let differing = differing_pixels(&before, &after);
    // Every changed pixel lies inside the spec's band (12,23)-(254,93).
    for (x, y) in &differing {
        assert!(
            in_list_area(*x, *y),
            "the change left the list band at {x},{y}"
        );
    }
    // And the change is exactly the first row's band.
    let band = row_rect(0, ROW_PITCH).expect("the first visible row");
    assert_eq!(differing, rect_pixels(band));
    // The band wears the skin's selection colour; the list background around
    // it is the skin's. The gap below the fourth row (y 75) and above the
    // bottom bar (y 78, S01 §4) is that background.
    assert_eq!(after.pixel(band.x, band.y), Some(SELECTED_BG));
    assert_eq!(after.pixel(LIST_LEFT, 76), Some(NORMAL_BG));
}

/// S06 §2: SEL (second real click at **(74,92)**) selects **all visible
/// rows** (click2-vs-click1 **12203 px**, bbox (12,23)-(254,93); after-vs-
/// before **13591 px**).
///
/// "All" is the visible page: the rows below the page are not selected.
#[test]
fn s06_section_2_sel_selects_exactly_the_visible_rows() {
    // The settled SEL point.
    assert_eq!(hit_test_button(74, 92), Some(PlaylistButton::Sel));

    let mut window = window_with(60);
    window.scroll_to(56);
    let visible = vec![EntryId(56), EntryId(57), EntryId(58), EntryId(59)];
    assert_eq!(
        window.press_sel(),
        Command::Select(SelectOp::Set(visible.clone()))
    );
    // Exactly the visible rows ...
    assert_eq!(window.selected_ids(), visible);
    assert_eq!(window.selected_ids().len(), VISIBLE_ROWS);
    // ... and not the whole list.
    assert_ne!(window.selected_ids().len(), window.len());
    assert!(!window.selected_ids().contains(&EntryId(55)));
    assert!(!window.selected_ids().contains(&EntryId(60)));
}

/// S06 §2: after SEL every visible row of the band (12,23)-(254,93) wears
/// the selection colour, not only the clicked one.
#[test]
fn s06_section_2_sel_highlights_every_visible_row() {
    let skin = conformance_skin();
    let mut window = window_with(60);
    window.scroll_to(56);
    window.mouse_down(60, 28, Modifiers::NONE);
    let one_row = render_playlist_window(&skin, &window, &[], true);
    window.press_sel();
    let all_rows = render_playlist_window(&skin, &window, &[], true);

    // Every visible row now wears the selection colour.
    for row in 0..visible_rows(ROW_PITCH) {
        let rect = row_rect(row, ROW_PITCH).expect("the visible row");
        assert_eq!(
            all_rows.pixel(rect.x, rect.y),
            Some(SELECTED_BG),
            "row {row}"
        );
    }
    // The change SEL made reaches the rows the single click left alone, and
    // stays inside the settled band: it is exactly the three bands below the
    // clicked row.
    let mut differing = differing_pixels(&one_row, &all_rows);
    for (x, y) in &differing {
        assert!(
            in_list_area(*x, *y),
            "the change left the list band at {x},{y}"
        );
    }
    differing.sort_unstable();
    let mut expected: Vec<(i32, i32)> = Vec::new();
    for row in 1..visible_rows(ROW_PITCH) {
        let rect = row_rect(row, ROW_PITCH).expect("the visible row");
        expected.extend(rect_pixels(rect));
    }
    expected.sort_unstable();
    assert_eq!(differing, expected, "SEL highlighted the wrong rows");
    // SEL again changes nothing: the reference's step 16 is byte-identical
    // to its step 15.
    window.press_sel();
    assert_eq!(
        render_playlist_window(&skin, &window, &[], true),
        all_rows,
        "a second SEL changes nothing"
    );
}

// ---------------------------------------------------------------------------
// §3 - scrolling: one arrow step is one page
// ---------------------------------------------------------------------------

/// S06 §3: the up arrow at **(263,81)** and the down arrow at **(263,87)**,
/// and one step is **one page of four rows**, not one row (the 1501 px
/// symmetry in both directions).
///
/// The spec's own reading of the first visible row - 57 → 53 → 57 - is the
/// page step on the 60-entry list's bottom page.
#[test]
fn s06_section_3_one_arrow_step_scrolls_one_page_not_one_row() {
    // The settled arrow points.
    assert_eq!(hit_test_scroll(263, 81), Some(ScrollDirection::Up));
    assert_eq!(hit_test_scroll(263, 87), Some(ScrollDirection::Down));

    let mut window = window_with(60);
    assert_eq!(window.visible_rows(), 4, "the page is four rows");

    // At the top of the list: an up step is clamped, a down step is a page.
    assert_eq!(window.first_visible(), 0);
    window.scroll_page(ScrollDirection::Up);
    assert_eq!(window.first_visible(), 0, "clamped at the top");
    window.scroll_page(ScrollDirection::Down);
    assert_eq!(window.first_visible(), 4, "one page, not one row");

    // In the middle of the list: a page up and a page down.
    window.scroll_to(28);
    window.scroll_page(ScrollDirection::Up);
    assert_eq!(window.first_visible(), 24);
    window.scroll_page(ScrollDirection::Down);
    assert_eq!(window.first_visible(), 28);

    // At the end of the list (the bottom page, rows 57-60): a down step is
    // clamped, an up step is a page.
    window.scroll_to(56);
    window.scroll_page(ScrollDirection::Down);
    assert_eq!(window.first_visible(), 56, "clamped at the end");
    window.scroll_page(ScrollDirection::Up);
    assert_eq!(window.first_visible(), 52);

    // The spec's row reading: the first visible row goes 57 → 53 → 57 (the
    // rows are counted from 1).
    window.scroll_to(56);
    assert_eq!(window.visible_ids()[0], EntryId(56), "row 57");
    window.scroll_page(ScrollDirection::Up);
    assert_eq!(window.visible_ids()[0], EntryId(52), "row 53");
    window.scroll_page(ScrollDirection::Down);
    assert_eq!(window.visible_ids()[0], EntryId(56), "back to row 57");

    // S01 §4.2's own reading of the same step: eight up-clicks from the
    // bottom page (rows 57-60) land exactly on rows 25-28.
    window.scroll_to(56);
    for _ in 0..8 {
        window.scroll_page(ScrollDirection::Up);
    }
    assert_eq!(window.first_visible(), 24);
    assert_eq!(window.visible_ids()[0], EntryId(24), "row 25");
}

/// S06 §3: the pages leave **no gap** and the **last page is full**.
///
/// The page step is a page, clamped at the ends of the list. The clamp is
/// this implementation's reading: S06 §3 settles the step size and the
/// symmetry, not the clamping, so a list whose length is not a whole number
/// of pages reaches its last (full) page with a shorter last step.
#[test]
fn s06_section_3_the_pages_leave_no_gap_and_the_last_page_is_full() {
    for len in [4u64, 5, 8, 9, 12, 13, 58, 59, 60, 61] {
        let mut window = window_with(len);
        let rows = window.visible_rows();

        // Walk the list down, page by page.
        let mut offsets = Vec::new();
        loop {
            offsets.push(window.first_visible());
            if window.first_visible() == window.max_first_visible() {
                break;
            }
            window.scroll_page(ScrollDirection::Down);
        }

        // The offsets grow, and no step is longer than a page.
        for pair in offsets.windows(2) {
            assert!(pair[1] > pair[0], "len {len}: the view did not move");
            assert!(
                pair[1] - pair[0] <= rows,
                "len {len}: a step was longer than a page"
            );
        }
        // No gap: every page shows a contiguous run of the list, and the run
        // is as long as the page allows.
        for offset in &offsets {
            window.scroll_to(*offset);
            let visible = window.visible_ids();
            assert_eq!(
                visible.len(),
                (rows as u64).min(len - *offset as u64) as usize,
                "len {len}: the page at {offset} is not full"
            );
            for (row, id) in visible.iter().enumerate() {
                assert_eq!(
                    *id,
                    EntryId((*offset + row) as u64),
                    "len {len}: a gap at row {row} of the page at {offset}"
                );
            }
        }
        // The last page is full and it is the end of the list.
        let last = *offsets.last().expect("at least one page");
        assert_eq!(last, (len as usize).saturating_sub(rows), "len {len}");
        window.scroll_to(last);
        assert_eq!(window.visible_ids().len(), rows, "len {len}");
    }
}

/// S06 §3: a dragged handle **moves the view, not a selection**.
///
/// The handle's positions are settled (S01 §4.2: y = 20 at the scroll top and
/// y = 60 at the bottom of the 60-entry list), so the drag's pixel-to-row
/// mapping is this implementation's own choice - the one
/// [`scroll_offset_for_handle_y`] owns. What the spec settles is the effect:
/// the view follows the handle and the selection stays where it was.
///
/// Only the model's half is pinned here: the app's scrollbar input routes
/// the arrow pair and the handle drag (S06 §3) - see the app's
/// `s06_playlist_conformance.rs`.
#[test]
fn s06_section_3_the_handle_drag_moves_the_view_and_not_the_selection() {
    let mut window = window_with(60);
    window.scroll_to(56);
    window.apply_select(&SelectOp::Set(vec![EntryId(56)]));

    // The handle is dragged from the bottom (y = 60) to the top (y = 20).
    let offset = scroll_offset_for_handle_y(20, window.len(), window.row_pitch());
    window.scroll_to(offset);

    // The view moved to the top of the list ...
    assert_eq!(window.first_visible(), 0);
    assert_eq!(window.visible_ids()[0], EntryId(0));
    // ... and the selection did not move with it.
    assert_eq!(window.selected_ids(), vec![EntryId(56)]);
}

// ---------------------------------------------------------------------------
// §4 - the dialogs and the MISC popup
// ---------------------------------------------------------------------------
/// S06 §4: the five bottom-bar buttons answer their settled points - ADD at
/// (23,92), REM at (46,92), SEL at (74,92), MISC at (110,92) and LIST OPTS
/// at (242,92) (S01 §4.2's posted and real positives).
#[test]
fn s06_section_4_the_bottom_bar_buttons_answer_their_settled_points() {
    assert_eq!(hit_test_button(23, 92), Some(PlaylistButton::Add));
    assert_eq!(hit_test_button(46, 92), Some(PlaylistButton::Rem));
    assert_eq!(hit_test_button(74, 92), Some(PlaylistButton::Sel));
    assert_eq!(hit_test_button(110, 92), Some(PlaylistButton::Misc));
    assert_eq!(hit_test_button(242, 92), Some(PlaylistButton::ListOpts));
}

/// S06 §4: ADD opens **"Add file(s) to playlist"** and LIST OPTS opens
/// **"Load Playlist"** (both `#32770` in the reference; the titles are the
/// settled part, read from the window metadata).
///
/// The reference's client size (1503 x 778 at dpi 144) belongs to the
/// reference's own dialog window; ours is a native dialog, so its size is
/// the operating system's and is not part of this contract.
#[test]
fn s06_section_4_add_and_list_opts_open_the_settled_dialogs() {
    assert_eq!(PlaylistDialog::AddFiles.title(), "Add file(s) to playlist");
    assert_eq!(PlaylistDialog::LoadPlaylist.title(), "Load Playlist");

    let window = PlaylistWindow::new();
    // ADD asks for the files face of the media dialog (RFC 0005 O5 leaves the
    // playlist add's `play` unmeasured, so it is `false`).
    assert_eq!(
        window.press_add(),
        Command::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        }
    );
    // LIST OPTS asks for the load-playlist dialog (RFC 0010).
    assert_eq!(window.press_list_opts(), PlaylistDialog::LoadPlaylist);
    assert_eq!(
        PlaylistDialog::LoadPlaylist.opening_command(),
        Some(Command::OpenPlaylist)
    );
}

/// S06 §4: MISC opens a **`#32768` popup** with an empty window title and
/// two rows - "Generate HTML playlist" (`Ctrl+Alt+G`) and "Read extended info
/// on selection" (`Ctrl+Alt+E`) - separated by a band.
///
/// The rows are a **visual reading** in the spec; what is settled is the
/// popup's existence and its class. The two rows carry the S03 §2 ids the
/// labels name (`40292` and `40293`), so the popup and the keyboard rows
/// cannot drift apart.
#[test]
fn s06_section_4_the_misc_popup_carries_the_two_settled_rows() {
    let menu = rustamp_ui::menu::native_playlist_misc_menu();
    // The popup's window class (S04 §1: every popup is a `#32768`).
    assert_eq!(rustamp_ui::menu::MAIN_MENU_CLASS, "#32768");

    let entries: Vec<(&str, Option<&str>)> = menu
        .rows
        .iter()
        .filter_map(|row| match row {
            PopupRow::Entry {
                label, accelerator, ..
            } => Some((label.as_str(), accelerator.as_deref())),
            _ => None,
        })
        .collect();
    assert_eq!(
        entries,
        vec![
            ("Generate HTML playlist", Some("(Ctrl+Alt+G)")),
            ("Read extended info on selection", Some("(Ctrl+Alt+E)")),
        ]
    );
    // The two rows are separated by a band (S06 §4's horizontal band).
    assert_eq!(menu.rows.len(), 3);
    assert!(matches!(menu.rows[1], PopupRow::Separator));
    // The rows carry the S03 §2 command ids.
    assert!(menu.choice_for_id(40292).is_some());
    assert!(menu.choice_for_id(40293).is_some());
}

// ---------------------------------------------------------------------------
// §5 - the removal and its after-state
// ---------------------------------------------------------------------------

/// S06 §5: REM removes the selected entry on the action click (**3475 px**,
/// bounds (12,23)-(254,93)), with **no menu choice required** (§5.2).
///
/// The after-state (§5.4) is the renumbering after the removal, with no
/// selection left: the list is one entry shorter, the rows below the removed
/// one shift up, and the view stays on the list's last full page.
#[test]
fn s06_section_5_rem_removes_the_staged_entry_and_leaves_the_after_state() {
    let mut window = window_with(60);
    window.scroll_to(56);
    // One row staged: the first visible one (S06 §5.3 stages one row).
    window.mouse_down(60, 28, Modifiers::NONE);
    assert_eq!(window.selected_ids(), vec![EntryId(56)]);

    // The removal is a plain command: no menu item is involved (§5.2).
    assert_eq!(window.press_rem(), Some(Command::Remove(vec![EntryId(56)])));

    // The after-state: the list is one entry shorter and nothing is
    // selected any more.
    assert_eq!(window.len(), 59);
    assert!(window.selected_ids().is_empty());
    // The renumbering: the rows below the removed one shifted up, so the
    // bottom page of the 59-entry list holds the entries around the gap.
    assert_eq!(window.first_visible(), 55);
    assert_eq!(
        window.visible_ids(),
        vec![EntryId(55), EntryId(57), EntryId(58), EntryId(59)]
    );
    // The removed entry is gone and the rest of the page is untouched.
    assert!(!window.visible_ids().contains(&EntryId(56)));
}

/// S06 §5.3: without a staged selection the button is a no-op - R2's REM
/// second click changed **0 px** because nothing was staged, R3's changed
/// 3475 px because one row was.
#[test]
fn s06_section_5_rem_without_a_staged_selection_changes_nothing() {
    let mut window = window_with(60);
    assert_eq!(window.press_rem(), None);
    assert_eq!(window.len(), 60);
    assert!(window.selected_ids().is_empty());
    assert_eq!(window.first_visible(), 0);
}

/// The list the removal renumbers keeps its order: the entries around the
/// removed one move up by one row, and the page after the removal is the
/// list's last **full** page (S06 §5.4's renumbering).
#[test]
fn s06_section_5_the_removal_renumbers_the_rows_below_it() {
    let mut window = window_with(60);
    window.scroll_to(56);
    // Remove the second visible row instead of the first one.
    window.mouse_down(60, 28 + ROW_PITCH, Modifiers::NONE);
    assert_eq!(window.selected_ids(), vec![EntryId(57)]);
    assert_eq!(window.press_rem(), Some(Command::Remove(vec![EntryId(57)])));
    assert_eq!(window.len(), 59);
    assert!(window.selected_ids().is_empty());
    // The rows below the removed one shifted up: the page now ends with the
    // list's last entry, and the gap sits above it.
    assert_eq!(window.first_visible(), 55);
    assert_eq!(
        window.visible_ids(),
        vec![EntryId(55), EntryId(56), EntryId(58), EntryId(59)]
    );
}
