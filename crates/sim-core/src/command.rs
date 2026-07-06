//! The `Command` — the **single channel** through which world state may change.
//!
//! Solo UI, the sim worker, and (later) the authoritative server all mutate the world only
//! by submitting commands. This is what makes `seed + params + command log` a complete,
//! replayable description of a world, and it is the seam the multiplayer phase builds on.
//!
//! Every command carries `actor_id` (solo = 0) and a reserved `cost` field from v1 so the
//! wire format never has to break to add anti-grief accounting later. Command payloads carry
//! only raw integer/seed inputs — never host-computed floats — so the log is machine
//! independent.

use crate::params::ParamId;

/// Bumped whenever the command wire format changes.
pub const COMMAND_SCHEMA_VERSION: u16 = 1;

/// Identifies who issued a command. Solo play uses `0`.
pub type ActorId = u32;

#[derive(Clone, Debug)]
pub struct Command {
    pub actor_id: ActorId,
    /// Reserved for Phase-3 anti-grief budgeting; unused in solo play.
    pub cost: u32,
    pub kind: CommandKind,
}

#[derive(Clone, Debug)]
pub enum CommandKind {
    /// Change a tunable parameter. Applied at the start of the tick.
    SetParam { key: ParamId, raw: i64 },
    /// Add resource into a disc of grid cells (a substance "brush" — integer grid coords).
    InjectSubstance {
        cx: i32,
        cy: i32,
        radius: i32,
        amount: i64,
    },
    /// Spawn an organism at the centre of grid cell `(cx, cy)` with a default genome.
    Spawn { cx: i32, cy: i32, energy: i64 },
    /// Kill all organisms whose cell falls in the inclusive grid rectangle.
    Kill {
        cx0: i32,
        cy0: i32,
        cx1: i32,
        cy1: i32,
    },
    /// Reset the world to a fresh state with a new seed (params retained).
    Reset { seed: u64 },
}

impl Command {
    /// Construct a local (solo) command with `actor_id = 0` and no cost.
    pub fn local(kind: CommandKind) -> Self {
        Command {
            actor_id: 0,
            cost: 0,
            kind,
        }
    }
}
