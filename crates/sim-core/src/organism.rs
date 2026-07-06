//! Organism storage — Structure-of-Arrays with a stable-id free-list slotmap.
//!
//! Each organism carries a fixed-topology recurrent brain: its weight vector and its live
//! hidden activations live in flat side arrays indexed by slot (`weights` has stride
//! [`N_W`], `hidden` has stride [`N_HID`]). The hidden activations are causally-required
//! state — snapshotted and hashed. Iteration is index-ordered; ties break by stable `id`.

use crate::brain::{Weights, N_HID, N_W};
use crate::math::Scalar;

/// The Phase-1 genome: body/metabolism traits + colour. The brain's weights are stored
/// separately (see [`Organisms::weights`]).
#[derive(Clone, Copy, Debug)]
pub struct Genome {
    /// Body size (mass): bigger can prey on smaller, but costs more upkeep and is slower.
    pub size: Scalar,
    pub metabolism: Scalar,
    /// Per-organism reproduction-threshold multiplier (selection acts on a distribution).
    pub repro: Scalar,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Genome {
    /// A neutral baseline genome (used by the spawn brush).
    pub fn base() -> Self {
        Genome {
            size: 1.0,
            metabolism: 1.0,
            repro: 1.0,
            r: 120,
            g: 200,
            b: 120,
        }
    }
}

#[derive(Clone, Default, Debug)]
pub struct Organisms {
    pub alive: Vec<bool>,
    pub id: Vec<u32>,
    pub px: Vec<Scalar>,
    pub py: Vec<Scalar>,
    pub vx: Vec<Scalar>,
    pub vy: Vec<Scalar>,
    pub energy: Vec<i64>,
    pub age: Vec<u32>,
    pub parent: Vec<u32>,
    pub birth_tick: Vec<u64>,
    pub g_size: Vec<Scalar>,
    pub g_metab: Vec<Scalar>,
    pub g_repro: Vec<Scalar>,
    pub cr: Vec<u8>,
    pub cg: Vec<u8>,
    pub cb: Vec<u8>,
    /// Running "how carnivorous" indicator in `0..=1` (rises on predation, decays). Display
    /// + a small behavioural signal; part of hashed state.
    pub carnivory: Vec<Scalar>,
    /// Brain weights, flat with stride `N_W`.
    pub weights: Vec<Scalar>,
    /// Brain hidden activations (recurrent state), flat with stride `N_HID`.
    pub hidden: Vec<Scalar>,

    pub free: Vec<u32>,
    pub next_id: u32,
    pub count: u32,
}

pub struct NewOrganism {
    pub px: Scalar,
    pub py: Scalar,
    pub vx: Scalar,
    pub vy: Scalar,
    pub energy: i64,
    pub parent: u32,
    pub birth_tick: u64,
    pub genome: Genome,
    pub weights: Weights,
}

impl Organisms {
    pub fn with_capacity(cap: usize) -> Self {
        Organisms {
            weights: Vec::with_capacity(cap * N_W),
            hidden: Vec::with_capacity(cap * N_HID),
            ..Default::default()
        }
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.alive.len()
    }

    /// Insert an organism, assigning a fresh stable id. Returns `(slot, id)`.
    pub fn insert(&mut self, s: NewOrganism) -> (usize, u32) {
        let id = self.next_id;
        self.next_id += 1;
        let slot = if let Some(free) = self.free.pop() {
            let i = free as usize;
            self.alive[i] = true;
            self.id[i] = id;
            self.px[i] = s.px;
            self.py[i] = s.py;
            self.vx[i] = s.vx;
            self.vy[i] = s.vy;
            self.energy[i] = s.energy;
            self.age[i] = 0;
            self.parent[i] = s.parent;
            self.birth_tick[i] = s.birth_tick;
            self.g_size[i] = s.genome.size;
            self.g_metab[i] = s.genome.metabolism;
            self.g_repro[i] = s.genome.repro;
            self.cr[i] = s.genome.r;
            self.cg[i] = s.genome.g;
            self.cb[i] = s.genome.b;
            self.carnivory[i] = 0.0;
            self.weights[i * N_W..(i + 1) * N_W].copy_from_slice(&s.weights);
            for h in &mut self.hidden[i * N_HID..(i + 1) * N_HID] {
                *h = 0.0;
            }
            i
        } else {
            self.alive.push(true);
            self.id.push(id);
            self.px.push(s.px);
            self.py.push(s.py);
            self.vx.push(s.vx);
            self.vy.push(s.vy);
            self.energy.push(s.energy);
            self.age.push(0);
            self.parent.push(s.parent);
            self.birth_tick.push(s.birth_tick);
            self.g_size.push(s.genome.size);
            self.g_metab.push(s.genome.metabolism);
            self.g_repro.push(s.genome.repro);
            self.cr.push(s.genome.r);
            self.cg.push(s.genome.g);
            self.cb.push(s.genome.b);
            self.carnivory.push(0.0);
            self.weights.extend_from_slice(&s.weights);
            self.hidden.extend([0.0f32; N_HID]);
            self.alive.len() - 1
        };
        self.count += 1;
        (slot, id)
    }

    pub fn kill(&mut self, slot: usize) {
        if self.alive[slot] {
            self.alive[slot] = false;
            self.free.push(slot as u32);
            self.count -= 1;
        }
    }

    #[inline]
    pub fn genome_at(&self, i: usize) -> Genome {
        Genome {
            size: self.g_size[i],
            metabolism: self.g_metab[i],
            repro: self.g_repro[i],
            r: self.cr[i],
            g: self.cg[i],
            b: self.cb[i],
        }
    }

    #[inline]
    pub fn weights_at(&self, i: usize) -> Weights {
        let mut w = [0.0; N_W];
        w.copy_from_slice(&self.weights[i * N_W..(i + 1) * N_W]);
        w
    }
}
