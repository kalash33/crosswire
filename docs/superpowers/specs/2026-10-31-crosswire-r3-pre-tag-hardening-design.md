# R3: pre-tag hardening (destination gate ranges + must-fix items before v0.1.0)

**Status:** approved 2026-11-01. **Parent:** 0.1.0 release tail (R1 schema freeze ✓, R2 release pipeline ✓, U1 UI ✓ → **R3** → R4 docs/launch → `v0.1.0`). **Product spec:** `docs/superpowers/specs/2026-10-22-crosswire-design.md` §11.2/line 383 (destination gate), §13 gate 7 (`docs/security-gates.md:22`).

## 1. Goal
Close the known gaps that would be embarrassing or unsafe in a public 0.1.0: SSRF-class holes in the destination gate, a release that never gathers planner statistics, an API route still emitting Rust `Debug` names, a default shortcut the browser owns, and five small UI inconsistencies the U1 review flagged. No new dependency, no wire-format change beyond one additive API field.

## 2. Facts (recon at 975aa54)
- `crates/crosswire-client/src/gate.rs` refuses loopback/unspecified (exact `0.0.0.0`/`::`), link-local (`169.254/16`, `fe80::/10`), three exact metadata addresses, and own ports only on loopback. Canonicalisation handles only `::ffff:a.b.c.d`. An **allowlisted (host, port) skips every check** (`:113-115`). An empty resolution is reported as `Loopback(0.0.0.0)` (`:105-107`). Not refused: NAT64 `64:ff9b::/96`, 6to4 `2002::/16`, IPv4-compatible `::/96`, Teredo `2001::/32`, `0.0.0.0/8`, multicast `224/4` and `ff00::/8`, broadcast, GCP `fd20:ce::254` and `metadata.google.internal`.
- `serve.rs:179` builds the proxy gate with only the proxy port (the API binds later, `:253`); the repeater gate has **no own ports** (`:232-235`).
- Store: `ANALYZE` lives only in dead migration arms (`schema.rs:63,103`); `PRAGMA optimize` nowhere; clean close is `Store::close → Stop::Clean → writer finish → checkpoint(TRUNCATE)` (`writer.rs:247-293`). `tests/indexes.rs:344-402` runs `ANALYZE` by hand.
- `routes/repeater.rs:102-107`: `op: format!("{:?}", …)`, `reason: format!("{:?}", …)`; no schema; UI only displays the strings (`assisted.ts:85`), types.ts documents fictional values.
- Keymap defaults (`keymap.ts:34-50`): `toggle-intercept = ctrl+t` — Chrome/Firefox reserve Ctrl+T, Ctrl+N, Ctrl+W (cannot be prevented); Ctrl+R/D/F can be prevented and the keymap already does. Hard-coded chord hints at `RepeaterResponse.svelte:152`, `ExchangeDetail.svelte:375`, `RepeaterTab.svelte:118,133`, `Intercept.svelte:162-173`.
- UI nits: `StatusBar.svelte:91` renders "Intruder {counts.intruder}" (no Intruder tool); `Scope.svelte:26-28` hint "Current server state unknown; the API has no read" because `GET /api/v1/scope` does not return `enforce_strict`; `DecoderPane.svelte:118-121` Back (Esc) button on a rail page; `Intercept.svelte:188` ByteEditor shows its own "Copy raw"; `RepeaterResponse.svelte:208-216` re-implements the hex pager instead of `HexWindow`.

## 3. Decisions

### D1 Destination gate: canonicalise, then refuse (crosswire-client `gate.rs`)
Every resolved address is first **canonicalised** to the innermost IPv4 when it is a transition form: IPv4-mapped `::ffff:a.b.c.d`, IPv4-compatible `::a.b.c.d` (`::/96`, excluding `::` and `::1`), NAT64 `64:ff9b::/96` (last 32 bits), 6to4 `2002:AABB:CCDD::/16` (bits 16–47), Teredo `2001:0000::/32` (last 32 bits XOR `0xffffffff`). Rules then run on the canonical address and, for IPv6, on the original too.
Refused, in every spelling, **not allowlistable**: unspecified `0.0.0.0/8` and `::`; multicast `224.0.0.0/4`, `ff00::/8`; broadcast `255.255.255.255`; cloud metadata `169.254.169.254`, `100.100.100.200`, `fd00:ec2::254`, `fd20:ce::254`; own listener ports (D3).
Refused, **allowlistable per (host, port)**: loopback `127.0.0.0/8`, `::1`; link-local `169.254.0.0/16` (minus the metadata address, which stays refused), `fe80::/10`.
Allowed by default (legitimate internal targets): RFC1918, CGNAT `100.64.0.0/10`, ULA `fc00::/7` (minus the metadata addresses), `198.18.0.0/15`.
**Names:** before resolution, `metadata.google.internal` and the bare label `metadata` (case-insensitive, trailing dot stripped) are refused as `GateRefusal::MetadataName(String)`; not allowlistable.
`GateRefusal` gains `Unresolved` (empty resolution), `Multicast(IpAddr)`, `Broadcast(IpAddr)`, `Unspecified(IpAddr)`, `MetadataName(String)`; Display texts: "no addresses resolved", "multicast address {0}", "broadcast address {0}", "unspecified address {0}", "cloud metadata host name {0}". `ClientError::Refused` wrapping unchanged.

