// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Seeded property tests for the preferences dialog's model.
//!
//! The unit tests next to the model pin hand-written cases. This file states
//! the invariants the model promises as properties over randomly drawn pages,
//! rows and settings, with a seeded generator, so that a failure is
//! reproducible and the crate needs no dependency.
//!
//! # The invariants
//!
//! * **The page does not depend on the history** ([`page_items`],
//!   [`PageLists`]): building a page, then another page, then the first page
//!   again yields the same items the first time. The state of a page is a
//!   function of the page and of the settings, never of the way the user got
//!   there. This is the bug class of wave240 and wave243: a list that kept
//!   the rows of the page the user came from, and a list that refilled
//!   itself on every push.
//! * **A list belongs to its page** ([`PluginListState::shows`],
//!   [`SkinListState::shows`], [`RootListState::shows`]): the state of one
//!   page's list is never the state of another page's list, whatever the rows
//!   say. The rows of two plug-in pages can be the very same text, and the
//!   player's id behind a row must belong to the row on screen.
//! * **A push is idempotent** (the three `apply` methods): pushing the same
//!   settings twice changes nothing the second time - the list is not
//!   refilled, the selection does not move and the state is the same. A push
//!   is not a click: the model never produces an action.
//! * **A vanished selection falls back** (the three `apply` methods): a row
//!   that is gone leaves the first row in its place, or nothing when the list
//!   is empty. This holds for the plug-in list, the skin list and the media
//!   library's root list.
//! * **Nothing panics**: no page sequence, no empty list, no unknown row and
//!   no selection index out of range makes the model panic.
//!
//! # What is not claimed
//!
//! * The properties are about the **model**, not about the window: the
//!   native controls are not part of them. [`PageState`] mirrors the
//!   window's page build - it calls the same constructors the window calls
//!   ([`PageLists::shown`]) and fills the lists the page has, which
//!   [`page_items`] says.
//! * The properties do not say that the pages are the reference's pages: the
//!   reference's preferences pages are unmeasured (S04 is open), so the pages
//!   and their wording are own choices.

use super::plugin_pages;
use crate::prefs_dialog::{
    Item, LibraryRootRow, OUTPUT_MODE_CHOICES, PageLists, PluginListState, PluginRow, PrefsNode,
    PrefsSettings, REPLAYGAIN_CHOICES, RootListState, RowId, SkinListState, TREE, VIS_MODE_CHOICES,
    page_items, snap_distance_text,
};

// ---------------------------------------------------------------------------
// The seeded generator
// ---------------------------------------------------------------------------

/// How many random scenarios one property runs.
const SEEDS: u64 = 24;

/// The seed base of the property "the page does not depend on the history".
const PAGE_BASE: u64 = 0xA1_0000;

/// The seed base of the property "a list belongs to its page".
const BELONG_BASE: u64 = 0xA2_0000;

/// The seed base of the property "a push is idempotent".
const PUSH_BASE: u64 = 0xA3_0000;

/// The seed base of the property "a vanished selection falls back".
const FALLBACK_BASE: u64 = 0xA4_0000;

/// The seed base of the property "nothing panics".
const PANIC_BASE: u64 = 0xA5_0000;

/// How many pages one scenario walks through.
const PAGES: usize = 12;

/// A seeded xorshift64 generator.
///
/// The property suite must be reproducible from its seed (a failure has to
/// be replayable) and the crate takes no dependency, so the generator lives
/// here. It is not a quality generator: it only has to spread the drawn
/// values evenly enough for the properties to mean something.
struct Rng(u64);

impl Rng {
    /// A generator for `seed`. The state must not be zero.
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A uniform index in `0..n`; `0` when there is nothing to choose from.
    fn index(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.next_u64() % n as u64) as usize
    }
}

// ---------------------------------------------------------------------------
// The random world
// ---------------------------------------------------------------------------

/// A random row text.
fn random_row_text(rng: &mut Rng) -> String {
    format!("row {}", rng.index(1000))
}

/// A random page of the tree.
fn random_node(rng: &mut Rng) -> PrefsNode {
    TREE[rng.index(TREE.len())].node
}

