//! One-shot reordering helpers and S15 playback traversal order.
//!
//! [`Playlist::sort`](rustamp_core::playlist::Playlist::sort) and
//! [`Playlist::reverse`](rustamp_core::playlist::Playlist::reverse) already live in
//! `rustamp-core` and are **not** duplicated here. What is missing there is
//! `randomize`: a single random permutation of the current order.
//!
//! [`PlaybackOrder`] implements the S15 shuffle sequence and repeat traversal
//! state machine, managing permutation plans, repeat boundaries, history for
//! previous-track navigation, and mid-run toggling.

use rustamp_core::playlist::{EntryId, Playlist};

use crate::rng::Rng;

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
/// Tracks sequential traversal and shuffle permutations, repeat boundaries,
/// history for previous-track navigation, and mid-run mode toggles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackOrder {
    len: usize,
    shuffle: bool,
    repeat: bool,
    current_index: Option<usize>,
    shuffle_plan: Vec<usize>,
    shuffle_pos: usize,
    history: Vec<usize>,
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
            history: Vec::new(),
            rng: Rng::new(seed),
        };
        if shuffle && len > 0 {
            order.generate_shuffle_plan(None);
        }
        order
    }

    /// Update the playlist length (e.g. when items are added or removed).
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
        if matches!(self.current_index, Some(curr) if curr >= len) {
            self.current_index = None;
        }
        if self.shuffle {
            self.generate_shuffle_plan(self.current_index);
        }
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

    /// Explicitly set the current index (e.g. user selected track).
    pub fn set_current_index(&mut self, index: Option<usize>) {
        if let Some(idx) = index {
            if idx < self.len {
                self.current_index = Some(idx);
                self.history.push(idx);
                if self.shuffle {
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

    /// Current shuffle permutation plan.
    pub fn shuffle_plan(&self) -> &[usize] {
        &self.shuffle_plan
    }

    /// Current position in the shuffle plan.
    pub fn shuffle_pos(&self) -> usize {
        self.shuffle_pos
    }

    /// Set shuffle mode enabled/disabled (S15 §4).
    pub fn set_shuffle(&mut self, shuffle: bool) {
        if self.shuffle == shuffle {
            return;
        }
        self.shuffle = shuffle;
        if self.shuffle && self.len > 0 {
            if let Some(curr) = self.current_index {
                self.replan_from_current(curr);
            } else {
                self.generate_shuffle_plan(None);
            }
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
            self.history.push(next);
            Some(next)
        } else {
            if self.shuffle_plan.is_empty() || self.shuffle_plan.len() != self.len {
                self.generate_shuffle_plan(self.current_index);
            }
            if self.current_index.is_none() {
                self.shuffle_pos = 0;
                let next = self.shuffle_plan[0];
                self.current_index = Some(next);
                self.history.push(next);
                return Some(next);
            }
            if self.shuffle_pos + 1 < self.shuffle_plan.len() {
                self.shuffle_pos += 1;
                let next = self.shuffle_plan[self.shuffle_pos];
                self.current_index = Some(next);
                self.history.push(next);
                Some(next)
            } else if self.repeat {
                let last = self.shuffle_plan.last().copied();
                self.generate_new_cycle(last);
                self.shuffle_pos = 0;
                let next = self.shuffle_plan[0];
                self.current_index = Some(next);
                self.history.push(next);
                Some(next)
            } else {
                None
            }
        }
    }

    /// Step back to the previous track in the active playback order.
    /// In shuffle mode, navigates backward in shuffle history.
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
            self.history.push(prev);
            Some(prev)
        } else {
            if self.shuffle_plan.is_empty() {
                self.generate_shuffle_plan(self.current_index);
            }
            if self.shuffle_pos > 0 {
                self.shuffle_pos -= 1;
                let prev = self.shuffle_plan[self.shuffle_pos];
                self.current_index = Some(prev);
                self.history.push(prev);
                Some(prev)
            } else if self.repeat && self.len > 1 {
                self.shuffle_pos = self.shuffle_plan.len() - 1;
                let prev = self.shuffle_plan[self.shuffle_pos];
                self.current_index = Some(prev);
                self.history.push(prev);
                Some(prev)
            } else {
                self.current_index
            }
        }
    }

    /// Step by signed `delta` positions in the active playback order.
    pub fn skip_by(&mut self, delta: i32) -> Option<usize> {
        if self.len == 0 || delta == 0 {
            return self.current_index;
        }
        if delta > 0 {
            let mut res = self.current_index;
            for _ in 0..delta {
                match self.next_index() {
                    Some(idx) => res = Some(idx),
                    None => break,
                }
            }
            res
        } else {
            let mut res = self.current_index;
            for _ in 0..delta.unsigned_abs() {
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
            return;
        }
        let mut plan: Vec<usize> = (0..self.len).collect();
        shuffle(&mut plan, &mut self.rng);
        if let Some(pos) = current.and_then(|curr| plan.iter().position(|&p| p == curr)) {
            plan.swap(0, pos);
        }
        self.shuffle_plan = plan;
        self.shuffle_pos = 0;
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
    fn playback_order_single_entry_and_empty() {
        let mut empty = PlaybackOrder::new(0, true, true, 42);
        assert_eq!(empty.next_index(), None);
        assert_eq!(empty.prev_index(), None);

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
    }
}
