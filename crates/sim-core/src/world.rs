//! The world and its tick — the one hot function the whole engine is built around.
//!
//! A **living, pressuring** world: food waxes and wanes on a day/night–season cycle and can
//! surge in transient **food-burst events** (see [`Bloom`]); predation is **skill-based** (a
//! predator only catches prey it is actually pursuing, so prey can evade); and organisms can
//! **emit and sense a signal** whose meaning evolution assigns. Every organism has a growing
//! recurrent brain ([`crate::brain`]). Nothing is hard-coded to a role or outcome — foraging,
//! fleeing, hunting, timing, and signalling all emerge (or don't) under these pressures.
//!
//! Fixed phase order per tick: 1. commands, 2. spawn/age food-burst events + regrow fields +
//! decay signal, 3. rebuild spatial hash, 4. organisms act (index order), 5. apply births.

use crate::brain::{self, Brain, N_CHAN, OUT_AX, OUT_AY};
use crate::command::{Command, CommandKind};
use crate::environment::SpatialHash;
use crate::event::{DeathCause, Event, EventBatch};
use crate::genome::{GeneKind, Genome};
use crate::math::{canonical_bits, clamp_abs, wrap, Scalar};
use crate::organism::{NewOrganism, Organisms};
use crate::params::WorldParams;
use crate::rng::{splitmix64, subsystem, Pcg32};

/// A transient **food-burst event**: a disc where food regrows faster, for a limited time.
///
/// Unlike the old permanent drifting oases, a bloom appears (from a rare random spawn or the
/// user's brush), boosts regrowth for `life` ticks, then vanishes — the food it grew is left to
/// be eaten down (the boom, then the bust). It only ever perturbs the *environment*: who
/// exploits the surge, and how, is left entirely to selection (no scripted "scavenger").
#[derive(Clone, Debug)]
pub struct Bloom {
    pub x: Scalar,
    pub y: Scalar,
    pub radius: Scalar,
    pub boost: i64,
    /// Ticks elapsed since the event began.
    pub age: u32,
    /// Total lifetime in ticks; the event is removed once `age >= life`.
    pub life: u32,
}

#[derive(Clone, Debug)]
pub struct World {
    pub params: WorldParams,
    pub seed: u64,
    pub tick_count: u64,
    /// Food field A (the "green" resource). Regrows where terrain is fertile.
    pub field: Vec<i64>,
    /// Food field B (the "amber" resource), anti-correlated with A: rich where A is poor. Two
    /// non-fungible foods, each digested by a different `diet` gene, so the population splits
    /// into dietary niches instead of one monoculture on a single resource.
    pub field_b: Vec<i64>,
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
    /// Generic chemical channels (M2): `N_CHAN` integer fields flattened as `k*cells + idx`.
    /// Their meaning (food / toxin / signal) is assigned by evolution, not by us.
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
    px: Scalar,
    py: Scalar,
}

