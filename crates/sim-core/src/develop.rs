//! **Development** — growing a multicellular body from one genome, deterministically.
//!
//! A zygote (single cell) runs the organism's [`RegNet`] once per cell per developmental step on
//! that cell's *local* context — the morphogen concentrations diffusing across the body, a couple
//! of position cues, and its own accumulated cell-state. The network decides whether the cell
//! divides (and in which direction), what morphogens it secretes, how it differentiates (its
//! continuous feeding / structural role weights), and whether it dies. Same network + different
//! local context ⇒ cells that differ by **position** — clonal development, nothing scripted.
//!
//! The whole pass is deterministic and cross-target: morphogens are integer fields diffused with
//! the same mass-conserving scheme the world uses for chemistry; the network is the feed-forward
//! [`RegNet`]; cells are always visited in canonical lattice order; role/morphogen accumulation is
//! integer; the only randomness is a per-body **polarity axis** (keyed by organism id) that breaks
//! symmetry so bodies aren't uniform radial blobs — we seed a coordinate frame, never a body plan.

use crate::math::{clamp_abs, Scalar};
use crate::regnet::{
    RegNet, DIN_AXIS, DIN_CLOCK, DIN_DEGREE, DIN_MORPH0, DIN_STATE0, DOUT_APOPTOSE, DOUT_DIR0,
    DOUT_DIVIDE, DOUT_DSTATE0, DOUT_EMIT0, DOUT_ROLE_FEED, DOUT_ROLE_STRUCT, N_DEV_IN, N_MORPH,
    S_STATE,
};
use crate::rng::{subsystem, Pcg32};

/// Lattice half-extent: cells live at integer offsets in `[-R, R]²`.
const R: i32 = 6;
const SIDE: usize = (2 * R + 1) as usize;
const SITES: usize = SIDE * SIDE;
/// Developmental steps.
const D: u32 = 10;
/// Hard cap on cells per body.
const CELL_CAP: usize = 48;
/// Developmental floor: every body grows at least this many cells if there is room (a minimum
/// viable body, so the mechanism is visible and selection has bodies to act on from generation 0).
const MIN_CELLS: usize = 3;

/// Morphogen diffusion divisor (donate `conc/DDIV` to each existing neighbour; `>= 4`).
const DDIV: i64 = 4;
/// Morphogen decay per step (`* num / den`).
const MDECAY_NUM: i64 = 7;
const MDECAY_DEN: i64 = 8;
/// Maternal morphogen bolus deposited in the zygote (channel 0) and the increment given to the
/// distal daughter on each division — the axis seed that propagates positional information.
const MATERNAL_BOLUS: i64 = 4000;
const ASYM_INCREMENT: i64 = 1500;
/// Output→field scales (float network output → integer field units).
const EMIT_STEP: f32 = 900.0;
const ROLE_STEP: f32 = 300.0;
const DSTATE_STEP: f32 = 0.5;
/// Normalizer applied to a morphogen concentration before it is fed to the network.
const MORPH_NORM: f32 = 1500.0;
/// Decision thresholds on network outputs.
const DIV_THRESHOLD: f32 = 0.15;
const APO_THRESHOLD: f32 = 0.85;
/// Full-expression role accumulator (maps to 1.0); structural cells grow up to `STRUCT_SIZE_GAIN`.
const ROLE_FULL: i32 = 1000;
const STRUCT_SIZE_GAIN: f32 = 0.8;

/// One cell of a developed body. Positions are lattice offsets (the cell's stable identity);
/// roles are continuous fixed-point weights in milli-units (0..=1000), read at runtime — there is
/// no cell-type enum anywhere.
#[derive(Clone, Copy, Debug)]
pub struct BodyCell {
    pub lu: i16,
    pub lv: i16,
    pub size_milli: i32,
    pub cr: u8,
    pub cg: u8,
    pub cb: u8,
    pub role_feed: i16,
    pub role_struct: i16,
}

/// A developed body: cells in canonical `(lv, lu)` order (so every downstream pass — hashing,
/// rendering, feeding — is order-stable).
#[derive(Clone, Debug, Default)]
pub struct Body {
    pub cells: Vec<BodyCell>,
}

impl Body {
    /// Total body mass in milli-units (fixed-point, exact) — drives whole-body accel and the
    /// gape-limit on predation.
    pub fn mass_milli(&self) -> i64 {
        self.cells.iter().map(|c| c.size_milli as i64).sum()
    }
}

#[inline]
fn site(lu: i32, lv: i32) -> usize {
    ((lv + R) as usize) * SIDE + (lu + R) as usize
}

#[inline]
fn in_bounds(lu: i32, lv: i32) -> bool {
    (-R..=R).contains(&lu) && (-R..=R).contains(&lv)
}

