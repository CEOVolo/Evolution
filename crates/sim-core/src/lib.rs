//! `sim-core` — the deterministic simulation engine for **Evolution**.
//!
//! The whole engine is a pure function of state:
//! `World::tick(&mut self, &[Command]) -> EventBatch`. Given the same
//! `seed + params + command log`, it produces a byte-identical world on every machine and
//! on both the native and wasm targets. That determinism is the foundation of replay tests,
//! shareable worlds, and (later) authoritative-server netcode.
//!
//! ## Phase 0
//!
//! This is the deterministic *skeleton*: a trivial-but-real mechanistic world (organisms
//! move, eat from a depletable field, pay energy costs, die into detritus, and reproduce
//! with mutation) with **no hard-coded roles** — every trait lives in an evolvable genome.
//! The point of Phase 0 is to prove the determinism spine end to end before layering on the
//! NEAT brains, chemistry, and rendering of later phases.
//!
//! ## Determinism disciplines enforced here
//!
//! - No `std`/`libm` transcendentals; [`math`] owns deterministic ones.
//! - No FMA / `f32::min`/`max` / `powi`/`powf` in state math.
//! - Energy and field quantities are **integers** (exactly associative, cross-target safe).
//! - The PRNG is hand-rolled ([`rng::Pcg32`]) and streams are keyed by stable identity, not
//!   call order.
//! - Iteration is index-ordered; ties break by stable entity id.

#![forbid(unsafe_code)]

pub mod brain;
pub mod command;
pub mod environment;
pub mod event;
pub mod genome;
pub mod math;
pub mod organism;
pub mod params;
pub mod presets;
pub mod rng;
pub mod snapshot;
pub mod world;

pub use command::{ActorId, Command, CommandKind, COMMAND_SCHEMA_VERSION};
pub use event::{DeathCause, Event, EventBatch};
pub use genome::{develop, Gene, GeneKind, Genome, Phenotype};
pub use params::{ParamId, WorldParams, PARAMS_SCHEMA_VERSION};
pub use world::World;
