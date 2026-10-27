# Security-Gate Hardening 2a Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Take four §13 DoD security gates (#15, #16, #5, corpus coverage) from PARTIAL to proven, each a real CI assertion.

**Architecture:** Four largely independent test additions, one per gate, each in the crate that owns the behavior: a chained-upstream anchor test (client), two hostile-project fixtures (store), an idle-egress test over the real serve binary (server), and corpus→CI wiring plus a Rust DiagnosticCode-coverage test. Product code changes only where a fixture surfaces a real defect.

**Tech Stack:** Rust (crosswire-client, crosswire-store, crosswire-server, crosswire-h1/core), rustls, rusqlite, Python 3 (corpus-check), GitHub Actions.

**Spec:** docs/superpowers/specs/2026-10-27-crosswire-security-gate-hardening-2a-design.md

## Global Constraints

- `unsafe_code = "forbid"` workspace-wide; MSRV 1.88; rustls ring-only; `cargo build` Node-free.
- Conventional Commits, ≤72-char subjects, **no attribution trailers** (verify `git log -1 | grep -c -i claude` == 0 after each commit).
- `cargo deny` clean; **no new external dependency** unless unavoidable (then justify + lockfile + deny-clean). Prefer std / existing dev-deps for fixtures.
- Additive only: no wire/schema/query-semantics change.
- FULL GATE before a task is done: `just lint`, `just doc`, `cargo deny check`, `cargo test` (the relevant crate(s)); the CI additions must themselves be exercised.
- Tests spawning a real serve reuse the harness in `crates/crosswire-server/tests/gates.rs` (`spawn_serve`, `wait_serve_startup`, `api_session`, `Reap`).
- Each task updates its own gate's row in `docs/security-gates.md` (distinct rows — no conflict under serialized execution).

## Review Focus

- A private-CA leaf otherwise well-formed (unexpired, correct SAN) — refused only for the unknown anchor, accepted only with it (Task 1).
- A SQLite trigger that fires on SELECT/read (not just INSERT) — the read path is what an opener hits (Task 2).
- A journal length header claiming gigabytes — must not pre-allocate from the attacker-controlled field (Task 2).
- An idle window long enough that a background reconnect/poll would surface (Task 3).
- A `DiagnosticCode` present in the enum but absent from every corpus case — the Rust coverage test must FAIL, not pass vacuously (Task 4).

---

### Task 1: Gate #15 — chained-upstream private-CA anchor

