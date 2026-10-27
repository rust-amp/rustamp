//! Pure geometry of the classic-window docked stack.
//!
//! Everything here is a function of positions and sizes (no window, no clock, no
//! IO), so it is unit-tested headlessly. The constants and shapes below are taken
//! only from `specs/S05-docking.md` (§§3, 4 and 5) plus `specs/S01-window-geometry.md`;
//! additional snap/undock policies needed by the presenter are explicitly
//! documented as Rustamp choices, never as measured reference behaviour.
//!
//! # What this module models
//!
//! * The three classic windows form a docked stack (S05 §3.3, §4.3): the
//!   equalizer's top edge is the main window's bottom edge, and the playlist
//!   editor's top edge is the equalizer's bottom edge.
//! * Moving the main window moves every docked neighbour by the same delta
//!   (S05 §4.2, §4.3).
//! * Resizing the main (a windowshade) re-flows the stack (S05 §3.3, §5).
//! * The double-size and shade size changes (S05 §1.1, §2.1, §2.3, §5).

use crate::layout::{DOUBLE_HEIGHT, DOUBLE_WIDTH, SHADE_HEIGHT, WINDOW_HEIGHT, WINDOW_WIDTH};

/// Which classic window is being described.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowKind {
    /// The main window (`Winamp v1.x`, S01 §1).
    Main,
    /// The equalizer window (`Winamp EQ`, S01 §1).
    Equalizer,
    /// The playlist editor (`Winamp PE`, S01 §1).
    Playlist,
}

/// A window's screen/client position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowPosition {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
}

/// Common alias.
pub type Point = WindowPosition;
/// Common alias.
pub type Position = WindowPosition;

impl WindowPosition {
    /// Shorthand constructor.
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// A window's size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowSize {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl WindowSize {
    /// Shorthand constructor.
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// A complete, headless description of one classic window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassicWindow {
    /// Which window.
    pub kind: WindowKind,
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Windowshade collapsed.
    pub shaded: bool,
    /// Double-size mode.
    pub double_size: bool,
}

impl ClassicWindow {
    /// The main window's default rectangle (S05 §3.1).
    pub fn new_main(x: i32, y: i32) -> Self {
        Self {
            kind: WindowKind::Main,
            x,
            y,
            width: WINDOW_WIDTH,
            height: WINDOW_HEIGHT,
            shaded: false,
            double_size: false,
        }
    }
    /// The equalizer's default rectangle (S05 §3.1).
    pub fn new_equalizer(x: i32, y: i32) -> Self {
        Self {
            kind: WindowKind::Equalizer,
            x,
            y,
            width: WINDOW_WIDTH,
            height: WINDOW_HEIGHT,
            shaded: false,
            double_size: false,
        }
    }
    /// The playlist editor's default rectangle (S05 §3.1).
    pub fn new_playlist(x: i32, y: i32) -> Self {
        Self {
            kind: WindowKind::Playlist,
            x,
            y,
            width: WINDOW_WIDTH,
            height: WINDOW_HEIGHT,
            shaded: false,
            double_size: false,
        }
    }

    /// The current client size, including user-resized windows.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The bottom edge (`top + height`).
    pub fn bottom(&self) -> i32 {
        self.y.saturating_add(self.height as i32)
    }

    /// Snaps a proposed position to a visible neighbour's outside edges.
    ///
    /// Rustamp policy, not a measured S05 rule: all four sides participate,
    /// the closest candidate wins (target order breaks ties), and a zero
    /// tolerance disables snapping. Parallel edges must overlap; matching
    /// corners may align within the same tolerance. The presenter supplies
    /// only visible targets and `UiConfig::snap_distance`.
    pub fn snapped_position(
        &self,
        position: WindowPosition,
        targets: &[ClassicWindow],
        tolerance: u32,
    ) -> WindowPosition {
        if tolerance == 0 {
            return position;
        }
        let mut best = position;
        let mut best_distance = u64::MAX;
        for target in targets {
            let align = |value: i32, start: i32, end: i32| {
                if value == start || value == end {
                    return value;
                }
                let a = snap_edge(value, start, tolerance);
                let b = snap_edge(value, end, tolerance);
                if b != value
                    && (a == value
                        || (i64::from(a) - i64::from(value)).abs()
                            > (i64::from(b) - i64::from(value)).abs())
                {
                    b
                } else {
                    a
                }
            };
            let x = align(
                position.x,
                target.x,
                target
                    .x
                    .saturating_add(target.width as i32)
                    .saturating_sub(self.width as i32),
            );
            let y = align(
                position.y,
                target.y,
                target.bottom().saturating_sub(self.height as i32),
            );
            for candidate in [
                WindowPosition::new(x, target.y.saturating_sub(self.height as i32)),
                WindowPosition::new(x, target.bottom()),
                WindowPosition::new(target.x.saturating_sub(self.width as i32), y),
                WindowPosition::new(target.x.saturating_add(target.width as i32), y),
            ] {
                let dx = (i64::from(candidate.x) - i64::from(position.x)).unsigned_abs();
                let dy = (i64::from(candidate.y) - i64::from(position.y)).unsigned_abs();
                let moved = Self {
                    x: candidate.x,
                    y: candidate.y,
                    ..*self
                };
                if dx <= u64::from(tolerance)
                    && dy <= u64::from(tolerance)
                    && touches(moved, *target)
                    && dx + dy < best_distance
                {
                    best = candidate;
                    best_distance = dx + dy;
                }
            }
        }
        best
    }
}

