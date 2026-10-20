// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Seeded property suite for the window layout math.
//!
//! The rules under test live in [`crate::docking`] (the docked stack, the
//! snap, the windowshade) and in [`crate::placement`] (the clamp to the
//! screen). They are pure geometry - no window, no clock, no IO - so the
//! whole suite runs headless.
//!
//! # How the suite is seeded
//!
//! Every property runs over a fixed list of seeds, and every failure names
//! the seed and the case that produced it, so a failure is reproducible by
//! hand. The generator is a small xorshift below: the crate takes no
//! dependency for it, and a fixed seed always yields the same cases.
//!
//! # The properties
//!
//! * **A docked chain moves with the main window, a detached window does
//!   not** - the docked stack is transitive, and a window that is not part
//!   of the chain stays where it is (S05 §4.2, §4.3).
//! * **A move preserves the connection graph** - a docked chain stays
//!   docked, and no window is left dangling (S05 §4.3).
//! * **A re-flow keeps the docked pairs docked** - shading or double-sizing
//!   a window re-flows its neighbours without tearing the stack apart
//!   (S05 §3.3, §5).
//! * **Docking and undocking a member reproduces the arrangement** - the
//!   round trip through the snap is stable (S05 §4.1, §4.3).
//! * **Snapping is idempotent** and never leaves a gap wider than the snap
//!   distance (S05 §4.1, open in the spec).
//! * **A shaded window keeps its width and its position, and shading twice
//!   is the original** (S05 §1.1, §3.2, §3.3).
//! * **The clamp leaves the title bar reachable** for any off-screen
//!   input, including the negative coordinates of a monitor to the left of
//!   or above the primary one.
//! * **The stack clamp leaves every title bar of the three windows
//!   reachable**, and it does not move a stack that already is.
//! * **Nothing panics** - not an empty target list, not a window that is
//!   its own snap target, not an extreme coordinate.
//!
//! # What the model does not have
//!
//! The suite states the rules the model actually has. Two cases of a
//! docking graph with window ids - docking a window to an unknown id, and a
//! cycle in the graph - have no counterpart here:
//!
//! * The stack has **no ids**: it holds the three classic windows, each
//!   named by its kind. The degenerate case of that model is the **empty
//!   target list**, and the suite snaps against it (and against a window
//!   that is its own target, which is the closest thing to a self-loop).
//! * A **cycle** cannot be built from three axis-aligned rectangles: a
//!   window that hangs below another one cannot also hang beside it, and
//!   hanging means "strictly below" or "strictly beside", so a chain of
//!   such steps cannot come back to where it started. The suite therefore
//!   checks the re-flow over every random arrangement instead - it has to
//!   terminate and leave the windows finite, which is what a cycle would
//!   break.

use rustamp_platform::{DisplaySnapshot, Monitor, ScreenRect};

use crate::docking::{
    ClassicWindow, DockedStack, WindowKind, WindowPosition, WindowSize, window_size,
};
use crate::placement::{
    clamp_into_work_area, clamp_stack_to_screen, clamp_to_screen, title_bar_inside,
};

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
const CASES: usize = 96;

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

/// The three classic windows, in the order the stack stores them.
const KINDS: [WindowKind; 3] = [
    WindowKind::Main,
    WindowKind::Equalizer,
    WindowKind::Playlist,
];

/// A window in a given state, at a given place.
fn window(kind: WindowKind, x: i32, y: i32, shaded: bool, double_size: bool) -> ClassicWindow {
    let (width, height) = window_size(kind, shaded, double_size);
    ClassicWindow {
        kind,
        x,
        y,
        width,
        height,
        shaded,
        double_size,
    }
}

/// The documented connection rule, spelled out here so the properties state
/// it independently of the implementation: two windows are docked when their
/// outside edges coincide exactly and they overlap by more than nothing.
fn docked(a: ClassicWindow, b: ClassicWindow) -> bool {
    let (ax, ay) = (i64::from(a.x), i64::from(a.y));
    let (bx, by) = (i64::from(b.x), i64::from(b.y));
    let (ar, ab) = (ax + i64::from(a.width), ay + i64::from(a.height));
    let (br, bb) = (bx + i64::from(b.width), by + i64::from(b.height));
    ((ab == by || bb == ay) && ax < br && bx < ar) || ((ar == bx || br == ax) && ay < bb && by < ab)
}

