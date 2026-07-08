//! `replay` — headless deterministic driver, golden-hash tool, and ecology-sweep harness.
//!
//! Usage:
//!   replay --seed 1 --ticks 1000 --checkpoints 100,500,1000   # single run
//!   replay sweep                                              # ecology parameter sweep

use sim_core::{World, WorldParams};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("sweep") {
        run_sweep();
        return;
    }

    let mut seed: u64 = 1;
    let mut ticks: u64 = 1000;
    let mut checkpoints: Vec<u64> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--seed" => seed = next_num(&args, &mut i),
            "--ticks" => ticks = next_num(&args, &mut i),
            "--checkpoints" => {
                i += 1;
                if let Some(list) = args.get(i) {
                    checkpoints = list
                        .split(',')
                        .filter_map(|s| s.trim().parse().ok())
                        .collect();
                }
            }
            "-h" | "--help" => {
                print_help();
                return;
            }
            other => {
                eprintln!("replay: unknown argument `{other}` (try --help)");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    if checkpoints.is_empty() {
        checkpoints = vec![ticks];
    }
    checkpoints.sort_unstable();

    let mut w = World::new(seed, WorldParams::default());
    let (mut births, mut deaths, mut cp) = (0usize, 0usize, 0usize);
    for t in 1..=ticks {
        let ev = w.tick(&[]);
        births += ev.births();
        deaths += ev.deaths();
        while cp < checkpoints.len() && checkpoints[cp] == t {
            println!(
                "tick={t} pop={} body={:.1} hash={:016x}",
                w.population(),
                avg_body(&w),
                w.state_hash()
            );
            cp += 1;
        }
    }
    let (dol_mean, dol_max) = dol(&w);
    let (da, dg, db) = diet_split(&w);
    let (hw, hs, hl) = habitat_split(&w);
    // Emergence readout: body = mean cells/body, maxbody = biggest body, DOL = division-of-labor
    // index, diff% = bodies with >=2 cell-types; diet/hab = dietary + habitat niches (Stage 2).
    println!(
        "final seed={seed} ticks={ticks} pop={} brain={:.1} regnet={:.1} body={:.1} maxbody={} DOL={:.3}/{:.3} diff%={:.0} div={:.2} diet(A/g/B)={da}/{dg}/{db} hab(w/s/l)={hw}/{hs}/{hl} births={births} deaths={deaths} hash={:016x}",
        w.population(),
        avg_complexity(&w),
        avg_regnet(&w),
        avg_body(&w),
        max_body(&w),
        dol_mean,
        dol_max,
        diff_frac(&w) * 100.0,
        diversity(&w),
        w.state_hash()
    );
}

/// Ecology sweep: score corners of the parameter space on survival, body size, and lineage
/// diversity, to steer the shipped presets toward the viable region.
#[allow(clippy::field_reassign_with_default)]
fn run_sweep() {
    let regrows = [4i64, 8, 12, 16];
    let eats = [45i64, 70];
    let basals = [1i64, 2];
    let seeds = [1u64, 2];
    let ticks = 1500u64;
    let ns = seeds.len() as f64;

    let mut rows = Vec::new();
    for &rg in &regrows {
        for &ea in &eats {
            for &ba in &basals {
                let (mut pop_sum, mut body_sum, mut div_sum, mut surv) = (0.0, 0.0, 0.0, 0u32);
                for &sd in &seeds {
                    let mut p = WorldParams::default();
                    p.field_regrow = rg;
                    p.eat_rate = ea;
                    p.basal_upkeep = ba;
                    let mut w = World::new(sd, p);
                    for _ in 0..ticks {
                        w.tick(&[]);
                    }
                    if w.population() > 50 {
                        surv += 1;
                    }
                    pop_sum += w.population() as f64;
                    body_sum += avg_body(&w) as f64;
                    div_sum += diversity(&w) as f64;
                }
                let pop = pop_sum / ns;
                let body = body_sum / ns;
                let div = div_sum / ns;
                let viable = surv as usize == seeds.len() && div > 0.5;
                rows.push((rg, ea, ba, surv, pop, body, div, viable));
            }
        }
    }
    rows.sort_by(|a, b| b.7.cmp(&a.7).then(b.4.partial_cmp(&a.4).unwrap()));

    println!(
        "ecology sweep — {} configs x {} seeds, {ticks} ticks",
        rows.len(),
        seeds.len()
    );
    println!("regrow  eat basal | surv   pop  body  diversity  viable");
    for (rg, ea, ba, surv, pop, body, div, viable) in rows {
        println!(
            "{rg:>6} {ea:>4} {ba:>5} | {surv}/{}  {pop:>5.0} {body:>5.1}  {div:>7.2}   {}",
            seeds.len(),
            if viable { "yes" } else { "" }
        );
    }
}

