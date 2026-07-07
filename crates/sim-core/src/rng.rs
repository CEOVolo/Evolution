//! Deterministic pseudo-random number generation.
//!
//! Hand-rolled so a dependency bump can never change a golden hash. Streams are derived from
//! **stable structured keys** (world seed + subsystem + entity key), never from call order,
//! so adding a draw in one subsystem cannot shift another subsystem's randomness.

/// SplitMix64 — used to mix structured keys into PRNG seeds.
#[inline]
pub fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Subsystem stream ids. Reserving distinct ids keeps subsystems' random streams
/// independent and append-only (never renumber existing ids — that would change every
/// existing seed's world).
pub mod subsystem {
    pub const SPAWN: u64 = 1;
    pub const MUTATION: u64 = 2;
    pub const BLOOM: u64 = 3;
    pub const TERRAIN: u64 = 4;
    pub const ELEVATION: u64 = 5;
}

/// PCG-XSH-RR 64/32. Small, portable, fully deterministic.
#[derive(Clone, Debug)]
pub struct Pcg32 {
    state: u64,
    inc: u64,
}

impl Pcg32 {
    /// Seed a generator from a 64-bit seed and a stream-selecting sequence.
    pub fn new(seed: u64, seq: u64) -> Self {
        let mut r = Pcg32 {
            state: 0,
            inc: (seq << 1) | 1,
        };
        let _ = r.next_u32();
        r.state = r.state.wrapping_add(seed);
        let _ = r.next_u32();
        r
    }

    /// Derive an independent stream from `(world_seed, subsystem, key)`.
    ///
    /// `key` should be a stable identity (e.g. an organism id mixed with the tick), so the
    /// stream does not depend on iteration or call order.
    pub fn from_key(seed: u64, subsystem: u64, key: u64) -> Self {
        let s = splitmix64(seed ^ splitmix64(subsystem.wrapping_add(splitmix64(key))));
        Pcg32::new(s, splitmix64(s | 1))
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Uniform `f32` in `[0, 1)` via the top-24-bits integer trick (one multiply by a
    /// constant — no transcendental, no per-call division).
    #[inline]
    pub fn next_f32_unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0f32 / (1u32 << 24) as f32)
    }

    /// Uniform `f32` in `[-1, 1)`.
    #[inline]
    pub fn next_f32_signed(&mut self) -> f32 {
        self.next_f32_unit() * 2.0 - 1.0
    }

    /// Uniform `u32` in `[0, bound)` via multiply-high (bias-free, deterministic).
    #[inline]
    pub fn below(&mut self, bound: u32) -> u32 {
        if bound == 0 {
            return 0;
        }
        let m = (self.next_u32() as u64) * (bound as u64);
        (m >> 32) as u32
    }

    /// Return `true` with probability `p` (clamped to `[0,1]`).
    #[inline]
    pub fn chance(&mut self, p: f32) -> bool {
        self.next_f32_unit() < p
    }

    /// Raw internal state, for snapshotting.
    pub fn raw(&self) -> (u64, u64) {
        (self.state, self.inc)
    }

    /// Rebuild from raw state (snapshot restore).
    pub fn from_raw(state: u64, inc: u64) -> Self {
        Pcg32 { state, inc }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_sequences() {
        let mut a = Pcg32::new(42, 1);
        let mut b = Pcg32::new(42, 1);
        for _ in 0..1000 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }

    #[test]
    fn from_key_is_stable_and_distinct() {
        let x = Pcg32::from_key(7, subsystem::MUTATION, 123).next_u32();
        let x2 = Pcg32::from_key(7, subsystem::MUTATION, 123).next_u32();
        let y = Pcg32::from_key(7, subsystem::MUTATION, 124).next_u32();
        assert_eq!(x, x2, "same key -> same stream");
        assert_ne!(
            x, y,
            "different key -> different stream (overwhelmingly likely)"
        );
    }

    #[test]
    fn unit_range() {
        let mut r = Pcg32::new(1, 1);
        for _ in 0..10_000 {
            let u = r.next_f32_unit();
            assert!((0.0..1.0).contains(&u));
        }
    }

    #[test]
    fn below_bound() {
        let mut r = Pcg32::new(9, 1);
        for _ in 0..10_000 {
            assert!(r.below(5) < 5);
        }
    }
}
