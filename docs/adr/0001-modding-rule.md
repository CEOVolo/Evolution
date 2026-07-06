# ADR 0001 — Modding adds mechanisms and data, never behaviour

- **Status:** accepted
- **Date:** 2026-07-06
- **Deciders:** maintainer

## Context

Two of the project's stated goals are in apparent tension:

1. **Others must be able to participate.** The audience arriving through a browser demo is
   largely JS/TS-capable, not Rust+WASM-capable. If "participate" means "recompile the Rust
   core," most of them are locked out.
2. **The engine must never script outcomes.** The core principle is that neither the program
   nor the user knows what will emerge — behaviours are read from an evolving genome, and
   there is no `if is_predator` branch. A naïve "let modders write per-organism behaviour in
   JS" mod API would reintroduce exactly the scripted behaviour the whole design exists to
   avoid, and would also destroy determinism and the allocation-free hot loop.

## Decision

**Mods add MECHANISMS and DATA. Mods never add per-organism behaviour.** The extension
surface is layered:

### Data-level modding (available early, Phases 1–2) — no Rust required

New substances, effect archetypes (nutrient / toxin / signal / …), presets, sensor and
actuator *definitions*, and metabolic constants are expressed as **versioned RON/JSON
tables**. `sim-core` loads them as part of the world's `params` (and therefore as part of
the `seed`-anchored, snapshotted input).

Because the data is part of the deterministic input — not executable behaviour — this
preserves determinism and the anti-scripting invariant completely. A contributor can add
"digest channel 5" or a new starting preset with zero Rust and zero recompile, and the
result is still fully reproducible and shareable.

### Mechanism-level modding (later, Phase 3+) — for Rust contributors

Deeper extensions register **documented Rust traits** (e.g. `trait EffectArchetype`,
`trait Sensor`) **at startup**, not as per-tick script hooks. This keeps the hot loop
allocation-free and keeps all behaviour mechanism-shaped rather than outcome-shaped.

### Forbidden

Per-organism, per-tick scripted behaviour hooks (in any language). They break determinism,
the performance model, and the core principle simultaneously.

## Consequences

- The JS/TS browser audience can meaningfully contribute (data mods) without Rust.
- Determinism and shareability hold because mod data is part of the seeded input.
- The `params` schema and the RON/JSON mod-table formats become versioned, migratable
  contracts (like snapshots) — changing them requires versioning, not a silent edit.
- Some powerful extensions require Rust and a maintainer-reviewed PR. That is intentional:
  the mechanism set is the part that must stay principled.
