//! The world and its tick — the one hot function the whole engine is built around.
//!
//! Phase 1+: each organism has an evolvable **growing** brain ([`crate::brain`]) that maps
//! sensors to actuators; brains gain structure over generations and are metabolically taxed
//! by their complexity. Organisms can **prey** on sufficiently-smaller neighbours. No
//! behaviour is hard-coded to a role — foraging, fleeing, and hunting all emerge.
//!
//! Fixed phase order per tick: 1. commands, 2. regrow fields, 3. rebuild spatial hash,
//! 4. organisms act (index order), 5. apply births.

use crate::brain::{self, Brain, OUT_AX, OUT_AY};
use crate::command::{Command, CommandKind};
use crate::environment::SpatialHash;
use crate::event::{DeathCause, Event, EventBatch};
use crate::math::{canonical_bits, clamp_abs, wrap, Scalar};
use crate::organism::{Genome, NewOrganism, Organisms};
use crate::params::WorldParams;
use crate::rng::{splitmix64, subsystem, Pcg32};

#[derive(Clone, Debug)]
pub struct World {
    pub params: WorldParams,
    pub seed: u64,
    pub tick_count: u64,
    pub field: Vec<i64>,
    pub orgs: Organisms,
    hash: SpatialHash,
}

struct PendingBirth {
    parent_id: u32,
    parent_genome: Genome,
    parent_brain: Brain,
    px: Scalar,
    py: Scalar,
}

impl World {
    pub fn new(seed: u64, params: WorldParams) -> Self {
        let cells = params.cell_count();
        let hash = SpatialHash::new(params.width, params.height, params.sense_radius);
        let mut w = World {
            field: vec![params.field_cap; cells],
            orgs: Organisms::with_capacity(params.initial_population as usize),
            hash,
            params,
            seed,
            tick_count: 0,
        };
        let mut rng = Pcg32::from_key(seed, subsystem::SPAWN, 0);
        for _ in 0..w.params.initial_population {
            let px = rng.next_f32_unit() * w.params.width;
            let py = rng.next_f32_unit() * w.params.height;
            let genome = Genome {
                size: 0.6 + rng.next_f32_unit() * 1.0,
                metabolism: 0.6 + rng.next_f32_unit() * 0.8,
                repro: 0.6 + rng.next_f32_unit() * 0.8,
                r: rng.below(256) as u8,
                g: rng.below(256) as u8,
                b: rng.below(256) as u8,
            };
            let brain = Brain::random_minimal(&mut rng);
            w.orgs.insert(NewOrganism {
                px,
                py,
                vx: 0.0,
                vy: 0.0,
                energy: w.params.initial_energy,
                parent: u32::MAX,
                birth_tick: 0,
                genome,
                brain,
            });
        }
        w
    }

    /// Reconstruct from snapshot parts (rebuilds the transient spatial hash).
    pub fn from_parts(
        params: WorldParams,
        seed: u64,
        tick_count: u64,
        field: Vec<i64>,
        orgs: Organisms,
    ) -> Self {
        let hash = SpatialHash::new(params.width, params.height, params.sense_radius);
        World {
            params,
            seed,
            tick_count,
            field,
            orgs,
            hash,
        }
    }

    #[inline]
    pub fn population(&self) -> u32 {
        self.orgs.count
    }

    #[inline]
    fn cell_of(&self, px: Scalar, py: Scalar) -> (i32, i32) {
        let cw = self.params.width / self.params.grid_w as f32;
        let ch = self.params.height / self.params.grid_h as f32;
        ((px / cw) as i32, (py / ch) as i32)
    }

    #[inline]
    fn fidx(&self, cx: i32, cy: i32) -> usize {
        let gw = self.params.grid_w as i32;
        let gh = self.params.grid_h as i32;
        let x = cx.rem_euclid(gw);
        let y = cy.rem_euclid(gh);
        (y * gw + x) as usize
    }

