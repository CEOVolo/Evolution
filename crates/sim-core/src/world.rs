//! The world and its tick — the one hot function the whole engine is built around.
//!
//! The unit of life is a multicellular **organism**: one genome grows a **body** of
//! differentiated cells ([`crate::develop`]) at birth; the whole body then shares one energy
//! pool, senses its surroundings, is driven by one growing brain ([`crate::brain`]), feeds
//! through its cells, and reproduces (clonally) by shedding a mutated seed that develops into a
//! new body. Cell roles (feeding, structure) are continuous evolved traits, never a coded type —
//! nothing here is hard-wired to an outcome.
//!
//! Fixed phase order per tick: 1. commands, 2. environment (food-burst events + regrow + decay),
//! 3. rebuild spatial hash, 4. organisms act (index order), 5. apply births.

use crate::brain::{self, Brain, N_CHAN};
use crate::command::{Command, CommandKind};
use crate::environment::SpatialHash;
use crate::event::{DeathCause, Event, EventBatch};
use crate::genome::{GeneKind, Genome};
use crate::math::{canonical_bits, clamp_abs, wrap, Scalar};
use crate::organism::{NewOrganism, Organisms};
use crate::params::WorldParams;
use crate::regnet::RegNet;
use crate::rng::{splitmix64, subsystem, Pcg32};

/// World units between adjacent body-lattice cells (sets how big a developed body is on screen).
pub const SPACING: Scalar = 2.5;
/// Feeding: a cell always harvests a base fraction, feeder cells much more (`role_feed` scales it).
const FEED_BASE: f32 = 0.30;
const FEED_GAIN: f32 = 0.80;
/// Upkeep per fully-structural cell (defense is not free — the role trade-off's cost side).
const STRUCT_UPKEEP_FULL: i64 = 2;
/// Below this squared speed the heading is kept (avoids normalizing a zero vector).
const HEADING_EPS: f32 = 1e-6;

/// A transient **food-burst event**: a disc where food regrows faster, for a limited time.
#[derive(Clone, Debug)]
pub struct Bloom {
    pub x: Scalar,
    pub y: Scalar,
    pub radius: Scalar,
    pub boost: i64,
    pub age: u32,
    pub life: u32,
}

#[derive(Clone, Debug)]
pub struct World {
    pub params: WorldParams,
    pub seed: u64,
    pub tick_count: u64,
    /// Food field (the resource organisms eat).
    pub field: Vec<i64>,
    /// Second food field, anti-correlated with `field`. Regrows but is unused by organisms in
    /// Stage 1 (kept resident so the two-food ecology can be re-attached as a cell function later).
    pub field_b: Vec<i64>,
    /// Static terrain fertility per cell (~0.15..1.9): scales local regrowth into rich/poor regions.
    pub terrain: Vec<Scalar>,
    /// Static elevation per cell (0=deep water .. 1=high land). Present for later habitat coupling.
    pub elevation: Vec<Scalar>,
    /// Corpse/detritus field: dead bodies leave mass here; it decomposes into the food field.
    pub detritus: Vec<i64>,
    /// Pheromone/signal field (decays); present for later re-attachment.
    pub signal: Vec<i64>,
    /// Generic chemical channels — resident but inert in Stage 1 (re-attached as a cell function).
    pub chan: Vec<i64>,
    /// Active transient food-burst events (usually empty; see [`Bloom`]).
    pub blooms: Vec<Bloom>,
    pub orgs: Organisms,
    hash: SpatialHash,
}

struct PendingBirth {
    parent_id: u32,
    parent_genome: Genome,
    parent_brain: Brain,
    parent_regnet: RegNet,
    px: Scalar,
    py: Scalar,
}