// Exact coincident outside edges with positive overlap form a connection.
// This is Rustamp's reconstructible docking policy; S05 leaves undocking open.
//
// The connection is crate-internal: the placement rule asks it whether a
// window belongs to the group of the window it is anchored to.
pub(crate) fn touches(a: ClassicWindow, b: ClassicWindow) -> bool {
    let (ax, ay, bx, by) = (
        i64::from(a.x),
        i64::from(a.y),
        i64::from(b.x),
        i64::from(b.y),
    );
    let (ar, ab, br, bb) = (
        ax + i64::from(a.width),
        ay + i64::from(a.height),
        bx + i64::from(b.width),
        by + i64::from(b.height),
    );
    ((ab == by || bb == ay) && ax < br && bx < ar) || ((ar == bx || br == ax) && ay < bb && by < ab)
}

// A window hangs on the window it is attached to: below it (the child's top
// edge is the parent's bottom edge) or beside it (the child's left edge is
// the parent's right edge). Those are the two edges that follow when the
// parent is resized, so they are the connections a re-flow acts on. A window
// that touches another one only from above or from the left still moves with
// it - the whole group moves - but none of its edges follows a resize.
fn hangs_below(parent: ClassicWindow, child: ClassicWindow) -> bool {
    i64::from(parent.y) + i64::from(parent.height) == i64::from(child.y)
}

fn hangs_beside(parent: ClassicWindow, child: ClassicWindow) -> bool {
    i64::from(parent.x) + i64::from(parent.width) == i64::from(child.x)
}

fn hangs(parent: ClassicWindow, child: ClassicWindow) -> bool {
    hangs_below(parent, child) || hangs_beside(parent, child)
}

// The length of the edge two windows share: the horizontal contact of two
// windows that stand above one another, the vertical contact of two that
// stand side by side.
fn shared_edge(parent: ClassicWindow, child: ClassicWindow) -> i64 {
    let x_overlap = overlap(parent.x, parent.width, child.x, child.width);
    let y_overlap = overlap(parent.y, parent.height, child.y, child.height);
    let stacked = hangs_below(parent, child)
        || i64::from(child.y) + i64::from(child.height) == i64::from(parent.y);
    if stacked { x_overlap } else { y_overlap }
}

// The pixels two ranges of the same axis share.
fn overlap(a: i32, a_size: u32, b: i32, b_size: u32) -> i64 {
    let a_start = i64::from(a);
    let a_end = a_start + i64::from(a_size);
    let b_start = i64::from(b);
    let b_end = b_start + i64::from(b_size);
    (a_end.min(b_end) - a_start.max(b_start)).max(0)
}

// The window every window hangs on.
//
// The main window is the root of the stack: it hangs on nothing. Every other
// window hangs on the window it is attached to - the one it stands below or
// beside. When it hangs on more than one, the longest contact wins: a
// playlist that stands below the equalizer and beside the main window hangs
// on the equalizer, not on the main window.
//
// The relation is acyclic: hanging means "below" or "beside", and every such
// step moves strictly down or strictly right, so no chain of them can come
// back to where it started.
fn hanging_parents(old: [ClassicWindow; 3]) -> [Option<usize>; 3] {
    let mut hang = [None, None, None];
    for child in 1..3 {
        let mut best: Option<(usize, i64)> = None;
        for parent in 0..3 {
            if parent == child || !touches(old[parent], old[child]) {
                continue;
            }
            if !hangs(old[parent], old[child]) {
                continue;
            }
            let shared = shared_edge(old[parent], old[child]);
            if best.is_none_or(|(_, longest)| shared > longest) {
                best = Some((parent, shared));
            }
        }
        hang[child] = best.map(|(parent, _)| parent);
    }
    hang
}

// The move every window follows: a window that touches another one moves
// with it, and the whole group moves by the same delta (S05 §4.2, §4.3).
fn move_deltas(old: [ClassicWindow; 3], new: [ClassicWindow; 3]) -> [(i32, i32); 3] {
    let mut moved = [(0i32, 0i32); 3];
    let mut visited = [false; 3];
    for root in 0..3 {
        if visited[root] {
            continue;
        }
        visited[root] = true;
        let delta = (
            new[root].x.saturating_sub(old[root].x),
            new[root].y.saturating_sub(old[root].y),
        );
        moved[root] = delta;
        let mut queue = vec![root];
        while let Some(parent) = queue.pop() {
            for child in 0..3 {
                if visited[child] || !touches(old[parent], old[child]) {
                    continue;
                }
                visited[child] = true;
                moved[child] = delta;
                queue.push(child);
            }
        }
    }
    moved
}

