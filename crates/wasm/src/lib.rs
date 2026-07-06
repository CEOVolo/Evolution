//! Browser host for `sim-core` via `wasm-bindgen`.
//!
//! A thin wrapper exposing the deterministic engine to JavaScript: advance the world, read
//! render buffers, and issue commands (the single mutation channel). All simulation logic
//! lives in `sim-core`; this crate only marshals data across the wasm boundary.

use sim_core::{Command, CommandKind, ParamId, World, WorldParams};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Sim {
    world: World,
    pending: Vec<Command>,
}

#[wasm_bindgen]
impl Sim {
    #[wasm_bindgen(constructor)]
    pub fn new(seed: u32) -> Sim {
        console_error_panic_hook::set_once();
        Sim {
            world: World::new(seed as u64, WorldParams::default()),
            pending: Vec::new(),
        }
    }

    pub fn tick(&mut self, n: u32) {
        if n == 0 {
            return;
        }
        let cmds = std::mem::take(&mut self.pending);
        self.world.tick(&cmds);
        for _ in 1..n {
            self.world.tick(&[]);
        }
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

    /// Live organism positions `[x0, y0, x1, y1, ...]` in world coordinates.
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

    /// Live organism colours `[r,g,b, ...]`, matching `positions` order.
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

    /// Live organism body sizes, matching `positions` order (for dot radius).
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

    /// Live organism "carnivory" `0..=255`, matching `positions` order (predator tint).
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

    /// Resource field normalized to `0..=255`, row-major.
    pub fn field(&self) -> Vec<u8> {
        let cap = self.world.params.field_cap.max(1);
        self.world
            .field
            .iter()
            .map(|&c| ((c.max(0) * 255) / cap) as u8)
            .collect()
    }

    /// Population averages `[size, metabolism, repro, carnivory]` (display only).
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
