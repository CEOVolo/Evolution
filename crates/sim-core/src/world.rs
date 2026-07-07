//! The world and its tick — the one hot function the whole engine is built around.
//!
//! A **living, pressuring** world: food waxes and wanes on a day/night–season cycle and
//! concentrates in drifting "bloom" patches; predation is **skill-based** (a predator only
//! catches prey it is actually pursuing, so prey can evade); and organisms can **emit and
//! sense a signal** whose meaning evolution assigns. Every organism has a growing recurrent
//! brain ([`crate::brain`]). Nothing is hard-coded to a role or outcome — foraging, fleeing,
//! hunting, timing, and signalling all emerge (or don't) under these pressures.
//!
//! Fixed phase order per tick: 1. commands, 2. drift blooms + regrow fields + decay signal,
//! 3. rebuild spatial hash, 4. organisms act (index order), 5. apply births.

use crate::brain::{self, Brain, OUT_AX, OUT_AY};
use crate::command::{Command, CommandKind};
use crate::environment::SpatialHash;
use crate::event::{DeathCause, Event, EventBatch};
use crate::math::{canonical_bits, clamp_abs, wrap, Scalar};
use crate::organism::{Genome, NewOrganism, Organisms};
use crate::params::WorldParams;
use crate::rng::{splitmix64, subsystem, Pcg32};

const BLOOM_RADIUS_SQ: Scalar = 45.0 * 45.0;
const BLOOM_DRIFT: Scalar = 1.5;

#[derive(Clone, Debug)]
pub struct World {
    pub params: WorldParams,
    pub seed: u64,
    pub tick_count: u64,
    pub field: Vec<i64>,
    /// Static terrain fertility per cell (~0.15..1.9): a landscape of rich and barren places
    /// that permanently scales local regrowth, so different regions become different habitats.
    pub terrain: Vec<Scalar>,
    /// Static elevation per cell (0=deep water .. 1=high land): seas, shores and highlands.
    /// Low ground is a barrier — organisms pay for being where their evolved `habitat` trait
    /// is a poor fit, so water divides the map and adapting to it (or leaving it) is selection.
    pub elevation: Vec<Scalar>,
    /// Corpse/detritus field: dead organisms leave body mass here; it slowly decomposes into
    /// the food field (soil enrichment) and can be eaten directly (scavenging).
    pub detritus: Vec<i64>,
    /// Pheromone/signal field (one integer per grid cell); decays over time.
    pub signal: Vec<i64>,
    /// Drifting bloom centres where regrowth is boosted.
    pub blooms: Vec<(Scalar, Scalar)>,
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
        let mut brng = Pcg32::from_key(seed, subsystem::BLOOM, 0);
        let blooms = (0..params.bloom_count)
            .map(|_| {
                (
                    brng.next_f32_unit() * params.width,
                    brng.next_f32_unit() * params.height,
                )
            })
            .collect();
        let terrain = make_terrain(seed, &params);
        let elevation = make_elevation(seed, &params);
        let mut w = World {
            field: vec![params.field_cap; cells],
            terrain,
            elevation,
            detritus: vec![0; cells],
            signal: vec![0; cells],
            blooms,
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
                habitat: rng.next_f32_unit(),
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

    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        params: WorldParams,
        seed: u64,
        tick_count: u64,
        field: Vec<i64>,
        terrain: Vec<Scalar>,
        elevation: Vec<Scalar>,
        detritus: Vec<i64>,
        signal: Vec<i64>,
        blooms: Vec<(Scalar, Scalar)>,
        orgs: Organisms,
    ) -> Self {
        let hash = SpatialHash::new(params.width, params.height, params.sense_radius);
        World {
            params,
            seed,
            tick_count,
            field,
            terrain,
            elevation,
            detritus,
            signal,
            blooms,
            orgs,
            hash,
        }
    }

    #[inline]
    pub fn population(&self) -> u32 {
        self.orgs.count
    }