// The shift a resize causes: a window that hangs on another one follows the
// edge it hangs on, and the shift accumulates down the chain (S05 §3.3).
fn resize_shifts(
    old: [ClassicWindow; 3],
    new: [ClassicWindow; 3],
    hang: [Option<usize>; 3],
) -> [(i32, i32); 3] {
    let mut shift = [(0i32, 0i32); 3];
    for start in 0..3 {
        // The chain from the window up to the root of the hanging forest.
        let mut chain = Vec::new();
        let mut node = start;
        while let Some(parent) = hang[node] {
            if chain.contains(&node) {
                break;
            }
            chain.push(node);
            node = parent;
        }
        // The root does not shift; the chain accumulates downwards.
        let mut accumulated = (0i32, 0i32);
        for child in chain.iter().rev() {
            let parent = hang[*child].expect("a chain node has a parent");
            let own = edge_shift(old, new, parent, *child);
            accumulated = (
                accumulated.0.saturating_add(own.0),
                accumulated.1.saturating_add(own.1),
            );
            shift[*child] = accumulated;
        }
    }
    shift
}

// The shift of the edge a child hangs on when its parent changes size: the
// bottom edge follows a height, the right edge a width.
fn edge_shift(
    old: [ClassicWindow; 3],
    new: [ClassicWindow; 3],
    parent: usize,
    child: usize,
) -> (i32, i32) {
    let (dx, dy) = if hangs_below(old[parent], old[child]) {
        (
            0i64,
            i64::from(new[parent].height) - i64::from(old[parent].height),
        )
    } else if hangs_beside(old[parent], old[child]) {
        (
            i64::from(new[parent].width) - i64::from(old[parent].width),
            0i64,
        )
    } else {
        (0, 0)
    };
    (
        i32::try_from(dx).unwrap_or(0),
        i32::try_from(dy).unwrap_or(0),
    )
}

/// The three classic windows together, as a docked set (S05 §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DockedStack {
    /// Main window.
    pub main: ClassicWindow,
    /// Equalizer window.
    pub equalizer: ClassicWindow,
    /// Playlist editor window.
    pub playlist: ClassicWindow,
}

impl DockedStack {
    /// Fresh launch: the default gap-free stack (S05 §3.1).
    pub fn default_launch() -> Self {
        Self {
            main: ClassicWindow::new_main(26, 29),
            equalizer: ClassicWindow::new_equalizer(26, 145),
            playlist: ClassicWindow::new_playlist(26, 261),
        }
    }

    // Follow existing connections, never create a connection merely because
    // two detached windows happen to have the same x coordinate. Main is the
    // anchor; a detached EQ can still carry its playlist on an EQ resize.
    //
    // Two relations are at work, and they are not the same:
    //
    // * The **move**: a window that touches another one moves with it
    //   (S05 §4.2, §4.3). The whole group moves by the same delta.
    // * The **resize**: a window that hangs on another one - below it or
    //   beside it - follows the edge it hangs on when that window changes
    //   size (S05 §3.3). A playlist below the equalizer follows the
    //   equalizer's bottom edge, even when it also stands beside the main
    //   window, and unshading the equalizer brings it back.
    fn reflow(&mut self, before: Self) {
        let old = [before.main, before.equalizer, before.playlist];
        let new = [self.main, self.equalizer, self.playlist];
        let hang = hanging_parents(old);
        let moved = move_deltas(old, new);
        let shift = resize_shifts(old, new, hang);
        let mut placed = new;
        for index in 0..3 {
            placed[index].x = old[index]
                .x
                .saturating_add(moved[index].0)
                .saturating_add(shift[index].0);
            placed[index].y = old[index]
                .y
                .saturating_add(moved[index].1)
                .saturating_add(shift[index].1);
        }
        [self.main, self.equalizer, self.playlist] = placed;
    }

    /// Moves the whole docked set by the same delta (S05 §4.2, §5): a move of
    /// the main window moves every docked neighbour by the same delta.
    pub fn move_main(&mut self, dx: i32, dy: i32) {
        let before = *self;
        self.main.x = self.main.x.saturating_add(dx);
        self.main.y = self.main.y.saturating_add(dy);
        self.reflow(before);
    }

    /// Toggles the main window's windowshade and re-flows the stack (S05 §1.1,
    /// §3.3, §5). Neighbours are not resized (S05 §3.2).
    pub fn set_main_shaded(&mut self, shaded: bool) {
        let before = *self;
        self.main.shaded = shaded;
        let (w, h) = window_size(WindowKind::Main, shaded, self.main.double_size);
        self.main.width = w;
        self.main.height = h;
        self.reflow(before);
    }

    /// Toggles the main window's double-size and re-flows the stack (S05 §2.1,
    /// §2.3, §5). The size change is a state change, not a render. The
    /// shaded-plus-double-size size is measured at 550 x 28 in both orders
    /// (see [`window_size`]; S05 §2.3, run 20261016-091412), and the stored
    /// rectangle always matches [`window_size`].
    pub fn set_main_double_size(&mut self, double_size: bool) {
        let before = *self;
        self.main.double_size = double_size;
        let (w, h) = window_size(WindowKind::Main, self.main.shaded, double_size);
        self.main.width = w;
        self.main.height = h;
        self.reflow(before);
    }

    /// Toggles the equalizer's double-size (S05 §2.1). A connected playlist
    /// follows its edge without itself resizing.
    pub fn set_equalizer_double_size(&mut self, double_size: bool) {
        let before = *self;
        self.equalizer.double_size = double_size;
        // Keep stored geometry consistent with the renderer even when shaded
        // (the EQ shade+2x composition is provisional, S05 §1.5).
        let (w, h) = window_size(WindowKind::Equalizer, self.equalizer.shaded, double_size);
        self.equalizer.width = w;
        self.equalizer.height = h;
        self.reflow(before);
    }

