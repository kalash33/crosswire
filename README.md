# Crosswire

An open-source web security testing platform in Rust, for authorised testing.
It is a pure community open-source project: no company, no paid tier, nothing gated.

**Status: pre-alpha.** The foundation libraries are built and tested; the proxy, API and UI are not started yet. Nothing here is usable for testing a web application today.

## Why

Professional web testers mostly use Burp Suite or Caido. Both are closed source. Crosswire aims to be the open alternative, built on one promise: **what you type is what is sent, byte for byte**, and what came back is shown exactly as received — including malformed, ambiguous and deliberately odd HTTP that compliant libraries refuse to produce. Projects never corrupt, memory stays bounded at a million requests, and team mode (later) is free and self-hosted.

The design is written up in full in [the spec](docs/superpowers/specs/2026-10-22-crosswire-design.md), including the honest competitive picture, the threat model for the tool itself, and a dated roadmap.

## Roadmap (short form)

- **alpha.1 — headless recorder** (in progress): done so far are the HTTP/1 library (`crosswire-h1`), the exchange schema (`crosswire-model`), the local CA (`crosswire-ca`), the upstream client (`crosswire-client`) and the crash-safe project store (`crosswire-store`). Next is the MITM proxy that records a browser session to a project directory.
- **alpha.2 — the sidecar**: history view, repeater (assisted and raw modes), request intercept, scope, browser launcher, REST API, web UI.
- **beta → v0.1.0**: match-and-replace, decoder, tags/notes, docs site, release pipeline.
- Later: intruder, HTTP/2, plugins, team mode, HTTP/3. Dates are in the spec §14.

## What exists today

Library crates, one integration-test crate and a test corpus. There is no proxy, no UI and no CLI yet.

- `crates/crosswire-core`: shared types (spans, diagnostics, profiles, change
  records). `bytes` is its only required dependency; `no_std` + `alloc` without
  the `std` feature. See [its README](crates/crosswire-core/README.md).
- `crates/crosswire-h1`: sans-IO HTTP/1 parsing and byte-exact writing, with
  diagnostics and spans for every element. See [its README](crates/crosswire-h1/README.md)
  and the [diagnostic catalogue](docs/h1-diagnostics.md).
- `crates/crosswire-model`: the exchange schema (`Exchange`, `Message`, `Event`,
  markers), the `ExchangeSink` trait and redaction. See
  [its README](crates/crosswire-model/README.md).
- `crates/crosswire-ca`: a local P-256 certificate authority and a cached,
  rate-limited leaf issuer; no OS trust installation. See
  [its README](crates/crosswire-ca/README.md).
- `crates/crosswire-client`: recording upstream HTTP/1.1 client with a
  destination gate and a rustls TLS backend; no proxy support, no HTTP/2. See
  [its README](crates/crosswire-client/README.md).
- `crates/crosswire-store`: the crash-safe project store (SQLite plus content
  packs, spill journal, audit log). See [its README](crates/crosswire-store/README.md)
  and [ADR-0002](docs/adr/0002-store-durability.md).
- `crates/crosswire-recorder-tests`: integration tests across the recorder
  crates (fetch over TLS and record into a project) and the `record_exchange`
  recipe the proxy will copy.
- `corpus/h1`: the golden fidelity corpus used by the h1 tests; see
  [corpus/h1/README.md](corpus/h1/README.md).

## Layout

```
crates/crosswire-core/   shared types
crates/crosswire-h1/     HTTP/1 library (tests/, benches/, fuzz/)
crates/crosswire-model/  exchange schema, sink trait, redaction
crates/crosswire-ca/     local CA and leaf issuer
crates/crosswire-client/ upstream HTTP/1.1 client, destination gate, TLS
crates/crosswire-store/  project store (tests/ includes the kill -9 test)
crates/crosswire-recorder-tests/  integration tests across the recorder crates
corpus/h1/               golden corpus (cases/*.msg + *.expect.json)
scripts/                 corpus coverage checker
docs/                    design spec and plans (superpowers/), ADRs (adr/),
                         project format (format.md), diagnostics catalogue,
                         velocity log (log.md)
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
(`specs/2026-10-22-crosswire-design.md`, `plans/`). Architecture decisions are in `docs/adr/`:
[ADR-0001](docs/adr/0001-h1-sans-io-contract.md) (the h1 sans-IO contract) and
[ADR-0002](docs/adr/0002-store-durability.md) (store durability). The on-disk
project format is in [docs/format.md](docs/format.md).

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) (what is open now, DCO sign-off, the AI-assisted-PR rule) and [SECURITY.md](SECURITY.md) (private reporting, 72-hour acknowledgement). The project follows the Contributor Covenant ([CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)).

## Licence

Licensed under either of Apache-2.0 ([LICENSE-APACHE](LICENSE-APACHE)) or MIT
([LICENSE-MIT](LICENSE-MIT)) at your option.
