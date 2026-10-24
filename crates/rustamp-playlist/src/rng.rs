//! A tiny deterministic pseudo-random generator.
//!
//! Written from scratch so `randomize` needs no dependency and so tests can be
//! seeded and therefore reproducible. This is the xorshift64* construction
//! (Marsaglia's xorshift, multiplied by a fixed odd constant), which is small,
//! fast and has a period of 2^64 - 1. It is **not** suitable for anything
//! security related.

/// A seeded xorshift64* generator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng {
    state: u64,
}

/// Seed used for [`Rng::default`] and for a zero seed (the all-zero state is a
/// fixed point of xorshift and must be avoided).
pub const DEFAULT_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

impl Rng {
    /// A generator with the given seed. A seed of `0` is replaced by
    /// [`DEFAULT_SEED`].
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { DEFAULT_SEED } else { seed },
        }
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A uniform value in `0..upper`.
    ///
    /// Uses rejection sampling so the distribution has no modulo bias.
    /// `upper == 0` and `upper == 1` return `0`.
    pub fn below(&mut self, upper: usize) -> usize {
        if upper <= 1 {
            return 0;
        }
        let n = upper as u64;
        let limit = u64::MAX / n * n;
        loop {
            let value = self.next_u64();
            if value < limit {
                return (value % n) as usize;
            }
        }
    }

    /// A uniform value in `0.0..1.0`.
    pub fn next_f64(&mut self) -> f64 {
        // 53 significant bits, the same as `f64::from_bits`-based generators.
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

impl Default for Rng {
    fn default() -> Self {
        Self::new(DEFAULT_SEED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_deterministic_for_a_seed() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..64 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_differ() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn zero_seed_is_avoided() {
        let mut a = Rng::new(0);
        assert_eq!(a.next_u64(), Rng::new(DEFAULT_SEED).next_u64());
        assert_ne!(a.next_u64(), 0);
    }

    #[test]
    fn below_stays_in_range_and_covers_it() {
        let mut rng = Rng::new(7);
        let mut seen = [false; 5];
        for _ in 0..1000 {
            let v = rng.below(5);
            assert!(v < 5);
            seen[v] = true;
        }
        assert!(seen.iter().all(|s| *s));
        assert_eq!(rng.below(0), 0);
        assert_eq!(rng.below(1), 0);
    }

    #[test]
    fn f64_is_in_unit_interval() {
        let mut rng = Rng::new(9);
        for _ in 0..1000 {
            let v = rng.next_f64();
            assert!((0.0..1.0).contains(&v));
        }
    }
}
