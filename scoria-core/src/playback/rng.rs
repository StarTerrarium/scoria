//! A small seedable random number generator, used only for shuffling.
//!
//! This is `rand` reimplemented badly on purpose, and the reasoning is worth
//! recording. Shuffling a playlist is not a cryptographic problem, and it is
//! the only place in the application that needs randomness at all -- so `rand`
//! would be a dependency with one caller. More usefully, a generator we own can
//! be seeded, which makes shuffle deterministic under test; that is what lets
//! `PlaybackSource`'s shuffle behaviour be asserted rather than eyeballed.
//!
//! The algorithm is xoshiro256++, chosen because it is short, well studied and
//! has more than enough quality for deciding what song plays next.

/// Seedable PRNG. Not cryptographically secure and not intended to be.
#[derive(Debug, Clone)]
pub struct Rng {
    state: [u64; 4],
}

impl Rng {
    /// Seeds from a single value. Any seed is usable, including zero.
    #[must_use]
    pub const fn seeded(seed: u64) -> Self {
        // SplitMix64 expands one value into four well-distributed words, which
        // is the standard way to avoid a poor initial state -- seeding
        // xoshiro's state directly from a small number leaves it correlated,
        // and a zero seed leaves it emitting nothing but zeroes.
        let (z, a) = Self::split_mix(seed);
        let (z, b) = Self::split_mix(z);
        let (z, c) = Self::split_mix(z);
        let (_, d) = Self::split_mix(z);
        Self {
            state: [a, b, c, d],
        }
    }

    /// One SplitMix64 step: the advanced state, and the value it yields.
    const fn split_mix(z: u64) -> (u64, u64) {
        let z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let x = z;
        let x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        let x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z, x ^ (x >> 31))
    }

    /// Seeds from the clock, for real use.
    ///
    /// Falls back to a fixed seed if the clock is before the epoch, rather than
    /// failing: a slightly less random shuffle is a better outcome than an
    /// error the user cannot act on.
    #[must_use]
    pub fn from_clock() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0x2545_F491_4F6C_DD1D, |d| d.as_nanos() as u64);
        Self::seeded(nanos)
    }

    /// Next value in the sequence.
    pub fn next_u64(&mut self) -> u64 {
        let [s0, s1, s2, s3] = self.state;

        let result = s0.wrapping_add(s3).rotate_left(23).wrapping_add(s0);
        let t = s1 << 17;

        let mut n2 = s2 ^ s0;
        let n3 = s3 ^ s1;
        let n1 = s1 ^ n2;
        let n0 = s0 ^ n3;
        n2 ^= t;
        let n3 = n3.rotate_left(45);

        self.state = [n0, n1, n2, n3];
        result
    }

    /// Uniform value in `0..bound`, or `None` when `bound` is zero.
    ///
    /// Uses Lemire's multiply-shift with rejection, so the result is unbiased
    /// rather than merely close -- a modulo would quietly favour low indices,
    /// and "shuffle prefers the start of the album" is exactly the kind of bug
    /// that never gets reported, only felt.
    pub fn below(&mut self, bound: u64) -> Option<u64> {
        if bound == 0 {
            return None;
        }
        let threshold = bound.wrapping_neg() % bound;
        loop {
            let value = self.next_u64();
            let product = u128::from(value).wrapping_mul(u128::from(bound));
            if (product as u64) >= threshold {
                return Some((product >> 64) as u64);
            }
        }
    }

    /// Fisher-Yates, in place.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        let len = items.len();
        for i in (1..len).rev() {
            // `i` is in bounds by construction, and `below` only returns None
            // for a zero bound, which cannot happen here.
            if let Some(j) = self.below(i as u64 + 1) {
                items.swap(i, j as usize);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;
    use std::collections::HashSet;

    #[test]
    fn the_same_seed_gives_the_same_sequence() {
        let mut a = Rng::seeded(42);
        let mut b = Rng::seeded(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::seeded(1);
        let mut b = Rng::seeded(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn a_zero_seed_still_produces_varied_output() {
        // A naive seeding would leave the state all zeroes and emit only zeroes.
        let mut rng = Rng::seeded(0);
        let values: HashSet<u64> = (0..50).map(|_| rng.next_u64()).collect();
        assert!(
            values.len() > 45,
            "got only {} distinct values",
            values.len()
        );
    }

    #[test]
    fn below_stays_in_range_and_rejects_a_zero_bound() {
        let mut rng = Rng::seeded(7);
        assert_eq!(rng.below(0), None);
        for _ in 0..1000 {
            assert!(rng.below(10).unwrap() < 10);
        }
        assert_eq!(rng.below(1), Some(0));
    }

    #[test]
    fn below_is_reasonably_uniform() {
        // Not a statistical proof, just enough to catch a modulo bias that
        // would favour the start of an album.
        let mut rng = Rng::seeded(99);
        let mut counts = [0u32; 8];
        for _ in 0..80_000 {
            counts[rng.below(8).unwrap() as usize] += 1;
        }
        for (bucket, count) in counts.iter().enumerate() {
            assert!(
                (8_000..12_000).contains(count),
                "bucket {bucket} got {count}, expected roughly 10000"
            );
        }
    }

    #[test]
    fn shuffle_is_a_permutation() {
        let mut rng = Rng::seeded(5);
        let mut items: Vec<u32> = (0..100).collect();
        rng.shuffle(&mut items);

        assert_eq!(items.len(), 100);
        let unique: HashSet<u32> = items.iter().copied().collect();
        assert_eq!(unique.len(), 100, "shuffle lost or duplicated an element");
        assert_ne!(
            items,
            (0..100).collect::<Vec<_>>(),
            "shuffle changed nothing"
        );
    }

    #[test]
    fn shuffle_handles_degenerate_lengths() {
        let mut rng = Rng::seeded(1);
        let mut empty: Vec<u32> = vec![];
        rng.shuffle(&mut empty);
        assert!(empty.is_empty());

        let mut one = vec![7];
        rng.shuffle(&mut one);
        assert_eq!(one, vec![7]);
    }
}