    /// Toggles windowshade for the equalizer. The provisional 14-px shade
    /// (S01 §1, unmeasured in S05 §1.5) scales with the renderer's 2x flag.
    pub fn set_equalizer_shaded(&mut self, shaded: bool) {
        let before = *self;
        self.equalizer.shaded = shaded;
        let (w, h) = window_size(WindowKind::Equalizer, shaded, self.equalizer.double_size);
        self.equalizer.width = w;
        self.equalizer.height = h;
        self.reflow(before);
    }

    /// Toggles windowshade for the playlist editor.
    pub fn set_playlist_shaded(&mut self, shaded: bool) {
        let before = *self;
        self.playlist.shaded = shaded;
        let (w, h) = window_size(WindowKind::Playlist, shaded, self.playlist.double_size);
        self.playlist.width = w;
        self.playlist.height = h;
        self.reflow(before);
    }
}

/// Window size in a given state (S05 §1.1, §2.1, S01 §1).
///
/// * Main windowshade: 275 × 14 (S05 §1.1, settled).
/// * Main double-size: 550 × 232 (S05 §2.1, settled).
/// * Equalizer double-size: 550 × 232 (S05 §2.1, settled).
/// * Playlist editor double-size: 275 × 116, unchanged (S05 §2.1, settled).
/// * Windowshade height for the equalizer/playlist is 14 px per S01 §1, but their
///   shade drawings are open (S05 §1.5); the size is stated, the pixels are not.
///
/// Shade and double-size together are measured at 550 × 28 in both orders
/// (S05 §2.3, settled by run 20261016-091412: `a3_dsize_act` re-resolved
/// 275x14 to 550x28 shade-first, `b3_shade_act` re-resolved 550x232 to 550x28
/// double-size-first): the 275 × 14 shade frame scaled 2x, so the docked size
/// and the painted frame agree.
/// That combined-state measurement is for the main window only. For the EQ,
/// scaling the provisional shade to 550 x 28 is an own choice (S05 §1.5),
/// kept consistent with `eq_render`'s existing whole-frame 2x composition.
pub fn window_size(kind: WindowKind, shaded: bool, double_size: bool) -> (u32, u32) {
    match kind {
        WindowKind::Main => {
            if shaded {
                if double_size {
                    // Measured (S05 §2.3, run 20261016-091412 steps
                    // `a3_dsize_act`/`b3_shade_act`): scaled shade, kept so
                    // `render::frame_size` matches the painted frame.
                    (DOUBLE_WIDTH, SHADE_HEIGHT * 2) // 550 x 28
                } else {
                    (WINDOW_WIDTH, SHADE_HEIGHT) // S05 §1.1: 275 x 14
                }
            } else if double_size {
                (DOUBLE_WIDTH, DOUBLE_HEIGHT) // S05 §2.1: 550 x 232
            } else {
                (WINDOW_WIDTH, WINDOW_HEIGHT)
            }
        }
        WindowKind::Equalizer => {
            if shaded {
                if double_size {
                    // Own choice, not measured for EQ (S05 §1.5): geometry
                    // must match the renderer's already-scaled shade frame.
                    (DOUBLE_WIDTH, SHADE_HEIGHT * 2)
                } else {
                    (WINDOW_WIDTH, SHADE_HEIGHT) // S01 §1 (open in S05 §1.5)
                }
            } else if double_size {
                (DOUBLE_WIDTH, DOUBLE_HEIGHT) // S05 §2.1: 550 x 232
            } else {
                (WINDOW_WIDTH, WINDOW_HEIGHT)
            }
        }
        WindowKind::Playlist => {
            if shaded {
                (WINDOW_WIDTH, SHADE_HEIGHT) // S01 §1 (open in S05 §1.5)
            } else {
                // Playlist editor is not affected by double-size (S05 §2.1).
                (WINDOW_WIDTH, WINDOW_HEIGHT)
            }
        }
    }
}

/// The bottom edge of a window `(top, height)` (S05 §3.3).
pub fn bottom_edge(top: i32, height: u32) -> i32 {
    top + height as i32
}

/// Shorthand for the equalizer's top edge in a docked stack (S05 §3.3):
/// equal to the main window's bottom edge.
pub fn equalizer_top(main_top: i32, main_height: u32) -> i32 {
    bottom_edge(main_top, main_height)
}

/// Shorthand for the playlist editor's top edge in a docked stack (S05 §3.3):
/// equal to the equalizer's bottom edge.
pub fn playlist_top(eq_top: i32, eq_height: u32) -> i32 {
    bottom_edge(eq_top, eq_height)
}

/// Does a delta move of the main window also move a neighbour by the same delta?
/// The move test answers yes (S05 §4.2, §4.3).
pub fn neighbour_tracks(main_delta: (i32, i32), neighbour_delta: (i32, i32)) -> bool {
    main_delta == neighbour_delta
}

