//! One-shot reordering helpers and S15 playback traversal order.
//!
//! [`Playlist::sort`](rustamp_core::playlist::Playlist::sort) and
//! [`Playlist::reverse`](rustamp_core::playlist::Playlist::reverse) already live in
//! `rustamp-core` and are **not** duplicated here. What is missing there is
//! `randomize`: a single random permutation of the current order.
//!
//! [`PlaybackOrder`] implements the S15 shuffle sequence and repeat traversal
//! state machine, managing permutation plans, repeat boundaries and mid-run
//! toggling. The step back to the previous track is a walk backwards through
//! the permutation and the cycle boundary it crossed, so the model keeps no
//! separate history of the played tracks. It is a pure, in-memory model over
//! queue indices: it does not touch the playlist's display order (S15 §5).

use rustamp_core::playlist::{EntryId, Playlist};

use crate::rng::Rng;

/// The largest number of entries a single [`PlaybackOrder::skip_by`] walks.
///
/// The model steps through the traversal one entry at a time, so the delta
/// decides how many steps are taken. The user interface asks for ten entries
/// at a time (RFC 0005 §5.1), so this bound leaves every legitimate request
/// exact. A delta that exceeds it is clamped to the bound: the walk stops
/// there instead of spinning the audio thread for 2^31 steps.
pub const MAX_SKIP_STEPS: u32 = 1024;

/// Shuffles a slice in place with the Fisher-Yates algorithm.
pub fn shuffle<T>(items: &mut [T], rng: &mut Rng) {
    for i in (1..items.len()).rev() {
        let j = rng.below(i + 1);
        items.swap(i, j);
    }
}

/// A random permutation of `ids`.
pub fn shuffled_ids(ids: &[EntryId], rng: &mut Rng) -> Vec<EntryId> {
    let mut out = ids.to_vec();
    shuffle(&mut out, rng);
    out
}

/// Applies one random permutation to the playlist, from a seeded generator.
///
/// Entry ids, the selection and the current entry are untouched:
/// [`Playlist::apply_order`] only changes the order.
pub fn randomize(playlist: &mut Playlist, seed: u64) {
    let mut rng = Rng::new(seed);
    randomize_with(playlist, &mut rng);
}

/// Like [`randomize`], but draws from a caller-owned generator so several
/// reorderings can share one stream.
pub fn randomize_with(playlist: &mut Playlist, rng: &mut Rng) {
    let ids: Vec<_> = playlist.iter().map(|e| e.id).collect();
    let order = shuffled_ids(&ids, rng);
    playlist.apply_order(&order);
}

/// Playback order traversal manager, implementing S15 shuffle and repeat modes.
///
/// Tracks sequential traversal and shuffle permutations, repeat boundaries and
/// mid-run mode toggles. The plan is stored over queue indices; the visible
/// playlist order is never reordered (S15 §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackOrder {
    len: usize,
    shuffle: bool,
    repeat: bool,
    current_index: Option<usize>,
    shuffle_plan: Vec<usize>,
    shuffle_pos: usize,
    prev_shuffle_plan: Option<Vec<usize>>,
    rng: Rng,
}

impl PlaybackOrder {
    /// Creates a new playback order manager.
    pub fn new(len: usize, shuffle: bool, repeat: bool, seed: u64) -> Self {
        let mut order = Self {
            len,
            shuffle,
            repeat,
            current_index: None,
            shuffle_plan: Vec::new(),
            shuffle_pos: 0,
            prev_shuffle_plan: None,
            rng: Rng::new(seed),
        };
        if shuffle && len > 0 {
            order.generate_shuffle_plan(None);
        }
        order
    }

    /// Number of items in the queue.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether shuffle mode is active.
    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    /// Whether repeat mode is active.
    pub fn repeat(&self) -> bool {
        self.repeat
    }

    /// Current queue index, if any.
    pub fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    /// Current shuffle permutation plan.
    pub fn shuffle_plan(&self) -> &[usize] {
        &self.shuffle_plan
    }

