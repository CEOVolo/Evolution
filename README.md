# Evolution

An open-ended evolution sandbox: agent-based organisms with genomes evolve **from a cell
to a creature with properties**. Highly visual, live-tweakable, and shareable — you change
the starting conditions and paint substances into a living world, then watch what happens.

> **Core principle:** neither the program nor the user knows what will emerge. The engine
> simulates *mechanisms* (energy, chemistry, genome→body/behaviour expression, selection
> pressure) — never scripted roles. There is no `if is_predator` branch anywhere; every
> behaviour is read out of an evolving genome at runtime.

## Status

Early **Phase 0** — building the deterministic simulation core. Not yet usable.

## Architecture (one core, two hosts)

The simulation lives in a single deterministic Rust crate, [`sim-core`](crates/sim-core),
compiled two ways from the same source:

- **wasm** ([`crates/wasm`](crates/wasm)) — runs solo in the browser (the MVP host).
- **native server** ([`crates/server`](crates/server)) — authoritative host for shared
  online worlds (a later phase).

`sim-core` is a pure function of state: `tick(state, &[Command]) -> (state', events)`.
Given the same `seed + params + command log`, it produces a byte-identical world on every
machine. That determinism is the foundation of replay tests, shareable worlds, and netcode.

See the full plan in the project design docs. Roadmap: **Phase 0** deterministic skeleton →
**0.5** ecology calibration → **1** gorgeous single-world browser MVP → **2** shareable
snapshots → **3** shared multiplayer worlds → **4** community & gallery.

## Building

Requires the Rust toolchain (see [`rust-toolchain.toml`](rust-toolchain.toml)).

```sh
cargo build                 # build the native crates
cargo test                  # run the determinism gate + unit tests
cargo run -p replay -- --seed 1 --ticks 1000   # headless deterministic replay
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) and [GOVERNANCE.md](GOVERNANCE.md). Mods add
**mechanisms and data, never per-organism behaviour** — see
[docs/adr/0001-modding-rule.md](docs/adr/0001-modding-rule.md).

## License

[Apache-2.0](LICENSE).