    #[inline]
    fn deposit_detritus(&mut self, fi: usize) {
        let v = self.field[fi] + self.params.death_deposit;
        let cap = self.params.field_cap;
        self.field[fi] = if v > cap { cap } else { v };
    }

    pub fn tick(&mut self, cmds: &[Command]) -> EventBatch {
        let mut events = EventBatch::new();

        for c in cmds {
            self.apply_command(c, &mut events);
        }

        let cap = self.params.field_cap;
        let regrow = self.params.field_regrow;
        for f in self.field.iter_mut() {
            let v = *f + regrow;
            *f = if v > cap { cap } else { v };
        }

        self.hash.rebuild(&self.orgs);

        let tick = self.tick_count;
        let mut births: Vec<PendingBirth> = Vec::new();
        let mut scratch: Vec<Scalar> = Vec::new();
        let n = self.orgs.capacity();
        for i in 0..n {
            if !self.orgs.alive[i] {
                continue;
            }
            self.step_organism(i, tick, &mut scratch, &mut births, &mut events);
        }

        for (seq, b) in births.into_iter().enumerate() {
            let key = splitmix64(b.parent_id as u64)
                ^ splitmix64(tick.wrapping_mul(0x0100_0000_01b3))
                ^ splitmix64(seq as u64);
            let mut rng = Pcg32::from_key(self.seed, subsystem::MUTATION, key);
            let genome = mutate_genome(b.parent_genome, &mut rng, &self.params);
            let mut brain = b.parent_brain;
            brain.mutate(
                &mut rng,
                self.params.mutation_rate,
                self.params.weight_mut_delta,
                self.params.add_conn_prob,
                self.params.add_node_prob,
            );
            brain.reset_state();
            let dx = rng.next_f32_signed() * self.params.spawn_radius;
            let dy = rng.next_f32_signed() * self.params.spawn_radius;
            let px = wrap(b.px + dx, self.params.width);
            let py = wrap(b.py + dy, self.params.height);
            let (_, id) = self.orgs.insert(NewOrganism {
                px,
                py,
                vx: 0.0,
                vy: 0.0,
                energy: self.params.offspring_energy,
                parent: b.parent_id,
                birth_tick: tick,
                genome,
                brain,
            });
            events.events.push(Event::Birth {
                id,
                parent: b.parent_id,
                tick,
            });
        }

        self.tick_count += 1;
        events
    }