    /// Day/night–season light level in `0..=1` (a triangle wave over `day_period` ticks).
    pub fn daylight(&self) -> f32 {
        let period = self.params.day_period.max(1) as u64;
        let ph = (self.tick_count % period) as f32 / period as f32;
        if ph < 0.5 {
            ph * 2.0
        } else {
            2.0 - ph * 2.0
        }
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

    /// Total edible value at a cell: living food plus scavengeable corpses.
    #[inline]
    fn food_at(&self, cx: i32, cy: i32) -> i64 {
        let i = self.fidx(cx, cy);
        self.field[i] + self.detritus[i]
    }

    /// A dead organism leaves body mass (scaled by size) in the detritus field.
    #[inline]
    fn deposit_corpse(&mut self, fi: usize, size: Scalar) {
        let mass =
            self.params.death_deposit + (size * self.params.corpse_size_factor as f32) as i64;
        let capd = self.params.field_cap * 4;
        let v = self.detritus[fi] + mass;
        self.detritus[fi] = if v > capd { capd } else { v };
    }

    pub fn tick(&mut self, cmds: &[Command]) -> EventBatch {
        let mut events = EventBatch::new();

        for c in cmds {
            self.apply_command(c, &mut events);
        }

        // Drift the bloom centres (seeded Brownian walk).
        let (seed, tick_c, w_w, w_h) = (
            self.seed,
            self.tick_count,
            self.params.width,
            self.params.height,
        );
        for (k, bloom) in self.blooms.iter_mut().enumerate() {
            let mut r = Pcg32::from_key(
                seed,
                subsystem::BLOOM,
                splitmix64(tick_c) ^ splitmix64(k as u64),
            );
            bloom.0 = wrap(bloom.0 + r.next_f32_signed() * BLOOM_DRIFT, w_w);
            bloom.1 = wrap(bloom.1 + r.next_f32_signed() * BLOOM_DRIFT, w_h);
        }

        // Regrow the field: modulated by daylight and boosted near bloom centres.
        let daylight = self.daylight();
        let day_factor = 0.4 + daylight * 1.2; // [0.4, 1.6]
        let cap = self.params.field_cap;
        let base = self.params.field_regrow;
        let boost = self.params.bloom_boost;
        let gw = self.params.grid_w as usize;
        let gh = self.params.grid_h as usize;
        let cw = self.params.width / gw as f32;
        let ch = self.params.height / gh as f32;
        for cyi in 0..gh {
            let ccy = (cyi as f32 + 0.5) * ch;
            for cxi in 0..gw {
                let ccx = (cxi as f32 + 0.5) * cw;
                let idx = cyi * gw + cxi;
                let mut rg = (base as f32 * day_factor * self.terrain[idx]) as i64;
                for &(bx, by) in &self.blooms {
                    let dx = ccx - bx;
                    let dy = ccy - by;
                    if dx * dx + dy * dy <= BLOOM_RADIUS_SQ {
                        rg += boost;
                        break;
                    }
                }
                let v = self.field[idx] + rg;
                self.field[idx] = if v > cap { cap } else { v };
            }
        }

        // Decompose corpses slowly into soil food.
        let ddiv = self.params.decompose_div.max(1);
        for c in 0..self.field.len() {
            let dec = self.detritus[c] / ddiv;
            if dec > 0 {
                self.detritus[c] -= dec;
                let v = self.field[c] + dec;
                self.field[c] = if v > cap { cap } else { v };
            }
        }

        // Decay the signal field.
        for s in self.signal.iter_mut() {
            *s = *s * 9 / 10;
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
            self.step_organism(i, tick, daylight, &mut scratch, &mut births, &mut events);
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
        daylight: f32,
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
        let here = self.fidx(cx, cy);
        let food = self.food_at(cx, cy) as f32 / cap;
        let east = self.food_at(cx + 1, cy) as f32;
        let west = self.food_at(cx - 1, cy) as f32;
        let north = self.food_at(cx, cy - 1) as f32;
        let south = self.food_at(cx, cy + 1) as f32;
        let grad_x = (east - west) / cap;
        let grad_y = (south - north) / cap;
        let energy_in = (self.orgs.energy[i] as f32 / p.repro_threshold as f32).min(2.0) - 1.0;
        let signal_in = (self.signal[here] as f32 / p.signal_cap.max(1) as f32).min(1.0);
        let elev_in = self.elevation[here] * 2.0 - 1.0;

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
            1.0,
            food,
            grad_x,
            grad_y,
            energy_in,
            nn_dx,
            nn_dy,
            nn_relsize,
            daylight * 2.0 - 1.0,
            signal_in,
            elev_in,
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
        let from_field = want.min(self.field[fi]).max(0);
        self.field[fi] -= from_field;
        // Scavenging corpses is slower than grazing, so carcasses linger (and are visible).
        let scav = ((want - from_field) / 2).max(0);
        let from_det = scav.min(self.detritus[fi]);
        self.detritus[fi] -= from_det;
        self.orgs.energy[i] += from_field + from_det;

        // --- emit signal ---
        let emit = out[brain::OUT_EMIT];
        if emit > 0.0 {
            let v = self.signal[fi] + (emit * p.emit_scale as f32) as i64;
            self.signal[fi] = if v > p.signal_cap { p.signal_cap } else { v };
        }

        // --- predation: catch a smaller neighbour you are actually pursuing ---
        if let Some(j) = nn {
            if self.orgs.alive[j] && size >= self.orgs.g_size[j] * p.predation_size_ratio {
                let dxp = self.orgs.px[j] - self.orgs.px[i];
                let dyp = self.orgs.py[j] - self.orgs.py[i];
                let toward = dxp * vx + dyp * vy; // moving toward the prey?
                if toward > 0.0 && dxp * dxp + dyp * dyp <= p.contact_radius * p.contact_radius {
                    let victim = self.orgs.energy[j];
                    if victim > 0 {
                        let steal = p.bite_amount.min(victim);
                        self.orgs.energy[j] -= steal;
                        let gain = steal * p.predation_gain_num / p.predation_gain_den;
                        self.orgs.energy[i] += gain;
                        self.orgs.carnivory[i] = (self.orgs.carnivory[i] + 0.3).min(1.0);
                        if self.orgs.energy[j] <= 0 {
                            let vsize = self.orgs.g_size[j];
                            let (jcx, jcy) = self.cell_of(self.orgs.px[j], self.orgs.py[j]);
                            let fj = self.fidx(jcx, jcy);
                            let mass =
                                p.death_deposit + (vsize * p.corpse_size_factor as f32) as i64;
                            let capd = p.field_cap * 4;
                            let v = self.detritus[fj] + mass;
                            self.detritus[fj] = if v > capd { capd } else { v };
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

        // --- metabolism (brain complexity is taxed above a free threshold) ---
        let sq = vx * vx + vy * vy;
        let move_cost = (sq * (p.move_cost_coeff as f32) * size) as i64;
        let size_cost = ((size - 1.0) * p.size_upkeep as f32).max(0.0) as i64;
        let brain_cost = (self.orgs.brains[i].complexity() as i64 - 18).max(0) * p.brain_cost;
        // habitat mismatch: being where your evolved `habitat` trait doesn't fit the local
        // elevation drains energy (quadratically). Deep water is thus lethal to the ill-adapted
        // — a barrier — and matching a place beats generalising, so water/land split into niches.
        let mism = self.elevation[fi] - self.orgs.g_habitat[i];
        let habitat_cost = ((p.habitat_cost as f32) * mism * mism) as i64;
        self.orgs.energy[i] -= p.basal_upkeep + brain_cost + size_cost + move_cost + habitat_cost;
        self.orgs.carnivory[i] *= 0.99;
        self.orgs.age[i] += 1;

        // --- death ---
        if self.orgs.energy[i] <= 0 {
            self.deposit_corpse(fi, size);
            events.events.push(Event::Death {
                id: self.orgs.id[i],
                cause: DeathCause::Starved,
                tick,
            });
            self.orgs.kill(i);
            return;
        }
        if self.orgs.age[i] >= p.max_age {
            self.deposit_corpse(fi, size);
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
                    habitat: rng.next_f32_unit(),
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

    /// A 64-bit fingerprint of the entire world state — fields, signal, blooms, and every
    /// organism (including brain structure + recurrent activations), hashed in id order.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv::new();
        h.u64(self.tick_count);
        h.u64(self.seed);
        h.u32(self.field.len() as u32);
        for &c in &self.field {
            h.i64(c);
        }
        for &t in &self.terrain {
            h.u32(canonical_bits(t));
        }
        for &el in &self.elevation {
            h.u32(canonical_bits(el));
        }
        for &dt in &self.detritus {
            h.i64(dt);
        }
        for &s in &self.signal {
            h.i64(s);
        }
        for &(bx, by) in &self.blooms {
            h.u32(canonical_bits(bx));
            h.u32(canonical_bits(by));
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
            h.u32(canonical_bits(o.g_habitat[i]));
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

/// Generate a static fertility landscape from the seed: a few fertile and barren centres with
/// linear falloff, giving smooth rich/poor regions. Deterministic (only `+ - * / sqrt`).
fn make_terrain(seed: u64, p: &WorldParams) -> Vec<Scalar> {
    let gw = p.grid_w as i32;
    let gh = p.grid_h as i32;
    let cw = p.width / p.grid_w as f32;
    let ch = p.height / p.grid_h as f32;
    let radius = p.width * 0.28;
    let mut rng = Pcg32::from_key(seed, subsystem::TERRAIN, 0);
    let mut centres: Vec<(Scalar, Scalar, Scalar)> = Vec::new();
    for _ in 0..6 {
        centres.push((
            rng.next_f32_unit() * p.width,
            rng.next_f32_unit() * p.height,
            0.9,
        ));
    }
    for _ in 0..5 {
        centres.push((
            rng.next_f32_unit() * p.width,
            rng.next_f32_unit() * p.height,
            -0.9,
        ));
    }
    let mut t = vec![1.0f32; p.cell_count()];
    for cy in 0..gh {
        let py = (cy as f32 + 0.5) * ch;
        for cx in 0..gw {
            let px = (cx as f32 + 0.5) * cw;
            let mut f = 1.0f32;
            for &(bx, by, st) in &centres {
                let dx = px - bx;
                let dy = py - by;
                let d = (dx * dx + dy * dy).sqrt();
                if d < radius {
                    f += st * (1.0 - d / radius);
                }
            }
            t[(cy * gw + cx) as usize] = f.clamp(0.15, 1.9);
        }
    }
    t
}

/// Generate a static elevation map (0 = deep water .. 1 = high land) from the seed: a few sea
/// basins carved into a mid-level land, plus some highlands, each with linear falloff. The low
/// ground (below `water_level`) reads as water and — via the habitat-mismatch cost — acts as a
/// barrier that can split the map into isolated habitats. Deterministic (`+ - * / sqrt` only).
fn make_elevation(seed: u64, p: &WorldParams) -> Vec<Scalar> {
    let gw = p.grid_w as i32;
    let gh = p.grid_h as i32;
    let cw = p.width / p.grid_w as f32;
    let ch = p.height / p.grid_h as f32;
    let sea_r = p.width * 0.38;
    let peak_r = p.width * 0.25;
    let base = 0.62f32;
    let mut rng = Pcg32::from_key(seed, subsystem::ELEVATION, 0);
    let mut seas: Vec<(Scalar, Scalar)> = Vec::new();
    for _ in 0..3 {
        seas.push((
            rng.next_f32_unit() * p.width,
            rng.next_f32_unit() * p.height,
        ));
    }
    let mut peaks: Vec<(Scalar, Scalar)> = Vec::new();
    for _ in 0..3 {
        peaks.push((
            rng.next_f32_unit() * p.width,
            rng.next_f32_unit() * p.height,
        ));
    }
    let mut e = vec![base; p.cell_count()];
    for cy in 0..gh {
        let py = (cy as f32 + 0.5) * ch;
        for cx in 0..gw {
            let px = (cx as f32 + 0.5) * cw;
            let mut h = base;
            for &(sx, sy) in &seas {
                let dx = px - sx;
                let dy = py - sy;
                let d = (dx * dx + dy * dy).sqrt();
                if d < sea_r {
                    h -= 0.85 * (1.0 - d / sea_r);
                }
            }
            for &(mx, my) in &peaks {
                let dx = px - mx;
                let dy = py - my;
                let d = (dx * dx + dy * dy).sqrt();
                if d < peak_r {
                    h += 0.4 * (1.0 - d / peak_r);
                }
            }
            e[(cy * gw + cx) as usize] = h.clamp(0.0, 1.0);
        }
    }
    e
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
        g.habitat = (g.habitat + rng.next_f32_signed() * d).clamp(0.0, 1.0);
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

    #[test]
    fn daylight_cycles() {
        let w = World::new(1, WorldParams::default());
        let d = w.daylight();
        assert!((0.0..=1.0).contains(&d));
    }
}
