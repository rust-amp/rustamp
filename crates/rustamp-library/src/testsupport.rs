// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The helpers the crate's tests share.
//!
//! The property tests of the crate draw their cases from a seeded
//! generator, and the fixtures of the crate are WAV files. Both live here,
//! so every test module of the crate agrees on what a seed means and on
//! what a WAV fixture looks like.

use std::fs;
use std::path::Path;

use crate::Library;

/// A deterministic pseudo random generator (xorshift64*).
///
/// The tests need data that is random enough to hit the corners and
/// reproducible enough to replay a failure: the seed is the whole case.
/// The generator is a few lines long and needs no dependency, and it gives
/// the same sequence on every machine.
pub(crate) struct Rng(u64);

impl Rng {
    /// A generator for `seed`. The state is never zero: xorshift needs a
    /// non-zero state to leave.
    pub(crate) fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    /// The next number of the sequence.
    pub(crate) fn next_u64(&mut self) -> u64 {
        let mut state = self.0;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        self.0 = state;
        state.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A number in `0..bound`.
    pub(crate) fn below(&mut self, bound: usize) -> usize {
        assert!(bound > 0, "the bound must not be empty");
        (self.next_u64() % bound as u64) as usize
    }

    /// `true` in half of the cases.
    pub(crate) fn flip(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    /// One of `items`.
    pub(crate) fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// Writes a minimal WAV file (a header without audio) to `path`.
///
/// The scan only records what it finds, so a fixture does not have to carry
/// tags: a valid header is enough for the tag reader to accept the file.
/// The file is written in one go and closed when the call returns, so the
/// modification time the scan sees is the one of this write.
pub(crate) fn make_wav(path: &Path) {
    let mut buf = Vec::with_capacity(44);
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&36u32.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM
    buf.extend_from_slice(&1u16.to_le_bytes()); // channels
    buf.extend_from_slice(&44100u32.to_le_bytes()); // sample rate
    buf.extend_from_slice(&88200u32.to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&0u32.to_le_bytes());
    fs::write(path, buf).expect("the WAV fixture is written");
}

/// The last-scan time of the root `id`, as the database holds it.
pub(crate) fn last_scanned_at(lib: &Library, id: i64) -> Option<i64> {
    lib.roots()
        .expect("the roots are readable")
        .into_iter()
        .find(|root| root.id == id)
        .expect("the root is listed")
        .last_scanned_at
}

/// Puts a known time into the root's row.
///
/// A test that wants to tell "untouched" from "written again in the same
/// second" needs a time that no scan can produce: the scan writes the
/// second it runs in, so a time far away from now is unmistakable.
pub(crate) fn put_last_scanned_at(lib: &Library, id: i64, at: i64) {
    lib.conn
        .execute(
            "UPDATE roots SET last_scanned_at = ?1 WHERE id = ?2",
            rusqlite::params![at, id],
        )
        .expect("the time is written");
}