**Files:**
- Test: `crates/crosswire-client/tests/transport_tls.rs` (add one test; reuse the existing test-CA/leaf builder already in this file)
- Modify: `docs/security-gates.md` (#15 row → proven)

**Interfaces:**
- Consumes: `RustlsBackend::with_extra_roots` (`crosswire-client/src/rustls_backend.rs:43`); the existing in-file test CA machinery and TLS connect helper used by `verifies_ca_issued_leaf_and_refuses_unknown_chain` (transport_tls.rs:24).
- Produces: nothing downstream (terminal test).

- [ ] **Step 1: Write the failing test** `private_ca_leaf_refused_without_anchor_accepted_with_it`:
  - Generate a fresh **private CA** (not in the webpki default roots) and a well-formed leaf it signs (unexpired, SAN = the test host).
  - Assert the backend with **default roots** refuses the connection with a certificate-validation error (not a panic, not a timeout).
  - Assert the backend built with that CA's DER via `with_extra_roots` **accepts** the connection and the negotiated result reports a valid chain.
- [ ] **Step 2: Run** `cargo test -p crosswire-client --test transport_tls private_ca_leaf` → FAIL.
- [ ] **Step 3: Implement** the test using the file's existing CA-builder pattern; no product change is expected. If the backend cannot be given extra roots in the shape the test needs, STOP and surface it (do not alter the trust path without a ruling).
- [ ] **Step 4: Run** the test → PASS; `cargo test -p crosswire-client` green.
- [ ] **Step 5: FULL GATE**, then update `docs/security-gates.md` #15 → proven with the test path.
- [ ] **Step 6: Commit** `test(client): chained-upstream private-CA anchor gate`.

---

### Task 2: Gate #16 — hostile SQLite + malformed journal

**Files:**
- Test: `crates/crosswire-store/tests/` (two tests — add to `project.rs` or a new `hostile.rs`)
- Modify: `crosswire-store` source ONLY if a fixture reveals a real defect (execution, panic, unbounded alloc)
- Modify: `docs/security-gates.md` (#16 row → proven)

**Interfaces:**
- Consumes: the normal project-open path (`crosswire-store/src/project.rs`, which sets `trusted_schema=OFF` defensive mode at :317-319); the startup journal drain used by the existing journal tests; `rusqlite` (already a dep) to build the hostile `.db` in-test.
- Produces: nothing downstream (terminal tests).

- [ ] **Step 1: Write the two failing tests:**
  - `hostile_sqlite_schema_does_not_execute_on_open`: build a `project.db` carrying a **trigger that fires on read/SELECT** (and/or a view whose body would run SQL) whose body would write a sentinel row or touch a sentinel file. Open it through the project-open path and assert: the sentinel side effect did **not** happen, the open either succeeds in defensive read-only mode or returns a typed error, and there is no panic.
  - `malformed_journal_is_skipped_without_panic_or_unbounded_alloc`: place in `journal/` (a) truncated/garbage bytes, (b) a record whose **length header claims a huge size**, (c) a plausible-but-corrupt record; run the startup drain/open and assert: no panic, the good records still load, the bad content is skipped/rejected, and memory stays bounded (the huge length field must not drive an allocation — assert via a modest cap or that the open completes promptly without OOM).
- [ ] **Step 2: Run** `cargo test -p crosswire-store` (the two new tests) → FAIL.
- [ ] **Step 3: Implement** the fixtures. If `trusted_schema=OFF` + defensive open already prevent execution and the drain already bounds allocation, the tests pass with **no product change** — confirm that is genuinely why (e.g. flip the pragma off in a scratch check to see the sentinel fire, proving the test is real). If a fixture reveals a real defect, fix it in `crosswire-store` and note it in the commit.
- [ ] **Step 4: Run** `cargo test -p crosswire-store` → green.
- [ ] **Step 5: FULL GATE**, then update `docs/security-gates.md` #16 → proven (hostile-SQLite + malformed-journal now covered).
- [ ] **Step 6: Commit** `test(store): hostile-SQLite and malformed-journal gates`.

---

### Task 3: Gate #5 — idle-egress on the real binary

**Files:**
- Test: `crates/crosswire-server/tests/gates.rs` (add a real-serve idle-egress test)
- Modify: `docs/security-gates.md` (#5 row → proven)

**Interfaces:**
- Consumes: `spawn_serve`, `wait_serve_startup`, `api_session`, `Reap` (gates.rs); a Linux `/proc/<pid>/net/tcp` + `/proc/<pid>/fd` read for the socket count (std only). The existing in-process `idle_egress_zero_connections` (`crosswire-api/tests/gates.rs:337`) stays as the fast unit check.
- Produces: nothing downstream (terminal test).

- [ ] **Step 1: Write the failing test** `real_serve_makes_no_outbound_connection_while_idle`:
  - Spawn a real `crosswire serve`; open a session and the event stream; record the set of the process's established **outbound** (non-loopback, non-listener) TCP connections; idle a bounded window (≥ 5 s); record again; assert the set did not grow (zero new outbound connections).
  - On a non-Linux host where the `/proc` probe is unavailable, the test documents itself as skipped (`eprintln!` + early return) — the CI gate that counts runs on ubuntu.
- [ ] **Step 2: Run** `cargo test -p crosswire-server --test gates real_serve_makes_no_outbound` → FAIL (or skip-with-notice on macOS; confirm it FAILS meaningfully on Linux if a stray connection existed by temporarily asserting the opposite).
- [ ] **Step 3: Implement** the `/proc`-based outbound-socket probe (std fs parsing; no new dep) and the test.
- [ ] **Step 4: Run** `cargo test -p crosswire-server` → green.
- [ ] **Step 5: FULL GATE**, then update `docs/security-gates.md` #5 → proven (real-binary, not only in-process).
- [ ] **Step 6: Commit** `test(server): idle-egress gate over the real serve binary`.

---

### Task 4: corpus→CI + DiagnosticCode coverage

**Files:**
- Modify: `justfile` (add `corpus-check`), `.github/workflows/ci.yml` (run it on the ubuntu `check` job)
- Test: a Rust test asserting per-code coverage — `crates/crosswire-h1/tests/corpus.rs` (extend) or a new test near `corpus/`
- Modify: `docs/security-gates.md` (record corpus coverage + CI wiring)

**Interfaces:**
- Consumes: `scripts/corpus-check.py` (already checks 26/26 codes); `DiagnosticCode::ALL` (`crosswire-core`, 26 variants; `crosswire-core/tests/diagnostic_codes.rs` asserts the count); the `corpus/h1/cases` directory and its per-case code tags (read how an existing corpus test discovers cases + their codes).
- Produces: a `just corpus-check` recipe; a CI step; a Rust coverage test.

- [ ] **Step 1: Write the failing Rust test** `every_diagnostic_code_has_a_corpus_case`: read `corpus/h1/cases`, collect the set of `DiagnosticCode`s each case covers (via the same tagging the Python script reads), and assert every `DiagnosticCode::ALL` variant appears in at least one case. Prove it can fail: temporarily remove a case/tag and confirm the test fails (note in the report).
- [ ] **Step 2: Run** `cargo test -p crosswire-h1` (the new test) → PASS on the current corpus (all 26 covered today); the failure proof is the temporary-removal check in Step 1.
- [ ] **Step 3: Implement** `just corpus-check` (runs `python3 scripts/corpus-check.py`), add it to the ubuntu `check` job in `ci.yml` (pin no new action; reuse the existing python3 on the runner — confirm the script is stdlib-only), and finalize the Rust coverage test.
- [ ] **Step 4: Run** `just corpus-check` locally → green; `cargo test -p crosswire-h1` → green; validate the CI YAML (`python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"`).
- [ ] **Step 5: FULL GATE**, then update `docs/security-gates.md` to record corpus coverage + the CI wiring.
- [ ] **Step 6: Commit** `ci: wire corpus-check and assert per-DiagnosticCode coverage`.

---

## Notes for the executor

- The four tasks are independent (different crates/files); run them in order 1→4. Each touches a distinct `docs/security-gates.md` row, so no conflict under serialized execution.
- Suggested models: Task 1 (TLS trust correctness) and Task 2 (defensive store, security) → opus; Task 3 (idle-egress, mechanical /proc probe) and Task 4 (CI wiring + coverage test) → sonnet.
- The "≥ 200 corpus captures" DoD-count is NOT in scope here (tracked for a corpus-growth task before 0.1.0); this plan guarantees per-code coverage + CI enforcement only.
- If any fixture surfaces a real product defect, that is the gate working — fix it in the owning crate, note it in the commit, and keep the fixture that found it.
