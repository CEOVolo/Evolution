//! Deterministic scalar math.
//!
//! `Scalar` is `f32` today, behind an alias so a future partial move to fixed-point is
//! localized. NOTE (per the determinism design): the alias localizes the swap *point* but a
//! real fixed-point migration is not a drop-in — keep literals and division going through
//! the helpers here so the surface stays contained.
//!
//! Everything in this module uses only the cross-target-deterministic float ops
//! (`+ - * /`, comparisons). No transcendentals from `std`/`libm`, no FMA, no
//! `f32::min`/`max` (whose NaN semantics differ between native and wasm).

/// The simulation scalar. Positions and velocities use this; energy/field use integers.
pub type Scalar = f32;

/// Collapse `-0.0` to `+0.0` and return the raw bits, for canonical hashing.
///
/// Non-finite values must never reach state; we assert that in debug builds so a NaN/inf
/// leak is caught immediately rather than silently poisoning a golden hash.
#[inline]
pub fn canonical_bits(x: Scalar) -> u32 {
    debug_assert!(x.is_finite(), "non-finite Scalar reached hashing: {x}");
    // `x == 0.0` is true for both +0.0 and -0.0; adding +0.0 normalizes the sign bit.
    let x = if x == 0.0 { 0.0 } else { x };
    x.to_bits()
}

/// Clamp `x` to `[-limit, limit]` using only comparisons (no transcendentals, no FMA).
#[inline]
pub fn clamp_abs(x: Scalar, limit: Scalar) -> Scalar {
    if x > limit {
        limit
    } else if x < -limit {
        -limit
    } else {
        x
    }
}

/// Wrap `v` into `[0, size)` for a toroidal world. `%` on `f32` is IEEE `rem`, deterministic.
#[inline]
pub fn wrap(v: Scalar, size: Scalar) -> Scalar {
    let mut r = v % size;
    if r < 0.0 {
        r += size;
    }
    // Guard the boundary: `(-1e-7) % size + size` can round to exactly `size`.
    if r >= size {
        r = 0.0;
    }
    r
}

/// Deterministic `tanh` approximation (rational, uses only `+ - * /`).
///
/// Reserved for the NEAT brain in Phase 1; to be replaced by a Remez minimax polynomial
/// with a stated max-ULP budget and a validation harness against an f64 reference. Kept
/// `pub` so it is part of the crate's tested surface even before the brain lands.
#[inline]
pub fn tanh_det(x: Scalar) -> Scalar {
    // tanh(x) ~= x*(27 + x^2) / (27 + 9*x^2) on |x| <= 3; saturate beyond.
    let c = clamp_abs(x, 3.0);
    let x2 = c * c;
    c * (27.0 + x2) / (27.0 + 9.0 * x2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_zero_signs_match() {
        assert_eq!(canonical_bits(0.0), canonical_bits(-0.0));
    }

    #[test]
    fn wrap_stays_in_range() {
        for &(v, s) in &[
            (0.5, 1.0),
            (-0.5, 1.0),
            (1.5, 1.0),
            (-1e-7, 1.0),
            (2.0, 1.0),
        ] {
            let r = wrap(v, s);
            assert!((0.0..s).contains(&r), "wrap({v},{s}) = {r} out of range");
        }
    }

    #[test]
    fn tanh_monotone_and_bounded() {
        assert!(tanh_det(0.0).abs() < 1e-6);
        assert!(tanh_det(10.0) <= 1.0 && tanh_det(10.0) > 0.9);
        assert!(tanh_det(-10.0) >= -1.0 && tanh_det(-10.0) < -0.9);
        assert!(tanh_det(0.5) < tanh_det(1.0));
    }
}
