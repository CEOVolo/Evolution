//! `replay` — headless deterministic driver + golden-hash tool.
//!
//! Runs `sim-core` with no rendering or I/O beyond stdout, printing `state_hash` at
//! checkpoints. This is the determinism oracle: CI runs it on native and (later) wasm and
//! compares hashes; the golden hashes it produces are committed and guarded against drift.
//!
//! Usage:
//!   replay --seed 1 --ticks 1000
//!   replay --seed 7 --ticks 2000 --checkpoints 100,500,1000,2000

use sim_core::{World, WorldParams};

fn main() {
    let args: Vec<String> = std::env::args().collect();
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
                        .filter_map(|s| s.trim().parse::<u64>().ok())
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
    let mut births = 0usize;
    let mut deaths = 0usize;
    let mut cp = 0usize;

    for t in 1..=ticks {
        let ev = w.tick(&[]);
        births += ev.births();
        deaths += ev.deaths();
        while cp < checkpoints.len() && checkpoints[cp] == t {
            println!(
                "tick={t} pop={} hash={:016x}",
                w.population(),
                w.state_hash()
            );
            cp += 1;
        }
    }

    println!(
        "final seed={seed} ticks={ticks} pop={} births={births} deaths={deaths} hash={:016x}",
        w.population(),
        w.state_hash()
    );
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
         USAGE:\n  replay [--seed N] [--ticks T] [--checkpoints a,b,c]\n\n\
         Prints state_hash at each checkpoint and a final summary line."
    );
}