    /// The plan of the previous shuffle cycle, if one was just exhausted and
    /// retained for backward navigation across the cycle boundary (S15 §3.2).
    pub fn prev_shuffle_plan(&self) -> Option<&[usize]> {
        self.prev_shuffle_plan.as_deref()
    }

    /// Current position in the shuffle plan.
    pub fn shuffle_pos(&self) -> usize {
        self.shuffle_pos
    }

    /// Explicitly set the current index (e.g. user selected track).
    ///
    /// In shuffle mode the active permutation is kept and the cursor moves
    /// into it: the traversal continues behind the selected track, and the
    /// entries before it in the permutation are deferred to the next cycle.
    /// The permutation itself is only rebuilt by [`set_shuffle`](Self::set_shuffle),
    /// [`set_len`](Self::set_len) and [`invalidate`](Self::invalidate).
    ///
    /// The engine's decode thread relies on that: it re-aligns its own
    /// place in a permutation the engine re-planned (S15 §4.1) without
    /// disturbing it.
    pub fn set_current_index(&mut self, index: Option<usize>) {
        if let Some(idx) = index {
            if idx < self.len {
                self.current_index = Some(idx);
                if self.shuffle {
                    self.prev_shuffle_plan = None;
                    if let Some(pos) = self.shuffle_plan.iter().position(|&p| p == idx) {
                        self.shuffle_pos = pos;
                    } else {
                        self.replan_from_current(idx);
                    }
                }
            } else {
                self.current_index = None;
            }
        } else {
            self.current_index = None;
        }
    }

