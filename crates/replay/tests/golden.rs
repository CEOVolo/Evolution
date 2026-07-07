//! Golden determinism gate.
//!
//! These committed hashes pin the exact behaviour of the deterministic core. If a change
//! moves them, it changed the simulation — regenerate them deliberately **in the same PR**
//! and say so (see CONTRIBUTING.md). CI additionally runs the same seeds on the wasm target
//! and asserts the hashes match native (cross-target determinism).

use sim_core::{World, WorldParams};

fn hash_after(seed: u64, ticks: u64) -> u64 {
    let mut w = World::new(seed, WorldParams::default());
    for _ in 0..ticks {
        w.tick(&[]);
    }
    w.state_hash()
}

#[test]
fn golden_hashes() {
    // Phase-1 behaviour + landscape (terrain, elevation/habitat barrier). Regenerated deliberately.
    assert_eq!(hash_after(1, 100), 0x3b2a_7a55_78e4_5b7a, "seed=1 tick=100");
    assert_eq!(hash_after(1, 500), 0x0f0a_1d9c_ac37_713e, "seed=1 tick=500");
    assert_eq!(
        hash_after(1, 1000),
        0x3023_bcf3_767e_f95a,
        "seed=1 tick=1000"
    );
    assert_eq!(
        hash_after(2, 2000),
        0xd9de_da30_0a82_5bcb,
        "seed=2 tick=2000"
    );
}

#[test]
fn same_seed_is_bit_identical() {
    assert_eq!(hash_after(7, 800), hash_after(7, 800));
}

#[test]
fn snapshot_midrun_matches_uninterrupted() {
    use sim_core::snapshot;
    let (seed, k, n) = (55u64, 200u64, 700u64);

    let mut a = World::new(seed, WorldParams::default());
    for _ in 0..n {
        a.tick(&[]);
    }

    let mut b = World::new(seed, WorldParams::default());
    for _ in 0..k {
        b.tick(&[]);
    }
    let mut c = snapshot::from_bytes(&snapshot::to_bytes(&b)).unwrap();
    for _ in k..n {
        c.tick(&[]);
    }

    assert_eq!(a.state_hash(), c.state_hash());
}
