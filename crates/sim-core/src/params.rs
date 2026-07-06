//! World parameters — the tunable starting conditions.
//!
//! Energy/field-related quantities are integers on purpose (exact, associative, cross-target
//! deterministic). Spatial quantities are `Scalar` (`f32`). `Default` is a playable-ish
//! Phase-0 preset — not yet ecology-calibrated (that is the gated Phase 0.5 deliverable).

use crate::math::Scalar;

/// Bumped whenever the `WorldParams` layout or field meaning changes.
pub const PARAMS_SCHEMA_VERSION: u16 = 1;

/// Addressable parameters for the `SetParam` command. Values arrive as raw integers (never
/// host-computed floats) and are interpreted per key, so the command log stays machine
/// independent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParamId {
    /// Mutation probability per trait, in ten-thousandths (`raw = 1000` → 0.10).
    MutationRate,
    /// Field regrowth per cell per tick (raw energy units).
    FieldRegrow,
    /// Max speed per axis, in thousandths (`raw = 2000` → 2.0).
    MaxSpeed,
    /// Max energy an organism can eat from a cell per tick.
    EatRate,
    /// Base reproduction energy threshold.
    ReproThreshold,
}

#[derive(Clone, PartialEq, Debug)]
pub struct WorldParams {
    // --- world geometry ---
    pub width: Scalar,
    pub height: Scalar,
    pub grid_w: u32,
    pub grid_h: u32,

    // --- field (resource) dynamics ---
    pub field_cap: i64,
    pub field_regrow: i64,

    // --- movement ---
    pub max_speed: Scalar,
    pub steer_accel: Scalar,
    pub move_cost_coeff: i64,

    // --- metabolism ---
    pub basal_upkeep: i64,
    pub eat_rate: i64,
    pub death_deposit: i64,
    pub max_age: u32,

    // --- reproduction ---
    pub repro_threshold: i64,
    pub repro_cost: i64,
    pub offspring_energy: i64,
    pub spawn_radius: Scalar,

    // --- mutation ---
    pub mutation_rate: Scalar,
    pub mutation_delta: Scalar,

    // --- initial conditions / limits ---
    pub initial_population: u32,
    pub initial_energy: i64,
    pub max_population: u32,
}

impl Default for WorldParams {
    fn default() -> Self {
        WorldParams {
            width: 512.0,
            height: 512.0,
            grid_w: 64,
            grid_h: 64,

            field_cap: 1000,
            field_regrow: 6,

            max_speed: 2.0,
            steer_accel: 0.6,
            move_cost_coeff: 1,

            basal_upkeep: 2,
            eat_rate: 60,
            death_deposit: 25,
            max_age: 2000,

            repro_threshold: 420,
            repro_cost: 40,
            offspring_energy: 150,
            spawn_radius: 3.0,

            mutation_rate: 0.10,
            mutation_delta: 0.10,

            initial_population: 200,
            initial_energy: 200,
            max_population: 20_000,
        }
    }
}

impl WorldParams {
    /// Apply a `SetParam` command. Raw integer inputs keep the command log deterministic.
    pub fn set(&mut self, key: ParamId, raw: i64) {
        match key {
            ParamId::MutationRate => self.mutation_rate = (raw.clamp(0, 10_000) as f32) / 10_000.0,
            ParamId::FieldRegrow => self.field_regrow = raw.max(0),
            ParamId::MaxSpeed => self.max_speed = (raw.max(0) as f32) / 1000.0,
            ParamId::EatRate => self.eat_rate = raw.max(0),
            ParamId::ReproThreshold => self.repro_threshold = raw.max(1),
        }
    }

    #[inline]
    pub fn cell_count(&self) -> usize {
        (self.grid_w as usize) * (self.grid_h as usize)
    }
}
