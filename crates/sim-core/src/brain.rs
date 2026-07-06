//! An evolvable, **growing** recurrent neural network — the organism's controller.
//!
//! Unlike the earlier fixed-topology net, brains here start minimal and gain nodes and
//! connections over generations (NEAT-style structural mutation): `add-connection` wires two
//! existing nodes; `add-node` splits a connection with identity-preserving weights (in-edge
//! = 1, out-edge = old weight) so the new structure is behaviourally near-neutral and can be
//! tuned later. Brain size is metabolically taxed, so complexity only sticks when it pays.
//!
//! Node layout by index: `0..N_IN` inputs, `N_IN..N_IN+N_OUT` outputs, the rest hidden
//! (append-only, so indices are stable across mutation and reproduction). The forward pass is
//! a single synchronous update: sums are accumulated from the *previous* tick's activations
//! (recurrent), inputs are injected same-tick, then all non-input nodes update together —
//! fixed order, deterministic, only `+ - *` and the deterministic [`tanh_det`].

use crate::math::{tanh_det, Scalar};
use crate::rng::Pcg32;

pub const N_IN: usize = 10;
pub const N_OUT: usize = 4;
pub const OUT_BASE: usize = N_IN;

// Input semantics.
pub const IN_BIAS: usize = 0;
pub const IN_FOOD: usize = 1;
pub const IN_GRAD_X: usize = 2;
pub const IN_GRAD_Y: usize = 3;
pub const IN_ENERGY: usize = 4;
pub const IN_NN_DX: usize = 5;
pub const IN_NN_DY: usize = 6;
pub const IN_NN_RELSIZE: usize = 7;
/// Day/night–season light level (a global cycle brains can time against).
pub const IN_DAYLIGHT: usize = 8;
/// Local pheromone/signal concentration (what others have emitted here).
pub const IN_SIGNAL: usize = 9;

// Output semantics.
pub const OUT_AX: usize = 0;
pub const OUT_AY: usize = 1;
pub const OUT_REPRO: usize = 2;
/// Emit into the signal field — meaning is assigned by evolution, not us.
pub const OUT_EMIT: usize = 3;

#[derive(Clone, Debug)]
pub struct Conn {
    pub from: u32,
    pub to: u32,
    pub w: Scalar,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub struct Brain {
    pub n_hidden: u16,
    /// Bias per node (`len == total_nodes`; input biases unused).
    pub bias: Vec<Scalar>,
    /// Recurrent activations per node (`len == total_nodes`).
    pub act: Vec<Scalar>,
    pub conns: Vec<Conn>,
}

impl Brain {
    #[inline]
    pub fn total_nodes(&self) -> usize {
        N_IN + N_OUT + self.n_hidden as usize
    }

    /// A minimal random brain: a few random input→output connections, no hidden nodes.
    pub fn random_minimal(rng: &mut Pcg32) -> Brain {
        let total = N_IN + N_OUT;
        let mut b = Brain {
            n_hidden: 0,
            bias: vec![0.0; total],
            act: vec![0.0; total],
            conns: Vec::new(),
        };
        for o in 0..N_OUT {
            let n_links = 1 + rng.below(2) as usize; // 1..=2
            for _ in 0..n_links {
                let from = rng.below(N_IN as u32);
                b.conns.push(Conn {
                    from,
                    to: (OUT_BASE + o) as u32,
                    w: rng.next_f32_signed() * 1.2,
                    enabled: true,
                });
            }
            b.bias[OUT_BASE + o] = rng.next_f32_signed() * 0.5;
        }
        b
    }

    /// Zero the recurrent activations (called at birth, so development is deterministic).
    pub fn reset_state(&mut self) {
        for a in self.act.iter_mut() {
            *a = 0.0;
        }
    }

    /// One synchronous forward pass. `scratch` is a reusable buffer (grows to the brain's node
    /// count) so the hot path does not allocate per organism.
    pub fn forward(
        &mut self,
        inputs: &[Scalar; N_IN],
        scratch: &mut Vec<Scalar>,
    ) -> [Scalar; N_OUT] {
        let total = self.total_nodes();
        self.act[..N_IN].copy_from_slice(inputs);
        scratch.clear();
        scratch.resize(total, 0.0);
        scratch[N_IN..total].copy_from_slice(&self.bias[N_IN..total]);
        for c in &self.conns {
            if c.enabled {
                scratch[c.to as usize] += self.act[c.from as usize] * c.w;
            }
        }
        let mut out = [0.0f32; N_OUT];
        for node in N_IN..total {
            let a = tanh_det(scratch[node]);
            self.act[node] = a;
            if (OUT_BASE..OUT_BASE + N_OUT).contains(&node) {
                out[node - OUT_BASE] = a;
            }
        }
        out
    }

    pub fn enabled_conns(&self) -> usize {
        self.conns.iter().filter(|c| c.enabled).count()
    }

    /// Hidden nodes + enabled connections — used for the metabolic tax and for display.
    pub fn complexity(&self) -> usize {
        self.n_hidden as usize + self.enabled_conns()
    }

    /// Mutate weights, biases, and (rarely) topology.
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
        for node in N_IN..self.total_nodes() {
            if rng.chance(rate * 0.5) {
                self.bias[node] =
                    (self.bias[node] + rng.next_f32_signed() * w_delta).clamp(-5.0, 5.0);
            }
        }
        // add-connection: wire any node -> a non-input node (skip exact duplicates).
        if rng.chance(add_conn_p) {
            let total = self.total_nodes() as u32;
            let from = rng.below(total);
            let to = OUT_BASE as u32 + rng.below((N_OUT + self.n_hidden as usize) as u32);
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
                let new_idx = (N_IN + N_OUT + self.n_hidden as usize) as u32;
                self.n_hidden += 1;
                self.bias.push(0.0);
                self.act.push(0.0);
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
        let mut rng = Pcg32::new(1, 1);
        let mut b = Brain::random_minimal(&mut rng);
        let inp = [1.0, 0.4, -0.2, 0.1, 0.7, -0.3, 0.2, 0.5, 0.6, -0.1];
        let mut s1 = Vec::new();
        let mut s2 = Vec::new();
        let mut ba = b.clone();
        for _ in 0..50 {
            assert_eq!(b.forward(&inp, &mut s1), ba.forward(&inp, &mut s2));
        }
        let before = b.complexity();
        // force a bunch of structural mutations
        let mut m = Pcg32::new(9, 1);
        for _ in 0..40 {
            b.mutate(&mut m, 0.0, 0.0, 1.0, 1.0);
        }
        assert!(b.complexity() > before, "brain did not grow");
        // still evaluates without panicking and is deterministic
        let mut c = b.clone();
        assert_eq!(b.forward(&inp, &mut s1), c.forward(&inp, &mut s2));
    }
}
