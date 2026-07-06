//! The world and its tick — the one hot function the whole engine is built around.
//!
//! Phase 0 is a trivial-but-real mechanistic world: organisms sense a depletable resource
//! field, steer up its gradient, eat, pay energy costs, die into detritus, and reproduce
//! with mutation. No behaviour is hard-coded to a role — every trait lives in the genome.
//!
//! Fixed phase order per tick (the ordering is part of the determinism contract):
//! 1. apply commands, 2. regrow fields, 3. organisms act (index order), 4. apply births.

use crate::command::{Command, CommandKind};
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
    /// Resource field, one integer per grid cell (row-major).
    pub field: Vec<i64>,
    pub orgs: Organisms,
}

struct PendingBirth {
    parent_id: u32,
    parent_genome: Genome,
    px: Scalar,
    py: Scalar,
}

impl World {
    /// Create a world and spawn its initial population deterministically from `seed`.
    pub fn new(seed: u64, params: WorldParams) -> Self {
        let cells = params.cell_count();
        let mut w = World {
            field: vec![params.field_cap; cells],
            orgs: Organisms::with_capacity(params.initial_population as usize),
            params,
            seed,
            tick_count: 0,
        };
        let mut rng = Pcg32::from_key(seed, subsystem::SPAWN, 0);
        for _ in 0..w.params.initial_population {
            let px = rng.next_f32_unit() * w.params.width;
            let py = rng.next_f32_unit() * w.params.height;
            let genome = Genome {
                speed: rng.next_f32_unit(),
                metabolism: 0.6 + rng.next_f32_unit() * 0.8,
                repro: 0.6 + rng.next_f32_unit() * 0.8,
                r: rng.below(256) as u8,
                g: rng.below(256) as u8,
                b: rng.below(256) as u8,
            };
            w.orgs.insert(NewOrganism {
                px,
                py,
                vx: 0.0,
                vy: 0.0,
                energy: w.params.initial_energy,
                parent: u32::MAX,
                birth_tick: 0,
                genome,
            });
        }
        w
    }

    #[inline]
    pub fn population(&self) -> u32 {
        self.orgs.count
    }

    // --- grid helpers -------------------------------------------------------

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

    // --- tick ---------------------------------------------------------------