/// The three windows of a stack, in the model's order.
fn windows(stack: &DockedStack) -> [ClassicWindow; 3] {
    [stack.main, stack.equalizer, stack.playlist]
}

/// How far a window moved.
fn moved(from: ClassicWindow, to: ClassicWindow) -> (i32, i32) {
    (to.x.saturating_sub(from.x), to.y.saturating_sub(from.y))
}

/// An arrangement of the three windows plus the pairs it intends to be
/// docked.
struct Arrangement {
    /// The stack itself.
    stack: DockedStack,
    /// The docked pairs, as indices into [`KINDS`].
    docked: Vec<(usize, usize)>,
}

impl Arrangement {
    /// Is the window at `index` part of the main window's chain?
    ///
    /// The chain is the connected component of the main window in the
    /// *geometry*: a window that touches another one moves with it, and that
    /// holds through the whole component. The graph is the documented
    /// connection rule, spelled out in [`docked`], so the property states
    /// the rule independently of the implementation.
    fn in_main_chain(&self, index: usize) -> bool {
        if index == 0 {
            return true;
        }
        let windows = windows(&self.stack);
        let mut chain = vec![0usize];
        let mut grew = true;
        while grew {
            grew = false;
            for a in 0..3 {
                if !chain.contains(&a) {
                    continue;
                }
                for b in 0..3 {
                    if a == b || chain.contains(&b) {
                        continue;
                    }
                    if docked(windows[a], windows[b]) {
                        chain.push(b);
                        grew = true;
                    }
                }
            }
        }
        chain.contains(&index)
    }
}