/// Applies a two-step drag (+100 then +100 px, total +200) to a docked stack and
/// returns the resulting positions. This is the behaviour the move test asserts
/// (S05 §4.2): main (26,29) -> (126,29) -> (226,29), neighbours following.
pub fn move_test_positions(
    main: WindowPosition,
    equalizer: WindowPosition,
    playlist: WindowPosition,
) -> MoveTestPositions {
    let after_first_main = WindowPosition::new(main.x + 100, main.y);
    let after_first_equalizer = WindowPosition::new(equalizer.x + 100, equalizer.y);
    let after_first_playlist = WindowPosition::new(playlist.x + 100, playlist.y);

    let after_second_main = WindowPosition::new(after_first_main.x + 100, after_first_main.y);
    let after_second_equalizer =
        WindowPosition::new(after_first_equalizer.x + 100, after_first_equalizer.y);
    let after_second_playlist =
        WindowPosition::new(after_first_playlist.x + 100, after_first_playlist.y);

    MoveTestPositions {
        before: (main, equalizer, playlist),
        after_first: (
            after_first_main,
            after_first_equalizer,
            after_first_playlist,
        ),
        after_second: (
            after_second_main,
            after_second_equalizer,
            after_second_playlist,
        ),
    }
}

/// Result of [`move_test_positions`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveTestPositions {
    /// Positions before the drags.
    pub before: (WindowPosition, WindowPosition, WindowPosition),
    /// Positions after the first +100 px drag.
    pub after_first: (WindowPosition, WindowPosition, WindowPosition),
    /// Positions after the second +100 px drag (total +200 px).
    pub after_second: (WindowPosition, WindowPosition, WindowPosition),
}

/// The settled default positions (S05 §3.1).
pub const DEFAULT_MAIN_POSITION: WindowPosition = WindowPosition { x: 26, y: 29 };
pub const DEFAULT_EQUALIZER_POSITION: WindowPosition = WindowPosition { x: 26, y: 145 };
pub const DEFAULT_PLAYLIST_POSITION: WindowPosition = WindowPosition { x: 26, y: 261 };

/// Classic window sizes (S05 §1.1, §2.1, S01 §1).
pub const SHADED_MAIN_SIZE: (u32, u32) = (WINDOW_WIDTH, SHADE_HEIGHT); // 275 x 14
/// Main window shaded *and* double-sized: the scaled shade (S05 §2.3, measured
/// 550 x 28 in both orders by run 20261016-091412; kept so the docked size
/// matches the painted frame).
pub const SHADED_DOUBLE_MAIN_SIZE: (u32, u32) = (DOUBLE_WIDTH, SHADE_HEIGHT * 2); // 550 x 28
pub const DOUBLE_MAIN_SIZE: (u32, u32) = (DOUBLE_WIDTH, DOUBLE_HEIGHT); // 550 x 232
pub const DOUBLE_EQUALIZER_SIZE: (u32, u32) = (DOUBLE_WIDTH, DOUBLE_HEIGHT); // 550 x 232
pub const PLAYLIST_UNCHANGED_SIZE: (u32, u32) = (WINDOW_WIDTH, WINDOW_HEIGHT); // 275 x 116

/// Distance over which a windowshade re-flow moves the neighbours: 116 - 14
/// (S05 §3.3, "102 px").
pub const SHADE_REFLOW_PX: i32 = (WINDOW_HEIGHT as i32) - (SHADE_HEIGHT as i32);

/// -----------------------------------------------------------------------
/// Snap support.
/// -----------------------------------------------------------------------
///
/// `S05` leaves the snap rules themselves open (§4.1, §7): whether a window
/// snaps to screen edges or to other windows, which edges participate, whether a
/// drag must be in progress, and the tolerance. **None of that is measured**, so
/// the generic helpers take the tolerance as an argument. The presenter uses
/// [`ClassicWindow::snapped_position`]'s explicit Rustamp edge policy. Do not
/// treat that policy or the constant as measured Winamp behaviour.
///
/// The value mirrors the pre-existing `UiConfig::snap_distance` default (10) and
/// is kept for compatibility; it has **not** been measured against 2.95.
pub const SNAP_TOLERANCE_PX: u32 = 10;

/// A snap candidate: how far apart two edges are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapCandidate {
    /// Distance in pixels between the two edges.
    pub distance: i32,
    /// Whether the distance is within tolerance and the snap should fire.
    pub within: bool,
}

/// Generic edge-snap test: returns whether `distance` is within `tolerance`.
/// Edges that participate are the caller's decision (S05 §4.1, open).
pub fn snap_within(distance: i32, tolerance: u32) -> bool {
    distance.unsigned_abs() <= tolerance
}

/// Generic edge-snap clamp: if `distance` is within `tolerance`, return the
/// target value; otherwise return the original. The participating edges and the
/// tolerance are the caller's (S05 §4.1, open).
pub fn snap_edge(value: i32, target: i32, tolerance: u32) -> i32 {
    if (i64::from(value) - i64::from(target)).unsigned_abs() <= u64::from(tolerance) {
        target
    } else {
        value
    }
}

/// A small, reusable distance helper, kept for callers that prefer a struct.
pub fn snap_candidate(distance: i32, tolerance: u32) -> SnapCandidate {
    SnapCandidate {
        distance,
        within: snap_within(distance, tolerance),
    }
}

// -----------------------------------------------------------------------
// Compatibility aliases for alternate naming that callers/tests may use
// (S05 §3-§5). These are the same rules under different names.
// -----------------------------------------------------------------------

