//! Organism storage — Structure-of-Arrays with a stable-id free-list slotmap.
//!
//! Phase 0 keeps fixed-width per-organism columns only (no variable-topology brain yet —
//! that arrives in Phase 1 as a separate indexed arena). Slots are reused from a LIFO
//! free-list on death; iteration is index-ordered and ties break by the stable `id`, so the
//! whole thing is deterministic. Compaction is intentionally deferred (a perf concern, not a
//! correctness one) — dead slots are simply skipped via `alive`.

use crate::math::Scalar;

/// The Phase-0 genome: a handful of evolvable traits. (NEAT brains replace/extend this in
/// Phase 1.) `repro` is a per-organism reproduction-threshold multiplier, so selection acts
/// on a distribution rather than one global cliff.
#[derive(Clone, Copy, Debug)]
pub struct Genome {
    pub speed: Scalar,
    pub metabolism: Scalar,
    pub repro: Scalar,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Genome {
    /// A neutral baseline genome.
    pub fn base() -> Self {
        Genome {
            speed: 0.5,
            metabolism: 1.0,
            repro: 1.0,
            r: 120,
            g: 200,
            b: 120,
        }
    }
}

/// Struct-of-Arrays store. Every column has the same length (`capacity`); `alive[i]` says
/// whether slot `i` currently holds a live organism.
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
    pub g_speed: Vec<Scalar>,
    pub g_metab: Vec<Scalar>,
    pub g_repro: Vec<Scalar>,
    pub cr: Vec<u8>,
    pub cg: Vec<u8>,
    pub cb: Vec<u8>,

    /// Free slot indices (LIFO — deterministic reuse order).
    pub free: Vec<u32>,
    /// Next stable entity id to hand out.
    pub next_id: u32,
    /// Number of live organisms.
    pub count: u32,
}

/// All the data needed to place a new organism.
pub struct NewOrganism {
    pub px: Scalar,
    pub py: Scalar,
    pub vx: Scalar,
    pub vy: Scalar,
    pub energy: i64,
    pub parent: u32,
    pub birth_tick: u64,
    pub genome: Genome,
}

impl Organisms {
    pub fn with_capacity(cap: usize) -> Self {
        let mut o = Organisms::default();
        o.reserve(cap);
        o
    }

    fn reserve(&mut self, extra: usize) {
        self.alive.reserve(extra);
        self.id.reserve(extra);
        self.px.reserve(extra);
        self.py.reserve(extra);
        self.vx.reserve(extra);
        self.vy.reserve(extra);
        self.energy.reserve(extra);
        self.age.reserve(extra);
        self.parent.reserve(extra);
        self.birth_tick.reserve(extra);
        self.g_speed.reserve(extra);
        self.g_metab.reserve(extra);
        self.g_repro.reserve(extra);
        self.cr.reserve(extra);
        self.cg.reserve(extra);
        self.cb.reserve(extra);
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.alive.len()
    }

    /// Insert an organism, assigning it a fresh stable id. Returns `(slot, id)`.
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
            self.g_speed[i] = s.genome.speed;
            self.g_metab[i] = s.genome.metabolism;
            self.g_repro[i] = s.genome.repro;
            self.cr[i] = s.genome.r;
            self.cg[i] = s.genome.g;
            self.cb[i] = s.genome.b;
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
            self.g_speed.push(s.genome.speed);
            self.g_metab.push(s.genome.metabolism);
            self.g_repro.push(s.genome.repro);
            self.cr.push(s.genome.r);
            self.cg.push(s.genome.g);
            self.cb.push(s.genome.b);
            self.alive.len() - 1
        };
        self.count += 1;
        (slot, id)
    }

    /// Mark a slot dead and return it to the free-list.
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
            speed: self.g_speed[i],
            metabolism: self.g_metab[i],
            repro: self.g_repro[i],
            r: self.cr[i],
            g: self.cg[i],
            b: self.cb[i],
        }
    }
}
