# Crosswire security-gate hardening 2a — design spec

> **Status:** second "Harden & prove (alpha.2 DoD)" sub-plan (2a of 2). Takes four
> §13 DoD security gates from PARTIAL to proven. Authority: the product spec
> `docs/superpowers/specs/2026-10-22-crosswire-design.md` (§9.1, §11.1, §11.6, §11.9,
> §13). Where this document and the product spec disagree, the product spec wins.
> The #18 Limits flat-RSS matrix is sub-plan 2b; the preview zero-egress layer
> (gate #4) is its own later sub-plan.

## 1. Goal

Close the gaps the gap analysis found in four §13 security gates so each is a real,
non-vacuous CI assertion:

- **#15** chained-upstream private-CA anchor: an upstream behind a private CA is
  refused without the anchor and accepted with it.
- **#16** hostile-project corpus: a malicious `project.db` (trigger/view/attached
  schema) and a malformed `journal/` file are handled defensively, no execution,
  no panic, bounded memory.
- **#5** idle-egress on the real binary: a spawned `crosswire serve`, idle with the
  event stream open, makes zero outbound OS connections.
- **corpus→CI + coverage**: `scripts/corpus-check.py` runs in CI, and a Rust test
  asserts every `DiagnosticCode` is covered by a corpus case.

Each is a test (plus small plumbing); product code changes only if a fixture
reveals a real defect — which is the gate doing its job.

## 2. Scope

**In scope:** the four gates above, their fixtures, the CI/justfile wiring, and the
`docs/security-gates.md` status edits (PARTIAL → proven as each lands).

