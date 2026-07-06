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
                "tick={t} pop={} brain={:.1} hash={:016x}",
                w.population(),
                avg_complexity(&w),
                w.state_hash()
            );
            cp += 1;
        }
    }
    println!(
        "final seed={seed} ticks={ticks} pop={} brain={:.1} births={births} deaths={deaths} hash={:016x}",
        w.population(),
        avg_complexity(&w),
        w.state_hash()
    );
}

/// Ecology sweep: score corners of the parameter space on survival, trophic depth, and
/// phenotype diversity, to steer the shipped presets toward the viable region (Phase 0.5).
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
                let (mut pop_sum, mut carn_sum, mut div_sum, mut surv) = (0.0, 0.0, 0.0, 0u32);
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
                    carn_sum += carn_frac(&w) as f64;
                    div_sum += diversity(&w) as f64;
                }
                let pop = pop_sum / ns;
                let carn = carn_sum / ns * 100.0;
                let div = div_sum / ns;
                let viable = surv as usize == seeds.len() && carn > 2.0 && div > 1.0;
                rows.push((rg, ea, ba, surv, pop, carn, div, viable));
            }
        }
    }
    rows.sort_by(|a, b| b.7.cmp(&a.7).then(b.4.partial_cmp(&a.4).unwrap()));

    println!(
        "ecology sweep — {} configs x {} seeds, {ticks} ticks",
        rows.len(),
        seeds.len()
    );
    println!("regrow  eat basal | surv   pop  carn%  diversity  viable");
    for (rg, ea, ba, surv, pop, carn, div, viable) in rows {
        println!(
            "{rg:>6} {ea:>4} {ba:>5} | {surv}/{}  {pop:>5.0} {carn:>5.1}%  {div:>7.2}   {}",
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

fn carn_frac(w: &World) -> f32 {
    let o = &w.orgs;
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

fn diversity(w: &World) -> f32 {
    let o = &w.orgs;
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
