# Crosswire

**The wire-exact web security proxy, in Rust. What you send is what goes on the wire; what you record is what came back, byte for byte.**

[![CI](https://github.com/kalash33/crosswire/actions/workflows/ci.yml/badge.svg)](https://github.com/kalash33/crosswire/actions/workflows/ci.yml)
[![License: Apache-2.0 OR MIT](https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue.svg)](#licence)
[![MSRV 1.88](https://img.shields.io/badge/MSRV-1.88-orange.svg)](Cargo.toml)
[![Status: alpha.1](https://img.shields.io/badge/status-0.1.0--alpha.1-yellow.svg)](CHANGELOG.md)

Crosswire is an open-source web application security testing platform for
authorised testing. Every other open proxy is built on a normalising HTTP
stack, so the bytes a tester sends and sees are not always the bytes on the
wire. Crosswire is built on a byte-exact one, which makes two things possible
that the others cannot do: fidelity-sensitive testing where malformed and
ambiguous HTTP matters, and **evidence-grade captures** that are hashed,
crash-safe and verifiable. The daily workflow (history, repeater, intercept,
scope) is built underneath that. It is a pure community project: no company
behind it, nothing gated, free self-hosted team mode when it ships.

> **Status: 0.1.0-alpha.1, a headless recorder.** The `crosswire` binary runs an
> HTTP/1.1 intercepting proxy that records a browser session into a project
> directory, byte for byte. There is no UI, repeater or intercept yet. The API
> and the project format are not frozen before v0.1.0.

## The promise

Most proxies rebuild requests from a parsed model, so what you typed is not
always what went on the wire. Crosswire is built around one rule:

- **What you send is sent byte for byte**, and what came back is recorded
  exactly as received, including malformed, ambiguous and deliberately odd
  HTTP that compliant libraries refuse to produce. Every deviation from the
  RFCs is kept as a diagnostic with the exact byte span, never silently fixed.
- **Projects never corrupt.** Anything whose `committed` event you saw survives
  a crash; every loss is bracketed by a gap marker. Verified by a kill -9 test.
- **Memory stays bounded** under hostile servers and pages: every limit has a
  default, an action and an observable.

## How it compares

Checked against GitHub on 2026-10-24; the fuller version is in the spec §2.

| Tool | What it is | Why Crosswire exists beside it |
|---|---|---|
| ZAP by Checkmarx | The established free scanner and proxy (Java) | Scanner-first; its proxy normalises HTTP |
| OpenTRAP | New Burp-style UI on the ZAP engine (Java, AGPL) | Same normalising proxy underneath |
| mitmproxy | The developer's interception tool (Python) | No tester workflow; not byte-exact |
| proxelar | Rust traffic workbench on hyper | hyper rewrites framing; no diagnostics |
| Burp Suite, Caido | The commercial workbenches | Closed source; Crosswire is the open sidecar beside them |

None of these records diagnostics with byte spans, none sends a deliberately
malformed message unchanged, and none makes a crash-safety or bounded-memory
promise. A measured comparison, the fidelity corpus run through each tool, is a
planned deliverable before the first public release.

## Quick start

Requires a Rust toolchain (1.88 or newer) on macOS or Linux.

```sh
cargo run -p crosswire-server -- serve --project ./demo
```

```
proxy listening on 127.0.0.1:8080
ca page at http://crosswire/ (via the proxy) or http://127.0.0.1:8080/
```

Point a browser's HTTP and HTTPS proxy at `127.0.0.1:8080`, then either:

- open `http://crosswire/` through the proxy, download the CA (`/ca.der` or
  `/ca.pem`) and trust it in the browser; or
- install nothing and launch Chrome or Chromium with the CA's pin:

  ```sh
  --proxy-server=127.0.0.1:8080 \
  --ignore-certificate-errors-spki-list=$(cargo run -q -p crosswire-server -- ca spki)
  ```

Everything the browser does is now recorded into `./demo`. Ctrl-C stops the
proxy and closes the project cleanly. Per-browser and per-OS instructions,
including Firefox, Android and iOS, are in [docs/ca-install.md](docs/ca-install.md);
every `serve` option is in [the server README](crates/crosswire-server/README.md).

By default the proxy refuses to reach loopback, link-local and cloud-metadata
addresses and its own port, evaluated after DNS resolution. Allow a target
with `--allow HOST:PORT`; every allowance is audited.

## What works today

| Area | alpha.1 |
|---|---|
| HTTP/1.1 relay | Byte-for-byte on both sides, keep-alive, pipelining, `Expect: 100-continue`, chunked with extensions and trailers, until-close bodies |
| TLS | Termination with leaves minted per host from a local CA (IP SANs included), SNI passthrough per host or suffix, TLS 1.2+ with rustls |
| Upstream policy | Certificates verified by default; per-host override, recorded on the exchange and audited |
| Recording | Every exchange with timing, TLS details, diagnostics, interim 1xx heads, the rewrite diffs, and bodies up to a capture cap with the full hash and length |
| Failure handling | Generated 400/502/504 pages recorded as error exchanges; timeouts; undelimitable responses and WebSocket upgrades become counted opaque tunnels |
| Project store | SQLite plus content packs, bounded writer queue with a fsynced spill journal, gap markers, audit log |
| Safety | Destination gate, connection and pending-CONNECT caps, proxy authentication on non-loopback binds, a hostile-input fixture per limit |
| Proof | 104-case golden corpus relayed whole and fragmented (208 captures, all 26 diagnostic codes), a real headless-Chrome session recorded in CI, a kill -9 durability test |

**Not there yet:** UI, repeater, intercept (holding and editing messages),
scope and auto-passthrough, a browser launcher, HTTP/2 proxying, OS keychain
integration or CA installation. See [CHANGELOG.md](CHANGELOG.md) for details.

## Roadmap

| Milestone | Contents | State |
|---|---|---|
| **alpha.1** headless recorder | h1 library, exchange schema, local CA, upstream client, crash-safe store, MITM proxy, `crosswire serve` | **Built** |
| **alpha.2** the sidecar | History view, repeater (assisted and raw), request intercept, scope, browser launcher, REST API, web UI | Next |
| **beta** daily tools | Match-and-replace, decoder, tags and notes, copy-as-curl, docs site | Planned |
| **v0.1.0** | Security gates green, release pipeline, launch | Planned |
| Later | Intruder, HTTP/2, WebSocket editing, plugins, free self-hosted team mode, HTTP/3 | Spec §14 |

The full design, including the honest competitive picture, the threat model
for the tool itself and dated estimates, is in
[the spec](docs/superpowers/specs/2026-10-22-crosswire-design.md).

## Architecture

```
            browser ──► crosswire-proxy ──► crosswire-client ──► target
                         │  (MITM, relay)      (gate, TLS)
                         │
                         ▼  ExchangeSink
                       crosswire-store  (SQLite + packs + journal + audit)

   crosswire-h1     sans-IO HTTP/1 parser and byte-exact writer
   crosswire-model  frozen exchange schema, events, redaction
   crosswire-ca     local CA and cached leaf issuer
   crosswire-core   spans, diagnostics, shared types
   crosswire-server the `crosswire` binary
```

| Crate | Responsibility | Docs |
|---|---|---|
| `crosswire-core` | Byte spans, the 26-code diagnostic catalogue, shared types; `no_std` + `alloc` | [README](crates/crosswire-core/README.md) |
| `crosswire-h1` | Sans-IO HTTP/1 parsing and byte-exact writing with spans and diagnostics for every element | [README](crates/crosswire-h1/README.md), [catalogue](docs/h1-diagnostics.md), [ADR-0001](docs/adr/0001-h1-sans-io-contract.md) |
| `crosswire-model` | The exchange schema, `ExchangeSink`, redaction on the way out | [README](crates/crosswire-model/README.md) |
| `crosswire-ca` | P-256 CA, file key store, rate-limited leaf issuer | [README](crates/crosswire-ca/README.md) |
| `crosswire-client` | Recording upstream HTTP/1.1 client, destination gate, rustls backend | [README](crates/crosswire-client/README.md) |
| `crosswire-store` | Crash-safe project store | [README](crates/crosswire-store/README.md), [ADR-0002](docs/adr/0002-store-durability.md), [format](docs/format.md) |
| `crosswire-proxy` | HTTP/1.1 MITM proxy engine | [README](crates/crosswire-proxy/README.md), [ADR-0003](docs/adr/0003-proxy-relay.md) |
| `crosswire-server` | The `crosswire` binary: `serve`, `ca` | [README](crates/crosswire-server/README.md) |

`corpus/h1` holds the golden fidelity corpus ([README](corpus/h1/README.md));
`crates/crosswire-recorder-tests` holds cross-crate integration tests.

## Developing

Commands live in the `justfile`:

| Command | What it does |
|---|---|
| `just test` | All tests, all features |
| `just lint` | Clippy with `-D warnings` and `cargo fmt --check` |
| `just features` | `cargo check` with no default features and each feature alone |
| `just browser-test` | A headless Chromium session through the proxy (set `CROSSWIRE_BROWSER` or have `google-chrome` on `PATH`) |
| `just fuzz-smoke` | 60 s of each h1 fuzz target (nightly + `cargo-fuzz`) |
| `just bench` | Builds the h1 vs `httparse` criterion benchmark |
| `just serve --project ./demo` | The quick start |

CI runs lint, tests, feature checks, docs, packaging and the browser session on
Ubuntu and macOS, plus an MSRV 1.88 check and `cargo deny`. Design decisions
are recorded in [docs/adr/](docs/adr/); the on-disk project format in
[docs/format.md](docs/format.md); the velocity log in [docs/log.md](docs/log.md).

## Contributing and security

Contributions are welcome from the beta onward in the areas listed in
[CONTRIBUTING.md](CONTRIBUTING.md) (DCO sign-off, the AI-assisted-PR rule,
which crates are design-locked until their 0.1 release). Report security
issues privately as described in [SECURITY.md](SECURITY.md) (72-hour
acknowledgement). The project follows the Contributor Covenant
([CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)).

Crosswire is for authorised testing only. Point it at systems you own or have
written permission to test.

## Licence

Licensed under either of Apache-2.0 ([LICENSE-APACHE](LICENSE-APACHE)) or MIT
([LICENSE-MIT](LICENSE-MIT)) at your option.
