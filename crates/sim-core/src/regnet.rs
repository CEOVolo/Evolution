//! The **developmental regulatory network** (GRN) — the genome's body-building program.
//!
//! A [`RegNet`] is a growing network with the same NEAT-style structural mutation as the
//! behavioural [`crate::brain::Brain`] (add-connection, identity-preserving add-node), but with
//! development-specific inputs/outputs and — crucially — a **stateless feed-forward** evaluation:
//! every body cell runs the *same* network on its *own* local context (local morphogen levels,
//! its position signals, its current cell-state), and the network says what that cell should do
//! (divide / emit morphogen / differentiate / die). Same genome + different local context ⇒
//! differentiated cells: this is clonal development, and nothing about a cell's role is scripted.
//!
//! Evaluation is a single pass over nodes in index order, so an edge from a not-yet-computed
//! node contributes 0 — a deterministic feed-forward truncation (no recurrence, no stored state;
//! a cell's memory lives in its explicit cell-state vector, fed back in as input each step). Only
//! `+ - *` and the deterministic [`tanh_det`], so it is bit-identical native↔wasm.

use crate::brain::Conn;
use crate::math::{tanh_det, Scalar};
use crate::rng::Pcg32;

/// Morphogen channels diffused on the body lattice during development.
pub const N_MORPH: usize = 2;
/// Per-cell developmental state vector width (the cell's persistent "memory" across dev steps).
pub const S_STATE: usize = 3;

// --- inputs (local context a cell reads) ---
pub const N_DEV_IN: usize = 1 + 1 + N_MORPH + 1 + 1 + S_STATE; // = 9
pub const DIN_BIAS: usize = 0;
/// Developmental clock: `step / dev_steps`, mapped to [-1, 1].
pub const DIN_CLOCK: usize = 1;
/// First local-morphogen slot; channel `k` at `DIN_MORPH0 + k`.
pub const DIN_MORPH0: usize = 2;
/// Occupied-neighbour fraction (0..1): a free geometry-only radial cue (surface vs interior).
pub const DIN_DEGREE: usize = DIN_MORPH0 + N_MORPH; // 4
/// Projection of this cell's lattice offset onto the body polarity axis (a longitudinal coord).
pub const DIN_AXIS: usize = DIN_DEGREE + 1; // 5
/// First cell-state slot (recurrent memory), fed back in each step.
pub const DIN_STATE0: usize = DIN_AXIS + 1; // 6

// --- outputs (mechanisms, never a role enum) ---
pub const N_DEV_OUT: usize = 1 + 4 + N_MORPH + S_STATE + 2 + 1; // = 13
pub const DOUT_DIVIDE: usize = 0;
/// Division-direction preference over the 4 lattice neighbours (+u, -u, +v, -v); argmax picks one.
pub const DOUT_DIR0: usize = 1;
/// Morphogen emission into this cell's own site, channel `k` at `DOUT_EMIT0 + k`.
pub const DOUT_EMIT0: usize = DOUT_DIR0 + 4; // 5
/// Cell-state nudge, slot `s` at `DOUT_DSTATE0 + s`.
pub const DOUT_DSTATE0: usize = DOUT_EMIT0 + N_MORPH; // 7
/// Differentiation drive into the feeding role accumulator.
pub const DOUT_ROLE_FEED: usize = DOUT_DSTATE0 + S_STATE; // 10
/// Differentiation drive into the structural/defense role accumulator.
pub const DOUT_ROLE_STRUCT: usize = DOUT_ROLE_FEED + 1; // 11
/// Programmed cell death (lets bodies carve non-convex shapes).
pub const DOUT_APOPTOSE: usize = DOUT_ROLE_STRUCT + 1; // 12

#[derive(Clone, Debug)]
pub struct RegNet {
    pub n_hidden: u16,
    /// Bias per node (`len == total_nodes`; input biases unused).
    pub bias: Vec<Scalar>,
    pub conns: Vec<Conn>,
}

impl RegNet {
    #[inline]
    pub fn total_nodes(&self) -> usize {
        N_DEV_IN + N_DEV_OUT + self.n_hidden as usize
    }

    /// A minimal founder GRN. **Founders cannot divide**: the divide output is left unwired with a
    /// strongly negative bias, so a founder develops into a single cell. Multicellularity is not
    /// given — it must EVOLVE (a mutation that wires an input into the divide output, or drives its
    /// bias positive, and only spreads if being bigger pays). The other outputs wire from the
    /// positional cues so differentiation has something to read once bodies do appear.
    pub fn random_minimal(rng: &mut Pcg32) -> RegNet {
        let total = N_DEV_IN + N_DEV_OUT;
        let mut r = RegNet {
            n_hidden: 0,
            bias: vec![0.0; total],
            conns: Vec::new(),
        };
        for o in 0..N_DEV_OUT {
            if o == DOUT_DIVIDE {
                // Founders are single-celled: with no incoming edge, the divide output is
                // tanh(-0.6) < the divide threshold, so a founder never divides. But the bias is
                // only mildly negative, so ONE mutation (a positive-weighted edge, or a few bias
                // jitters) can cross the threshold — division is *reachable*, not *present*.
                // Whether such a mutant then spreads is decided by whether being bigger pays.
                r.bias[N_DEV_IN + o] = -0.6;
                continue;
            }
            let n_links = 1 + rng.below(2) as usize; // 1..=2
            for _ in 0..n_links {
                let from = rng.below(N_DEV_IN as u32);
                r.conns.push(Conn {
                    from,
                    to: (N_DEV_IN + o) as u32,
                    w: rng.next_f32_signed() * 1.2,
                    enabled: true,
                });
            }
            r.bias[N_DEV_IN + o] = rng.next_f32_signed() * 0.5;
        }
        r
    }

