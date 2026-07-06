# Contributing to Evolution

Thanks for wanting to build on this. The project has two hard rules that everything else
follows from — read these first.

## The two non-negotiables

1. **The engine simulates mechanisms, never scripted outcomes.** There is no `if
   is_predator` branch and there never will be. Behaviours and traits are read out of an
   evolving genome at runtime. A change that hard-codes a specific creature, role, or
   outcome will be rejected on principle — see [ADR 0001](docs/adr/0001-modding-rule.md).

2. **Determinism is sacred.** `sim-core` must produce a byte-identical world from the same
   `seed + params + command log`, on native and wasm, run after run. Anything that can
   break this is banned in the core (see below). The determinism test gate must stay green.

## Mechanical bar every PR must pass

CI runs — and your PR must pass — all of:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace          # includes the golden-replay determinism gate
```

Plus the cross-target determinism gate (native vs wasm golden hashes). If you change sim
behaviour on purpose, regenerate the golden hashes in the same PR and say so explicitly.

## Determinism rules inside `sim-core` (enforced by lints + review)

- **No** `std`/`libm` transcendentals (`sin`, `cos`, `exp`, `ln`, `tanh`, `powf`, …). Use
  the deterministic implementations in `math.rs`. Both the hot loop and development must use
  the *same* implementations.
- **No** `f32::mul_add`/FMA, `powi`, `powf` — write `x * x` etc.
- **No** `f32::min`/`f32::max` (native↔wasm NaN-semantics mismatch) — use explicit compares
  or `total_cmp`.
- **No** parallel float reductions (float add is not associative). Reduce serially in a
  canonical order, or accumulate in integer/fixed-point.
- **No** iteration over `std::HashMap`/`HashSet` in state-affecting code. Use ordered
  structures; break ties by entity id, never by memory address or insertion order.
- **No** `rand_distr` or `rng.gen::<f32>()` — samplers are hand-rolled on the deterministic
  transcendentals; `u32 → f32` uses an explicit integer trick.
- **No** wall-clock, threads, globals, or I/O in `sim-core`. The only inputs are
  `(seed, params, commands)`.
- Every mutation to the world is a `Command`. UI/tools emit commands; they never mutate
  world state directly.

## Workflow

- Fork / branch, keep PRs focused.
- Sign off your commits (**DCO**): `git commit -s`. By signing off you certify the
  [Developer Certificate of Origin](https://developercertificate.org/). We use DCO rather
  than a CLA.
- Discuss larger changes in an issue first, especially anything touching the determinism
  spine, the `Command` schema, or the snapshot format (these are wire-format-frozen).

## Ways to contribute without writing Rust

You do **not** need to write Rust to extend the world. Data-level mods — new substances,
effect archetypes, presets, sensor/actuator definitions, metabolic constants — are
versioned RON/JSON tables loaded as part of the world's params. See
[ADR 0001](docs/adr/0001-modding-rule.md).
