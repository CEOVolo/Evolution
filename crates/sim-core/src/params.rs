//! World parameters — the tunable starting conditions.
//!
//! Energy/field quantities are integers (exact, associative, cross-target deterministic).
//! Spatial quantities are `Scalar` (`f32`). `Default` is a playable-ish preset — not yet
//! ecology-calibrated (that is the gated Phase 0.5 deliverable).

use crate::math::Scalar;

pub const PARAMS_SCHEMA_VERSION: u16 = 7;

/// Addressable parameters for the `SetParam` command. Values arrive as raw integers (never
/// host-computed floats) and are interpreted per key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParamId {
    /// Mutation probability per gene, in ten-thousandths (`raw = 1000` → 0.10).
    MutationRate,
    /// Field regrowth per cell per tick (raw energy units).
    FieldRegrow,
    /// Max energy an organism can eat from a cell per tick.
    EatRate,
    /// Base reproduction energy threshold.
    ReproThreshold,
    /// Energy a predator steals per bite.
    BiteAmount,
    /// Habitat mismatch cost — how harshly the wrong substrate (esp. water) drains energy,
    /// i.e. how strong a barrier water/low ground is between habitats.
    HabitatCost,
}

#[derive(Clone, PartialEq, Debug)]
pub struct WorldParams {
    // world geometry
    pub width: Scalar,
    pub height: Scalar,
    pub grid_w: u32,
    pub grid_h: u32,

    // field (resource) dynamics
    pub field_cap: i64,
    pub field_regrow: i64,

    // living-world dynamics
    /// Length of the day/night–season cycle in ticks (food waxes and wanes).
    pub day_period: u32,
    /// Number of drifting "bloom" centres where regrowth is boosted.
    pub bloom_count: u32,
    /// Extra regrowth per tick near a bloom centre.
    pub bloom_boost: i64,

    // death & decomposition
    /// Corpse decomposition divisor: `detritus / decompose_div` returns to soil each tick.
    pub decompose_div: i64,
    /// Extra corpse mass deposited per unit of body size on death.
    pub corpse_size_factor: i64,

    // landscape / habitat (water & barriers as an emergent divide)
    /// Elevation (0=deep water .. 1=high land) below which a cell reads as underwater. Purely
    /// the waterline for display and the water/land tally; the barrier itself is `habitat_cost`.
    pub water_level: Scalar,
    /// Energy drained per tick, scaled by the squared mismatch between an organism's evolved
    /// `habitat` trait and the local elevation. High values make water a hard barrier and make
    /// water vs land two distinct niches; who adapts to which is left entirely to selection.
    pub habitat_cost: i64,

    // signals (emergent communication)
    pub emit_scale: i64,
    pub signal_cap: i64,

    // movement (brain-driven)
    pub accel_scale: Scalar,
    pub max_speed: Scalar,
    pub move_cost_coeff: i64,

    // sensing / predation
    pub sense_radius: Scalar,
    pub contact_radius: Scalar,
    pub predation_size_ratio: Scalar,
    pub bite_amount: i64,
    /// Energy nibbled on incidental contact (no pursuit) — lets a proto-predator survive on
    /// random contacts and *then* evolve to hunt, instead of hunting having to appear at once.
    pub innate_bite: i64,
    pub predation_gain_num: i64,
    pub predation_gain_den: i64,

    // density-dependent competition (keeps the world from packing to a uniform carpet)
    /// Radius over which local crowding is counted (must be `<= sense_radius`).
    pub crowd_radius: Scalar,
    /// Energy drained per tick per neighbour inside `crowd_radius`.
    pub crowd_cost: i64,

    // metabolism
    pub basal_upkeep: i64,
    pub brain_cost: i64,
    pub size_upkeep: i64,
    pub eat_rate: i64,
    pub death_deposit: i64,
    pub max_age: u32,

    // reproduction
    pub repro_threshold: i64,
    pub repro_cost: i64,
    pub offspring_energy: i64,
    pub spawn_radius: Scalar,

    // mutation
    pub mutation_rate: Scalar,
    pub mutation_delta: Scalar,
    pub weight_mut_delta: Scalar,
    /// Per-birth probability of adding a brain connection.
    pub add_conn_prob: Scalar,
    /// Per-birth probability of adding (splitting in) a brain node.
    pub add_node_prob: Scalar,

    // initial conditions / limits
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

            day_period: 1500,
            bloom_count: 4,
            bloom_boost: 16,

            decompose_div: 40,
            corpse_size_factor: 55,

            water_level: 0.4,
            habitat_cost: 90,

            emit_scale: 120,
            signal_cap: 1000,

            accel_scale: 0.5,
            max_speed: 2.5,
            move_cost_coeff: 1,

            sense_radius: 28.0,
            contact_radius: 5.0,
            predation_size_ratio: 1.15,
            bite_amount: 60,
            innate_bite: 12,
            predation_gain_num: 3,
            predation_gain_den: 4,

            crowd_radius: 10.0,
            crowd_cost: 2,

            basal_upkeep: 1,
            brain_cost: 1,
            size_upkeep: 2,
            eat_rate: 60,
            death_deposit: 40,
            max_age: 2500,

            repro_threshold: 420,
            repro_cost: 40,
            offspring_energy: 150,
            spawn_radius: 3.0,

            mutation_rate: 0.10,
            mutation_delta: 0.10,
            weight_mut_delta: 0.35,
            add_conn_prob: 0.06,
            add_node_prob: 0.02,

            initial_population: 300,
            initial_energy: 250,
            max_population: 20_000,
        }
    }
}

impl WorldParams {
    pub fn set(&mut self, key: ParamId, raw: i64) {
        match key {
            ParamId::MutationRate => self.mutation_rate = (raw.clamp(0, 10_000) as f32) / 10_000.0,
            ParamId::FieldRegrow => self.field_regrow = raw.max(0),
            ParamId::EatRate => self.eat_rate = raw.max(0),
            ParamId::ReproThreshold => self.repro_threshold = raw.max(1),
            ParamId::BiteAmount => self.bite_amount = raw.max(0),
            ParamId::HabitatCost => self.habitat_cost = raw.max(0),
        }
    }

    #[inline]
    pub fn cell_count(&self) -> usize {
        (self.grid_w as usize) * (self.grid_h as usize)
    }
}