/// A size in pixels as a coordinate offset.
fn px(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

/// Builds a random arrangement of the three classic windows.
///
/// The windows are placed at exact adjacencies, so the intended docked
/// pairs are docked. A pair that is not intended may touch as well - a
/// window below the equalizer and beside the main window touches both - and
/// the model follows the geometry. The shapes cover a plain vertical stack,
/// a side connection, a window that is docked to the main window while the
/// equalizer is not, and a stack where nothing is docked at all.
fn random_arrangement(rng: &mut Rng) -> Arrangement {
    let origin_x = rng.range(-600, 900);
    let origin_y = rng.range(-600, 900);
    let main = window(WindowKind::Main, origin_x, origin_y, rng.coin(), rng.coin());
    let equalizer = window(
        WindowKind::Equalizer,
        origin_x,
        origin_y,
        rng.coin(),
        rng.coin(),
    );
    let playlist = window(WindowKind::Playlist, origin_x, origin_y, rng.coin(), false);
    let shape = rng.below(5);
    let (equalizer, playlist, docked) = match shape {
        // A plain vertical stack: main, equalizer below it, playlist below
        // the equalizer.
        0 => (
            equalizer_at(equalizer, main.x, main.y + px(main.height)),
            playlist_at(
                playlist,
                main.x,
                main.y + px(main.height) + px(equalizer.height),
            ),
            vec![(0, 1), (1, 2)],
        ),
        // The equalizer hangs at the right of the main window, the playlist
        // below the equalizer.
        1 => (
            equalizer_at(equalizer, main.x + px(main.width), main.y),
            playlist_at(
                playlist,
                main.x + px(main.width),
                main.y + px(equalizer.height),
            ),
            vec![(0, 1), (1, 2)],
        ),
        // The playlist is docked to the main window, the equalizer is not.
        2 => (
            equalizer_at(equalizer, main.x + 900, main.y + 700),
            playlist_at(playlist, main.x, main.y + px(main.height)),
            vec![(0, 2)],
        ),
        // The equalizer hangs below the main window, the playlist at the
        // right of the equalizer.
        3 => (
            equalizer_at(equalizer, main.x, main.y + px(main.height)),
            playlist_at(
                playlist,
                main.x + px(equalizer.width),
                main.y + px(main.height),
            ),
            vec![(0, 1), (1, 2)],
        ),
        // Nothing is docked: the windows are scattered.
        _ => (
            equalizer_at(equalizer, main.x + 900, main.y + 700),
            playlist_at(playlist, main.x - 900, main.y + 700),
            Vec::new(),
        ),
    };
    Arrangement {
        stack: DockedStack {
            main,
            equalizer,
            playlist,
        },
        docked,
    }
}

/// The equalizer at a given place, keeping its size.
fn equalizer_at(equalizer: ClassicWindow, x: i32, y: i32) -> ClassicWindow {
    ClassicWindow { x, y, ..equalizer }
}

/// The playlist at a given place, keeping its size.
fn playlist_at(playlist: ClassicWindow, x: i32, y: i32) -> ClassicWindow {
    ClassicWindow { x, y, ..playlist }
}

/// A random screen: one to three monitors, some of them to the left of or
/// above the primary one, so the negative coordinates are exercised.
fn random_snapshot(rng: &mut Rng) -> DisplaySnapshot {
    let count = 1 + rng.below(3) as usize;
    let mut monitors = Vec::with_capacity(count);
    for index in 0..count {
        let width = 800 + rng.below(1200) as i32;
        let height = 600 + rng.below(500) as i32;
        // The first monitor is the primary one; the others sit left of or
        // above it, which is where the negative coordinates come from.
        let (x, y) = match index {
            0 => (0, 0),
            1 => (-width - rng.below(200) as i32, 0),
            _ => (0, -height - rng.below(200) as i32),
        };
        let work_area = ScreenRect {
            x,
            y,
            width,
            height: height - 48,
        };
        monitors.push(Monitor {
            id: format!("\\\\.\\DISPLAY{index}"),
            bounds: ScreenRect {
                x,
                y,
                width,
                height,
            },
            work_area,
            is_primary: index == 0,
            dpi: 96,
        });
    }
    DisplaySnapshot::new(monitors)
}

/// A random classic window, often far off any screen.
fn random_window(rng: &mut Rng) -> ClassicWindow {
    let kind = KINDS[rng.below(KINDS.len() as u64) as usize];
    let shaded = rng.coin();
    let double_size = rng.coin();
    let x = match rng.below(4) {
        0 => rng.range(-4000, 4000),
        1 => rng.range(-300, 300),
        2 => rng.range(i32::MIN / 2, i32::MAX / 2),
        _ => rng.range(-40, 40),
    };
    let y = match rng.below(4) {
        0 => rng.range(-4000, 4000),
        1 => rng.range(-300, 300),
        2 => rng.range(i32::MIN / 2, i32::MAX / 2),
        _ => rng.range(-40, 40),
    };
    window(kind, x, y, shaded, double_size)
}

/// A random snap distance, including the zero that switches snapping off.
fn random_tolerance(rng: &mut Rng) -> u32 {
    match rng.below(4) {
        0 => 0,
        1 => rng.below(4) as u32,
        2 => rng.below(41) as u32,
        _ => rng.below(200) as u32,
    }
}

/// A docked column of the three classic windows that lies on the primary
/// monitor, so every title bar of it is reachable.
fn reachable_stack(rng: &mut Rng) -> DockedStack {
    let x = rng.range(0, 400);
    let y = rng.range(0, 200);
    let main = window(WindowKind::Main, x, y, false, false);
    let equalizer = window(WindowKind::Equalizer, x, y + px(main.height), false, false);
    let playlist = window(
        WindowKind::Playlist,
        x,
        y + px(main.height) + px(equalizer.height),
        false,
        false,
    );
    DockedStack {
        main,
        equalizer,
        playlist,
    }
}

// ---------------------------------------------------------------------------
// The properties
// ---------------------------------------------------------------------------

/// A docked chain moves with the main window; a window that is not part of
/// the chain stays where it is (S05 §4.2, §4.3).
#[test]
fn a_docked_chain_moves_with_the_main_window_and_a_detached_one_does_not() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let arrangement = random_arrangement(&mut rng);
            let before = windows(&arrangement.stack);
            let (dx, dy) = (rng.range(-500, 500), rng.range(-500, 500));
            let mut stack = arrangement.stack;
            stack.move_main(dx, dy);
            let after = windows(&stack);
            for index in 0..3 {
                let delta = moved(before[index], after[index]);
                if arrangement.in_main_chain(index) {
                    assert_eq!(
                        delta,
                        (dx, dy),
                        "seed {seed:#x}, case {case}: the docked window {index} must follow the main window"
                    );
                } else {
                    assert_eq!(
                        delta,
                        (0, 0),
                        "seed {seed:#x}, case {case}: the detached window {index} must not move"
                    );
                }
            }
        }
    }
}

