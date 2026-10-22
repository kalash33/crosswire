# Crosswire

An open-source web security testing platform in Rust, for authorised testing.
It is a pure community open-source project: no company, no paid tier, nothing gated.

**Status: pre-alpha.** The foundation library is built and tested; the proxy, API and UI are not started yet. Nothing here is usable for testing a web application today.

## Why

Professional web testers mostly use Burp Suite or Caido. Both are closed source. Crosswire aims to be the open alternative, built on one promise: **what you type is what is sent, byte for byte**, and what came back is shown exactly as received — including malformed, ambiguous and deliberately odd HTTP that compliant libraries refuse to produce. Projects never corrupt, memory stays bounded at a million requests, and team mode (later) is free and self-hosted.

The design is written up in full in [the spec](docs/superpowers/specs/2026-10-22-crosswire-design.md), including the honest competitive picture, the threat model for the tool itself, and a dated roadmap.

## Roadmap (short form)

- **alpha.1 — headless recorder** (in progress): the HTTP/1 library (done), local CA, TLS client, crash-safe project store, then a MITM proxy that records a browser session to a project directory.
- **alpha.2 — the sidecar**: history view, repeater (assisted and raw modes), request intercept, scope, browser launcher, REST API, web UI.
- **beta → v0.1.0**: match-and-replace, decoder, tags/notes, docs site, release pipeline.
- Later: intruder, HTTP/2, plugins, team mode, HTTP/3. Dates are in the spec §14.

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

The design spec and implementation plans are under `docs/superpowers/`
(`specs/2026-10-22-crosswire-design.md`, `plans/`). Architecture decisions are in `docs/adr/`.

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) (what is open now, DCO sign-off, the AI-assisted-PR rule) and [SECURITY.md](SECURITY.md) (private reporting, 72-hour acknowledgement). The project follows the Contributor Covenant ([CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)).

## Licence

Licensed under either of Apache-2.0 ([LICENSE-APACHE](LICENSE-APACHE)) or MIT
([LICENSE-MIT](LICENSE-MIT)) at your option.