### D2 Allowlist semantics
An allowlist entry exempts only the allowlistable classes above. The check order per address: canonicalise → non-allowlistable refusals (always) → own-port rule (always) → allowlistable refusals unless `(host, port)` is allowlisted. So `--allow metadata.google.internal:80`, `--allow evil.example:80` resolving to `169.254.169.254`, or `--allow 127.0.0.1:<own port>` are all refused.

### D3 Own listener ports on every local address, without a dependency
`DestinationGate::new(own_ports)` keeps the port list. A destination with `port ∈ own_ports` is refused as `OwnListener` when the canonical IP is loopback or unspecified **or is a local interface address**, determined by `std::net::UdpSocket::bind((ip, 0))` succeeding (binding a UDP socket to a specific IP only succeeds for an address the host owns; the socket is dropped immediately, nothing is sent). The probe runs only when the port matches, so it costs nothing on normal traffic. `serve.rs` binds the API listener **before** building the gates and passes `vec![proxy_port, api_port]` to both the proxy gate and the repeater gate. Test harnesses keep `Vec::new()`.

### D4 Planner statistics (crosswire-store)
On **clean close**, after the final commit and before the `TRUNCATE` checkpoint, the writer runs `ANALYZE` when `sqlite_stat1` has no row for `exchanges` or when the session inserted ≥ `analyze_threshold` exchanges (default 1000; `StoreOptions.analyze_threshold` for tests), otherwise `PRAGMA optimize`. Fresh projects therefore get statistics at their first clean close with real data. `tests/indexes.rs` drops the manual `analyse()` and instead seeds, closes cleanly with a threshold of 100, reopens, and asserts the `idx_exchanges_resp_len` plan. Dead migration code stays (it compiles, it is documented as historical; removal lands with the first real 6→7 migration).

### D5 Repeater `ChangeDto` emits the frozen codes (crosswire-api)
`op` becomes the serde code (`insert`/`delete`/`replace`), `reason` becomes `serde_json::Value` as the frozen serde form (`"hop-by-hop"` or `{"intruder-position":3}`), produced with `serde_json::to_value` of the core types so the API can never drift from the model. `ChangeDto` gets `ToSchema` and the `send_request` 200 response gets a `SendOutcomeDto` body schema (additive; snapshot regenerated). `docs/api.md` documents the shape. UI: `types.ts` mirrors it; `assisted.ts` `renderChanges` shows `op @ start‥end` with the reason label = the string or the object's single key; tests updated to the real values.

### D6 Shortcuts a browser cannot steal
`toggle-intercept` moves from `ctrl+t` to **`ctrl+i`** (no browser default in Chrome/Firefox/Safari; the keymap already prevents default). All other defaults stay (Ctrl+R/D/F are preventable and the keymap prevents them; Ctrl+Space is documented as conflicting with macOS input-source switching when that system shortcut is enabled). Every hard-coded chord hint is replaced by `chordFor(action)` so the UI can never disagree with the keymap; `settings.spec` asserts `Ctrl+I`. Persistent remapping stays 0.1.x.

### D7 UI consistency nits
Status bar drops the Intruder count (the API field stays). `GET /api/v1/scope` gains `enforce_strict: bool` (additive; snapshot regenerated); the UI reads it and the "server state unknown" hint disappears. Decoder route: no Back button and Escape does nothing on the page. Intercept: `showCopyRaw={false}` on the editor, Copy raw moves into the Forward/Drop toolbar. Repeater response Hex tab uses `HexWindow`. XSS corpus gains the Hex tab and an expanded diagnostics group with hostile data. `perf-scroll` is run once (report-only) and its numbers recorded in the PR body.

## 4. Out of scope
Persistent keymap remapping UI; removing the dead migration code; `ANALYZE` scheduling during long sessions (optimize-on-close is enough for 0.1); a project name in the top bar; the bulk-tag/detail-save race (narrow, documented).

## 5. Constraints carried
No new dependency; `unsafe_code = "forbid"`; MSRV 1.88; `cargo build` Node-free (bundle rebuilt and committed, `ui-dist-check` clean); `cargo deny` clean; §11.1 rules untouched; full CI gate before push; Conventional Commits ≤72; no AI attribution. The §13 gate-7 row in `docs/security-gates.md` is updated to list the new ranges and tests and stays "proven".

## 6. Risks
- The UDP-bind probe could misbehave on a host that forbids unprivileged UDP binds (rare; failure = "not local", i.e. the old behaviour, never a false refusal).
- Teredo decoding is rarely exercised; the test fixes the vectors from RFC 4380 §4.
- The `ANALYZE` on close adds milliseconds per clean close at 1k+ rows; measured in the store test log.
