//! Browser host for `sim-core` via `wasm-bindgen`.
//!
//! A thin wrapper exposing the deterministic engine to JavaScript: advance the world, read
//! render buffers and diagnostics, and issue commands (the single mutation channel). All
//! simulation logic lives in `sim-core`; this crate only marshals data across the boundary.
//! Diagnostic readouts (diversity, fractions, averages) are display-only and never feed back
//! into the sim.

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

    // --- readouts ---------------------------------------------------------

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

    pub fn positions(&self) -> Vec<f32> {
        let o = &self.world.orgs;
        let mut v = Vec::with_capacity(self.world.population() as usize * 2);
        for i in 0..o.capacity() {
            if o.alive[i] {
                v.push(o.px[i]);
                v.push(o.py[i]);
            }
        }
        v
    }

    pub fn colors(&self) -> Vec<u8> {
        let o = &self.world.orgs;
        let mut v = Vec::with_capacity(self.world.population() as usize * 3);
        for i in 0..o.capacity() {
            if o.alive[i] {
                v.push(o.cr[i]);
                v.push(o.cg[i]);
                v.push(o.cb[i]);
            }
        }
        v
    }

    pub fn sizes(&self) -> Vec<f32> {
        let o = &self.world.orgs;
        let mut v = Vec::with_capacity(self.world.population() as usize);
        for i in 0..o.capacity() {
            if o.alive[i] {
                v.push(o.g_size[i]);
            }
        }
        v
    }

    pub fn carnivory(&self) -> Vec<u8> {
        let o = &self.world.orgs;
        let mut v = Vec::with_capacity(self.world.population() as usize);
        for i in 0..o.capacity() {
            if o.alive[i] {
                v.push((o.carnivory[i].clamp(0.0, 1.0) * 255.0) as u8);
            }
        }
        v
    }

    pub fn field(&self) -> Vec<u8> {
        let cap = self.world.params.field_cap.max(1);
        self.world
            .field
            .iter()
            .map(|&c| ((c.max(0) * 255) / cap) as u8)
            .collect()
    }

    /// Day/night–season light level `0..=1`.
    pub fn daylight(&self) -> f32 {
        self.world.daylight()
    }

    /// Signal/pheromone field normalized to `0..=255`, row-major.
    pub fn signal(&self) -> Vec<u8> {
        let cap = self.world.params.signal_cap.max(1);
        self.world
            .signal
            .iter()
            .map(|&s| ((s.max(0) * 255) / cap) as u8)
            .collect()
    }

    /// Bloom (food-patch) centres as `[x0, y0, x1, y1, ...]` in world coordinates.
    pub fn blooms(&self) -> Vec<f32> {
        let mut v = Vec::with_capacity(self.world.blooms.len() * 2);
        for &(x, y) in &self.world.blooms {
            v.push(x);
            v.push(y);
        }
        v
    }

    /// `[size, metabolism, repro, carnivory]` averages (display only).
    pub fn avg_traits(&self) -> Vec<f32> {
        let o = &self.world.orgs;
        let (mut sz, mut m, mut r, mut cn, mut n) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0u32);
        for i in 0..o.capacity() {
            if o.alive[i] {
                sz += o.g_size[i];
                m += o.g_metab[i];
                r += o.g_repro[i];
                cn += o.carnivory[i];
                n += 1;
            }
        }
        if n == 0 {
            vec![0.0, 0.0, 0.0, 0.0]
        } else {
            let n = n as f32;
            vec![sz / n, m / n, r / n, cn / n]
        }
    }

    // --- world-health diagnostics (Phase 0.5) -----------------------------

    /// Deaths by cause over the last `tick(n)` call: `[starved, old_age, killed, eaten]`.
    pub fn deaths_recent(&self) -> Vec<u32> {
        self.last_deaths.to_vec()
    }

    /// Colour diversity as Shannon entropy over 64 quantized colour bins (0 = monoculture,
    /// ~4.16 = maximally varied).
    pub fn diversity(&self) -> f32 {
        let o = &self.world.orgs;
        let mut bins = [0u32; 64];
        let mut n = 0u32;
        for i in 0..o.capacity() {
            if o.alive[i] {
                let b = ((o.cr[i] as usize >> 6) << 4)
                    | ((o.cg[i] as usize >> 6) << 2)
                    | (o.cb[i] as usize >> 6);
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

    /// Fraction of the population currently hunting (carnivory > 0.12).
    pub fn frac_carnivore(&self) -> f32 {
        let o = &self.world.orgs;
        let (mut c, mut n) = (0u32, 0u32);
        for i in 0..o.capacity() {
            if o.alive[i] {
                n += 1;
                if o.carnivory[i] > 0.12 {
                    c += 1;
                }
            }
        }
        if n == 0 {
            0.0
        } else {
            c as f32 / n as f32
        }
    }

    /// Mean speed (a "fraction moving" proxy).
    pub fn avg_speed(&self) -> f32 {
        let o = &self.world.orgs;
        let (mut s, mut n) = (0.0f32, 0u32);
        for i in 0..o.capacity() {
            if o.alive[i] {
                s += (o.vx[i] * o.vx[i] + o.vy[i] * o.vy[i]).sqrt();
                n += 1;
            }
        }
        if n == 0 {
            0.0
        } else {
            s / n as f32
        }
    }

    /// Average brain complexity (hidden nodes + enabled connections) — watch it climb.
    pub fn avg_brain_complexity(&self) -> f32 {
        let o = &self.world.orgs;
        let (mut s, mut n) = (0u64, 0u64);
        for i in 0..o.capacity() {
            if o.alive[i] {
                s += o.brains[i].complexity() as u64;
                n += 1;
            }
        }
        if n == 0 {
            0.0
        } else {
            s as f32 / n as f32
        }
    }

    /// Per-colour-group ("species") breakdown as a JSON array, sorted by count descending.
    pub fn species_json(&self) -> String {
        const NAMES: [&str; 8] = [
            "тёмные",
            "синие",
            "зелёные",
            "бирюзовые",
            "красные",
            "розовые",
            "жёлтые",
            "светлые",
        ];
        let o = &self.world.orgs;
        let mut count = [0u32; 8];
        let mut size = [0f32; 8];
        let mut carn = [0f32; 8];
        let mut brain = [0f32; 8];
        for i in 0..o.capacity() {
            if o.alive[i] {
                let bkt = (((o.cr[i] > 127) as usize) << 2)
                    | (((o.cg[i] > 127) as usize) << 1)
                    | ((o.cb[i] > 127) as usize);
                count[bkt] += 1;
                size[bkt] += o.g_size[i];
                carn[bkt] += o.carnivory[i];
                brain[bkt] += o.brains[i].complexity() as f32;
            }
        }
        let mut order: Vec<usize> = (0..8).filter(|&b| count[b] > 0).collect();
        order.sort_by(|&a, &b| count[b].cmp(&count[a]));
        let mut out = String::from("[");
        for (k, &b) in order.iter().enumerate() {
            if k > 0 {
                out.push(',');
            }
            let n = count[b] as f32;
            let r = if b & 4 != 0 { 210 } else { 45 };
            let g = if b & 2 != 0 { 210 } else { 45 };
            let bl = if b & 1 != 0 { 210 } else { 45 };
            out.push_str(&format!(
                "{{\"name\":\"{}\",\"count\":{},\"size\":{:.2},\"carn\":{:.2},\"brain\":{:.1},\"r\":{},\"g\":{},\"b\":{}}}",
                NAMES[b], count[b], size[b] / n, carn[b] / n, brain[b] / n, r, g, bl
            ));
        }
        out.push(']');
        out
    }

    // --- presets ----------------------------------------------------------

    pub fn preset_count() -> u32 {
        presets::COUNT
    }

    pub fn preset_name(id: u32) -> String {
        presets::name(id).to_string()
    }

    /// Restart with a named preset and a new seed.
    pub fn load_preset(&mut self, id: u32, seed: u32) {
        self.world = World::new(seed as u64, presets::preset(id));
        self.pending.clear();
        self.last_deaths = [0; 4];
    }

    /// Nearest live organism to a world point, for the inspector. Returns
    /// `[px, py, energy, age, size, metabolism, repro, r, g, b, id, carnivory]` or empty.
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
        let i = best as usize;
        vec![
            o.px[i],
            o.py[i],
            o.energy[i] as f32,
            o.age[i] as f32,
            o.g_size[i],
            o.g_metab[i],
            o.g_repro[i],
            o.cr[i] as f32,
            o.cg[i] as f32,
            o.cb[i] as f32,
            o.id[i] as f32,
            o.carnivory[i],
            o.brains[i].complexity() as f32,
        ]
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

    pub fn set_mutation_rate(&mut self, per_ten_thousand: i32) {
        self.set_param(ParamId::MutationRate, per_ten_thousand as i64);
    }

    pub fn set_field_regrow(&mut self, raw: i32) {
        self.set_param(ParamId::FieldRegrow, raw as i64);
    }

    pub fn set_eat_rate(&mut self, raw: i32) {
        self.set_param(ParamId::EatRate, raw as i64);
    }

    pub fn set_bite_amount(&mut self, raw: i32) {
        self.set_param(ParamId::BiteAmount, raw as i64);
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
}