/// A move preserves the connection graph: the docked pairs stay docked and
/// no pair that was apart comes together (S05 §4.3).
#[test]
fn a_move_preserves_the_connection_graph() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let arrangement = random_arrangement(&mut rng);
            let before = windows(&arrangement.stack);
            let mut stack = arrangement.stack;
            stack.move_main(rng.range(-500, 500), rng.range(-500, 500));
            let after = windows(&stack);
            for a in 0..3 {
                for b in (a + 1)..3 {
                    assert_eq!(
                        docked(before[a], before[b]),
                        docked(after[a], after[b]),
                        "seed {seed:#x}, case {case}: the move changed the connection between {a} and {b}"
                    );
                }
            }
        }
    }
}

/// The arrangement the property builds says what it means: the intended
/// docked pairs are docked. A pair that is not intended may be docked as
/// well - a window that stands below the equalizer and beside the main
/// window touches both - and the model follows the geometry.
#[test]
fn the_random_arrangements_dock_what_they_mean_to() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let arrangement = random_arrangement(&mut rng);
            let windows = windows(&arrangement.stack);
            for (a, b) in &arrangement.docked {
                assert!(
                    docked(windows[*a], windows[*b]),
                    "seed {seed:#x}, case {case}: the arrangement does not dock {a} and {b}"
                );
            }
        }
    }
}

/// A re-flow (a windowshade or a double-size) keeps the docked pairs
/// docked (S05 §3.3, §5).
#[test]
fn a_reflow_keeps_the_docked_pairs_docked() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let arrangement = random_arrangement(&mut rng);
            let mut stack = arrangement.stack;
            match rng.below(4) {
                0 => stack.set_main_shaded(!stack.main.shaded),
                1 => stack.set_main_double_size(!stack.main.double_size),
                2 => stack.set_equalizer_shaded(!stack.equalizer.shaded),
                _ => stack.set_playlist_shaded(!stack.playlist.shaded),
            }
            let after = windows(&stack);
            for (a, b) in &arrangement.docked {
                assert!(
                    docked(after[*a], after[*b]),
                    "seed {seed:#x}, case {case}: the re-flow tore {a} and {b} apart"
                );
            }
        }
    }
}

/// Docking a chain and undocking any member reproduces the arrangement: the
/// snap that docks a window is stable, and undocking one member leaves the
/// rest of the chain alone (S05 §4.1, §4.3).
#[test]
fn docking_and_undocking_a_member_reproduces_the_arrangement() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            // A plain stack: the equalizer can be docked below the main
            // window, and the playlist below the equalizer.
            let origin_x = rng.range(-600, 900);
            let origin_y = rng.range(-600, 900);
            let mut stack = DockedStack {
                main: window(WindowKind::Main, origin_x, origin_y, false, false),
                equalizer: window(
                    WindowKind::Equalizer,
                    origin_x,
                    origin_y + 116,
                    false,
                    false,
                ),
                playlist: window(WindowKind::Playlist, origin_x, origin_y + 232, false, false),
            };
            // A snap distance that can reach the docking spot.
            let tolerance = 6 + rng.below(15) as u32;
            let before = windows(&stack);

            // Dock the equalizer to the main window: the snap is what puts a
            // window next to another one.
            let proposed = WindowPosition::new(
                stack.main.x + rng.range(-5, 5),
                stack.main.bottom() + rng.range(-5, 5),
            );
            let docked_at = stack
                .equalizer
                .snapped_position(proposed, &[stack.main], tolerance);
            stack.equalizer.x = docked_at.x;
            stack.equalizer.y = docked_at.y;
            let docked_equalizer = stack.equalizer;
            assert!(
                docked(docked_equalizer, stack.main),
                "seed {seed:#x}, case {case}: the snap did not dock the equalizer"
            );

            // Move the main window: the docked neighbour follows.
            let (dx, dy) = (rng.range(-300, 300), rng.range(-300, 300));
            stack.move_main(dx, dy);
            let after_move = windows(&stack);
            assert_eq!(
                moved(docked_equalizer, after_move[1]),
                (dx, dy),
                "seed {seed:#x}, case {case}: the docked neighbour did not follow"
            );

            // Undock the equalizer again: the rest of the chain stays where
            // it is.
            stack.equalizer.x = before[1].x;
            stack.equalizer.y = before[1].y;
            let after_undock = windows(&stack);
            assert_eq!(
                (after_undock[0], after_undock[2]),
                (after_move[0], after_move[2]),
                "seed {seed:#x}, case {case}: undocking a member moved the rest of the chain"
            );

            // Re-docking with the same snap gives the same place back: the
            // snap is a pure function of the proposal and the target.
            let redocked = stack
                .equalizer
                .snapped_position(proposed, &[stack.main], tolerance);
            assert_eq!(
                redocked,
                stack
                    .equalizer
                    .snapped_position(proposed, &[stack.main], tolerance),
                "seed {seed:#x}, case {case}: the snap is not reproducible"
            );

            // And the arrangement the round trip produces is the one the
            // chain had.
            stack.equalizer.x = redocked.x;
            stack.equalizer.y = redocked.y;
            assert_eq!(
                (stack.main, stack.playlist),
                (after_move[0], after_move[2]),
                "seed {seed:#x}, case {case}: the round trip changed the chain"
            );
        }
    }
}

