//! A small deterministic pseudo-random generator.
//!
//! The game deliberately does not use `macroquad::rand`. Level generation must be
//! reproducible from a seed alone — the same seed has to produce byte-identical
//! terrain on every machine and every run, because seeds are printed on the
//! briefing screen and written into saved `.pen` files. A generator we own and
//! can unit-test is the only way to promise that.
//!
//! The algorithm is xorshift64* (Marsaglia / Vigna): 64 bits of state, three
//! shifts and a multiply. It is not cryptographic and does not pretend to be.

#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    /// Seeds the generator. Any `u64` is acceptable, including zero — the seed is
    /// run through a SplitMix-style mixer first and forced odd, so the state can
    /// never land on the single fixed point (0) that would jam xorshift.
    pub fn new(seed: u64) -> Self {
        let mut z = seed
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(0x1234_5678_9ABC_DEF1);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Rng {
            state: (z ^ (z >> 31)) | 1,
        }
    }

    /// Derives an independent stream from this one. Used so that, say, enemy
    /// placement can be re-rolled without disturbing the terrain sequence.
    pub fn fork(&mut self, salt: u64) -> Rng {
        Rng::new(self.next_u64() ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform in `[0, 1)`. Built from the top 24 bits so every value is exactly
    /// representable as an `f32` and the distribution has no gaps.
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in `[-1, 1)`.
    pub fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    /// Uniform in `[lo, hi)`. Returns `lo` if the range is empty or inverted.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        if hi <= lo {
            return lo;
        }
        lo + (hi - lo) * self.unit()
    }

    /// Uniform integer in `[lo, hi)`. Returns `lo` if the range is empty.
    pub fn int(&mut self, lo: i32, hi: i32) -> i32 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next_u32() % (hi - lo) as u32) as i32
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_gives_same_sequence() {
        let mut a = Rng::new(20250725);
        let mut b = Rng::new(20250725);
        for _ in 0..2000 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn zero_seed_does_not_jam() {
        let mut r = Rng::new(0);
        let first = r.next_u64();
        // A jammed xorshift repeats its state forever; ten pulls is plenty to see it.
        assert!((0..10).any(|_| r.next_u64() != first));
    }

    #[test]
    fn unit_stays_in_range() {
        let mut r = Rng::new(7);
        for _ in 0..100_000 {
            let v = r.unit();
            assert!((0.0..1.0).contains(&v), "unit() produced {v}");
        }
    }

    #[test]
    fn int_respects_bounds_and_empty_ranges() {
        let mut r = Rng::new(99);
        for _ in 0..10_000 {
            let v = r.int(-5, 5);
            assert!((-5..5).contains(&v));
        }
        assert_eq!(r.int(3, 3), 3);
        assert_eq!(r.int(9, 2), 9);
    }

    #[test]
    fn forked_streams_are_independent() {
        let mut base = Rng::new(42);
        let mut a = base.fork(1);
        let mut b = base.fork(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }
}