/// A random settings push.
///
/// The rows are random in number and in content, and the skin in use is
/// sometimes a row that does not exist: the push must not panic and must not
/// select a row that is not there.
fn random_settings(rng: &mut Rng) -> PrefsSettings {
    // The skins have distinct names: the skin list knows a pick by its name,
    // and two rows of the same name would be one skin.
    let skins: Vec<String> = (0..rng.index(4)).map(|row| format!("skin {row}")).collect();
    let skin_selected = match rng.index(3) {
        0 => None,
        1 if skins.is_empty() => None,
        1 => Some(rng.index(skins.len())),
        // The player names a skin that is not among the rows.
        _ => Some(usize::MAX),
    };
    let pages = plugin_pages();
    let plugins: Vec<PluginRow> = (0..rng.index(4))
        .map(|_| PluginRow {
            node: pages[rng.index(pages.len())],
            // Every third row is an information row: it has no id.
            id: match rng.index(3) {
                0 => RowId::Info,
                _ => RowId::Plugin(rng.index(8)),
            },
            text: random_row_text(rng),
        })
        .collect();
    let library_roots: Vec<LibraryRootRow> = (0..rng.index(4))
        .map(|_| LibraryRootRow {
            path: format!(r"C:\Music\{}", rng.index(100)),
            last_scanned_at: match rng.index(2) {
                0 => None,
                _ => Some(1_700_000_000),
            },
        })
        .collect();
    PrefsSettings {
        always_on_top: rng.index(2) == 0,
        easy_move: rng.index(2) == 0,
        double_size: rng.index(2) == 0,
        shuffle: rng.index(2) == 0,
        repeat: rng.index(2) == 0,
        playlist_title_format: rng.index(2) == 0,
        playlist_title_format_text: format!("%title% {}", rng.index(100)),
        replaygain: REPLAYGAIN_CHOICES[rng.index(REPLAYGAIN_CHOICES.len())],
        output_mode: OUTPUT_MODE_CHOICES[rng.index(OUTPUT_MODE_CHOICES.len())],
        // A row that does not exist as well: the list must show a mode anyway.
        vis_mode_row: rng.index(VIS_MODE_CHOICES.len() + 2),
        snap_distance_text: snap_distance_text(rng.index(1_100) as u32),
        skins_dir_text: format!(r"C:\skins {}", rng.index(10)),
        skins,
        skin_selected,
        plugins,
        library_roots,
        // The report of the last import is the player's own text: it is not
        // part of the lists the model owns, so it stays empty here.
        winamp_ini_report: None,
        file_types_report: None,
        plugins_report: None,
    }
}

/// The settings with only the first `rows` plug-in rows of `page`.
///
/// The rows of the other pages stay: they are none of this page's business.
fn fewer_plugin_rows(settings: &PrefsSettings, page: PrefsNode, rows: usize) -> PrefsSettings {
    let mut out = settings.clone();
    let mut seen = 0;
    out.plugins.retain(|row| {
        if row.node != page {
            return true;
        }
        let keep = seen < rows;
        seen += 1;
        keep
    });
    out
}

/// The settings without the skin in the row `row`.
///
/// The skin in use stays the same skin: the index that names it moves with
/// the rows, and a skin that is gone is not named any more.
fn fewer_skins(settings: &PrefsSettings, row: usize) -> PrefsSettings {
    let mut out = settings.clone();
    out.skins.remove(row);
    out.skin_selected = match settings.skin_selected {
        None => None,
        Some(index) if index == row => None,
        Some(index) if index > row => Some(index - 1),
        Some(index) => Some(index),
    };
    out
}

// ---------------------------------------------------------------------------
// The page as the model holds it
// ---------------------------------------------------------------------------

/// The state of the page the dialog shows, as the model holds it.
///
/// The window builds the page's controls from [`page_items`] and keeps the
/// state of the three lists next to them ([`PageLists`]). This is the same
/// state without the controls: a page switch builds new controls, so the
/// lists start empty, and a push fills the lists the page has.
struct PageState {
    /// The page on screen.
    node: PrefsNode,
    /// The state of the three lists of the page.
    lists: PageLists,
}

/// The rows a page shows: its items and the rows of the lists it has.
#[derive(Debug, PartialEq, Eq)]
struct PageRows {
    /// The items of the page.
    items: Vec<Item>,
    /// The rows of the plug-in list.
    plugins: Vec<String>,
    /// The rows of the skin list.
    skins: Vec<String>,
    /// The rows of the media library's root list.
    roots: Vec<String>,
}

impl PageState {
    /// The page `node` that was just built.
    fn shown(node: PrefsNode) -> Self {
        Self {
            node,
            lists: PageLists::shown(node),
        }
    }

    /// The dialog shows `node`: the page's controls are built again, so the
    /// lists start empty.
    fn show_page(&mut self, node: PrefsNode) {
        self.node = node;
        self.lists = PageLists::shown(node);
    }

    /// What the page does with the player's push.
    ///
    /// Only the lists the page has are filled: the window has no control for
    /// the others.
    fn apply(&mut self, settings: &PrefsSettings) {
        let items = page_items(self.node);
        if items
            .iter()
            .any(|item| matches!(item, Item::PluginList { .. }))
        {
            self.lists.plugins_mut().apply(settings, self.node);
        }
        if items
            .iter()
            .any(|item| matches!(item, Item::SkinList { .. }))
        {
            self.lists.skins_mut().apply(settings);
        }
        if items
            .iter()
            .any(|item| matches!(item, Item::LibraryRoots { .. }))
        {
            self.lists.roots_mut().apply(settings);
        }
    }

