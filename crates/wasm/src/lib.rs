//! Browser host for `sim-core` via `wasm-bindgen`.
//!
//! A thin wrapper exposing the deterministic engine to JavaScript: advance the world, read
//! per-cell render buffers and body diagnostics, and issue commands. All simulation logic lives
//! in `sim-core`; this crate only marshals data. The render unit is a **cell** (a body is many
//! cells), so the `*_cells` buffers below are all in the same canonical cell order.

use sim_core::world::SPACING;
use sim_core::{presets, Command, CommandKind, DeathCause, Event, ParamId, World, WorldParams};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Sim {
    world: World,
    pending: Vec<Command>,
    /// Deaths by cause over the most recent `tick(n)` call: [starved, old age, killed, eaten].
    last_deaths: [u32; 4],
}

fn accumulate(batch: &sim_core::EventBatch, d: &mut [u32; 4]) {
    for e in &batch.events {
        if let Event::Death { cause, .. } = e {
            match cause {
                DeathCause::Starved => d[0] += 1,
                DeathCause::OldAge => d[1] += 1,
                DeathCause::Killed => d[2] += 1,
                DeathCause::Predated => d[3] += 1,
            }
        }
    }
}

/// World position of a body cell, mapped through the organism's centre + heading.
#[inline]
fn cell_world(px: f32, py: f32, hx: f32, hy: f32, lu: f32, lv: f32) -> (f32, f32) {
    (
        px + (lu * hx - lv * hy) * SPACING,
        py + (lu * hy + lv * hx) * SPACING,
    )
}

#[wasm_bindgen]
impl Sim {
    #[wasm_bindgen(constructor)]
    pub fn new(seed: u32) -> Sim {
        console_error_panic_hook::set_once();
        Sim {
            world: World::new(seed as u64, WorldParams::default()),
            pending: Vec::new(),
            last_deaths: [0; 4],
        }
    }

    pub fn tick(&mut self, n: u32) {
        if n == 0 {
            return;
        }
        let mut d = [0u32; 4];
        let cmds = std::mem::take(&mut self.pending);
        accumulate(&self.world.tick(&cmds), &mut d);
        for _ in 1..n {
            accumulate(&self.world.tick(&[]), &mut d);
        }
        self.last_deaths = d;
    }

    // --- world readouts ---------------------------------------------------

    pub fn population(&self) -> u32 {
        self.world.population()
    }
    pub fn tick_count(&self) -> u32 {
        self.world.tick_count as u32
    }
    pub fn world_w(&self) -> f32 {
        self.world.params.width
    }
    pub fn world_h(&self) -> f32 {
        self.world.params.height
    }
    pub fn grid_w(&self) -> u32 {
        self.world.params.grid_w
    }
    pub fn grid_h(&self) -> u32 {
        self.world.params.grid_h
    }
    pub fn state_hash(&self) -> String {
        format!("{:016x}", self.world.state_hash())
    }
    pub fn daylight(&self) -> f32 {
        self.world.daylight()
    }
    pub fn water_level(&self) -> f32 {
        self.world.params.water_level
    }

    /// Food field normalized to `0..=255`, row-major.
    pub fn field(&self) -> Vec<u8> {
        let cap = self.world.params.field_cap.max(1);
        self.world
            .field
            .iter()
            .map(|&c| ((c.max(0) * 255) / cap) as u8)
            .collect()
    }

    /// Static terrain fertility normalized to `0..=255`, row-major.
    pub fn terrain(&self) -> Vec<u8> {
        self.world
            .terrain
            .iter()
            .map(|&t| (((t - 0.15) / 1.75) * 255.0).clamp(0.0, 255.0) as u8)
            .collect()
    }

    /// Static elevation normalized to `0..=255` (0 = deep water, 255 = high land), row-major.
    pub fn elevation(&self) -> Vec<u8> {
        self.world
            .elevation
            .iter()
            .map(|&e| (e * 255.0).clamp(0.0, 255.0) as u8)
            .collect()
    }

    /// Active food-burst events as `[x, y, radius, life_frac, ...]`.
    pub fn blooms(&self) -> Vec<f32> {
        let mut v = Vec::with_capacity(self.world.blooms.len() * 4);
        for b in &self.world.blooms {
            v.push(b.x);
            v.push(b.y);
            v.push(b.radius);
            let life = b.life.max(1) as f32;
            v.push((life - b.age as f32) / life);
        }
        v
    }