    /// Evaluate the network on one cell's local context. Stateless: a fresh value array each call,
    /// nodes computed in index order (edges from later nodes contribute 0 — feed-forward
    /// truncation), so there is no recurrence and no allocation-free scratch to carry between cells
    /// beyond the caller-owned `scratch`.
    pub fn forward(
        &self,
        inputs: &[Scalar; N_DEV_IN],
        scratch: &mut Vec<Scalar>,
    ) -> [Scalar; N_DEV_OUT] {
        let total = self.total_nodes();
        scratch.clear();
        scratch.resize(total, 0.0);
        scratch[..N_DEV_IN].copy_from_slice(inputs);
        // Gather in index order: node j sums its enabled incoming edges, then activates. Edges
        // whose source index >= j read 0 (not yet computed) — a deterministic feed-forward pass.
        for j in N_DEV_IN..total {
            let mut s = self.bias[j];
            for c in &self.conns {
                if c.enabled && c.to as usize == j {
                    s += scratch[c.from as usize] * c.w;
                }
            }
            scratch[j] = tanh_det(s);
        }
        let mut out = [0.0f32; N_DEV_OUT];
        out.copy_from_slice(&scratch[N_DEV_IN..N_DEV_IN + N_DEV_OUT]);
        out
    }

    pub fn enabled_conns(&self) -> usize {
        self.conns.iter().filter(|c| c.enabled).count()
    }

    pub fn complexity(&self) -> usize {
        self.n_hidden as usize + self.enabled_conns()
    }

    /// Mutate weights, biases, and (rarely) topology — mirrors [`crate::brain::Brain::mutate`].
    pub fn mutate(
        &mut self,
        rng: &mut Pcg32,
        rate: f32,
        w_delta: f32,
        add_conn_p: f32,
        add_node_p: f32,
    ) {
        for c in self.conns.iter_mut() {
            if rng.chance(rate) {
                c.w = (c.w + rng.next_f32_signed() * w_delta).clamp(-5.0, 5.0);
            }
        }
        for node in N_DEV_IN..self.total_nodes() {
            if rng.chance(rate * 0.5) {
                self.bias[node] =
                    (self.bias[node] + rng.next_f32_signed() * w_delta).clamp(-5.0, 5.0);
            }
        }
        // add-connection: any node -> a non-input node (skip exact duplicates).
        if rng.chance(add_conn_p) {
            let total = self.total_nodes() as u32;
            let from = rng.below(total);
            let to = N_DEV_IN as u32 + rng.below((N_DEV_OUT + self.n_hidden as usize) as u32);
            if !self.conns.iter().any(|c| c.from == from && c.to == to) {
                self.conns.push(Conn {
                    from,
                    to,
                    w: rng.next_f32_signed() * 1.0,
                    enabled: true,
                });
            }
        }
        // add-node: split an enabled connection with identity-preserving weights.
        if rng.chance(add_node_p) {
            let enabled: Vec<usize> = (0..self.conns.len())
                .filter(|&i| self.conns[i].enabled)
                .collect();
            if !enabled.is_empty() {
                let pick = enabled[rng.below(enabled.len() as u32) as usize];
                let (from, to, w) = {
                    let c = &self.conns[pick];
                    (c.from, c.to, c.w)
                };
                self.conns[pick].enabled = false;
                let new_idx = (N_DEV_IN + N_DEV_OUT + self.n_hidden as usize) as u32;
                self.n_hidden += 1;
                self.bias.push(0.0);
                self.conns.push(Conn {
                    from,
                    to: new_idx,
                    w: 1.0,
                    enabled: true,
                });
                self.conns.push(Conn {
                    from: new_idx,
                    to,
                    w,
                    enabled: true,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_is_deterministic_and_grows() {
        let mut rng = Pcg32::new(3, 1);
        let mut r = RegNet::random_minimal(&mut rng);
        let inp = [1.0, 0.2, 0.5, -0.3, 0.4, 0.1, -0.2, 0.0, 0.6];
        let mut s1 = Vec::new();
        let mut s2 = Vec::new();
        assert_eq!(r.forward(&inp, &mut s1), r.forward(&inp, &mut s2));
        let before = r.complexity();
        let mut m = Pcg32::new(9, 1);
        for _ in 0..40 {
            r.mutate(&mut m, 0.0, 0.0, 1.0, 1.0);
        }
        assert!(r.complexity() > before, "regnet did not grow");
        assert_eq!(r.forward(&inp, &mut s1), r.forward(&inp, &mut s2));
    }
}
