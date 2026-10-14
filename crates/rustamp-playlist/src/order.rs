//! One-shot reordering helpers.
//!
//! [`Playlist::sort`](rustamp_core::playlist::Playlist::sort) and
//! [`Playlist::reverse`](rustamp_core::playlist::Playlist::reverse) already live in
//! `rustamp-core` and are **not** duplicated here. What is missing there is
//! `randomize`: a single random permutation of the current order.
//!
//! The shuffle *sequence* algorithm (the behaviour of the shuffle toggle, spec
//! S15) is explicitly out of scope. This module only produces one permutation,
//! on demand, from a seeded in-crate generator.

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
}
