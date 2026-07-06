# Governance

Evolution is in its early single-maintainer phase. This document says how decisions get
made so contributors know what to expect; it will grow as the project does.

## Roles

- **Maintainer** — currently the project founder. Sets direction, reviews and merges PRs,
  owns releases, and is the tie-breaker on design disputes.
- **Contributors** — anyone who submits a PR, issue, or data-level mod under the
  [DCO](CONTRIBUTING.md).

## How changes get in

1. Non-trivial changes start as an issue or discussion.
2. PRs must pass the mechanical bar (`fmt`, `clippy -D warnings`, `cargo test` including the
   determinism gate) — this is non-negotiable and applies to the maintainer too.
3. The maintainer reviews for correctness, the two non-negotiables (mechanisms-not-roles,
   determinism), and fit with the roadmap, then merges.

## Load-bearing decisions require an ADR

Anything that touches a frozen or foundational contract — the determinism strategy, the
`Command` schema, the snapshot/wire format, the modding boundary, the license — is recorded
as an Architecture Decision Record in [`docs/adr/`](docs/adr/). Changing one of these means
writing (or superseding) an ADR, not just a PR.

## License & contributions

Code is [Apache-2.0](LICENSE). Contributions are accepted under the same license via the
DCO sign-off. There is no CLA. Relicensing would require consent of contributors, which is
exactly why the license is fixed early.
