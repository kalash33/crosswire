# Security gates (spec §13)

Every security gate in the v0.1.0 definition of done (spec §13, "Security gates
in CI") is listed here with the executable test that proves it, as
`crate/tests/file.rs::test_name`. These run under `just test` (the normal
`cargo test --workspace`); there is no separate CI job.

Gates that can only be proven with the UI (Plan 5) or export tooling (Plan 6)
are marked and carry no Rust `just test` coverage yet. Rows marked **PARTIAL**
are proven in substance by prior-plan tests but are cited at file level (not
enumerated to the named test per clause) or cover the clause only in part; the
citation is being tightened and the status is conservative until then.

| # | §13 gate | Proven by | Status |
|---|----------|-----------|--------|
| 1 | route-auth: every route except the five unauthenticated ones is `401` without a credential | `crosswire-api/tests/gates.rs::route_auth_every_route_except_the_five_is_401` | proven |
| 2 | `read`-token-cannot-mutate: every send/admin route refuses a read token with `403` | `crosswire-api/tests/auth.rs::read_token_cannot_mutate` | proven |
| 3 | XSS corpus: loaded into history/repeater/intercept, nothing executes | `ui/e2e/xss-corpus.spec.ts` (history text-render surfaces + the detector self-test; `typed_tag_and_notes_payloads_render_as_text` types every corpus payload into the exchange tags and notes editor, tag payloads over the 64-char UI limit are typed as their first 64 chars; `decoder_outputs_are_text_for_every_payload` runs every payload through each decoder-pane codec in both directions and renders a JWT-shaped corpus string in the JWT view), `ui/e2e/xss-repeater-intercept.spec.ts` (repeater and intercept surfaces) | **PARTIAL** — UI e2e specs exist and drive the corpus into the rendered fields, but they run under Playwright (`just ui-e2e`), not the Rust `just test`, and the preview surface is covered separately by gate 4 |
| 4 | preview zero-egress: the preview corpus makes no outbound request | `ui/e2e/preview-zero-egress.spec.ts` (12 hostile vectors loaded into the real sandboxed iframe with the deny-all CSP and `Referrer-Policy`, zero requests reach an external host or the `localhost` API origin; the spec also pins its CSP to the server's `PREVIEW_CSP`); server side: `crosswire-api/tests/preview.rs` (HMAC token, 127.0.0.1-origin-only route, deny-all CSP, no `Set-Cookie`) and the token unit tests; iframe `sandbox=""` in `ExchangeDetail.svelte` asserted by `ui/e2e/preview.spec.ts` | proven |
| 5 | idle-egress: an idle sidecar (API open, event stream connected) makes zero upstream connections and zero DNS lookups | `crosswire-api/tests/gates.rs::idle_egress_zero_connections` (in-process); `crosswire-server/tests/gates.rs::real_serve_makes_no_outbound_connection_while_idle` (real binary, Linux `/proc` socket probe) | proven |
| 6 | CA-key-leakage: the CA private key appears in no API body, no project-dir file and not in the OpenAPI document | `crosswire-server/tests/gates.rs::ca_key_never_leaves` | proven |
| 7 | loopback-spelling relay refusal: addresses resolving to loopback/link-local/metadata ranges are refused, in every spelling | `crosswire-client/tests/gate.rs::spellings_that_resolve_to_refused_ranges_are_refused`, `::gate_refuses_after_resolution`, `::own_ports_are_refused_on_loopback_even_when_allowlisted` | proven |
| 8 | proxy origin-form refusal: an origin-form request to an unknown host is refused `400` | `crosswire-proxy/tests/front.rs::origin_form_to_unknown_host_is_refused_400` | proven |
| 9 | same-site cookie isolation: a page on another localhost port cannot read any route or the event stream | `crosswire-api/tests/gates.rs::another_localhost_port_cannot_read_any_route_or_events` | proven |
| 10 | host-421: a non-loopback `Host` is refused `421` on every route and the WS upgrade | `crosswire-api/tests/gates.rs::host_421` | proven |
| 11 | bootstrap single-use: a bootstrap token works once, then fails; it expires | `crosswire-api/tests/auth.rs::bootstrap_is_single_use_and_expires` | proven |
| 12 | non-loopback proxy-auth: a non-loopback proxy bind requires `--allow-non-loopback` and proxy auth | `crosswire-proxy/tests/config.rs::config_refuses_non_loopback_without_flag`, `::config_requires_auth_on_non_loopback`; `crosswire-proxy/tests/front.rs::non_loopback_bind_without_flag_fails`, `::proxy_auth_required_then_accepted` | proven |
| 13 | scope matrix incl. empty scope (§4.6): In/Out/Empty/HostMismatch verdicts drive history recording and the repeater's four cases | `crosswire-api/tests/gates.rs::scope_matrix_including_empty_scope` (repeater: empty/in/out + strict refusal); `crosswire-proxy/tests/intercept.rs::scope_state_is_recorded_on_history_exchanges` (history In/Out/Empty/HostMismatch); `crosswire-tools/tests/repeater.rs` (repeater verdicts and out-of-scope audit) | proven |
| 14 | redaction golden (§11.4): copy-as-curl (redacted), copy-raw and logs of a seeded project contain no seeded secret | `crosswire-server/tests/gates.rs::redaction_golden` | proven |
| 15 | bad-chain refusal and chained-upstream anchor (§11.9) | bad-chain refusal: `crosswire-client/tests/transport_tls.rs::verifies_ca_issued_leaf_and_refuses_unknown_chain` (unknown chain → `ClientError::Tls`), `::wrong_name_is_refused_under_webpki`; chained-upstream anchor: `crosswire-client/tests/transport_tls.rs::private_ca_leaf_refused_without_anchor_accepted_with_it` (same private-CA leaf refused under `webpki_roots()`, accepted under `with_extra_roots([that CA])`) | proven |
| 16 | hostile-project corpus (torn pack, zstd bomb, hostile SQLite and journal) | torn pack: `crosswire-store/tests/packs.rs::torn_tail_recovery`, `::frame_without_content_size_refused`, `::flipped_payload_byte_is_corrupt`, `::sweep_repairs_index_after_crash_between_truncate_and_cleanup`; zstd bomb: `::decompression_bomb_refused`; hostile SQLite: `crosswire-store/tests/hostile.rs::hostile_sqlite_schema_does_not_execute_on_open` (a planted trigger/view is refused on open with no code run); malformed journal: `::malformed_journal_is_skipped_without_panic_or_unbounded_alloc` (good record replays, bad records quarantined, an over-cap file size drives no allocation) and `crosswire-store/tests/project.rs::symlinked_journal_directory_is_refused` | proven |
| 17 | audit-completeness: every §9.4 audited action produces a row | `crosswire-server/tests/gates.rs::audit_completeness` | proven |
| 18 | one hostile fixture per `Limits` row with flat RSS | `crosswire-proxy/tests/limits_rss*.rs`, `crosswire-store/tests/limits_rss_store.rs` (per-row ledger below) | **proven** — every memory-relevant row has a flat-RSS fixture with the hostile input far over its cap; every counter/timeout row is bounded by a refusal, not a buffer (see "Gate 18 ledger") |

## Gate 18 ledger: flat RSS per Limits row

Each memory-relevant row drives hostile input far over its cap and asserts both
the observable refusal and that anonymous RSS stays flat (`RssAnon` on Linux,
`phys_footprint` on macOS; the RSS leg SKIPs where no probe exists, and CI-Linux
is the gate). Every flat-RSS test is `multi_thread` (or samples on its own
thread) so the sampler really runs, and none holds the attacker volume itself.

| Row | Flat-RSS test |
|---|---|
| Parse rows: head size, header count, line length, max chunks, chunk-size line, trailer size | `crosswire-proxy/tests/limits_rss_parse.rs` |
| `capture_cap` | `crosswire-proxy/tests/limits_rss_body.rs::capture_cap_flat_rss` |
| In-flight `body_budget` | `crosswire-proxy/tests/limits_rss_body.rs::inflight_budget_flat_rss` |
| Until-close response | `crosswire-proxy/tests/limits_rss.rs::rss_flat_under_large_until_close_response` |
| Pack-body decompression (64 MiB bound) | `crosswire-store/tests/limits_rss_store.rs::pack_decompression_flat_rss` |
| Store write queue (spill, journal cap, gap) | `crosswire-store/tests/limits_rss_store.rs::store_queue_flat_rss` |
| `intercept_hold` | `crosswire-proxy/tests/limits_rss_intercept.rs::intercept_hold_flat_rss` |

Rows bounded by a counter or refusal, not a buffer (no per-row RSS fixture; an
attacker cannot make these grow memory because they hold no attacker bytes or
cap their state by count):

| Row | Why it is not a memory-growth surface |
|---|---|
| Pending CONNECT | an atomic counter; past `max_pending_connect` the connection is refused at accept before any buffer exists |
| Per-IP connections | a per-peer-IP count (non-loopback only); past `max_per_client_ip` the accept is refused |
| Max connections | an `AtomicUsize` counter (`active`) checked at accept; past `max_connections` the accept is refused and the socket closed, never buffered or queued |
| SNI length | a length check on a fixed-size parsed field; over-long names are rejected, never stored |
| DNS timeout | a deadline on the lookup; holds one pending future, no attacker bytes |
| TLS-handshake timeout | a deadline per handshake; a stalled peer is dropped at the timeout |
| Handshake rejection | a per-host sliding-window counter in a bounded LRU (1,024 hosts, at most `rejections_per_window` timestamps each); reaching the threshold reports once per window and holds no attacker bytes |
| Leaf-cert LRU / mint | a fixed-capacity LRU of issued leaf certs, so the cache cannot grow past its capacity |
| Redirect-follow | a hop counter on the repeater; the chain stops at the limit and each response is capture-capped |
| WS event buffer | one bounded `broadcast` ring shared by all receivers; a receiver that falls behind gets `RecvError::Lagged`, surfaced as `Event::Lagged`, rather than growing the ring |
| `audit.jsonl` cap | rotates to one prior generation at `audit_rotate_bytes`; disk, not memory |
| Journal disk cap | `journal_cap` bounds spilled bytes on disk; past it exchanges are dropped with a gap marker (exercised in `store_queue_flat_rss`) |

Deferred: the Content-Encoding decode-view row (the 100:1 ratio bound of
spec 11.6, a derived view with a banner) is deferred. The decode view is a v0.2a
feature and is not yet implemented; its flat-RSS test lands when it ships. The
store-side pack decompression bound is covered by `pack_decompression_flat_rss`.


## Supporting gate: authentication-failure rate limiting

Not a numbered §13 row, but part of the hardening the gates assume: ten
authentication failures per peer per minute trip a `429`, and every failure is
audited. Proven end to end, including the WebSocket first-frame path (closed in
Task 6), by `crosswire-api/tests/gates.rs::auth_failures_rate_limited_and_audited`
and, for the bootstrap path, `crosswire-api/tests/auth.rs::bootstrap_rate_limited_after_10_failures_and_audited`.

## Supporting gate: corpus coverage

Every `DiagnosticCode` must be exercised by at least one golden case in
`corpus/h1/cases`. Two independent checks enforce it:
`crosswire-h1/tests/corpus.rs::every_diagnostic_code_has_a_corpus_case` (Rust,
derives the expected set from `DiagnosticCode::ALL`, so a new code without a case
fails under `just test`) and `scripts/corpus-check.py` (pairing, schema and
coverage), run in CI on the ubuntu `check` job as `just corpus-check`.

## Audited actions (used by audit-completeness)

The exact action strings the code emits, each produced once by
`audit_completeness`:

`session.bootstrap`, `auth.fail`, `token.create`, `token.revoke`, `scope.set`,
`intercept.enabled`, `intercept.rules`, `repeater.send`,
`repeater.send.out_of_scope`, `allow.add`, `cert.override.config`,
`browser.profile_reset`, `listener.bind`, `listener.stop`, `ca.generate`.

`PUT /api/v1/settings/enforce_scope_strictly` also writes an audit row (action
`enforce_scope_strictly`, result `enabled` or `disabled`). It is not in the list
above, so `audit_completeness` does not assert it.

## Notes

- The `idle-egress` gate reads a connect counter and a DNS counter added to
  `crosswire_client::Transport` (`connect_count()`, `dns_count()`); they only
  advance when the transport actually dials or resolves, so an idle run leaves
  them at zero.
- The `idle-egress` gate is scoped to the in-process repeater `Transport`, the
  single primitive every upstream dial and DNS lookup goes through. The test
  builds the API over that transport and watches its counters; it does not run
  the full `crosswire serve` binary. The test idles for about 10 seconds. The
  real-binary counterpart spawns `crosswire serve`, opens a session and the event
  stream, and asserts via `/proc/<pid>/{fd,net/*}` that the process holds no
  established non-loopback TCP connection (and no connected non-loopback UDP
  socket) over a 6 second idle window. It skips on non-Linux hosts; the probe's
  row parser is unit-tested on every host.
- The route-auth, cookie-isolation and host-421 gates walk the committed
  OpenAPI document (`crosswire_api::document()`), so a new route cannot escape
  the walk without also appearing in the document the snapshot test guards.
- `/api/v1/events` is excluded from the plain route-auth walk because it
  authenticates inside its handler (it may defer to a first-frame bearer); its
  refusals are covered by the cookie-isolation, host-421 and rate-limit gates.
