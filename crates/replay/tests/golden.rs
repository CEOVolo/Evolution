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
    // + Niche-construction chemistry: generic channels + Emit/Uptake/Resist/Sense genes (EVO10,
    // N_IN 11→17). Regenerated deliberately.
    assert_eq!(hash_after(1, 100), 0xd8d1_66fd_06d6_d446, "seed=1 tick=100");
    assert_eq!(hash_after(1, 500), 0x1004_adbc_21f4_cec4, "seed=1 tick=500");
    assert_eq!(
        hash_after(1, 1000),
        0x256c_f7a0_850a_717f,
        "seed=1 tick=1000"
    );
    assert_eq!(
        hash_after(2, 2000),
        0x6e84_3ef4_76ca_1002,
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
