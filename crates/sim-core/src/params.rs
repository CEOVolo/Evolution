//! World parameters — the tunable starting conditions.
//!
//! Energy/field quantities are integers (exact, associative, cross-target deterministic).
//! Spatial quantities are `Scalar` (`f32`). `Default` is a playable-ish preset — not yet
//! ecology-calibrated (that is the gated Phase 0.5 deliverable).

use crate::math::Scalar;

pub const PARAMS_SCHEMA_VERSION: u16 = 11;

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
    /// Chance per tick (in ten-thousandths) that a random transient food-burst event appears.
    /// `0` (default) = no automatic events; food bursts then only come from the user.
    BloomEventRate,
    /// Bond spring stiffness ×1000 (M3): global cohesion of bonded cells. Higher = tighter bodies.
    /// A knob for exploring whether bodies pay off; the *tendency* to bond stays a heritable trait.
    BondStiffness,
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
    /// Transient food-burst events ("blooms"). Unlike the old permanent drifting oases, each
    /// event appears, boosts regrowth in a disc for `bloom_life` ticks, then vanishes — leaving
    /// the food it grew to be eaten down (the "bust"). Events come from two sources: a rare
    /// random spawn (see `bloom_event_rate`, off by default) and the user's food-burst brush.
    ///
    /// Chance per tick, in ten-thousandths, that a random food-burst appears somewhere. `0`
    /// (default) turns automatic events off entirely — the world then only bursts on demand.
    pub bloom_event_rate: i64,
    /// How many ticks a food-burst event lasts before it vanishes.
    pub bloom_life: u32,
    /// Radius (world units) of a food-burst event's boosted disc.
    pub bloom_radius: Scalar,
    /// Extra regrowth per tick inside an active food-burst event.
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

    // chemistry (M2) — generic chemical channels whose meaning evolution assigns. All integer
    // for exact, cross-target-deterministic accumulation.
    /// Per-cell clamp on a channel's concentration.
    pub chan_cap: i64,
    /// Per-tick channel decay (`* num / den`), like the signal field.
    pub chan_decay_num: i64,
    pub chan_decay_den: i64,
    /// Diffusion divisor: each cell donates `conc/div` to EACH of 4 neighbours, so must be `>= 4`
    /// to stay non-negative; `< 4` (incl. the default `0`) turns diffusion off.
    pub chan_diffuse_div: i64,
    /// Excreted amount = `emit_base + intake * strength / emit_den` per emitting gene (free — the
    /// emitter is not debited, so cross-feeding can bootstrap).
    pub chan_emit_base: i64,
    pub chan_emit_den: i64,
    /// Max channel absorbed per tick at full uptake strength.
    pub chan_uptake_rate: i64,
    /// Uptake→energy conversion (`* num / den`, lossy: `num < den`). Lossiness *bounds* the free
    /// emit→uptake loop rather than forbidding it — keep `255·num < emit_den·den` (loop gain < 1),
    /// which `World::new` asserts. At defaults the loop converges to ~1.5× food energy.
    pub chan_uptake_num: i64,
    pub chan_uptake_den: i64,
    /// Metabolic upkeep per uptake gene carried.
    pub chan_uptake_upkeep: i64,
    /// A channel above this concentration is toxic to non-resistant organisms at the cell.
    pub chan_toxic_threshold: i64,
    /// Toxin drain = `(conc - threshold) * toxin_num / toxin_den` per toxic channel.
    pub chan_toxin_num: i64,
    pub chan_toxin_den: i64,
    /// Metabolic upkeep per resistance gene carried.
    pub chan_resist_upkeep: i64,
    /// Normalization divisor for a sensed channel fed to the brain.
    pub chan_sense_cap: i64,

    // bodies (M3) — bonds are springs holding bonded cells near contact; a "body" is a connected
    // cluster of them. The tendency to form bonds is the heritable `adhesion` trait; these are the
    // shared physics constants (no cluster ever gets a bonus — bodies pay for themselves here).
    /// Spring stiffness of a bond (Hooke's constant). Higher = tighter, more rigid bodies.
    pub bond_stiffness: Scalar,
    /// Rest length of a bond, scaled by the two cells' average size — where the spring is relaxed.
    pub bond_rest: Scalar,
    /// A bond snaps when stretched beyond `bond_rest * bond_break_factor` (a body tears apart).
    pub bond_break_factor: Scalar,

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
    /// Per-birth probability of duplicating a gene (the open genome's novelty reservoir).
    pub gene_dup_prob: Scalar,
    /// Per-birth probability of deleting a gene.
    pub gene_del_prob: Scalar,
    /// Per-birth probability of adding a fresh random gene.
    pub gene_add_prob: Scalar,

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
            bloom_event_rate: 0, // off by default — food bursts are user-triggered events
            bloom_life: 400,
            bloom_radius: 45.0,
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

            chan_cap: 1000,
            chan_decay_num: 9,
            chan_decay_den: 10,
            chan_diffuse_div: 0,
            chan_emit_base: 2,
            chan_emit_den: 255,
            chan_uptake_rate: 60,
            chan_uptake_num: 1,
            chan_uptake_den: 3,
            chan_uptake_upkeep: 2,
            chan_toxic_threshold: 400,
            chan_toxin_num: 1,
            chan_toxin_den: 20,
            chan_resist_upkeep: 2,
            chan_sense_cap: 1000,

            bond_stiffness: 0.15,
            bond_rest: 4.0,
            bond_break_factor: 2.5,

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
            gene_dup_prob: 0.03,
            gene_del_prob: 0.02,
            gene_add_prob: 0.02,

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
            ParamId::BloomEventRate => self.bloom_event_rate = raw.clamp(0, 10_000),
            ParamId::BondStiffness => self.bond_stiffness = (raw.max(0) as f32) / 1000.0,
        }
    }

    #[inline]
    pub fn cell_count(&self) -> usize {
        (self.grid_w as usize) * (self.grid_h as usize)
    }
}