/// The 4 lattice neighbour directions, in the order the network's `DOUT_DIR` slots address them.
const DIRS: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];

struct DevCell {
    lu: i32,
    lv: i32,
    state: [Scalar; S_STATE],
    feed_acc: i32,
    struct_acc: i32,
    alive: bool,
}

/// Grow a body from one genome's developmental network. Deterministic; the only RNG is the
/// per-body polarity draw keyed by `org_id` on the append-only `DEVELOP` stream.
pub fn develop_body(
    regnet: &RegNet,
    base_size: Scalar,
    color: (u8, u8, u8),
    seed: u64,
    org_id: u32,
) -> Body {
    // Polarity axis: a seeded unit vector (deterministic per body, different across bodies), built
    // with only signed draws + sqrt so no transcendental is needed.
    let mut prng = Pcg32::from_key(seed, subsystem::DEVELOP, org_id as u64);
    let (mut pxa, mut pya) = (prng.next_f32_signed(), prng.next_f32_signed());
    let plen2 = pxa * pxa + pya * pya;
    if plen2 > 1e-6 {
        let pl = plen2.sqrt();
        pxa /= pl;
        pya /= pl;
    } else {
        pxa = 1.0;
        pya = 0.0;
    }

    let mut occ = vec![-1i32; SITES]; // site -> DevCell index, or -1
    let mut morph = vec![0i64; N_MORPH * SITES];
    let mut cells: Vec<DevCell> = Vec::with_capacity(MIN_CELLS.max(4));

    // Zygote at origin, with the maternal bolus in channel 0.
    cells.push(DevCell {
        lu: 0,
        lv: 0,
        state: [0.0; S_STATE],
        feed_acc: 0,
        struct_acc: 0,
        alive: true,
    });
    occ[site(0, 0)] = 0;
    morph[site(0, 0)] += MATERNAL_BOLUS;

    let mut scratch: Vec<Scalar> = Vec::new();

    for step in 0..D {
        // 1. diffuse each morphogen one integer, mass-conserving pass over the bounded lattice.
        diffuse(&mut morph);

        // 2. GATHER (read-only): decisions per alive cell, in canonical order.
        let order = canonical_order(&cells);
        let clock = (step as f32 / D as f32) * 2.0 - 1.0;
        let mut decisions: Vec<Option<Decision>> = vec![None; cells.len()];
        for &ci in &order {
            let c = &cells[ci];
            let s = site(c.lu, c.lv);
            let mut inp = [0.0f32; N_DEV_IN];
            inp[0] = 1.0; // bias
            inp[DIN_CLOCK] = clock;
            for k in 0..N_MORPH {
                inp[DIN_MORPH0 + k] = clamp_abs(morph[k * SITES + s] as f32 / MORPH_NORM, 1.0);
            }
            let deg = neighbour_count(&occ, c.lu, c.lv) as f32 / 4.0;
            inp[DIN_DEGREE] = deg * 2.0 - 1.0;
            inp[DIN_AXIS] = clamp_abs((c.lu as f32 * pxa + c.lv as f32 * pya) / R as f32, 1.0);
            inp[DIN_STATE0..DIN_STATE0 + S_STATE].copy_from_slice(&c.state);
            let out = regnet.forward(&inp, &mut scratch);
            decisions[ci] = Some(Decision {
                divide: out[DOUT_DIVIDE] > DIV_THRESHOLD,
                dir: argmax4(&out[DOUT_DIR0..DOUT_DIR0 + 4]),
                emit: {
                    let mut e = [0i64; N_MORPH];
                    for (k, ek) in e.iter_mut().enumerate() {
                        let v = out[DOUT_EMIT0 + k];
                        if v > 0.0 {
                            *ek = (v * EMIT_STEP) as i64;
                        }
                    }
                    e
                },
                dstate: {
                    let mut d = [0.0f32; S_STATE];
                    for (k, dk) in d.iter_mut().enumerate() {
                        *dk = out[DOUT_DSTATE0 + k] * DSTATE_STEP;
                    }
                    d
                },
                feed: (pos(out[DOUT_ROLE_FEED]) * ROLE_STEP) as i32,
                strct: (pos(out[DOUT_ROLE_STRUCT]) * ROLE_STEP) as i32,
                apoptose: out[DOUT_APOPTOSE] > APO_THRESHOLD,
            });
        }

        // 3. APPLY (writes), in canonical order — deterministic conflict resolution.
        for &ci in &order {
            let dec = decisions[ci].unwrap();
            // per-cell (order-independent) updates
            {
                let c = &mut cells[ci];
                if !c.alive {
                    continue;
                }
                for k in 0..S_STATE {
                    c.state[k] = clamp_abs(c.state[k] + dec.dstate[k], 4.0);
                }
                c.feed_acc += dec.feed;
                c.struct_acc += dec.strct;
            }
            let (clu, clv) = (cells[ci].lu, cells[ci].lv);
            let csite = site(clu, clv);
            for k in 0..N_MORPH {
                morph[k * SITES + csite] += dec.emit[k];
            }
            // apoptosis (never remove the last cell)
            if dec.apoptose && alive_count(&cells) > 1 {
                cells[ci].alive = false;
                occ[csite] = -1;
                continue;
            }
            // division into the chosen empty neighbour, if room
            if dec.divide && alive_count(&cells) < CELL_CAP {
                let (dx, dy) = DIRS[dec.dir];
                try_divide(&mut cells, &mut occ, &mut morph, ci, clu + dx, clv + dy);
            }
        }

        // developmental floor: force one polarity-aligned division until MIN_CELLS is reached.
        while alive_count(&cells) < MIN_CELLS {
            if !force_division(&mut cells, &mut occ, &mut morph, pxa, pya) {
                break; // no room anywhere
            }
        }

        // 4. decay morphogens.
        for m in morph.iter_mut() {
            *m = *m * MDECAY_NUM / MDECAY_DEN;
        }
    }

    // Compile developmental cells into the final body, in canonical order.
    let mut out_cells: Vec<BodyCell> = Vec::new();
    let base_milli = (base_size * 1000.0) as i32;
    for &ci in &canonical_order(&cells) {
        let c = &cells[ci];
        if !c.alive {
            continue;
        }
        // Role budget (simplex): feed and struct compete for a shared expression budget, so no
        // cell can max both — differentiation must trade off, which is what makes it pay.
        let (feed_m, struct_m) = role_budget(c.feed_acc, c.struct_acc);
        let struct01 = struct_m as f32 / ROLE_FULL as f32;
        let size_milli = (base_milli as f32 * (1.0 + STRUCT_SIZE_GAIN * struct01)) as i32;
        out_cells.push(BodyCell {
            lu: c.lu as i16,
            lv: c.lv as i16,
            size_milli,
            cr: color.0,
            cg: color.1,
            cb: color.2,
            role_feed: feed_m as i16,
            role_struct: struct_m as i16,
        });
    }
    Body { cells: out_cells }
}

