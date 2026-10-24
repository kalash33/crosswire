# Crosswire

An open-source web security testing platform in Rust, for authorised testing.
It is a pure community open-source project: no company, no paid tier, nothing gated.

**Status: pre-alpha, at alpha.1: a usable headless recorder.** The `crosswire` binary runs an HTTP/1.1 intercepting proxy that records a browser session into a project directory, byte for byte. There is no UI, no repeater and no intercept yet, so it records but does not let you edit or replay. The API is not frozen and the project format may still change before v0.1.0.

## Quick start

You need a Rust toolchain (see `rust-toolchain.toml`) and a unix machine (macOS or Linux).

```
cargo run -p crosswire-server -- serve --project ./demo
```

That creates `./demo` as a project, generates a CA on first run, and prints:

```
proxy listening on 127.0.0.1:8080
ca page at http://crosswire/ (via the proxy) or http://127.0.0.1:8080/
```

Point a browser's HTTP and HTTPS proxy at `127.0.0.1:8080`, then either:

- open `http://crosswire/` through the proxy and download the CA (`/ca.der` or `/ca.pem`), and trust it in the browser; or
- skip installing anything and launch Chrome or Chromium with the CA's pin: `--proxy-server=127.0.0.1:8080 --ignore-certificate-errors-spki-list=$(cargo run -q -p crosswire-server -- ca spki)`.

Step-by-step instructions for each browser and operating system, including Firefox, Android and iOS, are in [docs/ca-install.md](docs/ca-install.md). Ctrl-C stops the proxy and closes the project cleanly. By default the proxy refuses to reach loopback, link-local and cloud-metadata addresses and its own port; allow a target with `--allow HOST:PORT`. The options are listed in [the server README](crates/crosswire-server/README.md).

**Not there yet:** no UI, no repeater, no intercept (holding and editing messages), no scope, and no `crosswire browser` launcher. Auto-passthrough for sites that reject the CA is off; use `--passthrough HOST`. HTTP/2 is not proxied. The CA is a plain file store with no OS keychain integration, and nothing installs or removes it for you.

## Why

Professional web testers mostly use Burp Suite or Caido. Both are closed source. Crosswire aims to be the open alternative, built on one promise: **what you type is what is sent, byte for byte**, and what came back is shown exactly as received — including malformed, ambiguous and deliberately odd HTTP that compliant libraries refuse to produce. Projects never corrupt, memory stays bounded at a million requests, and team mode (later) is free and self-hosted.

The design is written up in full in [the spec](docs/superpowers/specs/2026-10-22-crosswire-design.md), including the honest competitive picture, the threat model for the tool itself, and a dated roadmap.

## Roadmap (short form)

- **alpha.1 — headless recorder** (built, except the items deferred in spec §17): the HTTP/1 library (`crosswire-h1`), the exchange schema (`crosswire-model`), the local CA (`crosswire-ca`), the upstream client (`crosswire-client`), the crash-safe project store (`crosswire-store`), the MITM proxy (`crosswire-proxy`) and the `crosswire` binary (`crosswire-server`) that records a browser session to a project directory. Changes are listed in [CHANGELOG.md](CHANGELOG.md).
- **alpha.2 — the sidecar** (next): history view, repeater (assisted and raw modes), request intercept, scope (and with it auto-passthrough), browser launcher, REST API, web UI.
- **beta → v0.1.0**: match-and-replace, decoder, tags/notes, docs site, release pipeline.
- Later: intruder, HTTP/2, plugins, team mode, HTTP/3. Dates are in the spec §14.

## What exists today

Library crates, the proxy engine, the `crosswire` binary, one integration-test crate and a test corpus. There is no UI yet.

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
- `crates/crosswire-proxy`: the HTTP/1.1 MITM proxy engine (byte-for-byte relay,
  TLS termination, SNI passthrough, bounded capture, failure handling). See
  [its README](crates/crosswire-proxy/README.md) and
  [ADR-0003](docs/adr/0003-proxy-relay.md).
- `crates/crosswire-server`: the `crosswire` binary (`serve`, `ca`). See
  [its README](crates/crosswire-server/README.md).
- `crates/crosswire-recorder-tests`: integration tests across the recorder
  crates (fetch over TLS and record into a project) and the `record_exchange`
  client-side recording recipe (the repeater's, not the proxy's).
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
crates/crosswire-proxy/  MITM proxy engine (tests/ includes the corpus golden and a Chromium session)
crates/crosswire-server/ the `crosswire` binary
crates/crosswire-recorder-tests/  integration tests across the recorder crates
corpus/h1/               golden corpus (cases/*.msg + *.expect.json)
scripts/                 corpus coverage checker
docs/                    design spec and plans (superpowers/), ADRs (adr/),
                         project format (format.md), CA install guide
                         (ca-install.md), diagnostics catalogue, velocity log
                         (log.md)
```

## Working on it

Commands are in the `justfile`:

- `just test`: all tests with all features.
- `just browser-test`: a headless Chromium session through the proxy (needs a
  Chromium-based browser: set `CROSSWIRE_BROWSER` or have `google-chrome` or
  `chromium` on `PATH`).
- `just serve --project ./demo`: the quick start above.
- `just features`: `cargo check` with no default features and with each feature alone.
- `just fuzz-smoke`: 60 s each of the `parse_request` and `parse_response` fuzz
  targets (needs nightly and `cargo-fuzz`).
- `just bench`: builds the criterion benchmark (h1 against `httparse`) without
  running it; run `cargo bench -p crosswire-h1` to measure.
- `just lint`: clippy with `-D warnings` and `cargo fmt --check`.

The design spec and implementation plans are under `docs/superpowers/`
(`specs/2026-10-22-crosswire-design.md`, `plans/`). Architecture decisions are in `docs/adr/`:
[ADR-0001](docs/adr/0001-h1-sans-io-contract.md) (the h1 sans-IO contract),
[ADR-0002](docs/adr/0002-store-durability.md) (store durability) and
[ADR-0003](docs/adr/0003-proxy-relay.md) (the proxy relay). The on-disk
project format is in [docs/format.md](docs/format.md).

## Contributing and security

See [CONTRIBUTING.md](CONTRIBUTING.md) (what is open now, DCO sign-off, the AI-assisted-PR rule) and [SECURITY.md](SECURITY.md) (private reporting, 72-hour acknowledgement). The project follows the Contributor Covenant ([CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)).

## Licence

Licensed under either of Apache-2.0 ([LICENSE-APACHE](LICENSE-APACHE)) or MIT
([LICENSE-MIT](LICENSE-MIT)) at your option.
