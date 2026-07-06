//! Events emitted by a tick.
//!
//! Births carry `parent` and `tick` so a phylogenetic tree can be reconstructed from a
//! replay at zero extra simulation cost. Events are pure *observation* — they never feed
//! back into selection, preserving the mechanisms-not-outcomes principle.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeathCause {
    Starved,
    OldAge,
    Killed,
    /// Eaten by a predator.
    Predated,
}

#[derive(Clone, Copy, Debug)]
pub enum Event {
    Birth {
        id: u32,
        parent: u32,
        tick: u64,
    },
    Death {
        id: u32,
        cause: DeathCause,
        tick: u64,
    },
}

#[derive(Clone, Default, Debug)]
pub struct EventBatch {
    pub events: Vec<Event>,
}

impl EventBatch {
    pub fn new() -> Self {
        EventBatch { events: Vec::new() }
    }

    pub fn births(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e, Event::Birth { .. }))
            .count()
    }

    pub fn deaths(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e, Event::Death { .. }))
            .count()
    }
}
