# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### R1: schema freeze

#### Changed

- **Breaking, wire format** (`SCHEMA_VERSION` 2): enum codes are frozen
  (kebab-case or snake_case, no PascalCase), tagged unions use a `type` tag, and
  defaults are fixed. Unknown enum codes read back as an `Unknown` variant
  instead of failing, so a newer project stays readable. `cert_override` is no
  longer on the wire.
- **Breaking, project format 6**: a fresh project is created at `user_version`
  6. Projects written by pre-release builds (`user_version` 1 to 5) are refused
  on open, before any migration or snapshot; re-record the traffic.

#### Added

- **crosswire-api**: every gated response carries `Crosswire-Schema-Version: 2`,
  including the gate's own `401`, `421` and `404` refusals. `GET /api/v1/health`
  adds `schema_version` and `format_version` (`crosswire-store` exports
  `FORMAT_VERSION`).

### alpha.2: the sidecar backend (no UI yet)

The backend of `0.1.0-alpha.2`. Everything the web UI will drive exists and is
tested; the UI itself (Plan 5) does not. The API and the project format are not
frozen. Nothing here phones home: the launcher opens only the proxy's own CA
page or the URL you pass, and an idle sidecar makes no upstream connection
(the idle-egress gate).

#### Added

- **crosswire-h1**: `apply_edits` with re-parse (`Edit`, `Edited`, `EditError`),
  plus the `content_length_edit` and `crlf_edits` helpers. An edit addresses
  `head ++ body` and every change is returned as a `Change` in the new bytes.
- **crosswire-scope** (new): the scope rule grammar (exact, suffix, regex, IP and
  CIDR hosts, with ports, scheme and path), a precompiled evaluator with
  include and exclude precedence, two-identity evaluation (`PairVerdict` with
  `HostMismatch`), a program-list importer, and a JSON form pinned by tests.
- **crosswire-store**: schema version 2 (`0002_sidecar.sql`: `settings`,
  `tokens`, `drafts`), a migration that snapshots `project.db` to
  `project.db.v1.bak` first, a control connection (`Store::control`) for
  settings, API tokens, repeater drafts and editing an exchange's tags and notes,
  new `ExchangeFilter` fields (`scope_state`, `tag`) and `Counts.by_namespace`.
- **crosswire-tools** (new): the repeater (raw and assisted modes, a scope
  check on the connection and message identity, strict enforcement, redirects
  up to ten hops, drafts, an audit hook) and the shared recording recipe moved
  out of `crosswire-recorder-tests`.
- **crosswire-proxy**: request intercept (`Intercept`, `InterceptRules`):
  hold, edit and forward, drop and forward-all, skip rules (extensions and
  out-of-scope traffic by default), a 4 MiB editable-body limit, a hold timeout,
  a hold queue with an overflow policy, held bodies charged to the in-flight
  budget. `Handler` gained an asynchronous `on_request` and the `Modify` and
  `Respond504` decisions. The scope verdict is recorded on every exchange.
- **crosswire-api** (new): the REST and WebSocket API (exchanges with keyset
  paging and filters, bodies, markers, counts, scope, repeater, intercept,
  tokens, an event stream with sequence numbers and `lagged` frames) with a
  committed OpenAPI document (`docs/api/openapi.json`). Credentials: a
  single-use 60-second bootstrap token exchanged for a `cw_sid_<port>` session
  cookie (`HttpOnly`, `SameSite=Strict`, 30-minute idle expiry), and `read`,
  `send` and `admin` tokens hashed at rest. Cookie requests must be same-origin
  (`Sec-Fetch-Site` or an exact `Origin`), the `Host` must be
  `localhost` or `127.0.0.1` (else `421`), no CORS header is ever sent,
  authentication failures are rate-limited (10 per minute per peer) and audited,
  and the WebSocket can authenticate by a first-frame bearer.
- **crosswire-server**: `crosswire serve` now binds a separate loopback API
  listener (`--api-bind`, default `127.0.0.1:8081`) and delivers the bootstrap
  URL once on stdout or to a 0600 file (`--bootstrap-file`); scope and
  `--enforce-scope-strictly` persist in the project; `crosswire token`
  (`create`, `list`, `revoke`, `rotate`); `crosswire browser` (Chrome, Chromium,
  Edge or Brave with a 0700 `browser-profile/` under the project, exit code 3
  when none is found) and `crosswire browser reset-profile`.
- **Tests and tooling**: the section 13 security gates that can be proven
  without a UI, each with a named test (`docs/security-gates.md`), an audit
  completeness test over every audited action, and
  `scripts/first-capture.sh` (`just first-capture`, run in CI on Ubuntu), which
  times build-to-first-recorded-exchange through a real headless browser under
  120 seconds, with a browserless Rust counterpart that runs in every test run.
- **docs**: the API guide (`docs/api.md`), ADR-0004 (API credentials), the
  security-gates map, and `docs/format.md` for schema version 2.

#### Changed

- `Handler` and `ProxyDeps` changed shape (an async `on_request`, a `holds`
  method, and a `scope` in `ProxyDeps`); `crosswire_client::Transport` gained
  `connect_count` and `dns_count` for the idle-egress gate.
- Projects at schema version 1 are migrated to 2 on open. A project written by
  this version cannot be opened by an alpha.1 binary.

#### Not yet built

- The web UI (Plan 5) and the three gates that need it or the export tooling:
  the XSS corpus, preview zero-egress and the redaction golden.
- Response intercept (on the spec section 14 cut list), and persistence of the
  intercept rules (intercept always starts disabled with default rules).
- OS-keychain storage, CA installation, pack-writer streaming, and the
  1,000,000-exchange memory check (Plan 6).

### alpha.1: a headless recorder

The first release, `0.1.0-alpha.1`: a headless recorder. A browser pointed at
`crosswire serve` is recorded byte for byte into a project directory. There is
no UI yet. The minimum supported Rust version is 1.88.

#### Added

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
