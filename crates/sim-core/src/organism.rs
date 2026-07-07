//! Organism storage — Structure-of-Arrays with a stable-id free-list slotmap.
//!
//! Fixed-width scalar columns plus a per-organism [`Brain`] (variable-topology, so it lives
//! in its own `Vec<Brain>` column rather than a flat stride). Iteration is index-ordered;
//! ties break by stable `id`. Slots are reused from a LIFO free-list on death.

use crate::brain::Brain;
use crate::genome::{develop, Genome};
use crate::math::Scalar;

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
    /// Digestion specialization per organism; see [`Genome::diet`].
    pub g_diet: Vec<Scalar>,
    pub cr: Vec<u8>,
    pub cg: Vec<u8>,
    pub cb: Vec<u8>,
    pub carnivory: Vec<Scalar>,
    /// Per-organism growing brain.
    pub brains: Vec<Brain>,
    /// Per-organism open genome (the raw `Vec<Gene>`); the `g_*`/`c*` columns above are its
    /// developed phenotype, recomputed at insert. This is the authoritative heritable state.
    pub genomes: Vec<Genome>,

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
            genomes: Vec::with_capacity(cap),
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
        // Compile the genome into its phenotype once; the `g_*`/`c*` columns are develop()'s
        // output (what `step_organism` reads), while the raw genome is stored authoritatively.
        let ph = develop(&s.genome);
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
            self.g_size[i] = ph.size;
            self.g_metab[i] = ph.metabolism;
            self.g_repro[i] = ph.repro;
            self.g_habitat[i] = ph.habitat;
            self.g_diet[i] = ph.diet;
            self.cr[i] = ph.r;
            self.cg[i] = ph.g;
            self.cb[i] = ph.b;
            self.carnivory[i] = 0.0;
            self.brains[i] = s.brain;
            self.genomes[i] = s.genome;
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
            self.g_size.push(ph.size);
            self.g_metab.push(ph.metabolism);
            self.g_repro.push(ph.repro);
            self.g_habitat.push(ph.habitat);
            self.g_diet.push(ph.diet);
            self.cr.push(ph.r);
            self.cg.push(ph.g);
            self.cb.push(ph.b);
            self.carnivory.push(0.0);
            self.brains.push(s.brain);
            self.genomes.push(s.genome);
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

    /// The parent's raw genome, cloned to seed a child (which then mutates + develops it).
    #[inline]
    pub fn genome_at(&self, i: usize) -> Genome {
        self.genomes[i].clone()
    }
}
