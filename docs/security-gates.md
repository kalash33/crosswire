# Security gates (spec §13)

Every security gate in the v0.1.0 definition of done (spec §13, "Security gates
in CI") is listed here with the executable test that proves it, as
`crate/tests/file.rs::test_name`. These run under `just test` (the normal
`cargo test --workspace`); there is no separate CI job.

Gates that can only be proven with the UI (Plan 5) or export tooling (Plan 6)
are marked and carry no backend test yet.

| # | §13 gate | Proven by | Status |
|---|----------|-----------|--------|
| 1 | route-auth: every route except the five unauthenticated ones is `401` without a credential | `crosswire-api/tests/gates.rs::route_auth_every_route_except_the_five_is_401` | proven |
| 2 | `read`-token-cannot-mutate: every send/admin route refuses a read token with `403` | `crosswire-api/tests/auth.rs::read_token_cannot_mutate` | proven |
| 3 | XSS corpus: loaded into history/repeater/intercept, nothing executes | — | **Plan 5** (UI) |
| 4 | preview zero-egress: the preview corpus makes no outbound request | — | **Plan 5/6** (UI previews) |
| 5 | idle-egress: an idle sidecar (API open, event stream connected) makes zero upstream connections and zero DNS lookups | `crosswire-api/tests/gates.rs::idle_egress_zero_connections` | proven |
| 6 | CA-key-leakage: the CA private key appears in no API body, no project-dir file and not in the OpenAPI document | `crosswire-server/tests/gates.rs::ca_key_never_leaves` | proven |
| 7 | loopback-spelling relay refusal: addresses resolving to loopback/link-local/metadata ranges are refused, in every spelling | `crosswire-client/tests/gate.rs::spellings_that_resolve_to_refused_ranges_are_refused`, `::gate_refuses_after_resolution`, `::own_ports_are_refused_on_loopback_even_when_allowlisted` | proven |
| 8 | proxy origin-form refusal: an origin-form request to an unknown host is refused `400` | `crosswire-proxy/tests/front.rs::origin_form_to_unknown_host_is_refused_400` | proven |
| 9 | same-site cookie isolation: a page on another localhost port cannot read any route or the event stream | `crosswire-api/tests/gates.rs::another_localhost_port_cannot_read_any_route_or_events` | proven |
| 10 | host-421: a non-loopback `Host` is refused `421` on every route and the WS upgrade | `crosswire-api/tests/gates.rs::host_421` | proven |
| 11 | bootstrap single-use: a bootstrap token works once, then fails; it expires | `crosswire-api/tests/auth.rs::bootstrap_is_single_use_and_expires` | proven |
| 12 | non-loopback proxy-auth: a non-loopback proxy bind requires `--allow-non-loopback` and proxy auth | `crosswire-proxy/tests/config.rs::config_refuses_non_loopback_without_flag`, `::config_requires_auth_on_non_loopback`; `crosswire-proxy/tests/front.rs::non_loopback_bind_without_flag_fails`, `::proxy_auth_required_then_accepted` | proven |
| 13 | scope matrix incl. empty scope (§4.6): In/Out/Empty/HostMismatch verdicts drive history recording and the repeater's four cases | `crosswire-api/tests/gates.rs::scope_matrix_including_empty_scope` (repeater: empty/in/out + strict refusal); `crosswire-proxy/tests/intercept.rs::scope_state_is_recorded_on_history_exchanges` (history In/Out/Empty/HostMismatch); `crosswire-tools/tests/repeater.rs` (repeater verdicts and out-of-scope audit) | proven |
| 14 | redaction golden (§11.4): copy-as-curl (redacted), copy-raw and logs of a seeded project contain no seeded secret | — | **Plan 5/6** (copy-as-curl / export tooling) |
| 15 | bad-chain refusal and chained-upstream anchor (§11.9) | `crosswire-client/tests/transport_tls.rs`, `crosswire-proxy/tests/failures.rs` | proven (prior plan) |
| 16 | hostile-project corpus (torn pack, zstd bomb, hostile SQLite and journal) | `crosswire-store/tests/packs.rs`, `::durability.rs`, `::project.rs` | proven (prior plan) |
| 17 | audit-completeness: every §9.4 audited action produces a row | `crosswire-server/tests/gates.rs::audit_completeness` | proven |
| 18 | one hostile fixture per `Limits` row with flat RSS | `crosswire-proxy/tests/limits_rss.rs`, `::limits.rs` | proven (prior plan) |

## Supporting gate: authentication-failure rate limiting

Not a numbered §13 row, but part of the hardening the gates assume: ten
authentication failures per peer per minute trip a `429`, and every failure is
audited. Proven end to end, including the WebSocket first-frame path (closed in
Task 6), by `crosswire-api/tests/gates.rs::auth_failures_rate_limited_and_audited`
and, for the bootstrap path, `crosswire-api/tests/auth.rs::bootstrap_rate_limited_after_10_failures_and_audited`.

## Audited actions (used by audit-completeness)

The exact action strings the code emits, each produced once by
`audit_completeness`:

`session.bootstrap`, `auth.fail`, `token.create`, `token.revoke`, `scope.set`,
`intercept.enabled`, `intercept.rules`, `repeater.send`,
`repeater.send.out_of_scope`, `allow.add`, `cert.override.config`,
`browser.profile_reset`, `listener.bind`, `listener.stop`, `ca.generate`.

## Notes

- The `idle-egress` gate reads a connect counter and a DNS counter added to
  `crosswire_client::Transport` (`connect_count()`, `dns_count()`); they only
  advance when the transport actually dials or resolves, so an idle run leaves
  them at zero.
- The route-auth, cookie-isolation and host-421 gates walk the committed
  OpenAPI document (`crosswire_api::document()`), so a new route cannot escape
  the walk without also appearing in the document the snapshot test guards.
- `/api/v1/events` is excluded from the plain route-auth walk because it
  authenticates inside its handler (it may defer to a first-frame bearer); its
  refusals are covered by the cookie-isolation, host-421 and rate-limit gates.