    // --- per-cell render buffers (all in the same canonical cell order) ----

    fn n_cells(&self) -> usize {
        let o = &self.world.orgs;
        let mut n = 0usize;
        for i in 0..o.capacity() {
            if o.alive[i] {
                n += o.bodies[i].cells.len();
            }
        }
        n
    }

    /// Every body cell's world position `[x0, y0, x1, y1, ...]`.
    pub fn cell_positions(&self) -> Vec<f32> {
        let o = &self.world.orgs;
        let mut v = Vec::with_capacity(self.n_cells() * 2);
        for i in 0..o.capacity() {
            if !o.alive[i] {
                continue;
            }
            let (px, py, hx, hy) = (o.px[i], o.py[i], o.hx[i], o.hy[i]);
            for c in &o.bodies[i].cells {
                let (wx, wy) = cell_world(px, py, hx, hy, c.lu as f32, c.lv as f32);
                v.push(wx);
                v.push(wy);
            }
        }
        v
    }

    /// Per-cell radius (from cell size), matching `cell_positions` order.
    pub fn cell_sizes(&self) -> Vec<f32> {
        let o = &self.world.orgs;
        let mut v = Vec::with_capacity(self.n_cells());
        for i in 0..o.capacity() {
            if !o.alive[i] {
                continue;
            }
            for c in &o.bodies[i].cells {
                v.push(c.size_milli as f32 / 1000.0);
            }
        }
        v
    }

    /// Per-cell lineage colour `[r,g,b, ...]`, matching `cell_positions` order.
    pub fn cell_colors(&self) -> Vec<u8> {
        let o = &self.world.orgs;
        let mut v = Vec::with_capacity(self.n_cells() * 3);
        for i in 0..o.capacity() {
            if !o.alive[i] {
                continue;
            }
            for c in &o.bodies[i].cells {
                v.push(c.cr);
                v.push(c.cg);
                v.push(c.cb);
            }
        }
        v
    }

    /// Per-cell role weights `[feed, struct, ...]` as bytes (0..255), matching `cell_positions`.
    pub fn cell_roles(&self) -> Vec<u8> {
        let o = &self.world.orgs;
        let mut v = Vec::with_capacity(self.n_cells() * 2);
        for i in 0..o.capacity() {
            if !o.alive[i] {
                continue;
            }
            for c in &o.bodies[i].cells {
                v.push((c.role_feed.clamp(0, 1000) as u32 * 255 / 1000) as u8);
                v.push((c.role_struct.clamp(0, 1000) as u32 * 255 / 1000) as u8);
            }
        }
        v
    }

    /// Per-cell owning-organism id, matching `cell_positions` order — lets JS group cells into
    /// bodies to draw a membrane around each organism.
    pub fn cell_body_ids(&self) -> Vec<u32> {
        let o = &self.world.orgs;
        let mut v = Vec::with_capacity(self.n_cells());
        for i in 0..o.capacity() {
            if !o.alive[i] {
                continue;
            }
            let id = o.id[i];
            for _ in &o.bodies[i].cells {
                v.push(id);
            }
        }
        v
    }

    // --- body diagnostics -------------------------------------------------

    pub fn deaths_recent(&self) -> Vec<u32> {
        self.last_deaths.to_vec()
    }

    /// Mean cells per body (1 = single cells).
    pub fn avg_body(&self) -> f32 {
        let o = &self.world.orgs;
        let (mut s, mut n) = (0u64, 0u32);
        for i in 0..o.capacity() {
            if o.alive[i] {
                s += o.bodies[i].cells.len() as u64;
                n += 1;
            }
        }
        if n == 0 {
            0.0
        } else {
            s as f32 / n as f32
        }
    }

    /// Biggest body (most cells) alive.
    pub fn max_body(&self) -> u32 {
        let o = &self.world.orgs;
        let mut mx = 0u32;
        for i in 0..o.capacity() {
            if o.alive[i] {
                let k = o.bodies[i].cells.len() as u32;
                if k > mx {
                    mx = k;
                }
            }
        }
        mx
    }