    fn step_organism(
        &mut self,
        i: usize,
        tick: u64,
        scratch: &mut Vec<Scalar>,
        births: &mut Vec<PendingBirth>,
        events: &mut EventBatch,
    ) {
        let p = &self.params;
        let cap = p.field_cap as f32;
        let sx = self.orgs.px[i];
        let sy = self.orgs.py[i];
        let size = self.orgs.g_size[i];

        // --- sense ---
        let (cx, cy) = self.cell_of(sx, sy);
        let food = self.field[self.fidx(cx, cy)] as f32 / cap;
        let east = self.field[self.fidx(cx + 1, cy)] as f32;
        let west = self.field[self.fidx(cx - 1, cy)] as f32;
        let north = self.field[self.fidx(cx, cy - 1)] as f32;
        let south = self.field[self.fidx(cx, cy + 1)] as f32;
        let grad_x = (east - west) / cap;
        let grad_y = (south - north) / cap;
        let energy_in = (self.orgs.energy[i] as f32 / p.repro_threshold as f32).min(2.0) - 1.0;

        let nn = self.hash.nearest(&self.orgs, i, sx, sy, p.sense_radius);
        let (nn_dx, nn_dy, nn_relsize) = match nn {
            Some(j) => (
                clamp_abs((self.orgs.px[j] - sx) / p.sense_radius, 1.0),
                clamp_abs((self.orgs.py[j] - sy) / p.sense_radius, 1.0),
                clamp_abs(self.orgs.g_size[j] - size, 1.0),
            ),
            None => (0.0, 0.0, 0.0),
        };

        let inputs = [
            1.0, food, grad_x, grad_y, energy_in, nn_dx, nn_dy, nn_relsize,
        ];

        // --- think ---
        let out = self.orgs.brains[i].forward(&inputs, scratch);

        // --- act ---
        let accel = p.accel_scale / size;
        let vx = clamp_abs(self.orgs.vx[i] * 0.85 + out[OUT_AX] * accel, p.max_speed);
        let vy = clamp_abs(self.orgs.vy[i] * 0.85 + out[OUT_AY] * accel, p.max_speed);
        self.orgs.vx[i] = vx;
        self.orgs.vy[i] = vy;
        self.orgs.px[i] = wrap(sx + vx, p.width);
        self.orgs.py[i] = wrap(sy + vy, p.height);

        // --- eat from the field ---
        let (ncx, ncy) = self.cell_of(self.orgs.px[i], self.orgs.py[i]);
        let fi = self.fidx(ncx, ncy);
        let want = ((p.eat_rate as f32) * self.orgs.g_metab[i]) as i64;
        let intake = want.min(self.field[fi]).max(0);
        self.field[fi] -= intake;
        self.orgs.energy[i] += intake;

        // --- predation: innate bite of a sufficiently-smaller neighbour ---
        if let Some(j) = nn {
            if self.orgs.alive[j] && size >= self.orgs.g_size[j] * p.predation_size_ratio {
                let dx = self.orgs.px[j] - self.orgs.px[i];
                let dy = self.orgs.py[j] - self.orgs.py[i];
                if dx * dx + dy * dy <= p.contact_radius * p.contact_radius {
                    let victim = self.orgs.energy[j];
                    if victim > 0 {
                        let steal = p.bite_amount.min(victim);
                        self.orgs.energy[j] -= steal;
                        let gain = steal * p.predation_gain_num / p.predation_gain_den;
                        self.orgs.energy[i] += gain;
                        self.orgs.carnivory[i] = (self.orgs.carnivory[i] + 0.3).min(1.0);
                        if self.orgs.energy[j] <= 0 {
                            let (jcx, jcy) = self.cell_of(self.orgs.px[j], self.orgs.py[j]);
                            let fj = self.fidx(jcx, jcy);
                            let v = self.field[fj] + p.death_deposit;
                            self.field[fj] = if v > p.field_cap { p.field_cap } else { v };
                            events.events.push(Event::Death {
                                id: self.orgs.id[j],
                                cause: DeathCause::Predated,
                                tick,
                            });
                            self.orgs.kill(j);
                        }
                    }
                }
            }
        }

        // --- metabolism (brain complexity is taxed) ---
        let sq = vx * vx + vy * vy;
        let move_cost = (sq * (p.move_cost_coeff as f32) * size) as i64;
        let size_cost = ((size - 1.0) * p.size_upkeep as f32).max(0.0) as i64;
        // Free up to a threshold so brains can accumulate structure over generations
        // (visible growth); a firm tax past it prevents runaway bloat (a soft cap).
        let brain_cost = (self.orgs.brains[i].complexity() as i64 - 18).max(0) * p.brain_cost;
        self.orgs.energy[i] -= p.basal_upkeep + brain_cost + size_cost + move_cost;
        self.orgs.carnivory[i] *= 0.99;
        self.orgs.age[i] += 1;

        // --- death ---
        if self.orgs.energy[i] <= 0 {
            self.deposit_detritus(fi);
            events.events.push(Event::Death {
                id: self.orgs.id[i],
                cause: DeathCause::Starved,
                tick,
            });
            self.orgs.kill(i);
            return;
        }
        if self.orgs.age[i] >= p.max_age {
            self.deposit_detritus(fi);
            events.events.push(Event::Death {
                id: self.orgs.id[i],
                cause: DeathCause::OldAge,
                tick,
            });
            self.orgs.kill(i);
            return;
        }

        // --- reproduce (per-organism threshold, brain-gated) ---
        let threshold = ((p.repro_threshold as f32) * self.orgs.g_repro[i]) as i64;
        let wants_repro = out[brain::OUT_REPRO] > -0.2;
        if wants_repro && self.orgs.energy[i] >= threshold && self.orgs.count < p.max_population {
            self.orgs.energy[i] -= p.repro_cost + p.offspring_energy;
            births.push(PendingBirth {
                parent_id: self.orgs.id[i],
                parent_genome: self.orgs.genome_at(i),
                parent_brain: self.orgs.brains[i].clone(),
                px: self.orgs.px[i],
                py: self.orgs.py[i],
            });
        }
    }