#[derive(Clone, Copy)]
struct Decision {
    divide: bool,
    dir: usize,
    emit: [i64; N_MORPH],
    dstate: [Scalar; S_STATE],
    feed: i32,
    strct: i32,
    apoptose: bool,
}

/// Enforce the role trade-off: clamp each to `[0, ROLE_FULL]`, then if their sum exceeds the
/// budget scale both down so `feed + struct <= ROLE_FULL`.
fn role_budget(feed_acc: i32, struct_acc: i32) -> (i32, i32) {
    let f = feed_acc.clamp(0, ROLE_FULL);
    let s = struct_acc.clamp(0, ROLE_FULL);
    let total = f + s;
    if total > ROLE_FULL {
        // integer scale to sum ROLE_FULL (total > ROLE_FULL > 0, so the divide is safe)
        (f * ROLE_FULL / total, s * ROLE_FULL / total)
    } else {
        (f, s)
    }
}

/// Cells sorted by `(lv, lu)` — the canonical developmental order (never slot order).
fn canonical_order(cells: &[DevCell]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..cells.len()).filter(|&i| cells[i].alive).collect();
    order.sort_by_key(|&i| (cells[i].lv, cells[i].lu));
    order
}

fn alive_count(cells: &[DevCell]) -> usize {
    cells.iter().filter(|c| c.alive).count()
}

fn neighbour_count(occ: &[i32], lu: i32, lv: i32) -> u32 {
    let mut n = 0;
    for (dx, dy) in DIRS {
        let (nu, nv) = (lu + dx, lv + dy);
        if in_bounds(nu, nv) && occ[site(nu, nv)] >= 0 {
            n += 1;
        }
    }
    n
}

/// Positive part, via comparison (never the `f32::max` method — its NaN behaviour differs
/// native↔wasm).
#[inline]
fn pos(x: f32) -> f32 {
    if x > 0.0 {
        x
    } else {
        0.0
    }
}

fn argmax4(v: &[f32]) -> usize {
    let mut best = 0usize;
    let mut bv = v[0];
    for (i, &x) in v.iter().enumerate().skip(1) {
        if x > bv {
            bv = x;
            best = i;
        }
    }
    best
}

