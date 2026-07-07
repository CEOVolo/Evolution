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
    // Stage 1 — real multicellular ALife: the unit of life is now an ORGANISM whose body is grown
    // from one genome by a developmental regulatory network (lattice + morphogens), cells
    // differentiate by position, and the whole body shares one energy pool. M3 bonds retired.
    // Regenerated deliberately.
    assert_eq!(hash_after(1, 100), 0x1dee_8c34_c240_d716, "seed=1 tick=100");
    assert_eq!(hash_after(1, 500), 0xa4a5_c4a4_d1fe_ae7b, "seed=1 tick=500");
    assert_eq!(
        hash_after(1, 1000),
        0x9165_576e_97fc_45c7,
        "seed=1 tick=1000"
    );
    assert_eq!(
        hash_after(2, 2000),
        0xedd9_c107_8de8_ac73,
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
