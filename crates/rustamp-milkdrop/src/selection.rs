// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Which preset comes next. The order, the history and dropping a preset that
//! will not load are the engine's own [`lactea::Playlist`]; this is the small
//! surface the player needs, and it reports what actually got loaded.

use lactea::Playlist;

/// A step through the presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The first preset of the list, when a session starts.
    First,
    /// The one after the current one, wrapping.
    Next,
    /// The one shown before the current one.
    Previous,
    /// Any one of them.
    Random,
}

/// What a step ended in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// This preset (an item of the list) is now loaded.
    Loaded(String),
    /// Every preset that was tried refused to load; the last reason. What was
    /// on the screen stays.
    Failed(String),
    /// The list has run empty: no preset could be loaded.
    Empty,
}

/// The presets of a session and the position in them.
pub struct Selector {
    playlist: Playlist,
}

impl Selector {
    /// A selector over `items`. `seed` decides the random steps.
    pub fn new(items: &[String], seed: u64) -> Self {
        let mut playlist = Playlist::new(seed);
        playlist.add_presets(items, true);
        // Every preset gets its try: a pack may hold many that will not load.
        playlist.set_retry_count(playlist.len().max(1));
        Self { playlist }
    }

    /// How many presets are left: a preset that failed to load has left.
    pub fn len(&self) -> usize {
        self.playlist.len() as usize
    }

    /// Whether every preset has failed to load.
    pub fn is_empty(&self) -> bool {
        self.playlist.is_empty()
    }

    /// Takes `step`, calling `load` with the item and whether the step is a hard
    /// cut. A preset `load` refuses is dropped from the list and the step goes
    /// on to the next candidate.
    pub fn step(
        &mut self,
        step: Step,
        hard_cut: bool,
        mut load: impl FnMut(&str, bool) -> Result<(), String>,
    ) -> Outcome {
        let mut loaded = None;
        let mut failure = None;
        let attempt =
            |request: lactea::LoadRequest<'_>| match load(request.filename, request.hard_cut) {
                Ok(()) => {
                    loaded = Some(request.filename.to_owned());
                    Ok(())
                }
                Err(reason) => {
                    failure = Some(reason.clone());
                    Err(reason)
                }
            };
        match step {
            Step::First => {
                self.playlist.set_position(0, hard_cut, attempt);
            }
            Step::Next => {
                self.playlist.play_next(hard_cut, attempt);
            }
            Step::Previous => {
                self.playlist.play_last(hard_cut, attempt);
            }
            Step::Random => {
                self.playlist.set_shuffle(true);
                self.playlist.play_next(hard_cut, attempt);
                self.playlist.set_shuffle(false);
            }
        }
        match (loaded, failure) {
            (Some(item), _) => Outcome::Loaded(item),
            (None, Some(reason)) if !self.playlist.is_empty() => Outcome::Failed(reason),
            _ => Outcome::Empty,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn accept(_: &str, _: bool) -> Result<(), String> {
        Ok(())
    }

    fn loaded(outcome: Outcome) -> String {
        match outcome {
            Outcome::Loaded(item) => item,
            other => panic!("expected a loaded preset, got {other:?}"),
        }
    }

    #[test]
    fn next_walks_the_list_and_wraps() {
        let mut selector = Selector::new(&items(&["a", "b", "c"]), 1);
        assert_eq!(loaded(selector.step(Step::First, true, accept)), "a");
        assert_eq!(loaded(selector.step(Step::Next, true, accept)), "b");
        assert_eq!(loaded(selector.step(Step::Next, true, accept)), "c");
        assert_eq!(loaded(selector.step(Step::Next, true, accept)), "a");
    }

    #[test]
    fn previous_goes_back_through_what_was_shown_not_the_list() {
        let mut selector = Selector::new(&items(&["a", "b", "c", "d"]), 7);
        selector.step(Step::First, true, accept);
        let first_random = loaded(selector.step(Step::Random, true, accept));
        let second_random = loaded(selector.step(Step::Random, true, accept));
        if first_random != second_random {
            assert_eq!(
                loaded(selector.step(Step::Previous, true, accept)),
                first_random
            );
        }
        // Next returns to the list's own order: random leaves no shuffle behind.
        assert!(!selector.playlist.shuffle());
    }

    #[test]
    fn random_stays_in_the_list_and_the_seed_decides() {
        let list = items(&["a", "b", "c", "d", "e", "f", "g", "h"]);
        let run = |seed| {
            let mut selector = Selector::new(&list, seed);
            selector.step(Step::First, true, accept);
            (0..32)
                .map(|_| loaded(selector.step(Step::Random, true, accept)))
                .collect::<Vec<_>>()
        };
        let one = run(42);
        assert_eq!(one, run(42), "same seed, same walk");
        assert!(one.iter().all(|item| list.contains(item)));
        assert!(
            one.iter().collect::<std::collections::HashSet<_>>().len() > 1,
            "a random walk that never moves is not random: {one:?}"
        );
    }

    #[test]
    fn a_preset_that_will_not_load_is_skipped_and_leaves_the_list() {
        let mut selector = Selector::new(&items(&["a", "broken", "c"]), 1);
        selector.step(Step::First, true, accept);
        let outcome = selector.step(Step::Next, true, |item, _| {
            if item == "broken" {
                Err("no".to_owned())
            } else {
                Ok(())
            }
        });
        assert_ne!(loaded(outcome), "broken");
        assert_eq!(selector.len(), 2);
        assert!(
            !selector
                .playlist
                .items()
                .iter()
                .any(|item| item == "broken")
        );
    }

    #[test]
    fn when_nothing_loads_the_list_runs_empty_and_says_so() {
        let mut selector = Selector::new(&items(&["x", "y", "z"]), 1);
        let mut tried = Vec::new();
        let outcome = selector.step(Step::First, true, |item, _| {
            tried.push(item.to_owned());
            Err(format!("{item} is broken"))
        });
        assert!(
            matches!(&outcome, Outcome::Empty | Outcome::Failed(_)),
            "{outcome:?}"
        );
        assert_eq!(tried.len(), 3, "every preset got its try: {tried:?}");
    }

    #[test]
    fn the_hard_cut_flag_reaches_the_loader() {
        let mut selector = Selector::new(&items(&["a", "b"]), 1);
        let mut seen = None;
        selector.step(Step::First, false, |_, hard| {
            seen = Some(hard);
            Ok(())
        });
        assert_eq!(seen, Some(false));
    }
}