fn avg_complexity(w: &World) -> f32 {
    let o = &w.orgs;
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

fn avg_regnet(w: &World) -> f32 {
    let o = &w.orgs;
    let (mut s, mut n) = (0u64, 0u64);
    for i in 0..o.capacity() {
        if o.alive[i] {
            s += o.regnets[i].complexity() as u64;
            n += 1;
        }
    }
    if n == 0 {
        0.0
    } else {
        s as f32 / n as f32
    }
}

/// Mean cells per body — how multicellular the population has become (1 = single cells).
fn avg_body(w: &World) -> f32 {
    let o = &w.orgs;
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

/// The biggest body (most cells) alive.
fn max_body(w: &World) -> u32 {
    let o = &w.orgs;
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

/// Division-of-labor index (mean, max over bodies): mean pairwise divergence of cells' role
/// vectors within a body, normalized to [0, 1]. 0 = every cell identical, 1 = fully specialized —
/// the headline "did a new organism-level property emerge" detector.
fn dol(w: &World) -> (f32, f32) {
    let o = &w.orgs;
    let (mut sum, mut n, mut mx) = (0.0f32, 0u32, 0.0f32);
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
        let bdol = if pairs > 0 { d / pairs as f32 } else { 0.0 };
        sum += bdol;
        n += 1;
        if bdol > mx {
            mx = bdol;
        }
    }
    if n == 0 {
        (0.0, 0.0)
    } else {
        (sum / n as f32, mx)
    }
}

/// Fraction of multi-cell bodies that carry at least two distinct cell-types (role vectors
/// quantized into a coarse grid) — a body is "differentiated" if it isn't uniform.
fn diff_frac(w: &World) -> f32 {
    let o = &w.orgs;
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

/// Dietary niche split over bodies: (food-A specialists `d<0.35`, generalists, food-B `d>0.65`).
fn diet_split(w: &World) -> (u32, u32, u32) {
    let o = &w.orgs;
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
    (a, g, b)
}

/// Habitat niche split over bodies: (water `h<0.4`, shore, land `h>0.6`).
fn habitat_split(w: &World) -> (u32, u32, u32) {
    let o = &w.orgs;
    let (mut water, mut shore, mut land) = (0u32, 0u32, 0u32);
    for i in 0..o.capacity() {
        if o.alive[i] {
            let h = o.g_habitat[i];
            if h < 0.4 {
                water += 1;
            } else if h > 0.6 {
                land += 1;
            } else {
                shore += 1;
            }
        }
    }
    (water, shore, land)
}

/// Shannon diversity of lineage colours (coarse RGB bins) over bodies — rough species spread.
fn diversity(w: &World) -> f32 {
    let o = &w.orgs;
    let mut bins = [0u32; 64];
    let mut n = 0u32;
    for i in 0..o.capacity() {
        if o.alive[i] {
            let cells = &o.bodies[i].cells;
            if cells.is_empty() {
                continue;
            }
            let c0 = cells[0];
            let b =
                ((c0.cr as usize >> 6) << 4) | ((c0.cg as usize >> 6) << 2) | (c0.cb as usize >> 6);
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

fn next_num(args: &[String], i: &mut usize) -> u64 {
    *i += 1;
    match args.get(*i).and_then(|s| s.parse::<u64>().ok()) {
        Some(v) => v,
        None => {
            eprintln!("replay: expected a number after `{}`", args[*i - 1]);
            std::process::exit(2);
        }
    }
}

fn print_help() {
    println!(
        "replay — deterministic sim-core driver\n\n\
         USAGE:\n  replay [--seed N] [--ticks T] [--checkpoints a,b,c]\n  replay sweep\n\n\
         Prints state_hash at each checkpoint, or runs an ecology parameter sweep."
    );
}