    /// Division-of-labor index over the population (0 = all cells identical, 1 = fully
    /// specialized): the "did a new organism-level property emerge" detector.
    pub fn dol(&self) -> f32 {
        let o = &self.world.orgs;
        let (mut sum, mut n) = (0.0f32, 0u32);
        for i in 0..o.capacity() {
            if !o.alive[i] {
                continue;
            }
            let cells = &o.bodies[i].cells;
            if cells.len() < 2 {
                continue;
            }
            let (mut d, mut pairs) = (0.0f32, 0u32);
            for a in 0..cells.len() {
                for b in (a + 1)..cells.len() {
                    let df = (cells[a].role_feed - cells[b].role_feed).abs() as f32;
                    let ds = (cells[a].role_struct - cells[b].role_struct).abs() as f32;
                    d += (df + ds) / 2000.0;
                    pairs += 1;
                }
            }
            if pairs > 0 {
                sum += d / pairs as f32;
                n += 1;
            }
        }
        if n == 0 {
            0.0
        } else {
            sum / n as f32
        }
    }

    /// Fraction of multi-cell bodies carrying >=2 distinct cell-types (roles quantized coarsely).
    pub fn diff_frac(&self) -> f32 {
        let o = &self.world.orgs;
        let (mut diff, mut n) = (0u32, 0u32);
        for i in 0..o.capacity() {
            if !o.alive[i] {
                continue;
            }
            let cells = &o.bodies[i].cells;
            if cells.len() < 2 {
                continue;
            }
            n += 1;
            let mut types = std::collections::BTreeSet::new();
            for c in cells {
                types.insert((c.role_feed / 250, c.role_struct / 250));
            }
            if types.len() >= 2 {
                diff += 1;
            }
        }
        if n == 0 {
            0.0
        } else {
            diff as f32 / n as f32
        }
    }

    /// Shannon colour diversity over bodies (0 = monoculture).
    pub fn diversity(&self) -> f32 {
        let o = &self.world.orgs;
        let mut bins = [0u32; 64];
        let mut n = 0u32;
        for i in 0..o.capacity() {
            if o.alive[i] {
                let cells = &o.bodies[i].cells;
                if cells.is_empty() {
                    continue;
                }
                let c0 = cells[0];
                let b = ((c0.cr as usize >> 6) << 4)
                    | ((c0.cg as usize >> 6) << 2)
                    | (c0.cb as usize >> 6);
                bins[b] += 1;
                n += 1;
            }
        }
        if n == 0 {
            return 0.0;
        }
        let nf = n as f32;
        let mut h = 0.0f32;
        for &c in &bins {
            if c > 0 {
                let p = c as f32 / nf;
                h -= p * p.ln();
            }
        }
        h
    }

    pub fn avg_brain(&self) -> f32 {
        self.avg_over(|o, i| o.brains[i].complexity() as f32)
    }
    pub fn avg_regnet(&self) -> f32 {
        self.avg_over(|o, i| o.regnets[i].complexity() as f32)
    }
    pub fn avg_genome_len(&self) -> f32 {
        self.avg_over(|o, i| o.genomes[i].genes.len() as f32)
    }
    pub fn avg_speed(&self) -> f32 {
        self.avg_over(|o, i| (o.vx[i] * o.vx[i] + o.vy[i] * o.vy[i]).sqrt())
    }

    /// Nearest organism to a world point, for the inspector. Returns
    /// `[cx, cy, id, energy, age, ncells, dol, feed_avg, struct_avg, brain, regnet, genes]`.
    pub fn nearest(&self, wx: f32, wy: f32) -> Vec<f32> {
        let o = &self.world.orgs;
        let mut best: i64 = -1;
        let mut best_d = f32::INFINITY;
        for i in 0..o.capacity() {
            if o.alive[i] {
                let dx = o.px[i] - wx;
                let dy = o.py[i] - wy;
                let d = dx * dx + dy * dy;
                if d < best_d {
                    best_d = d;
                    best = i as i64;
                }
            }
        }
        if best < 0 {
            return Vec::new();
        }
        self.body_info(best as usize)
    }

    /// Look up an organism by id (follow mode). Same layout as [`Self::nearest`].
    pub fn by_id(&self, id: u32) -> Vec<f32> {
        let o = &self.world.orgs;
        for i in 0..o.capacity() {
            if o.alive[i] && o.id[i] == id {
                return self.body_info(i);
            }
        }
        Vec::new()
    }