    fn apply_command(&mut self, c: &Command, events: &mut EventBatch) {
        match &c.kind {
            CommandKind::SetParam { key, raw } => self.params.set(*key, *raw),
            CommandKind::InjectSubstance {
                cx,
                cy,
                radius,
                amount,
            } => {
                let r = (*radius).max(0);
                for dy in -r..=r {
                    for dx in -r..=r {
                        if dx * dx + dy * dy <= r * r {
                            let fi = self.fidx(cx + dx, cy + dy);
                            let v = self.field[fi] + *amount;
                            self.field[fi] = v.clamp(0, self.params.field_cap);
                        }
                    }
                }
            }
            CommandKind::Spawn { cx, cy, energy } => {
                let cw = self.params.width / self.params.grid_w as f32;
                let ch = self.params.height / self.params.grid_h as f32;
                let px = ((*cx).rem_euclid(self.params.grid_w as i32) as f32 + 0.5) * cw;
                let py = ((*cy).rem_euclid(self.params.grid_h as i32) as f32 + 0.5) * ch;
                let key = splitmix64(self.tick_count)
                    ^ splitmix64(self.orgs.next_id as u64)
                    ^ splitmix64((((*cx as i64) << 20) ^ *cy as i64) as u64);
                let mut rng = Pcg32::from_key(self.seed, subsystem::SPAWN, key);
                let genome = Genome {
                    size: 0.6 + rng.next_f32_unit() * 1.0,
                    metabolism: 0.6 + rng.next_f32_unit() * 0.8,
                    repro: 0.6 + rng.next_f32_unit() * 0.8,
                    r: rng.below(256) as u8,
                    g: rng.below(256) as u8,
                    b: rng.below(256) as u8,
                };
                let brain = Brain::random_minimal(&mut rng);
                self.orgs.insert(NewOrganism {
                    px,
                    py,
                    vx: 0.0,
                    vy: 0.0,
                    energy: *energy,
                    parent: u32::MAX,
                    birth_tick: self.tick_count,
                    genome,
                    brain,
                });
            }
            CommandKind::Kill { cx0, cy0, cx1, cy1 } => {
                let (xlo, xhi) = ((*cx0).min(*cx1), (*cx0).max(*cx1));
                let (ylo, yhi) = ((*cy0).min(*cy1), (*cy0).max(*cy1));
                let gw = self.params.grid_w as i32;
                let gh = self.params.grid_h as i32;
                for i in 0..self.orgs.capacity() {
                    if !self.orgs.alive[i] {
                        continue;
                    }
                    let (cx, cy) = self.cell_of(self.orgs.px[i], self.orgs.py[i]);
                    let cx = cx.rem_euclid(gw);
                    let cy = cy.rem_euclid(gh);
                    if cx >= xlo && cx <= xhi && cy >= ylo && cy <= yhi {
                        events.events.push(Event::Death {
                            id: self.orgs.id[i],
                            cause: DeathCause::Killed,
                            tick: self.tick_count,
                        });
                        self.orgs.kill(i);
                    }
                }
            }
            CommandKind::Reset { seed } => {
                *self = World::new(*seed, self.params.clone());
            }
        }
    }

    /// A 64-bit fingerprint of the entire world state, including every organism's brain
    /// structure and recurrent activations. Organisms are hashed in stable-id order.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv::new();
        h.u64(self.tick_count);
        h.u64(self.seed);
        h.u32(self.field.len() as u32);
        for &c in &self.field {
            h.i64(c);
        }