/// Snapping is idempotent: snapping an already snapped position does not
/// move it any further (S05 §4.1, open in the spec).
#[test]
fn snapping_is_idempotent() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let tolerance = random_tolerance(&mut rng);
            let mover = random_window(&mut rng);
            let targets: Vec<ClassicWindow> = (0..rng.below(3) as usize)
                .map(|_| random_window(&mut rng))
                .collect();
            let proposed = WindowPosition::new(rng.range(-2000, 2000), rng.range(-2000, 2000));
            let once = mover.snapped_position(proposed, &targets, tolerance);
            let twice = mover.snapped_position(once, &targets, tolerance);
            assert_eq!(
                once, twice,
                "seed {seed:#x}, case {case}: snapping twice moved the window again"
            );
        }
    }
}

/// A snap that fires leaves no gap: the window ends up exactly against the
/// target it snapped to, and it never moves further than the snap distance
/// (S05 §4.1, open in the spec).
#[test]
fn a_snap_never_leaves_a_gap_wider_than_the_snap_distance() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let tolerance = random_tolerance(&mut rng);
            let mover = random_window(&mut rng);
            let targets: Vec<ClassicWindow> = (0..rng.below(3) as usize)
                .map(|_| random_window(&mut rng))
                .collect();
            let proposed = WindowPosition::new(rng.range(-2000, 2000), rng.range(-2000, 2000));
            let snapped = mover.snapped_position(proposed, &targets, tolerance);
            let distance = (
                (i64::from(snapped.x) - i64::from(proposed.x)).unsigned_abs(),
                (i64::from(snapped.y) - i64::from(proposed.y)).unsigned_abs(),
            );
            assert!(
                distance.0 <= u64::from(tolerance) && distance.1 <= u64::from(tolerance),
                "seed {seed:#x}, case {case}: the snap moved the window by {distance:?}, tolerance {tolerance}"
            );
            if snapped != proposed {
                let moved_window = ClassicWindow {
                    x: snapped.x,
                    y: snapped.y,
                    ..mover
                };
                assert!(
                    targets.iter().any(|target| docked(moved_window, *target)),
                    "seed {seed:#x}, case {case}: the window snapped to nothing"
                );
            }
        }
    }
}

/// A shaded window keeps its width and its place, and shading it twice is
/// the original (S05 §1.1, §3.2, §3.3).
#[test]
fn a_shaded_window_keeps_its_width_and_its_place() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let arrangement = random_arrangement(&mut rng);
            let before = windows(&arrangement.stack);
            let mut stack = arrangement.stack;
            match rng.below(3) {
                0 => stack.set_main_shaded(!stack.main.shaded),
                1 => stack.set_equalizer_shaded(!stack.equalizer.shaded),
                _ => stack.set_playlist_shaded(!stack.playlist.shaded),
            }
            let after = windows(&stack);
            let index = match stack.main.shaded != before[0].shaded {
                true => 0,
                false => {
                    if stack.equalizer.shaded != before[1].shaded {
                        1
                    } else {
                        2
                    }
                }
            };
            assert_eq!(
                after[index].width, before[index].width,
                "seed {seed:#x}, case {case}: the shade changed the width of window {index}"
            );
            assert_eq!(
                (after[index].x, after[index].y),
                (before[index].x, before[index].y),
                "seed {seed:#x}, case {case}: the shade moved window {index}"
            );

            // Shading it a second time is the original.
            match index {
                0 => stack.set_main_shaded(before[0].shaded),
                1 => stack.set_equalizer_shaded(before[1].shaded),
                _ => stack.set_playlist_shaded(before[2].shaded),
            }
            assert_eq!(
                windows(&stack),
                before,
                "seed {seed:#x}, case {case}: shading twice is not the original"
            );
        }
    }
}