    /// The rows the page shows.
    fn rows(&self) -> PageRows {
        PageRows {
            items: page_items(self.node),
            plugins: self.lists.plugins().texts().to_vec(),
            skins: self.lists.skins().texts().to_vec(),
            roots: self.lists.roots().texts().to_vec(),
        }
    }
}

// ---------------------------------------------------------------------------
// The properties
// ---------------------------------------------------------------------------

#[test]
fn a_page_shows_the_same_rows_after_a_page_switch() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(PAGE_BASE + seed);
        let settings = random_settings(&mut rng);
        let mut state = PageState::shown(random_node(&mut rng));
        for _ in 0..PAGES {
            let node = random_node(&mut rng);
            // The user picks another page: the controls are built again and
            // the lists start empty.
            state.show_page(node);
            state.apply(&settings);

            // The page of a fresh build: the state of a page is a function of
            // the page and of the settings, never of the way the user got
            // there.
            let mut fresh = PageState::shown(node);
            fresh.apply(&settings);
            assert_eq!(
                state.rows(),
                fresh.rows(),
                "seed {seed}: the page {node:?} shows other rows than a fresh build"
            );
            assert_eq!(
                state.lists, fresh.lists,
                "seed {seed}: the page {node:?} kept something of the page the user came from"
            );
            // The list of the page belongs to the page, whatever the page
            // the user came from.
            if page_items(node)
                .iter()
                .any(|item| matches!(item, Item::PluginList { .. }))
            {
                let state_of_page = PluginListState::of(&settings, node);
                assert!(
                    state.lists.plugins().shows(&state_of_page),
                    "seed {seed}: the list of {node:?} is not the list of {node:?}"
                );
                for other in plugin_pages() {
                    if other == node {
                        continue;
                    }
                    assert!(
                        !state
                            .lists
                            .plugins()
                            .shows(&PluginListState::of(&settings, other)),
                        "seed {seed}: the list of {node:?} passes for the list of {other:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_list_of_one_page_is_never_the_list_of_another_page() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(BELONG_BASE + seed);
        let mut settings = random_settings(&mut rng);
        // The rows of every plug-in page are the very same text, with the
        // very same id: the list of one page must not pass for the list of
        // another one.
        let same = "No plug-ins of this type in C:\\Plugins".to_string();
        settings.plugins = plugin_pages()
            .iter()
            .map(|page| PluginRow {
                node: *page,
                id: RowId::Plugin(7),
                text: same.clone(),
            })
            .collect();
        for page in plugin_pages() {
            let state = PluginListState::of(&settings, page);
            for other in plugin_pages() {
                if other == page {
                    continue;
                }
                let other_state = PluginListState::of(&settings, other);
                assert_eq!(
                    state.texts(),
                    other_state.texts(),
                    "seed {seed}: the rows of {page:?} and {other:?} are the same"
                );
                assert!(
                    !state.shows(&other_state),
                    "seed {seed}: the list of {page:?} passes for the list of {other:?}"
                );
            }
        }
        // The skin list and the root list belong to their page as well: a
        // list that showed nothing is not the list of a page.
        assert!(
            !SkinListState::default().shows(&SkinListState::of(&settings)),
            "seed {seed}: a list that showed nothing is not the list of the Skins page"
        );
        assert!(
            !RootListState::default().shows(&RootListState::of(&settings)),
            "seed {seed}: a list that showed nothing is not the list of the Library page"
        );
    }
}

#[test]
fn a_push_of_the_same_settings_changes_nothing_the_second_time() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(PUSH_BASE + seed);
        let settings = random_settings(&mut rng);
        let pages = plugin_pages();
        let page = pages[rng.index(pages.len())];

        // The plug-in list.
        let mut plugins = PluginListState::shown(page);
        let first = plugins.apply(&settings, page);
        let after_first = plugins.clone();
        let second = plugins.apply(&settings, page);
        if !after_first.texts().is_empty() {
            assert!(first.refill, "seed {seed}: the first push fills the list");
        }
        assert!(
            !second.refill,
            "seed {seed}: the same push does not refill the list"
        );
        assert_eq!(
            first.selected, second.selected,
            "seed {seed}: the selection does not move"
        );
        assert_eq!(
            after_first, plugins,
            "seed {seed}: the state of the plug-in list changed"
        );

        // The skin list.
        let mut skins = SkinListState::shown();
        let first = skins.apply(&settings);
        let after_first = skins.clone();
        let second = skins.apply(&settings);
        if !after_first.texts().is_empty() {
            assert!(
                first.refill,
                "seed {seed}: the first push fills the skin list"
            );
        }
        assert!(
            !second.refill,
            "seed {seed}: the same push does not refill the skin list"
        );
        assert_eq!(
            first.selected, second.selected,
            "seed {seed}: the selection of the skin list does not move"
        );
        assert_eq!(
            after_first, skins,
            "seed {seed}: the state of the skin list changed"
        );

        // The media library's root list.
        let mut roots = RootListState::shown();
        let first = roots.apply(&settings);
        let after_first = roots.clone();
        let second = roots.apply(&settings);
        if !after_first.texts().is_empty() {
            assert!(
                first.refill,
                "seed {seed}: the first push fills the root list"
            );
        }
        assert!(
            !second.refill,
            "seed {seed}: the same push does not refill the root list"
        );
        assert_eq!(
            first.selected, second.selected,
            "seed {seed}: the selection of the root list does not move"
        );
        assert_eq!(
            after_first, roots,
            "seed {seed}: the state of the root list changed"
        );
    }
}