**Out of scope (tracked):** #18 Limits flat-RSS matrix (2b); preview zero-egress
(gate #4); FTS/search gates (v0.2a); any gate already proven.

## 3. Global constraints (verbatim from the product spec)

- `unsafe_code = "forbid"` workspace-wide; MSRV 1.88; rustls ring-only; `cargo build`
  Node-free.
- Conventional Commits, ≤72-char subjects, no attribution trailers.
- `cargo deny` clean; no new external dependency unless unavoidable (justified +
  lockfile + deny-clean). Prefer std / existing deps for fixtures.
- Additive only: no wire/schema/query-semantics change.
- FULL GATE before a task is done: `just lint`, `just doc`, `cargo deny check`,
  `cargo test`, and the CI additions must themselves be exercised.
- Tests that spawn a real `crosswire serve` reuse the harness in
  `crates/crosswire-server/tests/gates.rs` (`spawn_serve`, `wait_serve_startup`,
  `api_session`, `Reap`).

## 4. Gate #15 — chained-upstream private-CA anchor

**Today:** bad-chain refusal is proven (`crosswire-proxy/tests/failures.rs:240`
`upstream_bad_chain_is_502_without_override_and_relayed_with_it`;
`crosswire-client/tests/transport_tls.rs:24`). The anchor primitive
`RustlsBackend::with_extra_roots` exists (`crosswire-client/src/rustls_backend.rs:43`).
**Missing:** a test that a chained upstream presenting a **private-CA-issued** leaf
is refused with the default roots and **accepted when that private CA is supplied as
an extra anchor**.

**Design:** a test (in `crosswire-client/tests/transport_tls.rs`, matching the
existing TLS-backend tests) that: (a) builds a leaf signed by a freshly-generated
private CA not in the webpki roots; (b) asserts the backend **refuses** it with the
default roots (a validation error, not a panic); (c) asserts the backend **accepts**
it when that CA DER is passed via `with_extra_roots`, and the negotiated connection
reports the expected chain/validation result. Reuse the test CA machinery the
existing TLS tests already use. No product change expected.

## 5. Gate #16 — hostile project corpus

**Today:** torn-pack and zstd-bomb are proven (`crosswire-store/tests/packs.rs`);
defensive mode is set (`crosswire-store/src/project.rs:317-319`, `trusted_schema=OFF`
on the read connection) and the pragma is asserted (`tests/project.rs:195`). Garbage
file / symlink refusal exist. **Missing:** a hostile-**SQLite** fixture (a malicious
trigger/view/attached schema) and a malformed-**journal** fixture.

**Design (two tests in `crosswire-store/tests/`):**

- **Hostile SQLite:** craft a `project.db` (built in-test with rusqlite, or a
  committed fixture) that carries a trigger and/or view whose body would run SQL on
  read (e.g. a `SELECT` trigger, or a view over a function), plus a schema that a
  trusting opener would act on. Open it through the normal project-open path and
  assert: the trigger/view does **not** execute (no side effect — e.g. no row is
  written to a sentinel table, no file touched), the open either succeeds in
  defensive read-only mode or is rejected with a typed error, and there is no panic.
  Confirm `trusted_schema=OFF` and defensive open are what prevent execution.
- **Malformed journal:** place in `journal/` a file with (a) truncated/garbage
  bytes, (b) an oversized length header, and (c) a valid-looking but corrupt record,
  then run the startup drain/open. Assert: the drain skips or rejects the bad
  content with no panic, bounded memory (it must not allocate from an attacker-
  controlled length field), and the project still opens with the good records
  intact. A `gap`/skip is acceptable; a crash or unbounded allocation is not.

If either fixture reveals a real defect (execution, panic, or unbounded alloc), fix
it in `crosswire-store` and note it in the commit.

## 6. Gate #5 — idle-egress on the real binary

**Today:** `crosswire-api/tests/gates.rs:337` `idle_egress_zero_connections` watches
the **in-process** `Transport` connect/dns counters only (~10 s idle with the event
stream open). **Missing:** proof the **real serve binary** is quiet.

**Design:** add (or upgrade) a gate in `crosswire-server/tests/gates.rs` that spawns
a real `crosswire serve`, opens a session and the event stream, idles a bounded
window (≥ 5 s), and asserts **zero outbound connections** made by the process. Count
at the OS level without a new dependency: on Linux, read the process's socket
inodes via `/proc/<pid>/net/tcp` + `/proc/<pid>/fd` (or compare established-socket
counts before/after); on macOS, the probe may be best-effort / skipped (documented),
since the CI gate that matters runs on ubuntu. The assertion is: no new **outbound**
(non-loopback, non-listener) connection appears while idle. The existing in-process
test stays as a fast unit check.

## 7. corpus→CI + DiagnosticCode coverage

**Today:** `corpus/h1/cases` has ~104 cases + README; `scripts/corpus-check.py`
checks 26/26 codes but is **not** wired into `justfile` or CI; no Rust test asserts
per-code coverage. `crosswire-core/tests/diagnostic_codes.rs` asserts
`DiagnosticCode::ALL.len() == 26`.

**Design:**

- Add `just corpus-check` running `scripts/corpus-check.py` (Python 3, already a CI
  dependency via Playwright's toolchain — confirm; if not, keep it pure-stdlib and
  invoke `python3`), and add it to the ubuntu CI `check` job.
- Add a Rust test (in `crosswire-h1/tests/corpus.rs` or `crosswire-core`) asserting
  **every** `DiagnosticCode::ALL` variant appears in at least one `corpus/h1/cases`
  message — so a new code without a case fails CI in Rust, independent of the Python
  script. The test reads the corpus directory and the per-case code tags.
- The "≥ 200 captures" DoD-count question (104 cases today) is **noted but not
  resolved here**: this sub-plan guarantees per-code coverage and CI enforcement;
  growing the corpus to ≥ 200 is tracked for the corpus-growth task before the
  0.1.0 tag.

## 8. Review focus (inputs the tests must cover)

- A private-CA leaf whose chain is otherwise well-formed (not expired, correct SAN)
  — refused only for the unknown anchor, accepted only with it (#15).
- A SQLite trigger that fires on SELECT/read, not just INSERT — the read path is
  what an opener hits (#16).
- A journal length header claiming gigabytes — must not pre-allocate (#16).
- An idle window long enough that a background reconnect/poll would show (#5).
- A DiagnosticCode that exists in the enum but has no corpus case — the Rust
  coverage test must fail, not silently pass (corpus).

## 9. Acceptance

- Four gates green under the normal gate (`cargo test` + the new `just corpus-check`
  in CI), each a real assertion that would fail if the property broke.
- `docs/security-gates.md`: #15, #16, #5 move PARTIAL → proven with the test paths;
  the corpus coverage/CI wiring is recorded.
- No new dependency; `unsafe_code = "forbid"` held; `cargo deny` clean.
- Any real defect a fixture surfaced is fixed in the owning crate, noted in the
  commit, and covered by the fixture that found it.