    // --- presets ----------------------------------------------------------

    pub fn preset_count() -> u32 {
        presets::COUNT
    }
    pub fn preset_name(id: u32) -> String {
        presets::name(id).to_string()
    }
    pub fn load_preset(&mut self, id: u32, seed: u32) {
        self.world = World::new(seed as u64, presets::preset(id));
        self.pending.clear();
        self.last_deaths = [0; 4];
    }

    // --- commands (the sole mutation channel) -----------------------------

    pub fn inject(&mut self, cx: i32, cy: i32, radius: i32, amount: i32) {
        self.pending
            .push(Command::local(CommandKind::InjectSubstance {
                cx,
                cy,
                radius,
                amount: amount as i64,
            }));
    }
    pub fn spawn(&mut self, cx: i32, cy: i32, energy: i32) {
        self.pending.push(Command::local(CommandKind::Spawn {
            cx,
            cy,
            energy: energy as i64,
        }));
    }
    pub fn kill(&mut self, cx0: i32, cy0: i32, cx1: i32, cy1: i32) {
        self.pending
            .push(Command::local(CommandKind::Kill { cx0, cy0, cx1, cy1 }));
    }
    pub fn bloom(&mut self, cx: i32, cy: i32) {
        self.pending
            .push(Command::local(CommandKind::Bloom { cx, cy }));
    }
    pub fn set_mutation_rate(&mut self, per_ten_thousand: i32) {
        self.set_param(ParamId::MutationRate, per_ten_thousand as i64);
    }
    pub fn set_field_regrow(&mut self, raw: i32) {
        self.set_param(ParamId::FieldRegrow, raw as i64);
    }
    pub fn set_eat_rate(&mut self, raw: i32) {
        self.set_param(ParamId::EatRate, raw as i64);
    }
    /// Predation bite strength — the pressure under which being big (multicellular) pays.
    pub fn set_bite_amount(&mut self, raw: i32) {
        self.set_param(ParamId::BiteAmount, raw as i64);
    }
    pub fn set_bloom_rate(&mut self, per_ten_thousand: i32) {
        self.set_param(ParamId::BloomEventRate, per_ten_thousand as i64);
    }
    pub fn reset(&mut self, seed: u32) {
        self.pending
            .push(Command::local(CommandKind::Reset { seed: seed as u64 }));
    }
}

impl Sim {
    fn set_param(&mut self, key: ParamId, raw: i64) {
        self.pending
            .push(Command::local(CommandKind::SetParam { key, raw }));
    }

    fn avg_over(&self, f: impl Fn(&sim_core::organism::Organisms, usize) -> f32) -> f32 {
        let o = &self.world.orgs;
        let (mut s, mut n) = (0.0f32, 0u32);
        for i in 0..o.capacity() {
            if o.alive[i] {
                s += f(o, i);
                n += 1;
            }
        }
        if n == 0 {
            0.0
        } else {
            s / n as f32
        }
    }

    fn body_info(&self, i: usize) -> Vec<f32> {
        let o = &self.world.orgs;
        let cells = &o.bodies[i].cells;
        let nc = cells.len().max(1) as f32;
        let (mut feed, mut strc) = (0.0f32, 0.0f32);
        for c in cells {
            feed += c.role_feed as f32 / 1000.0;
            strc += c.role_struct as f32 / 1000.0;
        }
        // per-body DOL
        let mut dol = 0.0f32;
        if cells.len() >= 2 {
            let (mut d, mut pairs) = (0.0f32, 0u32);
            for a in 0..cells.len() {
                for b in (a + 1)..cells.len() {
                    let df = (cells[a].role_feed - cells[b].role_feed).abs() as f32;
                    let ds = (cells[a].role_struct - cells[b].role_struct).abs() as f32;
                    d += (df + ds) / 2000.0;
                    pairs += 1;
                }
            }
            if pairs > 0 {
                dol = d / pairs as f32;
            }
        }
        vec![
            o.px[i],
            o.py[i],
            o.id[i] as f32,
            o.energy[i] as f32,
            o.age[i] as f32,
            cells.len() as f32,
            dol,
            feed / nc,
            strc / nc,
            o.brains[i].complexity() as f32,
            o.regnets[i].complexity() as f32,
            o.genomes[i].genes.len() as f32,
        ]
    }
}
