//! A small **evolvable recurrent neural network** — the organism's controller.
//!
//! Phase 1 uses a *fixed topology* (evolving weights, not structure yet — topology-growing
//! NEAT comes later). It is recurrent: each hidden node keeps its activation between ticks,
//! giving the organism memory. That recurrent state is causally-required world state, so it
//! is snapshotted and hashed (see `World::state_hash` / `snapshot`).
//!
//! Determinism: the forward pass uses only `+ - *` and the deterministic [`tanh_det`]; it is
//! a single synchronous update that reads the *previous* tick's hidden activations, evaluated
//! in fixed index order. No `std`/`libm` transcendentals, no FMA.

use crate::math::{tanh_det, Scalar};

/// Sensor inputs.
pub const N_IN: usize = 8;
/// Recurrent hidden nodes (the organism's "memory").
pub const N_HID: usize = 6;
/// Actuator outputs.
pub const N_OUT: usize = 4;

/// Total weights: in→hid, hid→hid (recurrent), hid→out, hidden biases, output biases.
pub const N_W: usize = N_IN * N_HID + N_HID * N_HID + N_HID * N_OUT + N_HID + N_OUT;

/// A brain's weight vector.
pub type Weights = [Scalar; N_W];

// Input indices (kept as named constants so sensing/thinking agree).
pub const IN_BIAS: usize = 0;
pub const IN_FOOD: usize = 1;
pub const IN_GRAD_X: usize = 2;
pub const IN_GRAD_Y: usize = 3;
pub const IN_ENERGY: usize = 4;
pub const IN_NN_DX: usize = 5;
pub const IN_NN_DY: usize = 6;
pub const IN_NN_RELSIZE: usize = 7;

// Output indices.
pub const OUT_AX: usize = 0;
pub const OUT_AY: usize = 1;
pub const OUT_BITE: usize = 2;
pub const OUT_REPRO: usize = 3;

// Weight-block offsets within the flat vector.
const OFF_WHH: usize = N_IN * N_HID;
const OFF_WHO: usize = OFF_WHH + N_HID * N_HID;
const OFF_BH: usize = OFF_WHO + N_HID * N_OUT;
const OFF_BO: usize = OFF_BH + N_HID;

/// Zeroed brain (a passive organism — outputs are `tanh(0) = 0`).
pub fn zero_weights() -> Weights {
    [0.0; N_W]
}

/// One synchronous forward pass. Reads `hidden` (previous tick) and overwrites it with the
/// new activations; returns the output vector.
pub fn forward(
    w: &[Scalar],
    hidden: &mut [Scalar; N_HID],
    inputs: &[Scalar; N_IN],
) -> [Scalar; N_OUT] {
    let mut nh = [0.0f32; N_HID];
    for j in 0..N_HID {
        let mut s = w[OFF_BH + j];
        for i in 0..N_IN {
            s += inputs[i] * w[i * N_HID + j];
        }
        for k in 0..N_HID {
            s += hidden[k] * w[OFF_WHH + k * N_HID + j];
        }
        nh[j] = tanh_det(s);
    }
    let mut out = [0.0f32; N_OUT];
    for o in 0..N_OUT {
        let mut s = w[OFF_BO + o];
        for j in 0..N_HID {
            s += nh[j] * w[OFF_WHO + j * N_OUT + o];
        }
        out[o] = tanh_det(s);
    }
    *hidden = nh;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_count_matches() {
        assert_eq!(N_W, 48 + 36 + 24 + 6 + 4);
        assert_eq!(zero_weights().len(), N_W);
    }

    #[test]
    fn zero_brain_is_passive() {
        let w = zero_weights();
        let mut h = [0.0; N_HID];
        let out = forward(&w, &mut h, &[0.0; N_IN]);
        assert_eq!(out, [0.0; N_OUT]);
    }

    #[test]
    fn forward_is_deterministic() {
        let mut w = zero_weights();
        for (i, x) in w.iter_mut().enumerate() {
            *x = ((i as f32 * 0.123).fract() - 0.5) * 2.0;
        }
        let inp = [1.0, 0.4, -0.2, 0.1, 0.7, -0.3, 0.2, 0.5];
        let mut ha = [0.1; N_HID];
        let mut hb = [0.1; N_HID];
        for _ in 0..50 {
            let a = forward(&w, &mut ha, &inp);
            let b = forward(&w, &mut hb, &inp);
            assert_eq!(a, b);
        }
    }
}