        let mut idx: Vec<usize> = (0..self.orgs.capacity())
            .filter(|&i| self.orgs.alive[i])
            .collect();
        idx.sort_by_key(|&i| self.orgs.id[i]);
        h.u32(idx.len() as u32);
        let o = &self.orgs;
        for &i in &idx {
            h.u32(o.id[i]);
            h.u32(canonical_bits(o.px[i]));
            h.u32(canonical_bits(o.py[i]));
            h.u32(canonical_bits(o.vx[i]));
            h.u32(canonical_bits(o.vy[i]));
            h.i64(o.energy[i]);
            h.u32(o.age[i]);
            h.u32(o.parent[i]);
            h.u64(o.birth_tick[i]);
            h.u32(canonical_bits(o.g_size[i]));
            h.u32(canonical_bits(o.g_metab[i]));
            h.u32(canonical_bits(o.g_repro[i]));
            h.u8(o.cr[i]);
            h.u8(o.cg[i]);
            h.u8(o.cb[i]);
            h.u32(canonical_bits(o.carnivory[i]));
            let br = &o.brains[i];
            h.u32(br.n_hidden as u32);
            h.u32(br.conns.len() as u32);
            for c in &br.conns {
                h.u32(c.from);
                h.u32(c.to);
                h.u32(canonical_bits(c.w));
                h.u8(c.enabled as u8);
            }
            for &x in &br.bias {
                h.u32(canonical_bits(x));
            }
            for &x in &br.act {
                h.u32(canonical_bits(x));
            }
        }
        h.finish()
    }
}

fn mutate_genome(mut g: Genome, rng: &mut Pcg32, p: &WorldParams) -> Genome {
    let rate = p.mutation_rate;
    let d = p.mutation_delta;
    if rng.chance(rate) {
        g.size = (g.size + rng.next_f32_signed() * d).clamp(0.4, 2.2);
    }
    if rng.chance(rate) {
        g.metabolism = (g.metabolism + rng.next_f32_signed() * d).clamp(0.3, 2.0);
    }
    if rng.chance(rate) {
        g.repro = (g.repro + rng.next_f32_signed() * d).clamp(0.5, 1.5);
    }
    if rng.chance(rate) {
        g.r = mut_u8(g.r, rng);
    }
    if rng.chance(rate) {
        g.g = mut_u8(g.g, rng);
    }
    if rng.chance(rate) {
        g.b = mut_u8(g.b, rng);
    }
    g
}

#[inline]
fn mut_u8(v: u8, rng: &mut Pcg32) -> u8 {
    let delta = rng.below(21) as i32 - 10;
    (v as i32 + delta).clamp(0, 255) as u8
}

struct Fnv(u64);

impl Fnv {
    #[inline]
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    #[inline]
    fn u8(&mut self, b: u8) {
        self.0 ^= b as u64;
        self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
    }
    #[inline]
    fn u32(&mut self, v: u32) {
        for b in v.to_le_bytes() {
            self.u8(b);
        }
    }
    #[inline]
    fn i64(&mut self, v: i64) {
        for b in v.to_le_bytes() {
            self.u8(b);
        }
    }
    #[inline]
    fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.u8(b);
        }
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(seed: u64, ticks: u64) -> World {
        let mut w = World::new(seed, WorldParams::default());
        for _ in 0..ticks {
            w.tick(&[]);
        }
        w
    }

    #[test]
    fn deterministic_same_seed() {
        assert_eq!(run(12345, 300).state_hash(), run(12345, 300).state_hash());
    }

    #[test]
    fn different_seed_diverges() {
        assert_ne!(run(1, 300).state_hash(), run(2, 300).state_hash());
    }

    #[test]
    fn world_is_alive_and_dynamic() {
        let mut w = World::new(777, WorldParams::default());
        let mut births = 0usize;
        for _ in 0..400 {
            births += w.tick(&[]).births();
        }
        assert!(w.population() > 0, "world went extinct");
        assert!(births > 0, "no reproduction happened");
    }
}
