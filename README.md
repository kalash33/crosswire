# Crosswire

An open-source web security testing platform in Rust, for authorised testing.
It is a pure community open-source project.

## What exists today

Two library crates and a test corpus. There is no proxy, no UI and no CLI yet.

- `crates/crosswire-core`: shared types (spans, diagnostics, profiles, change
  records). `bytes` is its only required dependency; `no_std` + `alloc` without
  the `std` feature. See [its README](crates/crosswire-core/README.md).
- `crates/crosswire-h1`: sans-IO HTTP/1 parsing and byte-exact writing, with
  diagnostics and spans for every element. See [its README](crates/crosswire-h1/README.md)
  and the [diagnostic catalogue](docs/h1-diagnostics.md).
- `corpus/h1`: the golden fidelity corpus used by the h1 tests; see
  [corpus/h1/README.md](corpus/h1/README.md).

## Layout

```
crates/crosswire-core/   shared types
crates/crosswire-h1/     HTTP/1 library (tests/, benches/, fuzz/)
corpus/h1/               golden corpus (cases/*.msg + *.expect.json)
scripts/                 corpus coverage checker
docs/                    design spec and plan (superpowers/), ADRs (adr/),
                         diagnostics catalogue, velocity log (log.md)
```

## Working on it

Commands are in the `justfile`:

- `just test`: all tests with all features.
- `just features`: `cargo check` with no default features and with each feature alone.
- `just fuzz-smoke`: 60 s each of the `parse_request` and `parse_response` fuzz
  targets (needs nightly and `cargo-fuzz`).
- `just bench`: builds the criterion benchmark (h1 against `httparse`) without
  running it; run `cargo bench -p crosswire-h1` to measure.
- `just lint`: clippy with `-D warnings` and `cargo fmt --check`.

The design spec and implementation plan are under `docs/superpowers/`
(`specs/2026-10-22-crosswire-design.md`).

## Licence

Licensed under either of Apache-2.0 ([LICENSE-APACHE](LICENSE-APACHE)) or MIT
([LICENSE-MIT](LICENSE-MIT)) at your option.