/// The clamp leaves the title bar of the window reachable: the strip lies
/// inside a work area - for any off-screen input, including the negative
/// coordinates of a monitor to the left of or above the primary one.
#[test]
fn the_clamp_leaves_the_title_bar_reachable() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let snapshot = random_snapshot(&mut rng);
            let window = random_window(&mut rng);
            let placed = clamp_to_screen(&snapshot, window);
            assert!(
                snapshot
                    .monitors()
                    .iter()
                    .any(|monitor| title_bar_inside(&placed, &monitor.work_area)),
                "seed {seed:#x}, case {case}: the title bar of {placed:?} is on no screen"
            );
        }
    }
}

/// The clamp leaves a window that is already reachable where it is.
#[test]
fn the_clamp_does_not_move_a_reachable_window() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let snapshot = random_snapshot(&mut rng);
            let window = random_window(&mut rng);
            let placed = clamp_to_screen(&snapshot, window);
            let reachable = snapshot
                .monitors()
                .iter()
                .any(|monitor| title_bar_inside(&window, &monitor.work_area));
            if reachable {
                assert_eq!(
                    (placed.x, placed.y),
                    (window.x, window.y),
                    "seed {seed:#x}, case {case}: a reachable window was moved"
                );
            }
        }
    }
}

/// The stack clamp leaves the title bar of **every** window reachable: the
/// strip of each of the three lies inside a work area, for any input,
/// including the negative coordinates of a monitor to the left of or above
/// the primary one.
#[test]
fn the_stack_clamp_leaves_every_title_bar_reachable() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let snapshot = random_snapshot(&mut rng);
            let stack = random_arrangement(&mut rng).stack;
            let placed = clamp_stack_to_screen(&snapshot, stack);
            for window in windows(&placed) {
                assert!(
                    snapshot
                        .monitors()
                        .iter()
                        .any(|monitor| title_bar_inside(&window, &monitor.work_area)),
                    "seed {seed:#x}, case {case}: the title bar of {window:?} is on no screen"
                );
            }
        }
    }
}

/// The stack clamp does not move a stack that is already reachable: the
/// player must not shift the windows the user placed on every start.
#[test]
fn the_stack_clamp_does_not_move_a_reachable_stack() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let snapshot = random_snapshot(&mut rng);
            let stack = reachable_stack(&mut rng);
            let placed = clamp_stack_to_screen(&snapshot, stack);
            assert_eq!(
                windows(&placed),
                windows(&stack),
                "seed {seed:#x}, case {case}: a reachable stack was moved"
            );
        }
    }
}

/// The work-area rule on its own: the title bar ends up inside the work
/// area, and a window that is already inside keeps its place.
#[test]
fn the_work_area_rule_keeps_the_title_bar_inside() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let work_area = ScreenRect {
                x: rng.range(-2000, 2000),
                y: rng.range(-2000, 2000),
                width: rng.range(1, 2000),
                height: rng.range(1, 2000),
            };
            let window = random_window(&mut rng);
            let placed = clamp_into_work_area(window, &work_area);
            assert!(
                title_bar_inside(&placed, &work_area),
                "seed {seed:#x}, case {case}: the title bar of {placed:?} is not inside {work_area:?}"
            );
        }
    }
}

