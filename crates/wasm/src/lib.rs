//! Browser host for `sim-core` via `wasm-bindgen`.
//!
//! A thin wrapper exposing the deterministic engine to JavaScript: advance the world, read
//! render buffers and diagnostics, and issue commands (the single mutation channel). All
//! simulation logic lives in `sim-core`; this crate only marshals data across the boundary.
//! Diagnostic readouts (diversity, fractions, averages) are display-only and never feed back
//! into the sim.

use sim_core::brain::N_CHAN;
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

    /// Detritus/corpse field normalized to `0..=255` (against `field_cap`), row-major.
    pub fn detritus(&self) -> Vec<u8> {
        let cap = self.world.params.field_cap.max(1);
        self.world
            .detritus
            .iter()
            .map(|&s| ((s.max(0) * 255) / cap).min(255) as u8)
            .collect()
    }

    /// Static terrain fertility normalized to `0..=255` (barren→0, fertile→255), row-major.
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

    /// The waterline in `0..=1`: cells below this elevation read as underwater (for rendering).
    pub fn water_level(&self) -> f32 {
        self.world.params.water_level
    }

    /// Chemical channel `k` normalized to `0..=255` (against `chan_cap`), row-major — for the
    /// per-substance map layer.
    pub fn channel(&self, k: u32) -> Vec<u8> {
        let cells = self.world.params.cell_count();
        let k = k as usize;
        if k >= N_CHAN {
            return vec![0u8; cells];
        }
        let cap = self.world.params.chan_cap.max(1);
        let base = k * cells;
        self.world.chan[base..base + cells]
            .iter()
            .map(|&c| ((c.max(0) * 255) / cap).min(255) as u8)
            .collect()
    }

    /// Per-live-organism diet gene `0..=255` (0 = food-A specialist, 255 = food-B), in
    /// `positions()` order — for the "colour by diet" layer.
    pub fn diets(&self) -> Vec<u8> {
        let o = &self.world.orgs;
        let mut v = Vec::with_capacity(self.world.population() as usize);
        for i in 0..o.capacity() {
            if o.alive[i] {
                v.push((o.g_diet[i].clamp(0.0, 1.0) * 255.0) as u8);
            }
        }
        v
    }

    /// Per-live-organism habitat gene `0..=255` (0 = water, 255 = high land), in `positions()`
    /// order — for the "colour by habitat" layer.
    pub fn habitats(&self) -> Vec<u8> {
        let o = &self.world.orgs;
        let mut v = Vec::with_capacity(self.world.population() as usize);
        for i in 0..o.capacity() {
            if o.alive[i] {
                v.push((o.g_habitat[i].clamp(0.0, 1.0) * 255.0) as u8);
            }
        }
        v
    }

    /// Per-live-organism role bitmask for channel `k` (bit0 emit, bit1 uptake, bit2 resist,
    /// bit3 sense), in `positions()` order — for the "colour by chemical role" layer, so producers
    /// and consumers of a substance are visible in place.
    pub fn chem_role_for(&self, k: u32) -> Vec<u8> {
        let o = &self.world.orgs;
        let k = k as usize;
        let mut v = Vec::with_capacity(self.world.population() as usize);
        for i in 0..o.capacity() {
            if o.alive[i] {
                let mut b = 0u8;
                if k < N_CHAN {
                    if o.emit_ch[i][k] > 0 {
                        b |= 1;
                    }
                    if o.uptake_ch[i][k] > 0 {
                        b |= 2;
                    }
                    if o.resist_mask[i] & (1 << k) != 0 {
                        b |= 4;
                    }
                    if o.sense_mask[i] & (1 << k) != 0 {
                        b |= 8;
                    }
                }
                v.push(b);
            }
        }
        v
    }

    /// Food field B normalized to `0..=255` (the second, "amber" resource), row-major.
    pub fn field_b(&self) -> Vec<u8> {
        let cap = self.world.params.field_cap.max(1);
        self.world
            .field_b
            .iter()
            .map(|&c| ((c.max(0) * 255) / cap) as u8)
            .collect()
    }

    /// Population split by evolved diet: `[food-A specialists, generalists, food-B specialists]`
    /// (diet < 0.35, 0.35..=0.65, > 0.65). Two full buckets = two dietary niches.
    pub fn diet_hist(&self) -> Vec<u32> {
        let o = &self.world.orgs;
        let (mut a, mut g, mut b) = (0u32, 0u32, 0u32);
        for i in 0..o.capacity() {
            if o.alive[i] {
                let d = o.g_diet[i];
                if d < 0.35 {
                    a += 1;
                } else if d > 0.65 {
                    b += 1;
                } else {
                    g += 1;
                }
            }
        }
        vec![a, g, b]
    }

    /// Active food-burst events as `[x, y, radius, life_frac, ...]` in world coordinates.
    /// `life_frac` is remaining lifetime in `[0, 1]` (1 = just spawned) so the UI can fade them.
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

    /// Active bonds as endpoint pairs `[ax, ay, bx, by, ...]` in world coordinates — the links
    /// that make bodies visible. Bonds are pruned in the spring phase, but a partner can still die
    /// (or have its slot recycled) *later in the same tick*, so we re-validate `(slot,id)` here to
    /// avoid drawing a stray link to a recycled cell until the next tick prunes it.
    pub fn bonds(&self) -> Vec<f32> {
        let o = &self.world.orgs;
        let mut v = Vec::with_capacity(self.world.bonds.len() * 4);
        for b in &self.world.bonds {
            let (a, c) = (b.sa as usize, b.sb as usize);
            if !(o.alive[a] && o.id[a] == b.ida && o.alive[c] && o.id[c] == b.idb) {
                continue;
            }
            v.push(o.px[a]);
            v.push(o.py[a]);
            v.push(o.px[c]);
            v.push(o.py[c]);
        }
        v
    }

    /// Number of active bonds (edges in the body graph).
    pub fn bond_count(&self) -> u32 {
        self.world.bonds.len() as u32
    }

    /// Average `adhesion` trait over living cells (0 = nobody sticks) — the driver of bodies.
    pub fn avg_adhesion(&self) -> f32 {
        let o = &self.world.orgs;
        let (mut s, mut n) = (0.0f32, 0u32);
        for i in 0..o.capacity() {
            if o.alive[i] {
                s += o.g_adhesion[i];
                n += 1;
            }
        }
        if n == 0 {
            0.0
        } else {
            s / n as f32
        }
    }

    /// Size of the largest body (biggest connected cluster of bonded cells); `1` = no bodies.
    pub fn max_body(&self) -> u32 {
        let cap = self.world.orgs.capacity();
        let mut parent: Vec<u32> = (0..cap as u32).collect();
        fn find(parent: &mut [u32], mut x: u32) -> u32 {
            while parent[x as usize] != x {
                parent[x as usize] = parent[parent[x as usize] as usize];
                x = parent[x as usize];
            }
            x
        }
        let o = &self.world.orgs;
        for b in &self.world.bonds {
            // only union bonds whose endpoints are still the same live cells (a partner may have
            // died and had its slot recycled later in the tick, before the next prune)
            let (sa, sb) = (b.sa as usize, b.sb as usize);
            if !(o.alive[sa] && o.id[sa] == b.ida && o.alive[sb] && o.id[sb] == b.idb) {
                continue;
            }
            let (a, c) = (find(&mut parent, b.sa), find(&mut parent, b.sb));
            if a != c {
                parent[a as usize] = c;
            }
        }
        let mut size = vec![0u32; cap];
        let mut best = 0u32;
        for i in 0..cap {
            if self.world.orgs.alive[i] {
                let r = find(&mut parent, i as u32) as usize;
                size[r] += 1;
                if size[r] > best {
                    best = size[r];
                }
            }
        }
        best
    }

    /// Live organism velocities `[vx0, vy0, ...]`, matching `positions` order (heading lines).
    pub fn velocities(&self) -> Vec<f32> {
        let o = &self.world.orgs;
        let mut v = Vec::with_capacity(self.world.population() as usize * 2);
        for i in 0..o.capacity() {
            if o.alive[i] {
                v.push(o.vx[i]);
                v.push(o.vy[i]);
            }
        }
        v
    }

    /// Look up a specific organism by id (for follow mode). Returns
    /// `[px, py, energy, age, size, metabolism, repro, r, g, b, carnivory, brain, habitat]`,
    /// or empty if that organism is no longer alive.
    pub fn by_id(&self, id: u32) -> Vec<f32> {
        let o = &self.world.orgs;
        for i in 0..o.capacity() {
            if o.alive[i] && o.id[i] == id {
                return vec![
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
                    o.carnivory[i],
                    o.brains[i].complexity() as f32,
                    o.g_habitat[i],
                    o.g_diet[i],
                    o.genomes[i].genes.len() as f32,
                    o.emit_ch[i].iter().filter(|&&x| x > 0).count() as f32,
                    o.uptake_ch[i].iter().filter(|&&x| x > 0).count() as f32,
                    o.resist_mask[i].count_ones() as f32,
                    o.sense_mask[i].count_ones() as f32,
                    o.g_adhesion[i],
                    self.bond_degree(i) as f32,
                ];
            }
        }
        Vec::new()
    }

    /// How many bonds currently touch the cell in slot `i` (its degree in the body graph).
    fn bond_degree(&self, i: usize) -> u32 {
        let id = self.world.orgs.id[i];
        self.world
            .bonds
            .iter()
            .filter(|b| (b.sa as usize == i && b.ida == id) || (b.sb as usize == i && b.idb == id))
            .count() as u32
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

    /// Population split by evolved habitat preference: `[water, shore, land]` (habitat < 0.4,
    /// 0.4..=0.6, > 0.6). A lineage crossing from one to another is adaptation you can watch.
    pub fn habitat_hist(&self) -> Vec<u32> {
        let o = &self.world.orgs;
        let (mut w, mut s, mut l) = (0u32, 0u32, 0u32);
        for i in 0..o.capacity() {
            if o.alive[i] {
                let h = o.g_habitat[i];
                if h < 0.4 {
                    w += 1;
                } else if h > 0.6 {
                    l += 1;
                } else {
                    s += 1;
                }
            }
        }
        vec![w, s, l]
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

    /// Per-channel population role counts, flattened `[emit, uptake, resist, sense]` × N_CHAN.
    /// Emit+uptake overlapping on a channel = cross-feeding forming; emit+resist = a detox /
    /// chemical-warfare dynamic. All read off measured genes; the engine labels no substance.
    pub fn channel_role_counts(&self) -> Vec<u32> {
        let o = &self.world.orgs;
        let mut counts = vec![0u32; N_CHAN * 4];
        for i in 0..o.capacity() {
            if o.alive[i] {
                let (em, up) = (o.emit_ch[i], o.uptake_ch[i]);
                let (rm, sm) = (o.resist_mask[i], o.sense_mask[i]);
                for (k, (&e, &u)) in em.iter().zip(up.iter()).enumerate() {
                    if e > 0 {
                        counts[k * 4] += 1;
                    }
                    if u > 0 {
                        counts[k * 4 + 1] += 1;
                    }
                    if rm & (1 << k) != 0 {
                        counts[k * 4 + 2] += 1;
                    }
                    if sm & (1 << k) != 0 {
                        counts[k * 4 + 3] += 1;
                    }
                }
            }
        }
        counts
    }

    /// Number of chemical channels (for reshaping [`Self::channel_role_counts`]).
    pub fn n_chan() -> u32 {
        N_CHAN as u32
    }

    /// Average genome length (number of genes) — the open genome grows/shrinks as lineages
    /// duplicate, delete and add genes.
    pub fn avg_genome_len(&self) -> f32 {
        let o = &self.world.orgs;
        let (mut s, mut n) = (0u64, 0u64);
        for i in 0..o.capacity() {
            if o.alive[i] {
                s += o.genomes[i].genes.len() as u64;
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
        let mut hab = [0f32; 8];
        let mut dt = [0f32; 8];
        let mut ener = [0f64; 8];
        let mut wsl = [[0u32; 3]; 8]; // water / shore / land tally per colour group
        for i in 0..o.capacity() {
            if o.alive[i] {
                let bkt = (((o.cr[i] > 127) as usize) << 2)
                    | (((o.cg[i] > 127) as usize) << 1)
                    | ((o.cb[i] > 127) as usize);
                count[bkt] += 1;
                size[bkt] += o.g_size[i];
                carn[bkt] += o.carnivory[i];
                brain[bkt] += o.brains[i].complexity() as f32;
                let h = o.g_habitat[i];
                hab[bkt] += h;
                dt[bkt] += o.g_diet[i];
                ener[bkt] += o.energy[i] as f64;
                let k = if h < 0.4 {
                    0
                } else if h > 0.6 {
                    2
                } else {
                    1
                };
                wsl[bkt][k] += 1;
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
                "{{\"bkt\":{},\"name\":\"{}\",\"count\":{},\"size\":{:.2},\"carn\":{:.2},\"brain\":{:.1},\"hab\":{:.2},\"diet\":{:.2},\"energy\":{:.0},\"water\":{},\"shore\":{},\"land\":{},\"r\":{},\"g\":{},\"b\":{}}}",
                b, NAMES[b], count[b], size[b] / n, carn[b] / n, brain[b] / n,
                hab[b] / n, dt[b] / n, ener[b] / n as f64, wsl[b][0], wsl[b][1], wsl[b][2], r, g, bl
            ));
        }
        out.push(']');
        out
    }

    /// Record-holders (display only): the single most extreme live organism in each category,
    /// as a JSON array `[{cat,id,val,env,r,g,b}]`. The `id` lets the UI follow it on click.
    pub fn records(&self) -> String {
        let o = &self.world.orgs;
        let mut best = [(f64::MIN, usize::MAX); 5]; // age, energy, size, brain, carnivory
        for i in 0..o.capacity() {
            if !o.alive[i] {
                continue;
            }
            let vals = [
                o.age[i] as f64,
                o.energy[i] as f64,
                o.g_size[i] as f64,
                o.brains[i].complexity() as f64,
                o.carnivory[i] as f64,
            ];
            for k in 0..5 {
                if vals[k] > best[k].0 {
                    best[k] = (vals[k], i);
                }
            }
        }
        const CATS: [&str; 5] = [
            "🕰 старейшина",
            "⚡ богач",
            "📏 гигант",
            "🧠 умник",
            "🦈 хищник",
        ];
        let mut out = String::from("[");
        for k in 0..5 {
            let (val, i) = best[k];
            if i == usize::MAX {
                continue;
            }
            if out.len() > 1 {
                out.push(',');
            }
            let vs = match k {
                2 => format!("{val:.2}"),            // size
                4 => format!("{:.0}%", val * 100.0), // carnivory
                _ => format!("{val:.0}"),            // age / energy / brain
            };
            let h = o.g_habitat[i];
            let env = if h < 0.4 {
                "🌊"
            } else if h > 0.6 {
                "⛰"
            } else {
                "🏖"
            };
            out.push_str(&format!(
                "{{\"cat\":\"{}\",\"id\":{},\"val\":\"{}\",\"env\":\"{}\",\"r\":{},\"g\":{},\"b\":{}}}",
                CATS[k], o.id[i], vs, env, o.cr[i], o.cg[i], o.cb[i]
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
    /// `[px, py, energy, age, size, metabolism, repro, r, g, b, id, carnivory, brain, habitat]`
    /// or empty.
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
            o.g_habitat[i],
            o.g_diet[i],
            o.genomes[i].genes.len() as f32,
            o.emit_ch[i].iter().filter(|&&x| x > 0).count() as f32,
            o.uptake_ch[i].iter().filter(|&&x| x > 0).count() as f32,
            o.resist_mask[i].count_ones() as f32,
            o.sense_mask[i].count_ones() as f32,
            o.g_adhesion[i],
            self.bond_degree(i) as f32,
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

    /// Trigger a transient food-burst event centred on grid cell `(cx, cy)`.
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

    pub fn set_bite_amount(&mut self, raw: i32) {
        self.set_param(ParamId::BiteAmount, raw as i64);
    }

    pub fn set_habitat_cost(&mut self, raw: i32) {
        self.set_param(ParamId::HabitatCost, raw as i64);
    }

    /// Chance per tick (in ten-thousandths) of a random food-burst event. `0` = off.
    pub fn set_bloom_rate(&mut self, per_ten_thousand: i32) {
        self.set_param(ParamId::BloomEventRate, per_ten_thousand as i64);
    }

    /// Bond spring stiffness ×1000 (M3) — global body cohesion. `0` = bonds exert no force.
    pub fn set_bond_stiffness(&mut self, per_thousand: i32) {
        self.set_param(ParamId::BondStiffness, per_thousand as i64);
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
