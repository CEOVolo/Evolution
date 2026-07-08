//! Organism storage — Structure-of-Arrays with a stable-id free-list slotmap.
//!
//! The unit of life is a whole **organism**: one authoritative genome, one behavioural brain, one
//! developmental network ([`RegNet`]), a single shared energy pool, a world position + heading,
//! and a **body** — a cluster of differentiated cells grown from the genome at birth by
//! [`develop_body`]. The body and the developed global traits (`g_*`, `mass_milli`) are *derived*
//! state: they are rebuilt from the heritable {genome, regnet} at insert and on snapshot load, so
//! they are never serialized. Iteration is index-ordered; slots are reused from a LIFO free-list
//! on death and always get a fresh `id`.

use crate::brain::Brain;
use crate::develop::{develop_body, Body};
use crate::genome::{develop, Genome};
use crate::math::Scalar;
use crate::regnet::RegNet;

#[derive(Clone, Default, Debug)]
pub struct Organisms {
    pub alive: Vec<bool>,
    pub id: Vec<u32>,
    pub px: Vec<Scalar>,
    pub py: Vec<Scalar>,
    pub vx: Vec<Scalar>,
    pub vy: Vec<Scalar>,
    /// Unit heading vector (orientation): the body footprint is mapped through it each tick.
    pub hx: Vec<Scalar>,
    pub hy: Vec<Scalar>,
    /// The one shared energy pool for the whole body.
    pub energy: Vec<i64>,
    pub age: Vec<u32>,
    pub parent: Vec<u32>,
    pub birth_tick: Vec<u64>,
    /// Developed whole-organism traits (from [`develop`]): metabolism and reproduction multipliers.
    pub g_metab: Vec<Scalar>,
    pub g_repro: Vec<Scalar>,
    /// Digestion specialization (Stage 2): 0 = food-A specialist, 1 = food-B; feeder cells eat
    /// each food scaled by a convex trade-off, so the population splits into dietary niches.
    pub g_diet: Vec<Scalar>,
    /// Preferred elevation (Stage 2): a body pays energy where the local terrain mismatches this,
    /// so deep water is a barrier and water vs land become distinct habitats.
    pub g_habitat: Vec<Scalar>,
    /// Total body mass in fixed-point milli-units (derived: Σ cell sizes). Drives accel + gape.
    pub mass_milli: Vec<i64>,
    /// Per-organism behavioural brain.
    pub brains: Vec<Brain>,
    /// Per-organism developmental network (grows the body).
    pub regnets: Vec<RegNet>,
    /// Per-organism authoritative genome (global traits via `develop`).
    pub genomes: Vec<Genome>,
    /// Per-organism developed body (derived; rebuilt at insert / on load, never serialized).
    pub bodies: Vec<Body>,

    pub free: Vec<u32>,
    pub next_id: u32,
    pub count: u32,
}

pub struct NewOrganism {
    pub px: Scalar,
    pub py: Scalar,
    pub vx: Scalar,
    pub vy: Scalar,
    pub hx: Scalar,
    pub hy: Scalar,
    pub energy: i64,
    pub parent: u32,
    pub birth_tick: u64,
    pub genome: Genome,
    pub brain: Brain,
    pub regnet: RegNet,
}

impl Organisms {
    pub fn with_capacity(cap: usize) -> Self {
        Organisms {
            brains: Vec::with_capacity(cap),
            regnets: Vec::with_capacity(cap),
            genomes: Vec::with_capacity(cap),
            bodies: Vec::with_capacity(cap),
            ..Default::default()
        }
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.alive.len()
    }

    /// Insert one organism, developing its body from {genome, regnet}. `seed` keys the (per-body,
    /// deterministic) development, so the same organism id always grows the same body — which is
    /// what lets snapshot load re-develop bodies instead of serializing them.
    pub fn insert(&mut self, seed: u64, s: NewOrganism) -> (usize, u32) {
        let id = self.next_id;
        self.next_id += 1;
        let ph = develop(&s.genome);
        let body = develop_body(&s.regnet, ph.size, (ph.r, ph.g, ph.b), seed, id);
        let mass = body.mass_milli();
        let slot = if let Some(free) = self.free.pop() {
            let i = free as usize;
            self.alive[i] = true;
            self.id[i] = id;
            self.px[i] = s.px;
            self.py[i] = s.py;
            self.vx[i] = s.vx;
            self.vy[i] = s.vy;
            self.hx[i] = s.hx;
            self.hy[i] = s.hy;
            self.energy[i] = s.energy;
            self.age[i] = 0;
            self.parent[i] = s.parent;
            self.birth_tick[i] = s.birth_tick;
            self.g_metab[i] = ph.metabolism;
            self.g_repro[i] = ph.repro;
            self.g_diet[i] = ph.diet;
            self.g_habitat[i] = ph.habitat;
            self.mass_milli[i] = mass;
            self.brains[i] = s.brain;
            self.regnets[i] = s.regnet;
            self.genomes[i] = s.genome;
            self.bodies[i] = body;
            i
        } else {
            self.alive.push(true);
            self.id.push(id);
            self.px.push(s.px);
            self.py.push(s.py);
            self.vx.push(s.vx);
            self.vy.push(s.vy);
            self.hx.push(s.hx);
            self.hy.push(s.hy);
            self.energy.push(s.energy);
            self.age.push(0);
            self.parent.push(s.parent);
            self.birth_tick.push(s.birth_tick);
            self.g_metab.push(ph.metabolism);
            self.g_repro.push(ph.repro);
            self.g_diet.push(ph.diet);
            self.g_habitat.push(ph.habitat);
            self.mass_milli.push(mass);
            self.brains.push(s.brain);
            self.regnets.push(s.regnet);
            self.genomes.push(s.genome);
            self.bodies.push(body);
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

    /// The parent's raw genome, cloned to seed a child.
    #[inline]
    pub fn genome_at(&self, i: usize) -> Genome {
        self.genomes[i].clone()
    }
}