    pub fn tick(&mut self, cmds: &[Command]) -> EventBatch {
        let mut events = EventBatch::new();

        // 1. Commands are the sole mutation channel.
        for c in cmds {
            self.apply_command(c, &mut events);
        }

        // 2. Fields: regrow toward cap (sources). Depletion (sink) happens during act.
        let cap = self.params.field_cap;
        let regrow = self.params.field_regrow;
        for f in self.field.iter_mut() {
            let v = *f + regrow;
            *f = if v > cap { cap } else { v };
        }

        // 3. Organisms act in index order. Births are deferred so newborns are not processed
        //    this tick and the order is stable.
        let tick = self.tick_count;
        let mut births: Vec<PendingBirth> = Vec::new();
        let n = self.orgs.capacity();
        for i in 0..n {
            if !self.orgs.alive[i] {
                continue;
            }
            self.step_organism(i, tick, &mut births, &mut events);
        }

        // 4. Apply births. Mutation RNG is keyed by (seed, parent_id, tick, birth_seq) so it
        //    is independent of slot layout and stable as features are added.
        for (seq, b) in births.into_iter().enumerate() {
            let key = splitmix64(b.parent_id as u64)
                ^ splitmix64(tick.wrapping_mul(0x0100_0000_01b3))
                ^ splitmix64(seq as u64);
            let mut rng = Pcg32::from_key(self.seed, subsystem::MUTATION, key);
            let genome = mutate(b.parent_genome, &mut rng, &self.params);
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
        births: &mut Vec<PendingBirth>,
        events: &mut EventBatch,
    ) {
        // --- sense: gradient among the 4 neighbours (ties: here, N, E, S, W) ---
        let (cx, cy) = self.cell_of(self.orgs.px[i], self.orgs.py[i]);
        let here = self.field[self.fidx(cx, cy)];
        // Neighbours in fixed tie-break order: stay, N, E, S, W. Steer toward the strictly
        // richest cell (ties keep the earlier candidate, so behaviour is order-stable).
        let candidates = [
            (here, 0.0f32, 0.0f32),
            (self.field[self.fidx(cx, cy - 1)], 0.0, -1.0),
            (self.field[self.fidx(cx + 1, cy)], 1.0, 0.0),
            (self.field[self.fidx(cx, cy + 1)], 0.0, 1.0),
            (self.field[self.fidx(cx - 1, cy)], -1.0, 0.0),
        ];
        let mut best = here;
        let mut dirx = 0.0f32;
        let mut diry = 0.0f32;
        for &(val, dx, dy) in candidates.iter().skip(1) {
            if val > best {
                best = val;
                dirx = dx;
                diry = dy;
            }
        }

        // --- act: damp, steer, clamp, move ---
        let accel = self.params.steer_accel * self.orgs.g_speed[i];
        let vmax = self.params.max_speed * self.orgs.g_speed[i];
        let mut vx = self.orgs.vx[i] * 0.85 + dirx * accel;
        let mut vy = self.orgs.vy[i] * 0.85 + diry * accel;
        vx = clamp_abs(vx, vmax);
        vy = clamp_abs(vy, vmax);
        self.orgs.vx[i] = vx;
        self.orgs.vy[i] = vy;
        self.orgs.px[i] = wrap(self.orgs.px[i] + vx, self.params.width);
        self.orgs.py[i] = wrap(self.orgs.py[i] + vy, self.params.height);

        // --- eat at the new cell (sink) ---
        let (ncx, ncy) = self.cell_of(self.orgs.px[i], self.orgs.py[i]);
        let fi = self.fidx(ncx, ncy);
        let want = ((self.params.eat_rate as f32) * self.orgs.g_metab[i]) as i64;
        let intake = want.min(self.field[fi]).max(0);
        self.field[fi] -= intake;
        self.orgs.energy[i] += intake;

        // --- costs ---
        let sq = vx * vx + vy * vy;
        let move_cost = (sq * (self.params.move_cost_coeff as f32)) as i64;
        self.orgs.energy[i] -= self.params.basal_upkeep + move_cost;
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
        if self.orgs.age[i] >= self.params.max_age {
            self.deposit_detritus(fi);
            events.events.push(Event::Death {
                id: self.orgs.id[i],
                cause: DeathCause::OldAge,
                tick,
            });
            self.orgs.kill(i);
            return;
        }

        // --- reproduce (per-organism threshold) ---
        let threshold = ((self.params.repro_threshold as f32) * self.orgs.g_repro[i]) as i64;
        if self.orgs.energy[i] >= threshold && self.orgs.count < self.params.max_population {
            self.orgs.energy[i] -= self.params.repro_cost + self.params.offspring_energy;
            births.push(PendingBirth {
                parent_id: self.orgs.id[i],
                parent_genome: self.orgs.genome_at(i),
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
                self.orgs.insert(NewOrganism {
                    px,
                    py,
                    vx: 0.0,
                    vy: 0.0,
                    energy: *energy,
                    parent: u32::MAX,
                    birth_tick: self.tick_count,
                    genome: Genome::base(),
                });
            }
            CommandKind::Kill { cx0, cy0, cx1, cy1 } => {
                let (xlo, xhi) = ((*cx0).min(*cx1), (*cx0).max(*cx1));
                let (ylo, yhi) = ((*cy0).min(*cy1), (*cy0).max(*cy1));
                let n = self.orgs.capacity();
                let gw = self.params.grid_w as i32;
                let gh = self.params.grid_h as i32;
                for i in 0..n {
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

    // --- determinism oracle -------------------------------------------------

    /// A 64-bit fingerprint of the entire world state. Organisms are hashed in stable-id
    /// order so the result is independent of slot layout. Includes every causally-relevant
    /// field (floats via canonicalized bits).
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv::new();
        h.u64(self.tick_count);
        h.u64(self.seed);
        hash_params(&mut h, &self.params);

        h.u32(self.field.len() as u32);
        for &c in &self.field {
            h.i64(c);
        }

        let mut idx: Vec<usize> = (0..self.orgs.capacity())
            .filter(|&i| self.orgs.alive[i])
            .collect();
        idx.sort_by_key(|&i| self.orgs.id[i]);
        h.u32(idx.len() as u32);
        for &i in &idx {
            h.u32(self.orgs.id[i]);
            h.u32(canonical_bits(self.orgs.px[i]));
            h.u32(canonical_bits(self.orgs.py[i]));
            h.u32(canonical_bits(self.orgs.vx[i]));
            h.u32(canonical_bits(self.orgs.vy[i]));
            h.i64(self.orgs.energy[i]);
            h.u32(self.orgs.age[i]);
            h.u32(self.orgs.parent[i]);
            h.u64(self.orgs.birth_tick[i]);
            h.u32(canonical_bits(self.orgs.g_speed[i]));
            h.u32(canonical_bits(self.orgs.g_metab[i]));
            h.u32(canonical_bits(self.orgs.g_repro[i]));
            h.u8(self.orgs.cr[i]);
            h.u8(self.orgs.cg[i]);
            h.u8(self.orgs.cb[i]);
        }
        h.finish()
    }
}

fn mutate(mut g: Genome, rng: &mut Pcg32, p: &WorldParams) -> Genome {
    let rate = p.mutation_rate;
    let d = p.mutation_delta;
    if rng.chance(rate) {
        g.speed = (g.speed + rng.next_f32_signed() * d).clamp(0.0, 1.0);
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

// --- FNV-1a 64-bit, over a canonical little-endian byte stream ---

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

fn hash_params(h: &mut Fnv, p: &WorldParams) {
    h.u32(canonical_bits(p.width));
    h.u32(canonical_bits(p.height));
    h.u32(p.grid_w);
    h.u32(p.grid_h);
    h.i64(p.field_cap);
    h.i64(p.field_regrow);
    h.u32(canonical_bits(p.max_speed));
    h.u32(canonical_bits(p.steer_accel));
    h.i64(p.move_cost_coeff);
    h.i64(p.basal_upkeep);
    h.i64(p.eat_rate);
    h.i64(p.death_deposit);
    h.u32(p.max_age);
    h.i64(p.repro_threshold);
    h.i64(p.repro_cost);
    h.i64(p.offspring_energy);
    h.u32(canonical_bits(p.spawn_radius));
    h.u32(canonical_bits(p.mutation_rate));
    h.u32(canonical_bits(p.mutation_delta));
    h.u32(p.initial_population);
    h.i64(p.initial_energy);
    h.u32(p.max_population);
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
        let a = run(12345, 500);
        let b = run(12345, 500);
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn different_seed_diverges() {
        let a = run(1, 500);
        let b = run(2, 500);
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn world_is_alive_and_dynamic() {
        // Sanity: the default preset should not instantly die out, and should show births.
        let mut w = World::new(777, WorldParams::default());
        let mut total_births = 0usize;
        for _ in 0..400 {
            total_births += w.tick(&[]).births();
        }
        assert!(w.population() > 0, "world went extinct");
        assert!(total_births > 0, "no reproduction happened");
    }
}
