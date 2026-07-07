//! Organism storage — Structure-of-Arrays with a stable-id free-list slotmap.
//!
//! Fixed-width scalar columns plus a per-organism [`Brain`] (variable-topology, so it lives
//! in its own `Vec<Brain>` column rather than a flat stride). Iteration is index-ordered;
//! ties break by stable `id`. Slots are reused from a LIFO free-list on death.

use crate::brain::Brain;
use crate::math::Scalar;

/// The genome's body/metabolism traits + colour. The brain is stored separately.
#[derive(Clone, Copy, Debug)]
pub struct Genome {
    pub size: Scalar,
    pub metabolism: Scalar,
    pub repro: Scalar,
    /// Preferred elevation in `0..=1` (0 = deep water, 1 = high land). With the local terrain
    /// this decides where the organism thrives vs. suffers — the seed of water/land niches.
    pub habitat: Scalar,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Genome {
    pub fn base() -> Self {
        Genome {
            size: 1.0,
            metabolism: 1.0,
            repro: 1.0,
            habitat: 0.5,
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
    /// Preferred elevation (habitat) per organism; see [`Genome::habitat`].
    pub g_habitat: Vec<Scalar>,
    pub cr: Vec<u8>,
    pub cg: Vec<u8>,
    pub cb: Vec<u8>,
    pub carnivory: Vec<Scalar>,
    /// Per-organism growing brain.
    pub brains: Vec<Brain>,

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
    pub brain: Brain,
}

impl Organisms {
    pub fn with_capacity(cap: usize) -> Self {
        Organisms {
            brains: Vec::with_capacity(cap),
            ..Default::default()
        }
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.alive.len()
    }

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
            self.g_habitat[i] = s.genome.habitat;
            self.cr[i] = s.genome.r;
            self.cg[i] = s.genome.g;
            self.cb[i] = s.genome.b;
            self.carnivory[i] = 0.0;
            self.brains[i] = s.brain;
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
            self.g_habitat.push(s.genome.habitat);
            self.cr.push(s.genome.r);
            self.cg.push(s.genome.g);
            self.cb.push(s.genome.b);
            self.carnivory.push(0.0);
            self.brains.push(s.brain);
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
            habitat: self.g_habitat[i],
            r: self.cr[i],
            g: self.cg[i],
            b: self.cb[i],
        }
    }
}