impl World {
    pub fn new(seed: u64, params: WorldParams) -> Self {
        let cells = params.cell_count();
        let hash = SpatialHash::new(params.width, params.height, params.sense_radius);
        let blooms = Vec::new();
        let terrain = make_terrain(seed, &params);
        let elevation = make_elevation(seed, &params);
        let mut w = World {
            field: vec![params.field_cap; cells],
            field_b: vec![params.field_cap; cells],
            terrain,
            elevation,
            detritus: vec![0; cells],
            signal: vec![0; cells],
            chan: vec![0; N_CHAN * cells],
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
            let genome = Genome::founder(&mut rng);
            let brain = Brain::random_minimal(&mut rng);
            let regnet = RegNet::random_minimal(&mut rng);
            w.orgs.insert(
                seed,
                NewOrganism {
                    px,
                    py,
                    vx: 0.0,
                    vy: 0.0,
                    hx: 1.0,
                    hy: 0.0,
                    energy: w.params.initial_energy,
                    parent: u32::MAX,
                    birth_tick: 0,
                    genome,
                    brain,
                    regnet,
                },
            );
        }
        w
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        params: WorldParams,
        seed: u64,
        tick_count: u64,
        field: Vec<i64>,
        field_b: Vec<i64>,
        terrain: Vec<Scalar>,
        elevation: Vec<Scalar>,
        detritus: Vec<i64>,
        signal: Vec<i64>,
        chan: Vec<i64>,
        blooms: Vec<Bloom>,
        orgs: Organisms,
    ) -> Self {
        let hash = SpatialHash::new(params.width, params.height, params.sense_radius);
        World {
            params,
            seed,
            tick_count,
            field,
            field_b,
            terrain,
            elevation,
            detritus,
            signal,
            chan,
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

        // Food-burst events: maybe spawn a random one (off unless `bloom_event_rate > 0`).
        let (seed, tick_c, w_w, w_h) = (
            self.seed,
            self.tick_count,
            self.params.width,
            self.params.height,
        );
        let rate = self.params.bloom_event_rate;
        if rate > 0 {
            let mut r = Pcg32::from_key(seed, subsystem::BLOOM, splitmix64(tick_c));
            if (r.below(10_000) as i64) < rate {
                let nb = Bloom {
                    x: r.next_f32_unit() * w_w,
                    y: r.next_f32_unit() * w_h,
                    radius: self.params.bloom_radius,
                    boost: self.params.bloom_boost,
                    age: 0,
                    life: self.params.bloom_life.max(1),
                };
                self.blooms.push(nb);
            }
        }

        // Regrow the field: modulated by daylight and boosted inside active food-burst events.
        let daylight = self.daylight();
        let day_factor = 0.4 + daylight * 1.2;
        let cap = self.params.field_cap;
        let base = self.params.field_regrow;
        let gw = self.params.grid_w as usize;
        let gh = self.params.grid_h as usize;
        let cw = self.params.width / gw as f32;
        let ch = self.params.height / gh as f32;
        for cyi in 0..gh {
            let ccy = (cyi as f32 + 0.5) * ch;
            for cxi in 0..gw {
                let ccx = (cxi as f32 + 0.5) * cw;
                let idx = cyi * gw + cxi;
                let t = self.terrain[idx];
                let mut rg_a = (base as f32 * day_factor * t * 0.5) as i64;
                let mut rg_b = (base as f32 * day_factor * (2.05 - t) * 0.5) as i64;
                for b in &self.blooms {
                    let dx = ccx - b.x;
                    let dy = ccy - b.y;
                    if dx * dx + dy * dy <= b.radius * b.radius {
                        rg_a += b.boost;
                        rg_b += b.boost;
                        break;
                    }
                }
                let v = self.field[idx] + rg_a;
                self.field[idx] = if v > cap { cap } else { v };
                let vb = self.field_b[idx] + rg_b;
                self.field_b[idx] = if vb > cap { cap } else { vb };
            }
        }

        // Age food-burst events and drop the expired ones.
        for b in self.blooms.iter_mut() {
            b.age += 1;
        }
        self.blooms.retain(|b| b.age < b.life);

        // Decompose corpses slowly into soil food.
        let ddiv = self.params.decompose_div.max(1);
        for c in 0..self.field.len() {
            let dec = self.detritus[c] / ddiv;
            if dec > 0 {
                self.detritus[c] -= dec;
                let half = dec / 2;
                let va = self.field[c] + half;
                self.field[c] = if va > cap { cap } else { va };
                let vb = self.field_b[c] + (dec - half);
                self.field_b[c] = if vb > cap { cap } else { vb };
            }
        }

        // Decay the signal field.
        for s in self.signal.iter_mut() {
            *s = *s * 9 / 10;
        }

        // Diffuse + decay the chemical channels (inert in Stage 1, but kept resident + deterministic).
        let cells = gw * gh;
        let cddiv = self.params.chan_diffuse_div;
        if cddiv >= 4 {
            let mut buf = vec![0i64; cells];
            for k in 0..N_CHAN {
                let cbase = k * cells;
                for (idx, b) in buf.iter_mut().enumerate() {
                    *b = self.chan[cbase + idx] / cddiv;
                }
                for cy in 0..gh {
                    for cx in 0..gw {
                        let i = cy * gw + cx;
                        let xm = (cx + gw - 1) % gw;
                        let xp = (cx + 1) % gw;
                        let ym = (cy + gh - 1) % gh;
                        let yp = (cy + 1) % gh;
                        let inflow = buf[cy * gw + xm]
                            + buf[cy * gw + xp]
                            + buf[ym * gw + cx]
                            + buf[yp * gw + cx];
                        self.chan[cbase + i] = self.chan[cbase + i] - 4 * buf[i] + inflow;
                    }
                }
            }
        }
        let (cdn, cdd) = (
            self.params.chan_decay_num,
            self.params.chan_decay_den.max(1),
        );
        for c in self.chan.iter_mut() {
            *c = *c * cdn / cdd;
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
            self.step_body(i, tick, daylight, &mut scratch, &mut births, &mut events);
        }

        for (seq, b) in births.into_iter().enumerate() {
            let key = splitmix64(b.parent_id as u64)
                ^ splitmix64(tick.wrapping_mul(0x0100_0000_01b3))
                ^ splitmix64(seq as u64);
            let mut grng = Pcg32::from_key(self.seed, subsystem::GENOME, key);
            let mut brng = Pcg32::from_key(self.seed, subsystem::MUTATION, key);
            let mut rrng = Pcg32::from_key(self.seed, subsystem::DEVELOP, key);
            let mut genome = b.parent_genome;
            genome.mutate(&mut grng, &self.params);
            let mut brain = b.parent_brain;
            brain.mutate(
                &mut brng,
                self.params.mutation_rate,
                self.params.weight_mut_delta,
                self.params.add_conn_prob,
                self.params.add_node_prob,
            );
            brain.reset_state();
            let mut regnet = b.parent_regnet;
            regnet.mutate(
                &mut rrng,
                self.params.mutation_rate,
                self.params.weight_mut_delta,
                self.params.add_conn_prob,
                self.params.add_node_prob,
            );
            let dx = brng.next_f32_signed() * self.params.spawn_radius;
            let dy = brng.next_f32_signed() * self.params.spawn_radius;
            let px = wrap(b.px + dx, self.params.width);
            let py = wrap(b.py + dy, self.params.height);
            let (_, id) = self.orgs.insert(
                self.seed,
                NewOrganism {
                    px,
                    py,
                    vx: 0.0,
                    vy: 0.0,
                    hx: 1.0,
                    hy: 0.0,
                    energy: self.params.offspring_energy,
                    parent: b.parent_id,
                    birth_tick: tick,
                    genome,
                    brain,
                    regnet,
                },
            );
            events.events.push(Event::Birth {
                id,
                parent: b.parent_id,
                tick,
            });
        }

        self.tick_count += 1;
        events
    }

    fn step_body(
        &mut self,
        i: usize,
        _tick: u64,
        daylight: f32,
        scratch: &mut Vec<Scalar>,
        births: &mut Vec<PendingBirth>,
        events: &mut EventBatch,
    ) {
        let p = &self.params;
        let cap = p.field_cap as f32;
        let (gw, gh) = (p.grid_w as i32, p.grid_h as i32);
        let (cw, ch) = (p.width / gw as f32, p.height / gh as f32);
        let sx = self.orgs.px[i];
        let sy = self.orgs.py[i];
        let metab = self.orgs.g_metab[i];
        let mass_units = {
            let m = self.orgs.mass_milli[i] as f32 / 1000.0;
            if m < 0.4 {
                0.4
            } else {
                m
            }
        };

        // --- aggregate sense (the whole body reads its centre) ---
        let (cx, cy) = ((sx / cw) as i32, (sy / ch) as i32);
        let here = self.fidx(cx, cy);
        let food = self.field[here] as f32 / cap;
        let east = self.field[self.fidx(cx + 1, cy)] as f32;
        let west = self.field[self.fidx(cx - 1, cy)] as f32;
        let north = self.field[self.fidx(cx, cy - 1)] as f32;
        let south = self.field[self.fidx(cx, cy + 1)] as f32;
        let grad_x = (east - west) / cap;
        let grad_y = (south - north) / cap;
        let energy_in = (self.orgs.energy[i] as f32 / p.repro_threshold as f32).min(2.0) - 1.0;
        let elev_in = self.elevation[here] * 2.0 - 1.0;

        let nn = self.hash.nearest(&self.orgs, i, sx, sy, p.sense_radius);
        let (nn_dx, nn_dy, nn_relsize) = match nn {
            Some(j) => (
                clamp_abs((self.orgs.px[j] - sx) / p.sense_radius, 1.0),
                clamp_abs((self.orgs.py[j] - sy) / p.sense_radius, 1.0),
                clamp_abs(
                    (self.orgs.mass_milli[j] - self.orgs.mass_milli[i]) as f32 / 8000.0,
                    1.0,
                ),
            ),
            None => (0.0, 0.0, 0.0),
        };

        let mut inputs = [0.0f32; brain::N_IN];
        inputs[brain::IN_BIAS] = 1.0;
        inputs[brain::IN_FOOD] = food;
        inputs[brain::IN_GRAD_X] = grad_x;
        inputs[brain::IN_GRAD_Y] = grad_y;
        inputs[brain::IN_ENERGY] = energy_in;
        inputs[brain::IN_NN_DX] = nn_dx;
        inputs[brain::IN_NN_DY] = nn_dy;
        inputs[brain::IN_NN_RELSIZE] = nn_relsize;
        inputs[brain::IN_DAYLIGHT] = daylight * 2.0 - 1.0;
        inputs[brain::IN_ELEVATION] = elev_in;

        // --- think ---
        let out = self.orgs.brains[i].forward(&inputs, scratch);

        // --- move whole body (accel scaled by total mass) ---
        let accel = p.accel_scale / mass_units;
        let vx = clamp_abs(
            self.orgs.vx[i] * 0.85 + out[brain::OUT_AX] * accel,
            p.max_speed,
        );
        let vy = clamp_abs(
            self.orgs.vy[i] * 0.85 + out[brain::OUT_AY] * accel,
            p.max_speed,
        );
        self.orgs.vx[i] = vx;
        self.orgs.vy[i] = vy;
        let nx = wrap(sx + vx, p.width);
        let ny = wrap(sy + vy, p.height);
        self.orgs.px[i] = nx;
        self.orgs.py[i] = ny;
        // update heading from velocity (guarded normalization)
        let sp2 = vx * vx + vy * vy;
        if sp2 > HEADING_EPS {
            let inv = 1.0 / sp2.sqrt();
            self.orgs.hx[i] = vx * inv;
            self.orgs.hy[i] = vy * inv;
        }
        let (hx, hy) = (self.orgs.hx[i], self.orgs.hy[i]);

        // --- feed (per cell): each cell harvests food from the grid cell under its footprint,
        // scaled by its feeding role; the whole body shares one pool. Cells visited in canonical
        // order; all sums are i64 (exact, order-free). ---
        let eat_rate = p.eat_rate as f32;
        let mut intake: i64 = 0;
        let mut struct_load: i64 = 0;
        let n_cells = self.orgs.bodies[i].cells.len() as i64;
        {
            let cells_ptr = &self.orgs.bodies[i];
            for c in &cells_ptr.cells {
                let feed01 = c.role_feed as f32 / 1000.0;
                let struct01 = c.role_struct as f32 / 1000.0;
                struct_load += (struct01 * STRUCT_UPKEEP_FULL as f32) as i64;
                let lu = c.lu as f32;
                let lv = c.lv as f32;
                let wx = nx + (lu * hx - lv * hy) * SPACING;
                let wy = ny + (lu * hy + lv * hx) * SPACING;
                let fx = ((wx / cw) as i32).rem_euclid(gw);
                let fy = ((wy / ch) as i32).rem_euclid(gh);
                let fi = (fy * gw + fx) as usize;
                let want = (eat_rate * metab * (FEED_BASE + FEED_GAIN * feed01)) as i64;
                if want > 0 {
                    let avail = self.field[fi];
                    let got = if want < avail { want } else { avail };
                    if got > 0 {
                        self.field[fi] -= got;
                        intake += got;
                    }
                }
            }
        }
        self.orgs.energy[i] += intake;

        // --- predation: a bigger body eats a smaller one it is touching. Gape-limited by total
        // mass, with NO predator/prey flag — who eats whom falls out of evolved body size. This is
        // the pressure under which being multicellular (hence bigger, hence hard to eat and able to
        // eat) can pay, so multicellularity can EMERGE from single cells instead of being given. ---
        if let Some(j) = nn {
            if self.orgs.alive[j] {
                let mi = self.orgs.mass_milli[i];
                let mj = self.orgs.mass_milli[j];
                if (mi as f32) >= (mj as f32) * p.predation_size_ratio {
                    let dxp = self.orgs.px[j] - nx;
                    let dyp = self.orgs.py[j] - ny;
                    // reach grows with the two bodies' sizes (a big mouth touching a small prey)
                    let reach = p.contact_radius + (mi + mj) as f32 / 1000.0 * 0.6;
                    if dxp * dxp + dyp * dyp <= reach * reach {
                        let prey_e = self.orgs.energy[j];
                        let steal = if p.bite_amount < prey_e {
                            p.bite_amount
                        } else {
                            prey_e
                        };
                        if steal > 0 {
                            self.orgs.energy[j] -= steal;
                            self.orgs.energy[i] +=
                                steal * p.predation_gain_num / p.predation_gain_den;
                            if self.orgs.energy[j] <= 0 {
                                let fx = ((self.orgs.px[j] / cw) as i32).rem_euclid(gw);
                                let fy = ((self.orgs.py[j] / ch) as i32).rem_euclid(gh);
                                let fj = (fy * gw + fx) as usize;
                                // deposit the prey's corpse (inlined so no &mut self method call
                                // conflicts with the `p` borrow held across this function)
                                let capd = p.field_cap * 4;
                                let dep = p.death_deposit
                                    + ((mj as f32 / 1000.0) * p.corpse_size_factor as f32) as i64;
                                let v = self.detritus[fj] + dep;
                                self.detritus[fj] = if v > capd { capd } else { v };
                                events.events.push(Event::Death {
                                    id: self.orgs.id[j],
                                    cause: DeathCause::Predated,
                                    tick: _tick,
                                });
                                self.orgs.kill(j);
                            }
                        }
                    }
                }
            }
        }

        // --- upkeep (per cell + whole-body) ---
        let move_cost = (sp2 * (p.move_cost_coeff as f32) * mass_units) as i64;
        let size_cost = (mass_units * p.size_upkeep as f32) as i64;
        let basal = n_cells * p.basal_upkeep;
        let brain_cost = (self.orgs.brains[i].complexity() as i64 - 18).max(0) * p.brain_cost;
        let regnet_cost = (self.orgs.regnets[i].complexity() as i64) / 4 * p.brain_cost;
        let crowd = self
            .hash
            .count_within(&self.orgs, i, sx, sy, p.crowd_radius);
        let crowd_cost = crowd as i64 * p.crowd_cost;
        self.orgs.energy[i] -=
            basal + size_cost + move_cost + brain_cost + regnet_cost + crowd_cost + struct_load;
        self.orgs.age[i] += 1;

        // --- death (the whole body dies at once) ---
        if self.orgs.energy[i] <= 0 {
            self.deposit_corpse(here, mass_units);
            events.events.push(Event::Death {
                id: self.orgs.id[i],
                cause: DeathCause::Starved,
                tick: _tick,
            });
            self.orgs.kill(i);
            return;
        }
        if self.orgs.age[i] >= p.max_age {
            self.deposit_corpse(here, mass_units);
            events.events.push(Event::Death {
                id: self.orgs.id[i],
                cause: DeathCause::OldAge,
                tick: _tick,
            });
            self.orgs.kill(i);
            return;
        }

        // --- reproduce (clonal: shed one mutated seed that develops into a new body) ---
        let threshold = ((p.repro_threshold as f32) * self.orgs.g_repro[i]) as i64;
        let wants_repro = out[brain::OUT_REPRO] > -0.2;
        if wants_repro && self.orgs.energy[i] >= threshold && self.orgs.count < p.max_population {
            self.orgs.energy[i] -= p.repro_cost + p.offspring_energy;
            births.push(PendingBirth {
                parent_id: self.orgs.id[i],
                parent_genome: self.orgs.genome_at(i),
                parent_brain: self.orgs.brains[i].clone(),
                parent_regnet: self.orgs.regnets[i].clone(),
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
                let genome = Genome::founder(&mut rng);
                let brain = Brain::random_minimal(&mut rng);
                let regnet = RegNet::random_minimal(&mut rng);
                self.orgs.insert(
                    self.seed,
                    NewOrganism {
                        px,
                        py,
                        vx: 0.0,
                        vy: 0.0,
                        hx: 1.0,
                        hy: 0.0,
                        energy: *energy,
                        parent: u32::MAX,
                        birth_tick: self.tick_count,
                        genome,
                        brain,
                        regnet,
                    },
                );
            }
            CommandKind::Bloom { cx, cy } => {
                let cw = self.params.width / self.params.grid_w as f32;
                let ch = self.params.height / self.params.grid_h as f32;
                let x = ((*cx).rem_euclid(self.params.grid_w as i32) as f32 + 0.5) * cw;
                let y = ((*cy).rem_euclid(self.params.grid_h as i32) as f32 + 0.5) * ch;
                self.blooms.push(Bloom {
                    x,
                    y,
                    radius: self.params.bloom_radius,
                    boost: self.params.bloom_boost,
                    age: 0,
                    life: self.params.bloom_life.max(1),
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

    /// A 64-bit fingerprint of the entire world state — fields, blooms, and every organism
    /// (genome, brain, developmental net, and a digest of its developed body), hashed in id order.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv::new();
        h.u64(self.tick_count);
        h.u64(self.seed);
        h.u32(self.field.len() as u32);
        for &c in &self.field {
            h.i64(c);
        }
        for &c in &self.field_b {
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
        for &c in &self.chan {
            h.i64(c);
        }
        for b in &self.blooms {
            h.u32(canonical_bits(b.x));
            h.u32(canonical_bits(b.y));
            h.u32(canonical_bits(b.radius));
            h.i64(b.boost);
            h.u32(b.age);
            h.u32(b.life);
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
            h.u32(canonical_bits(o.hx[i]));
            h.u32(canonical_bits(o.hy[i]));
            h.i64(o.energy[i]);
            h.u32(o.age[i]);
            h.u32(o.parent[i]);
            h.u64(o.birth_tick[i]);
            h.u32(canonical_bits(o.g_metab[i]));
            h.u32(canonical_bits(o.g_repro[i]));
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
            let rn = &o.regnets[i];
            h.u32(rn.n_hidden as u32);
            h.u32(rn.conns.len() as u32);
            for c in &rn.conns {
                h.u32(c.from);
                h.u32(c.to);
                h.u32(canonical_bits(c.w));
                h.u8(c.enabled as u8);
            }
            for &x in &rn.bias {
                h.u32(canonical_bits(x));
            }
            // Body digest (derived, cheap): catches any nondeterminism inside development directly.
            let body = &o.bodies[i];
            h.u32(body.cells.len() as u32);
            for cell in &body.cells {
                h.i16(cell.lu);
                h.i16(cell.lv);
                h.u32(cell.size_milli as u32);
                h.i16(cell.role_feed);
                h.i16(cell.role_struct);
            }
            // The raw genome is authoritative state.
            let gm = &o.genomes[i];
            h.u32(gm.genes.len() as u32);
            for gene in &gm.genes {
                h.u64(gene.id);
                hash_gene_kind(&mut h, gene.kind);
            }
        }
        h.finish()
    }
}

/// Generate a static fertility landscape from the seed. Deterministic (only `+ - * / sqrt`).
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

/// Generate a static elevation map (0 = deep water .. 1 = high land). Deterministic (`+ - * / sqrt`).
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
            let mut hgt = base;
            for &(sx, sy) in &seas {
                let dx = px - sx;
                let dy = py - sy;
                let d = (dx * dx + dy * dy).sqrt();
                if d < sea_r {
                    hgt -= 0.85 * (1.0 - d / sea_r);
                }
            }
            for &(mx, my) in &peaks {
                let dx = px - mx;
                let dy = py - my;
                let d = (dx * dx + dy * dy).sqrt();
                if d < peak_r {
                    hgt += 0.4 * (1.0 - d / peak_r);
                }
            }
            e[(cy * gw + cx) as usize] = hgt.clamp(0.0, 1.0);
        }
    }
    e
}

/// Hash a gene's kind (tag + payload) into the state fingerprint, canonical little-endian.
fn hash_gene_kind(h: &mut Fnv, k: GeneKind) {
    match k {
        GeneKind::TraitMod { channel, amount } => {
            h.u8(0);
            h.u8(channel);
            h.i16(amount);
        }
        GeneKind::Junk { payload } => {
            h.u8(1);
            h.u32(payload);
        }
        GeneKind::Emit { channel, rate } => {
            h.u8(2);
            h.u8(channel);
            h.i16(rate);
        }
        GeneKind::Sense { channel, gain } => {
            h.u8(3);
            h.u8(channel);
            h.i16(gain);
        }
        GeneKind::React { param, threshold } => {
            h.u8(4);
            h.u16(param);
            h.i16(threshold);
        }
        GeneKind::Uptake { channel, eff } => {
            h.u8(5);
            h.u8(channel);
            h.i16(eff);
        }
        GeneKind::Resist { channel } => {
            h.u8(6);
            h.u8(channel);
        }
    }
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
    fn u16(&mut self, v: u16) {
        for b in v.to_le_bytes() {
            self.u8(b);
        }
    }
    #[inline]
    fn i16(&mut self, v: i16) {
        for b in v.to_le_bytes() {
            self.u8(b);
        }
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
        assert_eq!(run(12345, 200).state_hash(), run(12345, 200).state_hash());
    }

    #[test]
    fn different_seed_diverges() {
        assert_ne!(run(1, 200).state_hash(), run(2, 200).state_hash());
    }

    #[test]
    fn world_is_alive() {
        let mut w = World::new(777, WorldParams::default());
        for _ in 0..300 {
            w.tick(&[]);
        }
        assert!(w.population() > 0, "world went extinct");
    }
}