impl World {
    pub fn new(seed: u64, params: WorldParams) -> Self {
        // Chemistry conservation guard: the emit→uptake loop's per-hop gain must be < 1, else a
        // channel pins at its cap and the population booms (see `step_organism`). Emit at max
        // strength adds `255/chan_emit_den` per unit intake; uptake returns `num/den` of it.
        debug_assert!(
            255 * params.chan_uptake_num < params.chan_emit_den * params.chan_uptake_den,
            "chemistry loop gain must be < 1: 255·chan_uptake_num < chan_emit_den·chan_uptake_den"
        );
        let cells = params.cell_count();
        let hash = SpatialHash::new(params.width, params.height, params.sense_radius);
        // Food-burst events start empty: the world is calm until one is triggered (randomly, if
        // `bloom_event_rate > 0`, or by the user).
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

    /// Effective edible value at a cell for an organism with digestion efficiencies `(ea, eb)`
    /// on the two food fields, plus scavengeable detritus (diet-neutral).
    #[inline]
    fn eff_food_at(&self, cx: i32, cy: i32, ea: Scalar, eb: Scalar) -> Scalar {
        let i = self.fidx(cx, cy);
        self.field[i] as f32 * ea + self.field_b[i] as f32 * eb + self.detritus[i] as f32
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

        // Food-burst events: maybe spawn a random one (off unless `bloom_event_rate > 0`), keyed
        // by the tick alone so it is reproducible and independent of any organism. User-placed
        // bursts arrived above via the command phase. Both kinds are aged and expired below.
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
        let day_factor = 0.4 + daylight * 1.2; // [0.4, 1.6]
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
                // Two foods, half the budget each and anti-correlated: A follows fertility, B is
                // rich exactly where A is poor. Blooms are lush green patches, so they boost A.
                let mut rg_a = (base as f32 * day_factor * t * 0.5) as i64;
                let mut rg_b = (base as f32 * day_factor * (2.05 - t) * 0.5) as i64;
                for b in &self.blooms {
                    let dx = ccx - b.x;
                    let dy = ccy - b.y;
                    if dx * dx + dy * dy <= b.radius * b.radius {
                        // a burst is a lush oasis for both foods — don't privilege A
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

        // Age food-burst events and drop the expired ones. `retain` keeps insertion order, so
        // remaining events stay in a canonical, cross-target-deterministic sequence.
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
                // corpses enrich both soils, so scavenging doesn't privilege the food-A niche
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

        // Diffuse + decay the chemical channels (M2). Integer, mass-conserving (double-buffered),
        // toroidal — so a substance one lineage emits spreads and settles before others sense it.
        // Each cell donates `conc/ddiv` to EACH of 4 neighbours, so `ddiv` must be >= 4 or a cell
        // over-donates and goes negative; `< 4` means diffusion is off.
        let cells = gw * gh;
        let ddiv = self.params.chan_diffuse_div;
        if ddiv >= 4 {
            let mut buf = vec![0i64; cells];
            for k in 0..N_CHAN {
                let base = k * cells;
                for (idx, b) in buf.iter_mut().enumerate() {
                    *b = self.chan[base + idx] / ddiv;
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
                        self.chan[base + i] = self.chan[base + i] - 4 * buf[i] + inflow;
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
            self.step_organism(i, tick, daylight, &mut scratch, &mut births, &mut events);
        }

        for (seq, b) in births.into_iter().enumerate() {
            let key = splitmix64(b.parent_id as u64)
                ^ splitmix64(tick.wrapping_mul(0x0100_0000_01b3))
                ^ splitmix64(seq as u64);
            // Separate streams for genome and brain, so adding gene draws (M2) never perturbs
            // brain evolution.
            let mut grng = Pcg32::from_key(self.seed, subsystem::GENOME, key);
            let mut brng = Pcg32::from_key(self.seed, subsystem::MUTATION, key);
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
            let dx = brng.next_f32_signed() * self.params.spawn_radius;
            let dy = brng.next_f32_signed() * self.params.spawn_radius;
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
        let cells = p.cell_count();
        let sx = self.orgs.px[i];
        let sy = self.orgs.py[i];
        let size = self.orgs.g_size[i];

        // --- sense (food is what THIS organism can actually digest, per its diet gene) ---
        // Convex trade-off (squared): a specialist eats its food at full rate while a generalist
        // is only ~¼ on each, so specialising strictly beats hedging — two diets, not one mush.
        let diet = self.orgs.g_diet[i];
        let a = 1.0 - diet;
        let (ea, eb) = (a * a, diet * diet);
        let (cx, cy) = self.cell_of(sx, sy);
        let here = self.fidx(cx, cy);
        let food = self.eff_food_at(cx, cy, ea, eb) / cap;
        let east = self.eff_food_at(cx + 1, cy, ea, eb);
        let west = self.eff_food_at(cx - 1, cy, ea, eb);
        let north = self.eff_food_at(cx, cy - 1, ea, eb);
        let south = self.eff_food_at(cx, cy + 1, ea, eb);
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
        inputs[brain::IN_SIGNAL] = signal_in;
        inputs[brain::IN_ELEVATION] = elev_in;
        // Chemical senses: a channel slot carries information only where a Sense gene made it live.
        let sm = self.orgs.sense_mask[i];
        if sm != 0 {
            let sense_cap = p.chan_sense_cap.max(1) as f32;
            for k in 0..N_CHAN {
                if sm & (1 << k) != 0 {
                    let c = self.chan[k * cells + here];
                    inputs[brain::IN_CHAN0 + k] = (c as f32 / sense_cap).min(1.0);
                }
            }
        }

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

        // --- eat both foods, each scaled by digestion efficiency (the diet trade-off) ---
        let (ncx, ncy) = self.cell_of(self.orgs.px[i], self.orgs.py[i]);
        let fi = self.fidx(ncx, ncy);
        let want = ((p.eat_rate as f32) * self.orgs.g_metab[i]) as i64;
        let got_a = ((want as f32 * ea) as i64).min(self.field[fi]).max(0);
        self.field[fi] -= got_a;
        let got_b = ((want as f32 * eb) as i64).min(self.field_b[fi]).max(0);
        self.field_b[fi] -= got_b;
        // Scavenging corpses is slower than grazing (so carcasses linger) and diet-neutral.
        let scav = ((want - got_a - got_b) / 2).max(0);
        let from_det = scav.min(self.detritus[fi]);
        self.detritus[fi] -= from_det;
        self.orgs.energy[i] += got_a + got_b + from_det;

        // --- chemistry: absorb channels this lineage digests ---
        // Emit (below) is a FREE byproduct — necessary so cross-feeding can bootstrap (a costed
        // emit would be pure altruism and get selected away). Uptake is lossy, which *bounds* the
        // emit→uptake loop rather than forbidding it: the per-hop gain
        // `(255/chan_emit_den)·(chan_uptake_num/chan_uptake_den)` is 1/3 at defaults (< 1), so the
        // geometric series converges to ~1.5× the underlying food energy (a modelled
        // residual-energy-of-waste input, not strict conservation). The `< 1` invariant is asserted
        // in `World::new`; break it and a channel pins at cap and the population booms.
        let up = self.orgs.uptake_ch[i];
        let mut chan_gain = 0i64;
        for (k, &uk) in up.iter().enumerate() {
            let u = uk as i64;
            if u > 0 {
                let ci = k * cells + fi;
                let want_c = p.chan_uptake_rate * u / 255;
                let got = want_c.min(self.chan[ci]).max(0);
                self.chan[ci] -= got;
                chan_gain += got * p.chan_uptake_num / p.chan_uptake_den;
            }
        }
        self.orgs.energy[i] += chan_gain;

        // --- chemistry: excrete byproducts of intake into the channels ---
        let em = self.orgs.emit_ch[i];
        let intake = got_a + got_b + from_det + chan_gain;
        for (k, &ek) in em.iter().enumerate() {
            let e = ek as i64;
            if e > 0 {
                let amt = p.chan_emit_base + intake * e / p.chan_emit_den;
                let ci = k * cells + fi;
                let v = self.chan[ci] + amt;
                self.chan[ci] = if v > p.chan_cap { p.chan_cap } else { v };
            }
        }

        // --- emit signal ---
        let emit = out[brain::OUT_EMIT];
        if emit > 0.0 {
            let v = self.signal[fi] + (emit * p.emit_scale as f32) as i64;
            self.signal[fi] = if v > p.signal_cap { p.signal_cap } else { v };
        }

        // --- predation: bite a smaller neighbour in contact. A full bite when actually pursuing;
        // a small innate nibble on incidental contact (so a proto-predator can bootstrap). ---
        if let Some(j) = nn {
            if self.orgs.alive[j] && size >= self.orgs.g_size[j] * p.predation_size_ratio {
                let dxp = self.orgs.px[j] - self.orgs.px[i];
                let dyp = self.orgs.py[j] - self.orgs.py[i];
                if dxp * dxp + dyp * dyp <= p.contact_radius * p.contact_radius {
                    let pursuing = dxp * vx + dyp * vy > 0.0; // moving toward the prey?
                    let (bite, carn_gain) = if pursuing {
                        (p.bite_amount, 0.3)
                    } else {
                        (p.innate_bite, 0.05)
                    };
                    let victim = self.orgs.energy[j];
                    if victim > 0 && bite > 0 {
                        let steal = bite.min(victim);
                        self.orgs.energy[j] -= steal;
                        let gain = steal * p.predation_gain_num / p.predation_gain_den;
                        self.orgs.energy[i] += gain;
                        self.orgs.carnivory[i] = (self.orgs.carnivory[i] + carn_gain).min(1.0);
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
        // density-dependent competition: crowding drains energy, so a niche fills to a carrying
        // capacity instead of the whole grid packing to a uniform carpet.
        let crowd = self
            .hash
            .count_within(&self.orgs, i, sx, sy, p.crowd_radius);
        let crowd_cost = crowd as i64 * p.crowd_cost;
        // chemistry costs: toxicity of non-resisted channels above threshold, plus upkeep for the
        // uptake and resistance machinery this lineage carries.
        let rm = self.orgs.resist_mask[i];
        let mut toxin_cost = 0i64;
        for k in 0..N_CHAN {
            let c = self.chan[k * cells + fi];
            if c > p.chan_toxic_threshold && rm & (1 << k) == 0 {
                toxin_cost += (c - p.chan_toxic_threshold) * p.chan_toxin_num / p.chan_toxin_den;
            }
        }
        let uptake_upkeep = up.iter().filter(|&&x| x > 0).count() as i64 * p.chan_uptake_upkeep;
        let resist_upkeep = rm.count_ones() as i64 * p.chan_resist_upkeep;
        self.orgs.energy[i] -= p.basal_upkeep
            + brain_cost
            + size_cost
            + move_cost
            + habitat_cost
            + crowd_cost
            + toxin_cost
            + uptake_upkeep
            + resist_upkeep;
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
                let genome = Genome::founder(&mut rng);
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
            h.i64(o.energy[i]);
            h.u32(o.age[i]);
            h.u32(o.parent[i]);
            h.u64(o.birth_tick[i]);
            h.u32(canonical_bits(o.g_size[i]));
            h.u32(canonical_bits(o.g_metab[i]));
            h.u32(canonical_bits(o.g_repro[i]));
            h.u32(canonical_bits(o.g_habitat[i]));
            h.u32(canonical_bits(o.g_diet[i]));
            h.u8(o.cr[i]);
            h.u8(o.cg[i]);
            h.u8(o.cb[i]);
            h.u32(canonical_bits(o.carnivory[i]));
            h.u16(o.sense_mask[i]);
            h.u16(o.resist_mask[i]);
            for &e in &o.emit_ch[i] {
                h.u8(e);
            }
            for &u in &o.uptake_ch[i] {
                h.u8(u);
            }
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
            // The raw genome is authoritative state: distinct gene lists can share a phenotype
            // yet mutate differently, so hash the genes themselves (id + kind), in list order.
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