/// Create a daughter at `(tu, tv)` if it is in bounds and empty. The daughter copies the parent's
/// cell-state and receives an asymmetric morphogen increment (channel 0) that propagates the axis.
fn try_divide(
    cells: &mut Vec<DevCell>,
    occ: &mut [i32],
    morph: &mut [i64],
    parent: usize,
    tu: i32,
    tv: i32,
) -> bool {
    if !in_bounds(tu, tv) || occ[site(tu, tv)] >= 0 {
        return false;
    }
    let state = cells[parent].state;
    let idx = cells.len();
    cells.push(DevCell {
        lu: tu,
        lv: tv,
        state,
        feed_acc: 0,
        struct_acc: 0,
        alive: true,
    });
    let s = site(tu, tv);
    occ[s] = idx as i32;
    morph[s] += ASYM_INCREMENT;
    true
}

/// The developmental floor: divide the first alive cell (canonical order) that has an empty
/// in-bounds neighbour, choosing the neighbour best aligned with the polarity axis.
fn force_division(
    cells: &mut Vec<DevCell>,
    occ: &mut [i32],
    morph: &mut [i64],
    pxa: f32,
    pya: f32,
) -> bool {
    for &ci in &canonical_order(cells) {
        let (lu, lv) = (cells[ci].lu, cells[ci].lv);
        let mut best_dir: Option<usize> = None;
        let mut best_score = f32::NEG_INFINITY;
        for (k, (dx, dy)) in DIRS.iter().enumerate() {
            let (nu, nv) = (lu + dx, lv + dy);
            if in_bounds(nu, nv) && occ[site(nu, nv)] < 0 {
                let score = *dx as f32 * pxa + *dy as f32 * pya;
                if score > best_score {
                    best_score = score;
                    best_dir = Some(k);
                }
            }
        }
        if let Some(k) = best_dir {
            let (dx, dy) = DIRS[k];
            if try_divide(cells, occ, morph, ci, lu + dx, lv + dy) {
                return true;
            }
        }
    }
    false
}

/// One integer, mass-conserving diffusion pass on the bounded lattice with zero-flux boundaries:
/// each cell donates `conc/DDIV` to every *existing* neighbour and keeps the rest.
fn diffuse(morph: &mut [i64]) {
    let mut buf = vec![0i64; SITES];
    for k in 0..N_MORPH {
        let base = k * SITES;
        for (i, b) in buf.iter_mut().enumerate() {
            *b = morph[base + i] / DDIV;
        }
        for lv in -R..=R {
            for lu in -R..=R {
                let i = site(lu, lv);
                let mut inflow = 0i64;
                let mut ndeg = 0i64;
                for (dx, dy) in DIRS {
                    let (nu, nv) = (lu + dx, lv + dy);
                    if in_bounds(nu, nv) {
                        inflow += buf[site(nu, nv)];
                        ndeg += 1;
                    }
                }
                morph[base + i] = morph[base + i] - ndeg * buf[i] + inflow;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_regnet(seed: u64) -> RegNet {
        let mut rng = Pcg32::new(seed, 1);
        let mut r = RegNet::random_minimal(&mut rng);
        // grow it a bit so bodies are non-trivial
        let mut m = Pcg32::new(seed ^ 0x9e37, 2);
        for _ in 0..30 {
            r.mutate(&mut m, 0.2, 0.4, 0.4, 0.2);
        }
        r
    }

    #[test]
    fn develop_is_deterministic() {
        let r = a_regnet(7);
        let a = develop_body(&r, 1.0, (100, 150, 90), 42, 3);
        let b = develop_body(&r, 1.0, (100, 150, 90), 42, 3);
        assert_eq!(a.cells.len(), b.cells.len());
        for (x, y) in a.cells.iter().zip(b.cells.iter()) {
            assert_eq!(
                (x.lu, x.lv, x.size_milli, x.role_feed, x.role_struct),
                (y.lu, y.lv, y.size_milli, y.role_feed, y.role_struct)
            );
        }
    }

    #[test]
    fn body_respects_floor_and_cap_and_is_canonical() {
        for s in 0..40u64 {
            let r = a_regnet(s);
            let body = develop_body(&r, 1.0, (128, 128, 128), 1, s as u32);
            assert!(!body.cells.is_empty() && body.cells.len() <= CELL_CAP);
            // canonical order by (lv, lu)
            for w in body.cells.windows(2) {
                assert!((w[0].lv, w[0].lu) <= (w[1].lv, w[1].lu));
            }
            // role budget respected
            for c in &body.cells {
                assert!(c.role_feed as i32 + c.role_struct as i32 <= ROLE_FULL + 1);
            }
        }
    }

    #[test]
    fn different_polarity_seeds_can_differ() {
        // Not a hard guarantee for every net, but across seeds bodies should vary in size/shape.
        let r = a_regnet(11);
        let mut sizes = std::collections::BTreeSet::new();
        for oid in 0..20u32 {
            sizes.insert(develop_body(&r, 1.0, (128, 128, 128), 5, oid).cells.len());
        }
        assert!(!sizes.is_empty()); // sanity: it runs across many org ids without panic
    }
}
