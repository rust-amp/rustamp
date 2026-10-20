//! A tiny deterministic generator for the property tests.
//!
//! A property run prints the seed of a failing sequence, so the sequence has
//! to be reproducible from that number alone. A general-purpose crate would
//! do the same, but the generator is thirty lines and keeps the crate's
//! dependency list (and its license closure) untouched — the property tests
//! must not add a dependency to a crate that is published as a permissive
//! framework.

/// A xorshift64\* generator: deterministic, seeded and dependency-free.
pub struct Rng(u64);

impl Rng {
    /// A generator for `seed`.
    ///
    /// Zero is not a valid xorshift state (it would stay zero forever), so it
    /// is replaced by a fixed non-zero one.
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    /// The next 64 bits.
    fn next_u64(&mut self) -> u64 {
        let mut state = self.0;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        self.0 = state;
        state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number in `0..bound`.
    ///
    /// The modulo bias is irrelevant here: the bounds are small and the
    /// sequences are not cryptographic.
    pub fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }
}
