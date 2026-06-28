//! A tiny, dependency-free pseudo-random generator.
//!
//! The simulation is deliberately no longer bit-for-bit identical from one run
//! to the next: the host (server or wasm wrapper) seeds it from entropy, so each
//! colony plays out differently and the world feels alive and interactive rather
//! than replaying a fixed script. What we *keep* is reproducibility **given a
//! seed** — the same seed always replays the same trajectory — so a particular
//! run can still be pinned for debugging and the test suite can assert against a
//! fixed seed. That contract only holds because every draw flows through this
//! one explicitly-seeded generator; nothing else in `colony-core` is allowed to
//! be a source of randomness.
//!
//! The algorithm is SplitMix64 (Steele, Lea & Flood): a single 64-bit state
//! advanced by a fixed odd increment and avalanched with two xorshift-multiply
//! rounds. It is fast, statistically fine for a toy simulation, and trivially
//! reproducible with no external crate — matching this crate's "pure, no
//! surprise dependencies" rule (the same reasoning behind the bespoke
//! [`crate::world`] cell hasher).

/// A reproducible-given-its-seed SplitMix64 generator.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Create a generator seeded explicitly. The same seed replays the same
    /// stream, which is the whole point — entropy enters only where the host
    /// chooses the seed, never inside the pure engine.
    pub fn from_seed(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Advance the state and return the next raw 64-bit value.
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Next `f64` uniformly distributed in `[0, 1)`. Built from the top 53 bits
    /// (the f64 mantissa width), the standard way to draw a uniform double.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform `f64` in `[lo, hi)`. `lo <= hi` is assumed (callers pass literals).
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_replays_the_same_stream() {
        let mut a = Rng::from_seed(12345);
        let mut b = Rng::from_seed(12345);
        for _ in 0..1_000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::from_seed(1);
        let mut b = Rng::from_seed(2);
        // Overwhelmingly likely to differ on the very first draw; assert it so a
        // regression that ignores the seed can't slip through.
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn next_f64_stays_in_unit_interval() {
        let mut rng = Rng::from_seed(99);
        for _ in 0..10_000 {
            let x = rng.next_f64();
            assert!((0.0..1.0).contains(&x), "draw out of [0, 1): {x}");
        }
    }
}
