# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

The first release, `0.1.0-alpha.1`: a headless recorder. A browser pointed at
`crosswire serve` is recorded byte for byte into a project directory. There is
no UI yet. The minimum supported Rust version is 1.88.

### Added

- **crosswire-core**: shared types (byte spans, the 26-code
  `DiagnosticCode` catalogue, profiles, change records); `no_std` + `alloc`
  without the `std` feature.
- **crosswire-h1**: sans-IO HTTP/1 parser and byte-exact writer with spans and
  diagnostics for every element, framing resolution (`Ambiguous` with the rule
  that resolved it), parser limits, a split-feed proptest and fuzz targets.
- **crosswire-model**: the exchange schema (`Exchange`, `Message`, `Head`,
  `Body`, `Event`, `Marker`), the `ExchangeSink` trait, redaction on the way
  out, and additive serde (old JSON still reads).
- **crosswire-ca**: a local P-256 certificate authority with a file key store
  and a cached, rate-limited leaf issuer (IP SANs, SNI up to 253 bytes).
- **crosswire-client**: a recording upstream HTTP/1.1 client with a destination
  gate (loopback, link-local and cloud-metadata refused unless allowed) and a
  rustls backend with per-host certificate overrides.
- **crosswire-store**: the crash-safe project store: SQLite plus content packs,
  a bounded writer queue with a fsynced spill journal, gap markers for every
  loss, an audit log, and a kill -9 durability test.
- **crosswire-proxy**: the HTTP/1.1 MITM proxy engine: a byte-for-byte relay
  through the h1 parser, TLS termination with minted leaves, SNI passthrough,
  bounded body capture under a shared in-flight budget, a recorder thread
  behind a bounded queue, generated 400/502/504 responses recorded as `error`
  exchanges, opaque tunnels for WebSockets, `CONNECT` answers and anything
  that cannot be delimited, hostile `Limits` fixtures, and a corpus golden of
  208 recorded captures (104 corpus cases, each written whole and fragmented)
  covering all 26 diagnostic codes.
- **crosswire-server**: the `crosswire` binary: `crosswire serve` (proxy into a
  project, audited `--allow` and `--accept-invalid-cert`, proxy auth for
  non-loopback binds, a CA directory kept outside the project) and
  `crosswire ca` (`export`, `path`, `spki`), plus a CA info page at
  `http://crosswire/`.
- **crosswire-recorder-tests**: integration tests across the recorder crates
  and the reference client-side recording recipe.
- **corpus/h1**: the golden HTTP/1 fidelity corpus with expectations.
- **docs**: the project format (`docs/format.md`), ADR-0001 to ADR-0003, the
  CA install guide and the h1 diagnostics catalogue.

[Unreleased]: https://github.com/kalash33/crosswire/commits/main