#[test]
fn a_vanished_selection_falls_back_to_the_first_row() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(FALLBACK_BASE + seed);
        let settings = random_settings(&mut rng);

        // The plug-in list: the user picks a row, the player drops the rows
        // from there on.
        let pages = plugin_pages();
        let page = pages[rng.index(pages.len())];
        let mut plugins = PluginListState::shown(page);
        plugins.apply(&settings, page);
        if !plugins.texts().is_empty() {
            let picked = rng.index(plugins.texts().len());
            plugins.pick(picked);
            let fewer = fewer_plugin_rows(&settings, page, picked);
            let update = plugins.apply(&fewer, page);
            // The picked row is not among the rows any more: the first row
            // takes its place, or nothing when the list is empty.
            let expected = if plugins.texts().is_empty() {
                None
            } else {
                Some(0)
            };
            assert_eq!(
                update.selected, expected,
                "seed {seed}: the plug-in that is gone did not leave the first row in its place"
            );
        }

        // The skin list: the user picks a skin, the skin leaves the folder.
        if !settings.skins.is_empty() {
            let mut skins = SkinListState::shown();
            skins.apply(&settings);
            let picked = rng.index(settings.skins.len());
            skins.pick(picked);
            let fewer = fewer_skins(&settings, picked);
            let update = skins.apply(&fewer);
            // The picked skin is not among the rows any more: the first row
            // takes its place, or nothing when the list is empty.
            let expected = if fewer.skins.is_empty() {
                None
            } else {
                Some(0)
            };
            assert_eq!(
                update.selected, expected,
                "seed {seed}: the skin that is gone did not leave the first row in its place"
            );
        }

        // The media library's root list: the user picks a root, the root is
        // removed from the library.
        if !settings.library_roots.is_empty() {
            let mut roots = RootListState::shown();
            roots.apply(&settings);
            let picked = rng.index(settings.library_roots.len());
            roots.pick(picked);
            let mut fewer = settings.clone();
            fewer.library_roots.truncate(picked);
            let update = roots.apply(&fewer);
            // The picked row is not among the rows any more: the first row
            // takes its place, or nothing when the list is empty.
            let expected = if fewer.library_roots.is_empty() {
                None
            } else {
                Some(0)
            };
            assert_eq!(
                update.selected, expected,
                "seed {seed}: the root that is gone did not leave the first row in its place"
            );
        }
    }
}

#[test]
fn nothing_panics() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(PANIC_BASE + seed);
        let mut settings = random_settings(&mut rng);
        // The player names a skin that is not among the rows.
        settings.skin_selected = Some(usize::MAX);
        for entry in TREE {
            let node = entry.node;
            // The page's items.
            assert!(
                !page_items(node).is_empty(),
                "seed {seed}: the page {node:?} has no items"
            );
            // The lists of the page, with a pick that does not exist.
            let mut plugins = PluginListState::shown(node);
            plugins.pick(usize::MAX);
            plugins.apply(&settings, node);
            let mut skins = SkinListState::shown();
            skins.pick(usize::MAX);
            skins.apply(&settings);
            let mut roots = RootListState::shown();
            roots.pick(usize::MAX);
            roots.apply(&settings);
        }
        // The settings' own answers to a row that does not exist.
        assert!(
            settings.library_root(usize::MAX).is_none(),
            "seed {seed}: a root that does not exist names nothing"
        );
        for page in plugin_pages() {
            let state = PluginListState::of(&settings, page);
            assert!(
                state.id(usize::MAX) == RowId::Info,
                "seed {seed}: a plug-in that does not exist has no id"
            );
        }
    }
}
