//! Named starting presets — tuned `WorldParams` bundles for one-click interesting starts.
//!
//! These are hand-set starting points; the `replay sweep` tool measures which corners of the
//! parameter space stay alive, diverse, and trophically layered, and the presets are tuned
//! toward that viable region.

use crate::params::WorldParams;

/// Number of presets (ids `0..COUNT`).
pub const COUNT: u32 = 4;

pub fn name(id: u32) -> &'static str {
    match id {
        1 => "Изобилие",
        2 => "Скудость",
        3 => "Хищный мир",
        _ => "Обычный",
    }
}

pub fn preset(id: u32) -> WorldParams {
    let mut p = WorldParams::default();
    match id {
        1 => {
            // Lush: plentiful food, cheap living -> big populations.
            p.field_regrow = 12;
            p.field_cap = 1500;
            p.basal_upkeep = 1;
            p.initial_population = 400;
            p.initial_energy = 300;
        }
        2 => {
            // Scarce: slow regrowth, small bites -> tense and sparse.
            p.field_regrow = 3;
            p.field_cap = 700;
            p.eat_rate = 45;
            p.initial_population = 250;
        }
        3 => {
            // Predator-friendly: rewarding, cheap hunting.
            p.field_regrow = 8;
            p.bite_amount = 90;
            p.predation_gain_num = 4;
            p.predation_gain_den = 5;
            p.predation_size_ratio = 1.1;
            p.initial_population = 400;
        }
        _ => {}
    }
    p
}
