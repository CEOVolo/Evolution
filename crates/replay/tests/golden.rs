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
    // Stage 2: emergent bodies (founders single-celled) + the base ecology re-added as body-level
    // functions — two foods with a dietary trade-off, and a water/land habitat cost. The world is
    // still (near-)unicellular at these early checkpoints and grows multicellular later.
    // Regenerated deliberately.
    assert_eq!(hash_after(1, 100), 0x81c0_8b93_a8a2_8ef9, "seed=1 tick=100");
    assert_eq!(hash_after(1, 500), 0x1e1e_1f92_3a96_0956, "seed=1 tick=500");
    assert_eq!(
        hash_after(1, 1000),
        0xba48_9e9e_6e58_9fc1,
        "seed=1 tick=1000"
    );
    assert_eq!(
        hash_after(2, 2000),
        0x3b46_9442_a198_dd0c,
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