/// The window every window hangs on follows the edge it hangs on, and it
/// comes back when the size returns.
///
/// The playlist stands below the equalizer and its left edge is the main
/// window's right edge, so it touches two windows. It hangs on the
/// equalizer - the longest contact - so a shade of the equalizer moves it
/// to the equalizer's new bottom edge, and unshading the equalizer brings
/// it back to the pixel (S05 §3.3).
#[test]
fn a_window_below_another_follows_its_bottom_edge_even_when_it_touches_a_third() {
    let mut stack = DockedStack {
        main: window(WindowKind::Main, 0, 0, false, false),
        equalizer: window(WindowKind::Equalizer, 275, 0, false, false),
        playlist: window(WindowKind::Playlist, 275, 116, false, false),
    };
    // The playlist touches the equalizer below it and the main window
    // beside it.
    assert!(docked(stack.equalizer, stack.playlist));
    assert!(!docked(stack.main, stack.playlist));

    stack.set_equalizer_shaded(true);
    assert_eq!(stack.equalizer.height, 14);
    assert_eq!(stack.playlist.y, stack.equalizer.bottom());

    stack.set_equalizer_shaded(false);
    assert_eq!(stack.playlist.y, 116);
    assert_eq!(stack.playlist.x, 275);
}

/// Nothing panics: no empty target list, no window that is its own snap
/// target, no extreme coordinate, no empty screen.
#[test]
fn no_layout_call_panics() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for case in 0..CASES {
            let tolerance = random_tolerance(&mut rng);
            let mover = random_window(&mut rng);
            let proposed = WindowPosition::new(
                rng.range(i32::MIN / 2, i32::MAX / 2),
                rng.range(i32::MIN / 2, i32::MAX / 2),
            );

            // An empty target list, and the window itself as its own target.
            let _ = mover.snapped_position(proposed, &[], tolerance);
            let _ = mover.snapped_position(proposed, &[mover], tolerance);

            // Extreme coordinates and sizes.
            let extreme = ClassicWindow {
                kind: WindowKind::Main,
                x: i32::MIN,
                y: i32::MIN,
                width: u32::MAX,
                height: u32::MAX,
                shaded: false,
                double_size: false,
            };
            let _ = extreme.snapped_position(proposed, &[mover], u32::MAX);
            let _ = extreme.bottom();

            // An empty screen, and a screen with a degenerate work area.
            let empty = DisplaySnapshot::new(Vec::new());
            let placed = clamp_to_screen(&empty, mover);
            assert_eq!(
                (placed.x, placed.y),
                (mover.x, mover.y),
                "seed {seed:#x}, case {case}: a screen-less clamp moved the window"
            );
            let degenerate = DisplaySnapshot::new(vec![Monitor {
                id: "\\\\.\\DISPLAY1".to_string(),
                bounds: ScreenRect {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                },
                work_area: ScreenRect {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                },
                is_primary: true,
                dpi: 96,
            }]);
            let _ = clamp_to_screen(&degenerate, mover);

            // A stack of extreme windows.
            let mut stack = DockedStack {
                main: extreme,
                equalizer: extreme,
                playlist: extreme,
            };
            stack.move_main(i32::MAX, i32::MIN);
            stack.set_main_shaded(true);
            stack.set_main_double_size(true);
            stack.set_equalizer_shaded(true);
            stack.set_equalizer_double_size(true);
            stack.set_playlist_shaded(true);
        }
    }
}

/// The sizes the model produces are the ones the shade and the double-size
/// promise, and a shaded window is never wider than its unshaded form.
#[test]
fn the_shade_keeps_the_width_of_every_window_kind() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for _ in 0..CASES {
            let kind = KINDS[rng.below(KINDS.len() as u64) as usize];
            for double_size in [false, true] {
                let (plain_width, plain_height) = window_size(kind, false, double_size);
                let (shaded_width, shaded_height) = window_size(kind, true, double_size);
                assert_eq!(
                    shaded_width, plain_width,
                    "the shade changed the width of {kind:?}"
                );
                assert!(
                    shaded_height < plain_height,
                    "the shade did not shrink {kind:?}"
                );
            }
        }
    }
}

/// A window of the model's own size keeps its size through the layout
/// calls: the model's rectangles are the measured ones.
#[test]
fn the_model_uses_the_measured_sizes() {
    let stack = DockedStack::default_launch();
    let windows = windows(&stack);
    for window in windows {
        let (width, height) = window_size(window.kind, window.shaded, window.double_size);
        assert_eq!((window.width, window.height), (width, height));
        let size = WindowSize::new(width, height);
        assert_eq!(size.width, window.width);
    }
}