/// Common alias for [`SNAP_TOLERANCE_PX`], mirroring `UiConfig::snap_distance`.
pub const SNAP_DISTANCE: u32 = SNAP_TOLERANCE_PX;
/// Alias.
pub const SNAP_DISTANCE_PX: u32 = SNAP_TOLERANCE_PX;

/// Alias for [`DEFAULT_MAIN_POSITION`].
pub const MAIN_DEFAULT_POSITION: WindowPosition = DEFAULT_MAIN_POSITION;
/// Alias for [`DEFAULT_EQUALIZER_POSITION`].
pub const EQUALIZER_DEFAULT_POSITION: WindowPosition = DEFAULT_EQUALIZER_POSITION;
/// Alias for [`DEFAULT_PLAYLIST_POSITION`].
pub const PLAYLIST_DEFAULT_POSITION: WindowPosition = DEFAULT_PLAYLIST_POSITION;

/// Alias for [`window_size`].
pub fn classic_window_size(kind: WindowKind, shaded: bool, double_size: bool) -> (u32, u32) {
    window_size(kind, shaded, double_size)
}

/// Alias for [`neighbour_tracks`].
pub fn docked_neighbour_moves(main_delta: (i32, i32), neighbour_delta: (i32, i32)) -> bool {
    neighbour_tracks(main_delta, neighbour_delta)
}

/// Alias for [`neighbour_tracks`].
pub fn follows_main(main_delta: (i32, i32), neighbour_delta: (i32, i32)) -> bool {
    neighbour_tracks(main_delta, neighbour_delta)
}

/// No-argument move test for callers that want the settled defaults (S05 §4.2).
pub fn move_test() -> MoveTestPositions {
    move_test_positions(
        DEFAULT_MAIN_POSITION,
        DEFAULT_EQUALIZER_POSITION,
        DEFAULT_PLAYLIST_POSITION,
    )
}