    /// Update the playlist length (e.g. when entries are added or removed).
    ///
    /// The shuffle plan is regenerated whenever it no longer covers the queue
    /// (S15 invalidation on add/remove). The current index is cleared if it
    /// falls outside the new length.
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
        if matches!(self.current_index, Some(curr) if curr >= len) {
            self.current_index = None;
        }
        self.prev_shuffle_plan = None;
        if self.shuffle {
            if len == 0 {
                self.shuffle_plan.clear();
                self.shuffle_pos = 0;
            } else {
                self.generate_shuffle_plan(self.current_index);
                if let Some(curr) = self.current_index
                    && let Some(pos) = self.shuffle_plan.iter().position(|&p| p == curr)
                {
                    self.shuffle_pos = pos;
                }
            }
        }
    }

    /// Invalidate the traversal order after a playlist mutation that does not
    /// change its length (e.g. sort, reverse, randomize) or on an explicit
    /// request. Regenerates the shuffle plan anchored at the current track
    /// (S15 §4.1) and drops the retained previous cycle.
    pub fn invalidate(&mut self) {
        self.prev_shuffle_plan = None;
        if !self.shuffle || self.len == 0 {
            return;
        }
        if let Some(curr) = self.current_index {
            self.replan_from_current(curr);
        } else {
            self.generate_shuffle_plan(None);
        }
        if let Some(curr) = self.current_index
            && let Some(pos) = self.shuffle_plan.iter().position(|&p| p == curr)
        {
            self.shuffle_pos = pos;
        }
    }

    /// Re-seed the permutation generator.
    ///
    /// The next permutation is drawn from the new seed. The active plan is
    /// left alone: it is the permutation the traversal follows, and a
    /// re-seed must not move the play cursor.
    pub fn set_seed(&mut self, seed: u64) {
        self.rng = Rng::new(seed);
    }

    /// Set shuffle mode enabled/disabled (S15 §4).
    pub fn set_shuffle(&mut self, shuffle: bool) {
        if self.shuffle == shuffle {
            return;
        }
        self.shuffle = shuffle;
        self.prev_shuffle_plan = None;
        if self.shuffle && self.len > 0 {
            if let Some(curr) = self.current_index {
                self.replan_from_current(curr);
            } else {
                self.generate_shuffle_plan(None);
            }
            if let Some(curr) = self.current_index {
                if let Some(pos) = self.shuffle_plan.iter().position(|&p| p == curr) {
                    self.shuffle_pos = pos;
                } else {
                    self.shuffle_pos = 0;
                }
            }
        } else {
            self.shuffle_plan.clear();
            self.shuffle_pos = 0;
        }
    }

    /// Toggle shuffle mode.
    pub fn toggle_shuffle(&mut self) {
        self.set_shuffle(!self.shuffle);
    }

    /// Set repeat mode enabled/disabled.
    pub fn set_repeat(&mut self, repeat: bool) {
        self.repeat = repeat;
    }

    /// Toggle repeat mode.
    pub fn toggle_repeat(&mut self) {
        self.set_repeat(!self.repeat);
    }

    /// Advance to the next track in the active playback order.
    /// Returns `Some(index)` of the new track, or `None` if the queue ends.
    pub fn next_index(&mut self) -> Option<usize> {
        if self.len == 0 {
            return None;
        }
        if !self.shuffle {
            let next = match self.current_index {
                None => 0,
                Some(curr) => {
                    if curr + 1 < self.len {
                        curr + 1
                    } else if self.repeat {
                        0
                    } else {
                        return None;
                    }
                }
            };
            self.current_index = Some(next);
            Some(next)
        } else {
            if self.shuffle_plan.is_empty() || self.shuffle_plan.len() != self.len {
                self.generate_shuffle_plan(self.current_index);
            }
            if self.current_index.is_none() {
                self.shuffle_pos = 0;
                let next = self.shuffle_plan[0];
                self.current_index = Some(next);
                return Some(next);
            }
            if self.shuffle_pos + 1 < self.shuffle_plan.len() {
                self.shuffle_pos += 1;
                let next = self.shuffle_plan[self.shuffle_pos];
                self.current_index = Some(next);
                Some(next)
            } else if self.repeat {
                let last = self.shuffle_plan.last().copied();
                // Retain the exhausted cycle for backward navigation (S15 §3.2).
                self.prev_shuffle_plan = Some(std::mem::take(&mut self.shuffle_plan));
                self.generate_new_cycle(last);
                self.shuffle_pos = 0;
                let next = self.shuffle_plan[0];
                self.current_index = Some(next);
                Some(next)
            } else {
                None
            }
        }
    }

    /// Step back to the previous track in the active playback order.
    ///
    /// In shuffle mode this walks backwards through the active permutation;
    /// at its head it crosses into the retained previous cycle when repeat
    /// is on (S15 §3.2). The step back is a walk over the permutation, so
    /// the model keeps no separate history of the played tracks.
    pub fn prev_index(&mut self) -> Option<usize> {
        if self.len == 0 {
            return None;
        }
        if !self.shuffle {
            let prev = match self.current_index {
                None => 0,
                Some(curr) => {
                    if curr > 0 {
                        curr - 1
                    } else if self.repeat {
                        self.len - 1
                    } else {
                        0
                    }
                }
            };
            self.current_index = Some(prev);
            Some(prev)
        } else {
            if self.shuffle_plan.is_empty() {
                self.generate_shuffle_plan(self.current_index);
            }
            if self.shuffle_pos > 0 {
                self.shuffle_pos -= 1;
                let prev = self.shuffle_plan[self.shuffle_pos];
                self.current_index = Some(prev);
                Some(prev)
            } else if self.repeat && self.len > 1 {
                // Restore the previous cycle when available (S15 §3.2).
                if let Some(prev_plan) = self.prev_shuffle_plan.take() {
                    self.shuffle_plan = prev_plan;
                    self.shuffle_pos = self.shuffle_plan.len().saturating_sub(1);
                    let prev = self.shuffle_plan[self.shuffle_pos];
                    self.current_index = Some(prev);
                    Some(prev)
                } else {
                    self.shuffle_pos = self.shuffle_plan.len().saturating_sub(1);
                    let prev = self.shuffle_plan[self.shuffle_pos];
                    self.current_index = Some(prev);
                    Some(prev)
                }
            } else {
                self.current_index
            }
        }
    }

    /// Step by signed `delta` positions in the active playback order.
    ///
    /// The walk is bounded: the number of steps is clamped to
    /// [`MAX_SKIP_STEPS`], so a delta that arrives over the wire cannot turn
    /// into an unbounded loop on the audio thread. Every delta within the
    /// bound keeps the exact wrap semantics of S15 §3.3; a delta beyond it
    /// walks the bound and stops there.
    pub fn skip_by(&mut self, delta: i32) -> Option<usize> {
        if self.len == 0 || delta == 0 {
            return self.current_index;
        }
        let steps = delta.unsigned_abs().min(MAX_SKIP_STEPS);
        let mut res = self.current_index;
        if delta > 0 {
            for _ in 0..steps {
                match self.next_index() {
                    Some(idx) => res = Some(idx),
                    None => break,
                }
            }
            res
        } else {
            for _ in 0..steps {
                match self.prev_index() {
                    Some(idx) => res = Some(idx),
                    None => break,
                }
            }
            res
        }
    }

    fn generate_shuffle_plan(&mut self, current: Option<usize>) {
        if self.len == 0 {
            self.shuffle_plan.clear();
            self.shuffle_pos = 0;
            self.prev_shuffle_plan = None;
            return;
        }
        let mut plan: Vec<usize> = (0..self.len).collect();
        shuffle(&mut plan, &mut self.rng);
        if let Some(pos) = current.and_then(|curr| plan.iter().position(|&p| p == curr)) {
            plan.swap(0, pos);
        }
        self.shuffle_plan = plan;
        self.shuffle_pos = 0;
        self.prev_shuffle_plan = None;
    }

    fn replan_from_current(&mut self, curr: usize) {
        if self.len == 0 {
            return;
        }
        let mut remaining: Vec<usize> = (0..self.len).filter(|&i| i != curr).collect();
        shuffle(&mut remaining, &mut self.rng);
        let mut plan = Vec::with_capacity(self.len);
        plan.push(curr);
        plan.extend(remaining);
        self.shuffle_plan = plan;
        self.shuffle_pos = 0;
        self.prev_shuffle_plan = None;
    }

    fn generate_new_cycle(&mut self, last_track: Option<usize>) {
        if self.len == 0 {
            return;
        }
        let mut plan: Vec<usize> = (0..self.len).collect();
        shuffle(&mut plan, &mut self.rng);
        // S15 §2.1 Anti-repeat guarantee: if N > 1, avoid playing last track immediately
        if self.len > 1 && last_track.is_some() && Some(plan[0]) == last_track {
            let swap_idx = 1 + self.rng.below(self.len - 1);
            plan.swap(0, swap_idx);
        }
        self.shuffle_plan = plan;
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rustamp_core::playlist::{EntryId, MediaLocation, Playlist};

    use super::*;

    fn playlist(n: usize) -> Playlist {
        let mut p = Playlist::new();
        for i in 0..n {
            p.push(MediaLocation::File(PathBuf::from(format!("track{i}.mp3"))));
        }
        p
    }

    fn ids(p: &Playlist) -> Vec<EntryId> {
        p.iter().map(|e| e.id).collect()
    }

    #[test]
    fn randomize_is_a_permutation_and_keeps_ids() {
        let mut p = playlist(20);
        let before = ids(&p);
        randomize(&mut p, 1234);
        let after = ids(&p);
        assert_eq!(after.len(), before.len());
        let mut sorted_before = before.clone();
        let mut sorted_after = after.clone();
        sorted_before.sort();
        sorted_after.sort();
        assert_eq!(sorted_before, sorted_after, "same ids, no loss");
        assert_ne!(
            before, after,
            "20 entries are very unlikely to stay in order"
        );
    }

    #[test]
    fn randomize_is_deterministic_for_a_seed() {
        let mut a = playlist(30);
        let mut b = playlist(30);
        randomize(&mut a, 7);
        randomize(&mut b, 7);
        assert_eq!(ids(&a), ids(&b));
        randomize(&mut b, 8);
        assert_ne!(ids(&a), ids(&b));
    }

    #[test]
    fn randomize_keeps_selection_and_current() {
        let mut p = playlist(5);
        let all = ids(&p);
        p.select_only(&[all[1], all[3]]);
        p.set_current(Some(all[2]));
        randomize(&mut p, 99);
        assert_eq!(p.selected_in_order().len(), 2);
        assert_eq!(p.current(), Some(all[2]));
    }

    #[test]
    fn empty_and_single_entry_playlists_are_safe() {
        let mut empty = Playlist::new();
        randomize(&mut empty, 5);
        assert!(empty.is_empty());

        let mut one = playlist(1);
        let before = ids(&one);
        randomize(&mut one, 5);
        assert_eq!(ids(&one), before);
    }

    #[test]
    fn shuffle_matches_a_hand_rolled_fisher_yates() {
        let mut items: Vec<u32> = (0..16).collect();
        let mut rng = Rng::new(2024);
        shuffle(&mut items, &mut rng);
        let mut sorted = items.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..16).collect::<Vec<_>>());

        // The same seed must reproduce the same permutation.
        let mut again: Vec<u32> = (0..16).collect();
        shuffle(&mut again, &mut Rng::new(2024));
        assert_eq!(items, again);
    }

    // --- S15 PlaybackOrder tests ---

    #[test]
    fn playback_order_sequential_no_repeat() {
        let mut order = PlaybackOrder::new(3, false, false, 42);
        assert_eq!(order.next_index(), Some(0));
        assert_eq!(order.next_index(), Some(1));
        assert_eq!(order.next_index(), Some(2));
        assert_eq!(order.next_index(), None, "ends after last track");
    }

    #[test]
    fn playback_order_sequential_repeat() {
        let mut order = PlaybackOrder::new(3, false, true, 42);
        assert_eq!(order.next_index(), Some(0));
        assert_eq!(order.next_index(), Some(1));
        assert_eq!(order.next_index(), Some(2));
        assert_eq!(order.next_index(), Some(0), "wraps to 0");
        assert_eq!(order.next_index(), Some(1));
    }

    #[test]
    fn playback_order_shuffle_no_repeat_is_a_permutation() {
        let n = 10;
        let mut order = PlaybackOrder::new(n, true, false, 12345);
        let mut visited = Vec::new();
        while let Some(idx) = order.next_index() {
            visited.push(idx);
        }
        assert_eq!(visited.len(), n, "visited exactly N tracks");
        let mut sorted = visited.clone();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            (0..n).collect::<Vec<_>>(),
            "every track played once"
        );
        assert_ne!(visited, (0..n).collect::<Vec<_>>(), "was shuffled");
        assert_eq!(order.next_index(), None, "ends after full permutation");
    }

    #[test]
    fn playback_order_shuffle_repeat_anti_repeat_holds() {
        let n = 5;
        let mut order = PlaybackOrder::new(n, true, true, 9999);
        let mut cycle1 = Vec::new();
        for _ in 0..n {
            cycle1.push(order.next_index().unwrap());
        }
        let last_c1 = *cycle1.last().unwrap();
        let first_c2 = order.next_index().unwrap();
        assert_ne!(
            last_c1, first_c2,
            "anti-repeat prevents immediate duplicate across cycle boundary"
        );
    }

    #[test]
    fn playback_order_anti_repeat_holds_for_many_seeds() {
        for seed in 0u64..64 {
            let n = 6;
            let mut order = PlaybackOrder::new(n, true, true, seed.wrapping_add(1));
            let mut cycle1 = Vec::new();
            for _ in 0..n {
                cycle1.push(order.next_index().unwrap());
            }
            let last_c1 = *cycle1.last().unwrap();
            let first_c2 = order.next_index().unwrap();
            assert_ne!(
                last_c1, first_c2,
                "seed {seed}: anti-repeat across boundary"
            );
        }
    }

    #[test]
    fn playback_order_shuffle_deterministic_for_seed() {
        let n = 8;
        let mut a = PlaybackOrder::new(n, true, false, 777);
        let mut b = PlaybackOrder::new(n, true, false, 777);
        let mut va = Vec::new();
        let mut vb = Vec::new();
        while let Some(x) = a.next_index() {
            va.push(x);
        }
        while let Some(x) = b.next_index() {
            vb.push(x);
        }
        assert_eq!(va, vb);
        // Different seed differs (overwhelmingly likely).
        let mut c = PlaybackOrder::new(n, true, false, 778);
        let mut vc = Vec::new();
        while let Some(x) = c.next_index() {
            vc.push(x);
        }
        assert_ne!(va, vc);
    }

    #[test]
    fn playback_order_manual_prev_sequential() {
        let mut order = PlaybackOrder::new(3, false, false, 42);
        order.set_current_index(Some(2));
        assert_eq!(order.prev_index(), Some(1));
        assert_eq!(order.prev_index(), Some(0));
        assert_eq!(
            order.prev_index(),
            Some(0),
            "clamps at start when repeat OFF"
        );

        let mut order_rep = PlaybackOrder::new(3, false, true, 42);
        order_rep.set_current_index(Some(0));
        assert_eq!(
            order_rep.prev_index(),
            Some(2),
            "wraps to end when repeat ON"
        );
    }

    #[test]
    fn playback_order_manual_prev_shuffle() {
        let mut order = PlaybackOrder::new(5, true, false, 42);
        let t0 = order.next_index().unwrap();
        let t1 = order.next_index().unwrap();
        let _t2 = order.next_index().unwrap();

        assert_eq!(order.prev_index(), Some(t1));
        assert_eq!(order.prev_index(), Some(t0));
        assert_eq!(order.prev_index(), Some(t0), "clamps at start of shuffle");
    }

    #[test]
    fn playback_order_prev_restores_previous_cycle() {
        let n = 4;
        let mut order = PlaybackOrder::new(n, true, true, 2024);
        let cycle1: Vec<usize> = (0..n).map(|_| order.next_index().unwrap()).collect();
        // cross boundary
        let first_c2 = order.next_index().unwrap();
        assert_ne!(first_c2, cycle1[n - 1]);
        // prev at first track of cycle 2 restores last track of cycle 1
        let restored = order.prev_index().unwrap();
        assert_eq!(restored, cycle1[n - 1]);
        assert_eq!(order.shuffle_plan(), &cycle1[..]);
    }

    #[test]
    fn playback_order_toggle_mid_run() {
        let mut order = PlaybackOrder::new(6, false, false, 555);
        assert_eq!(order.next_index(), Some(0));
        assert_eq!(order.next_index(), Some(1));
        // Toggle shuffle ON mid-run while track 1 is playing
        order.set_shuffle(true);
        assert!(order.shuffle());
        assert_eq!(order.current_index(), Some(1), "current track remains 1");
        // Remaining 5 tracks must all be played, starting with remainder of 0..6 (excluding 1)
        let mut remaining = Vec::new();
        while let Some(idx) = order.next_index() {
            remaining.push(idx);
        }
        assert_eq!(remaining.len(), 5);
        assert!(
            !remaining.contains(&1),
            "track 1 not repeated in same cycle"
        );
        let mut all = remaining.clone();
        all.push(1);
        all.sort_unstable();
        assert_eq!(all, (0..6).collect::<Vec<_>>());

        // Toggle shuffle OFF mid-run
        order.set_current_index(Some(3));
        order.set_shuffle(false);
        assert_eq!(order.next_index(), Some(4), "sequential next from 3 is 4");
    }

    #[test]
    fn playback_order_toggle_repeat_mid_run() {
        let mut order = PlaybackOrder::new(3, false, false, 1);
        assert_eq!(order.next_index(), Some(0));
        assert_eq!(order.next_index(), Some(1));
        assert_eq!(order.next_index(), Some(2));
        assert_eq!(order.next_index(), None, "stops at end when repeat OFF");
        // enable repeat: boundary behaviour changes
        order.set_repeat(true);
        order.set_current_index(Some(2));
        assert_eq!(order.next_index(), Some(0), "wraps when repeat ON");
    }

    #[test]
    fn playback_order_skip_by() {
        let mut order = PlaybackOrder::new(5, false, true, 1);
        order.set_current_index(Some(0));
        assert_eq!(order.skip_by(2), Some(2));
        assert_eq!(order.skip_by(-1), Some(1));
        assert_eq!(order.skip_by(0), Some(1));
    }

    #[test]
    fn playback_order_skip_by_shuffle() {
        let mut order = PlaybackOrder::new(5, true, false, 42);
        let t0 = order.next_index().unwrap();
        let t1 = order.next_index().unwrap();
        let _t2 = order.next_index().unwrap();
        // back one
        assert_eq!(order.skip_by(-1), Some(t1));
        assert_eq!(order.skip_by(-1), Some(t0));
    }

    #[test]
    fn playback_order_single_entry_and_empty() {
        let mut empty = PlaybackOrder::new(0, true, true, 42);
        assert_eq!(empty.next_index(), None);
        assert_eq!(empty.prev_index(), None);
        assert!(empty.is_empty());

        let mut single_no_rep = PlaybackOrder::new(1, true, false, 42);
        assert_eq!(single_no_rep.next_index(), Some(0));
        assert_eq!(
            single_no_rep.next_index(),
            None,
            "single track plays once with repeat OFF"
        );

        let mut single_rep = PlaybackOrder::new(1, true, true, 42);
        assert_eq!(single_rep.next_index(), Some(0));
        assert_eq!(
            single_rep.next_index(),
            Some(0),
            "single track loops with repeat ON"
        );

        // Prev on single entry does not advance
        let mut single_prev = PlaybackOrder::new(1, false, false, 42);
        single_prev.set_current_index(Some(0));
        assert_eq!(single_prev.prev_index(), Some(0));
    }

    #[test]
    fn playback_order_repeat_of_one_loops() {
        for shuffle in [false, true] {
            for _ in 0..3 {
                let mut order = PlaybackOrder::new(1, shuffle, true, 9);
                assert_eq!(order.next_index(), Some(0));
                assert_eq!(order.next_index(), Some(0));
                assert_eq!(order.next_index(), Some(0));
            }
        }
    }

    #[test]
    fn playback_order_invalidation_on_length_change() {
        let mut order = PlaybackOrder::new(4, true, false, 123);
        let plan_before = order.shuffle_plan().to_vec();
        // Remove (shrink) regenerates
        order.set_len(3);
        assert_eq!(order.len(), 3);
        assert_eq!(order.shuffle_plan().len(), 3);
        // Plan changed (not necessarily equal)
        assert!(order.shuffle_plan().iter().all(|&x| x < 3));
        // Grow
        order.set_len(5);
        assert_eq!(order.len(), 5);
        assert_eq!(order.shuffle_plan().len(), 5);
        let _ = plan_before;
    }

    #[test]
    fn playback_order_invalidation_on_sort() {
        let mut order = PlaybackOrder::new(5, true, false, 999);
        order.set_current_index(Some(2));
        let before_plan_len = order.shuffle_plan().len();
        order.invalidate();
        assert_eq!(order.shuffle_plan().len(), before_plan_len);
        assert!(order.shuffle_plan().contains(&2));
        // current remains anchored at front of plan
        assert_eq!(order.shuffle_plan()[0], 2);
        assert_eq!(order.current_index(), Some(2));
    }

    #[test]
    fn playback_order_empty_queue_safe() {
        let mut order = PlaybackOrder::new(0, false, false, 1);
        assert_eq!(order.next_index(), None);
        assert_eq!(order.prev_index(), None);
        assert_eq!(order.skip_by(3), None);
        order.set_len(0);
        order.invalidate();
        assert!(order.is_empty());
        order.set_shuffle(true);
        assert!(order.shuffle_plan().is_empty());
    }

    #[test]
    fn playback_order_modes_toggle_flags() {
        let mut order = PlaybackOrder::new(3, false, false, 1);
        assert!(!order.shuffle());
        assert!(!order.repeat());
        order.toggle_shuffle();
        assert!(order.shuffle());
        order.toggle_repeat();
        assert!(order.repeat());
        order.set_shuffle(false);
        assert!(!order.shuffle());
        order.set_repeat(false);
        assert!(!order.repeat());
    }
}