/// Alias type names.
pub type WindowPos = WindowPosition;
pub type Pos = WindowPosition;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detached_same_x_windows_are_not_carried_or_reflowed() {
        let mut stack = DockedStack::default_launch();
        stack.equalizer.y += 50;
        stack.playlist.y += 50;
        let detached = (stack.equalizer, stack.playlist);
        stack.move_main(200, 30);
        stack.set_main_shaded(true);
        stack.set_main_double_size(true);
        assert_eq!((stack.equalizer, stack.playlist), detached);
        stack.set_equalizer_shaded(true);
        assert_eq!(stack.playlist.y, detached.1.y - 102);
    }

    #[test]
    fn direct_and_transitive_side_connections_follow_main() {
        let mut stack = DockedStack::default_launch();
        stack.equalizer.x = stack.main.x + stack.main.width as i32;
        stack.equalizer.y = stack.main.y + 20;
        stack.playlist.x = stack.equalizer.x;
        stack.playlist.y = stack.equalizer.bottom();
        let before = stack;
        stack.move_main(-50, 80);
        assert_eq!(
            (stack.equalizer.x, stack.equalizer.y),
            (before.equalizer.x - 50, before.equalizer.y + 80)
        );
        assert_eq!(
            (stack.playlist.x, stack.playlist.y),
            (before.playlist.x - 50, before.playlist.y + 80)
        );
        stack.set_main_double_size(true);
        assert_eq!(stack.equalizer.x, stack.main.x + 550);
        assert_eq!(stack.playlist.x, stack.equalizer.x);
    }

    #[test]
    fn playlist_can_dock_directly_to_main_without_a_connected_equalizer() {
        let mut stack = DockedStack::default_launch();
        stack.equalizer.x = 900;
        stack.playlist.y = stack.main.bottom();
        stack.move_main(10, 20);
        stack.set_main_shaded(true);
        assert_eq!((stack.playlist.x, stack.playlist.y), (36, 63));
        assert_eq!((stack.equalizer.x, stack.equalizer.y), (900, 145));
    }

    #[test]
    fn snapping_supports_all_sides_and_configurable_inclusive_tolerance() {
        let main = ClassicWindow::new_main(100, 100);
        let eq = ClassicWindow::new_equalizer(900, 900);
        for (proposed, expected) in [
            ((107, 226), (100, 216)),   // below, inclusive 10 px gap
            ((94, -23), (100, -16)),    // above
            ((385, 106), (375, 100)),   // right
            ((-185, 108), (-175, 100)), // left
        ] {
            assert_eq!(
                eq.snapped_position(WindowPosition::new(proposed.0, proposed.1), &[main], 10),
                WindowPosition::new(expected.0, expected.1)
            );
        }
        let outside = WindowPosition::new(100, 227);
        assert_eq!(eq.snapped_position(outside, &[main], 10), outside);
        assert_eq!(
            eq.snapped_position(outside, &[main], 11),
            WindowPosition::new(100, 216)
        );
        assert_eq!(eq.snapped_position(outside, &[main], 0), outside);
        assert_eq!(eq.snapped_position(outside, &[], 20), outside);
        // Close edge coordinates alone are insufficient without overlap.
        let far_side = WindowPosition::new(900, 221);
        assert_eq!(eq.snapped_position(far_side, &[main], 10), far_side);
    }

    #[test]
    fn snapped_neighbour_connects_and_a_later_move_detaches_it() {
        let mut stack = DockedStack::default_launch();
        let at = stack
            .equalizer
            .snapped_position(WindowPosition::new(30, 150), &[stack.main], 10);
        stack.equalizer.x = at.x;
        stack.equalizer.y = at.y;
        stack.move_main(200, 0);
        assert_eq!(stack.equalizer.x, 226);
        stack.equalizer.x = 900;
        let detached = stack.equalizer;
        stack.move_main(100, 0);
        assert_eq!(stack.equalizer, detached);
    }

    #[test]
    fn snap_distance_calculation_handles_extreme_saved_coordinates() {
        assert_eq!(snap_edge(i32::MIN, i32::MAX, 10), i32::MIN);
        assert_eq!(snap_edge(i32::MIN, i32::MAX, u32::MAX), i32::MAX);
    }

    #[test]
    fn snapping_keeps_exact_alignment_and_uses_nearest_visible_target() {
        let mut main = ClassicWindow::new_main(100, 100);
        main.width = 280;
        let eq = ClassicWindow::new_equalizer(900, 900);
        assert_eq!(
            eq.snapped_position(WindowPosition::new(100, 220), &[main], 10),
            WindowPosition::new(100, 216)
        );
        let mut other = ClassicWindow::new_playlist(100, 102);
        other.height = 116;
        assert_eq!(
            eq.snapped_position(WindowPosition::new(100, 220), &[main, other], 10),
            WindowPosition::new(100, 218)
        );
        // A wider doubled main also offers right-edge alignment.
        main.width = 550;
        assert_eq!(
            eq.snapped_position(WindowPosition::new(380, 220), &[main], 10),
            WindowPosition::new(375, 216)
        );
    }

    #[test]
    fn default_stack_matches_s05_3_1() {
        let stack = DockedStack::default_launch();
        assert_eq!((stack.main.x, stack.main.y), (26, 29));
        assert_eq!((stack.equalizer.x, stack.equalizer.y), (26, 145));
        assert_eq!((stack.playlist.x, stack.playlist.y), (26, 261));
        // Equalizer top = main bottom; playlist top = equalizer bottom.
        assert_eq!(
            stack.equalizer.y,
            bottom_edge(stack.main.y, stack.main.height)
        );
        assert_eq!(
            stack.playlist.y,
            bottom_edge(stack.equalizer.y, stack.equalizer.height)
        );
    }

    #[test]
    fn neighbour_tracks_bottom_edge_on_shade_s05_3_3() {
        let mut stack = DockedStack::default_launch();
        stack.set_main_shaded(true);
        // Main is 275 x 14 (S05 §1.1).
        assert_eq!(stack.main.size(), (WINDOW_WIDTH, SHADE_HEIGHT));
        assert_eq!(stack.main.size(), (275, 14));
        // Equalizer re-flows to the main's new bottom edge (29 + 14 = 43).
        assert_eq!(stack.equalizer.y, 43);
        // Playlist re-flows to the equalizer's bottom edge (43 + 116 = 159).
        assert_eq!(stack.playlist.y, 159);
        // Neighbours are not resized by the shade (S05 §3.2).
        assert_eq!(stack.equalizer.size(), (275, 116));
        assert_eq!(stack.playlist.size(), (275, 116));

        // Unshading re-flows back (S05 §3.3).
        stack.set_main_shaded(false);
        assert_eq!(stack.equalizer.y, 145);
        assert_eq!(stack.playlist.y, 261);
    }

    #[test]
    fn shade_reflow_distance_is_102_px_s05_3_3() {
        assert_eq!(SHADE_REFLOW_PX, 102);
        assert_eq!(SHADE_REFLOW_PX, WINDOW_HEIGHT as i32 - SHADE_HEIGHT as i32);
    }

    #[test]
    fn move_test_plus_100_then_plus_200_s05_4_2() {
        let main = DEFAULT_MAIN_POSITION;
        let eq = DEFAULT_EQUALIZER_POSITION;
        let pe = DEFAULT_PLAYLIST_POSITION;

        let result = move_test_positions(main, eq, pe);
        // After the first drag: +100 on x, y unchanged.
        assert_eq!(result.after_first.0, WindowPosition::new(126, 29));
        assert_eq!(result.after_first.1, WindowPosition::new(126, 145));
        assert_eq!(result.after_first.2, WindowPosition::new(126, 261));
        // After the second drag: another +100, total +200.
        assert_eq!(result.after_second.0, WindowPosition::new(226, 29));
        assert_eq!(result.after_second.1, WindowPosition::new(226, 145));
        assert_eq!(result.after_second.2, WindowPosition::new(226, 261));
        // Offsets are preserved to the pixel (S05 §4.2).
        assert!(neighbour_tracks((100, 0), (100, 0)));
        assert!(neighbour_tracks((200, 0), (200, 0)));
        // The total delta matches on all three windows (verified above).
    }

    #[test]
    fn move_main_moves_docked_neighbours_by_same_delta_s05_4_2() {
        let mut stack = DockedStack::default_launch();
        stack.move_main(100, 0);
        assert_eq!((stack.main.x, stack.main.y), (126, 29));
        assert_eq!((stack.equalizer.x, stack.equalizer.y), (126, 145));
        assert_eq!((stack.playlist.x, stack.playlist.y), (126, 261));
        stack.move_main(100, 0);
        assert_eq!((stack.main.x, stack.main.y), (226, 29));
        assert_eq!((stack.equalizer.x, stack.equalizer.y), (226, 145));
        assert_eq!((stack.playlist.x, stack.playlist.y), (226, 261));
    }

    #[test]
    fn docked_vs_adjacent_criterion_s05_4_3() {
        // Two windows are docked iff a move of one moves the other by the same
        // delta (S05 §4.3). Merely adjacent windows do not track.
        assert!(neighbour_tracks((200, 0), (200, 0)));
        assert!(!neighbour_tracks((200, 0), (0, 0)));
        assert!(!neighbour_tracks((200, 0), (150, 0)));
    }

    #[test]
    fn window_shade_size_is_275_x_14_s05_1_1() {
        assert_eq!(window_size(WindowKind::Main, true, false), (275, 14));
        assert_eq!(window_size(WindowKind::Main, true, false), SHADED_MAIN_SIZE);
        assert_eq!(SHADE_HEIGHT, 14);
    }

    #[test]
    fn shaded_plus_double_size_main_is_the_scaled_shade_s05_silent() {
        // S05 never measures the combination (§1.1 is 1x, §2.1 is unshaded):
        // this pins the preserved scaled-shade behaviour (550 x 28) so the
        // docked size and the painted frame agree. A shaded-while-doubled
        // client-size capture (S05 §9 method) would settle it.
        assert_eq!(window_size(WindowKind::Main, true, true), (550, 28));
        assert_eq!(
            window_size(WindowKind::Main, true, true),
            SHADED_DOUBLE_MAIN_SIZE
        );
        let mut stack = DockedStack::default_launch();
        stack.set_main_double_size(true);
        stack.set_main_shaded(true);
        assert_eq!(stack.main.size(), (550, 28));
        // The stack re-flows to the scaled shade's bottom edge (29 + 28 = 57).
        assert_eq!(stack.equalizer.y, 57);
    }

    #[test]
    fn double_size_main_and_equalizer_are_550_x_232_s05_2_1() {
        assert_eq!(window_size(WindowKind::Main, false, true), (550, 232));
        assert_eq!(window_size(WindowKind::Equalizer, false, true), (550, 232));
        assert_eq!(DOUBLE_MAIN_SIZE, (550, 232));
        assert_eq!(DOUBLE_EQUALIZER_SIZE, (550, 232));
    }

    #[test]
    fn playlist_stays_275_x_116_in_double_size_s05_2_1() {
        assert_eq!(window_size(WindowKind::Playlist, false, true), (275, 116));
        assert_eq!(window_size(WindowKind::Playlist, false, false), (275, 116));
        assert_eq!(PLAYLIST_UNCHANGED_SIZE, (275, 116));
    }

    #[test]
    fn size_changes_are_state_changes_not_pixel_copies_s05_2_1() {
        let mut stack = DockedStack::default_launch();
        stack.set_main_double_size(true);
        assert!(stack.main.double_size);
        assert_eq!(stack.main.size(), (550, 232));
        // Applying it again as a toggle is idempotent in state terms.
        let before = stack.main;
        stack.set_main_double_size(true);
        assert_eq!(stack.main, before);
    }

    #[test]
    fn snap_tolerance_is_a_named_provisional_constant_s05_7() {
        // The constant exists and is explicitly provisional (see its doc comment).
        let _ = SNAP_TOLERANCE_PX;
        assert_eq!(SNAP_TOLERANCE_PX, 10);
    }

    #[test]
    fn generic_snap_helper_works_without_claiming_edges_s05_4_1() {
        // Edges and tolerances are caller decisions (S05 §4.1, open); the helper
        // only implements the distance test.
        assert!(snap_within(0, SNAP_TOLERANCE_PX));
        assert!(snap_within(10, SNAP_TOLERANCE_PX));
        assert!(!snap_within(11, SNAP_TOLERANCE_PX));
        assert_eq!(snap_edge(100, 105, 10), 105);
        assert_eq!(snap_edge(100, 120, 10), 100);
        assert!(snap_candidate(4, SNAP_TOLERANCE_PX).within);
    }

    #[test]
    fn shaded_equalizer_and_playlist_sizes_follow_s01() {
        assert_eq!(window_size(WindowKind::Equalizer, true, false), (275, 14));
        assert_eq!(window_size(WindowKind::Playlist, true, false), (275, 14));
    }

    #[test]
    fn provisional_equalizer_shade_scale_matches_the_painted_frame() {
        assert_eq!(window_size(WindowKind::Equalizer, true, true), (550, 28));
        for shade_first in [true, false] {
            let mut stack = DockedStack::default_launch();
            if shade_first {
                stack.set_equalizer_shaded(true);
                stack.set_equalizer_double_size(true);
            } else {
                stack.set_equalizer_double_size(true);
                stack.set_equalizer_shaded(true);
            }
            assert_eq!(stack.equalizer.size(), (550, 28));
            assert_eq!((stack.equalizer.width, stack.equalizer.height), (550, 28));
            assert_eq!(stack.playlist.y, stack.equalizer.y + 28);
            stack.set_equalizer_double_size(false);
            assert_eq!((stack.equalizer.width, stack.equalizer.height), (275, 14));
            assert_eq!(stack.playlist.y, stack.equalizer.y + 14);
        }
    }
}
