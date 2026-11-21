# 0.1.2 Dependency Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Take the minor and major Rust upgrades that Dependabot does not propose: webpki-roots 1, sha2 0.11, tower-http 0.7, dirs 7, criterion 0.8, base64 0.23, rand 0.10, ulid 3, rusqlite 0.43, plus the lockfile-only minors rust-embed 8.14 and rustls-pki-types 1.16. One commit per crate. The format-6 project, ULID text, stored SHA-256 values and the TLS trust set stay unchanged, as proven by tests written first.

**Architecture:** No design change. Task 0 adds the known-answer tests that 0.1.1 did not (SHA-256, ULID text, root count) on today's dependencies. Each later task changes one manifest line (or a tightly coupled set), makes the smallest source change, and re-runs the pinned tests plus the crates that use the dependency.

**Tech Stack:** Rust (MSRV 1.89), rusqlite with bundled SQLite 3.54.1, rustls/webpki-roots, axum/tower-http/utoipa, criterion.

**Spec:** `docs/superpowers/specs/2026-11-18-crosswire-dependency-refresh-design.md` (section and G/D numbers below refer to it).

**Branch:** `feat/dep-refresh` from `origin/main` (`2b4e2eb`, v0.1.1). The spec and this plan are its first commit.

## Already done (0.1.1, PR #33): do not redo
rusqlite 0.32 → 0.41.1 with `fallible_uint` and bundled SQLite 3.54.1; MSRV 1.89 everywhere it is stated; rustix 1.1.6; the 0.1.0 project fixture, `compat.rs::opens_0_1_0_project` and `packs.rs::offset_above_i64_max_is_refused`. Nothing in this plan touches the MSRV, the fixture or those tests, except to re-run them.

## Global Constraints
- No new direct dependency. The only manifest edits are the version and feature changes listed here.
- No schema, migration, `user_version` or `schema_version` change. No OpenAPI change (`docs/api/openapi.json` byte-identical).
- `unsafe_code = "forbid"` and `missing_docs = "deny"` stay. MSRV stays 1.89; no locked package may declare a higher `rust-version`.
- `cargo deny check` stays clean after every task.
- Every cargo command uses `--locked` once the task's own `cargo update -p <crate>` has run.
- **Cargo commands run sequentially, one at a time on the machine, never in parallel**, and never piped through `tail`: write each to a log in the scratchpad and read its summary and exit code. The ~40-minute workspace test runs **once**, in Task 12, in the background.
- Conventional Commits, subject ≤ 72 characters, a body that says what changed and how it was verified. NO `Co-Authored-By`, `Claude-Session` or any AI attribution.
- Each task ends in exactly the commit(s) it names, so each crate can be reverted alone.

## Review Focus
1. **Known answers before bumps.** `blob_hash_is_fips_sha256`, `relay_sha256_matches_kat`, `ulid_text.rs` and `webpki_root_count_is_unchanged` are committed in Task 0 and pass on the old dependencies; they are not edited by later tasks.
2. **The randomness sources for tokens, sessions, the preview key and the CSP nonce are still `ThreadRng`/`random()`** after the rand 0.10 renames (Task 7, G9).
3. **The OpenAPI snapshot is unchanged after ulid 3**, and `crosswire-model` still builds `--no-default-features` (Task 8).
4. **base64 is declared with `default-features = false, features = ["std"]`**, and `cargo tree -e features -i base64@0.23.1` shows no `simd-unsafe` (Task 6).
5. **rusqlite 0.43 keeps SQLite 3.54.1**, `opens_0_1_0_project` passes unchanged, and the G3 empty-SQL grep is in the commit body (Task 9).
6. **webpki-roots 1 keeps 1.0.10 (120 roots)** in its own commit; any later root-count change comes with an upstream diff (Task 1).

## Parallelism
```
T0 (KAT tests) ─> T1 ─> T2 ─> T3 ─> T4 ─> T5 ─> T6 ─> T7 (rand) ─> T8 (ulid) ─> T9 (rusqlite) ─> T10 ─┐
T11 (SECURITY.md, CHANGELOG, docs; no cargo) ────────────────────────────────────────────────────────┴─> T12 (full gate)
```
T1–T10 each edit `Cargo.toml` and `Cargo.lock`, so they run **in order on the one branch**; parallel worktrees would only produce lockfile conflicts. T0 must land before T1. T11 is docs-only and can run in a parallel worktree at any time (it runs no cargo command, so it never contends with a bump task). No two tasks may run cargo at the same time, in any worktree: they share the machine and `target/` lock.

---

### Task 0: known-answer tests on today's dependencies (opus)
**Files:**
- Modify `crates/crosswire-store/tests/packs.rs`: add `blob_hash_is_fips_sha256` (G5). Store a 20 KiB body of `0x61` through the pack path and assert `blobs.hash` equals a hex literal computed once with `shasum -a 256` (not through `sha2`).
- Modify `crates/crosswire-proxy/src/capture.rs` (`mod tests`): add `relay_sha256_matches_kat` (G5). `CaptureResult::empty().relay.sha256` equals `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`; a capture of `abc` equals `ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad`. Compare against hex literals.
- Create `crates/crosswire-model/tests/ulid_text.rs` (G6), using only `from_parts`, `from_string`, `nil`, `ULID_LEN` and `Display`, so it compiles on ulid 1 and 3 and with `--no-default-features`.
- Create `crates/crosswire-client/tests/roots.rs`: `webpki_root_count_is_unchanged` (G7): `TLS_SERVER_ROOTS.len() == 120` and an anchor whose `subject` contains the bytes `ISRG Root X1`. The doc comment explains how to update the count with a webpki-roots patch.
- [ ] Step 1: write the four tests. Pin the second ULID string by running the test once on ulid 1.2.1 and copying the value, and say so in a comment.
- [ ] Step 2: run, one at a time, `cargo test --locked -p crosswire-store --test packs --test compat`, `cargo test --locked -p crosswire-proxy --lib capture`, `cargo test --locked -p crosswire-model`, `cargo test --locked -p crosswire-client --test roots`, then `just lint` and `cargo deny check`. All green on the unchanged dependencies.
- [ ] **Commit** `test: pin SHA-256, ULID text and root-store answers before upgrades`. Body: what each test pins, that they pass on sha2 0.10, ulid 1 and webpki-roots 0.26.11, and that G1/G4 already exist from 0.1.1.

### Task 1: webpki-roots 0.26 shim → 1 (sonnet)
**Files:** `Cargo.toml` (`webpki-roots = "1"`), `Cargo.lock`. No source change: `crates/crosswire-client/src/rustls_backend.rs:36,48` and `crates/crosswire-proxy/src/sni.rs:205` use `TLS_SERVER_ROOTS` unchanged.
- [ ] Step 1: `cargo update -p webpki-roots@0.26.11`. Check `cargo tree -i webpki-roots` shows only `1.0.10` (do not take 1.0.12 here; Dependabot brings it with a reviewed count change).
- [ ] Step 2: `cargo test --locked -p crosswire-client`, then `cargo test --locked -p crosswire-proxy --lib sni`, then `cargo deny check`.
- [ ] **Commit** `build(deps): use webpki-roots 1 directly instead of the 0.26 shim`. Body: the trust set is byte-identical (1.0.10, 120 roots, G7).

### Task 2: sha2 0.10 → 0.11 (sonnet)
**Files:** `Cargo.toml` (`sha2 = "0.11"`), `Cargo.lock`. Call sites unchanged (store `bodies.rs`, `packs.rs`, `control.rs`, `writer.rs`; api `tokens.rs`, `routes/exchanges.rs`, `routes/preview.rs`, `preview_token.rs`; proxy `capture.rs`; ca `ca.rs`; tools; server dev).
- [ ] Step 1: `cargo update -p sha2@0.10.9`; `cargo tree -i sha2` shows only `0.11.x`.
- [ ] Step 2: `cargo test --locked -p crosswire-store --test packs --test compat`, `cargo test --locked -p crosswire-proxy --lib capture`, `cargo test --locked -p crosswire-api --lib preview_token`, `cargo test --locked -p crosswire-api --test auth --test tokens`, `cargo test --locked -p crosswire-ca`, then `just lint`, `cargo deny check`.
- [ ] **Commit** `build(deps): upgrade sha2 to 0.11`. Body cites the FIPS and RFC 4231 KATs and the 0.1.0 fixture hashes.

### Task 3: tower-http 0.6 → 0.7 (sonnet)
**Files:** `Cargo.toml` (`tower-http = { version = "0.7", features = ["trace"] }`), `Cargo.lock`. `crates/crosswire-api/src/app.rs:15,239` unchanged.
- [ ] Step 1: `cargo update -p tower-http`; `cargo test --locked -p crosswire-api --test auth --test gates`; `just lint`; `cargo deny check`.
- [ ] **Commit** `build(deps): upgrade tower-http to 0.7`. Body: only `TraceLayer` is used; 0.7's breaking changes are in modules we do not enable.

### Task 4: dirs 6 → 7 (sonnet)
**Files:** `Cargo.toml` (`dirs = "7"`), `Cargo.lock`. `crates/crosswire-server/src/paths.rs:14` unchanged.
- [ ] Step 1: `cargo update -p dirs`; `cargo test --locked -p crosswire-server --test cli`; `cargo deny check` (the `option-ext` MPL-2.0 exception must still match).
- [ ] **Commit** `build(deps): upgrade dirs to 7`. Body: only Windows `preference_dir` changed; we call `config_dir`.

### Task 5: criterion 0.5 → 0.8 (sonnet)
**Files:** `crates/crosswire-h1/Cargo.toml` and `crates/crosswire-store/Cargo.toml` dev-dep `criterion = { version = "0.8", default-features = false }`, `Cargo.lock`.
- [ ] Step 1: `cargo update -p criterion`; `just bench` (`--no-run` for `crosswire-h1/benches/parse.rs` and `crosswire-store/benches/history.rs`); `just lint`; `cargo deny check` (new: `alloca`, `page_size`, `winapi*`).
- [ ] **Commit** `build(deps): upgrade criterion to 0.8 for benches`.

### Task 6: base64 0.22 → 0.23 without the SIMD engines (sonnet)
**Files:** `Cargo.toml` (`base64 = { version = "0.23", default-features = false, features = ["std"] }`), `Cargo.lock`. Call sites unchanged (api `preview_token.rs`, `session.rs`, `tokens.rs`, `routes/intercept.rs`, `routes/repeater.rs`; proxy; store; server).
- [ ] Step 1: `cargo update -p base64@0.22.1` (the 0.22.1 entry stays for axum). `cargo tree -e features -i base64@0.23.1 | grep simd` prints nothing.
- [ ] Step 2: `cargo test --locked -p crosswire-api --test auth --test repeater --test intercept --test preview`, `cargo test --locked -p crosswire-model`, `cargo test --locked -p crosswire-store --test compat`, `just lint`, `cargo deny check`.
- [ ] **Commit** `build(deps): upgrade base64 to 0.23 with SIMD engines off`. Body: why `simd-unsafe` stays off (spec §8 Q3).

### Task 7: rand 0.9 → 0.10 (sonnet)
**Files:** `Cargo.toml` (`rand = "0.10"`), `Cargo.lock`; `crates/crosswire-store/src/synthetic.rs:33` and `crates/crosswire-store/tests/perf_ingest.rs:33` → `use rand::{RngExt, SeedableRng};`; `crates/crosswire-server/src/ui.rs:27` → `use rand::Rng;`. Review only: `crates/crosswire-store/src/control.rs:136`, `crates/crosswire-api/src/session.rs:37`, `tokens.rs:43`, `preview_token.rs:45`, `crates/crosswire-server/src/ui.rs:157`.
- [ ] Step 1 (before the bump): `cargo run --locked -p crosswire-store --features synthetic --example seed_project <scratch>/a 1000`, then `sqlite3 <scratch>/a/project.db "SELECT ulid, json FROM exchanges ORDER BY ulid" | shasum -a 256`.
- [ ] Step 2: `cargo update -p rand@0.9.5`, then `cargo update -p rand@0.10.3` (to 0.10.4), and apply the three import renames.
- [ ] Step 3: repeat Step 1 into `<scratch>/b` and compare. If the digests differ, add one sentence to `bench/README.md` (the synthetic dataset changed in 0.1.2; earlier numbers are not directly comparable) and run `just perf-smoke` once, report-only, for the PR body. Do not try to restore the old values.
- [ ] Step 4: `cargo test --locked -p crosswire-store --features synthetic --test synthetic`, `cargo test --locked -p crosswire-api --test auth --test tokens`, `cargo test --locked -p crosswire-server --test ui` (CSP nonce), `just bench`, `just lint`, `cargo deny check`.
- [ ] **Commit** `build(deps): upgrade rand to 0.10`. Body: the renames, the G9 review (each source still `ThreadRng`/`random()`), the synthetic digest result.

### Task 8: ulid 1 → 3 (sonnet; after Task 7)
**Files:** `Cargo.toml` (`ulid = "3"`), `crates/crosswire-model/Cargo.toml` (`ulid = { version = "3", default-features = false }`), `Cargo.lock`; `Ulid::new()` → `Ulid::generate()` at all 33 sites in 15 files:
- `crates/crosswire-model/src/exchange.rs`, `src/marker.rs`
- `crates/crosswire-proxy/src/intercept.rs`, `tests/intercept.rs`
- `crates/crosswire-api/src/routes/repeater.rs`, `src/preview_token.rs` (7)
- `crates/crosswire-store/src/audit.rs`, `src/control.rs`, `src/bin/killtest-child.rs`, `tests/control.rs` (5), `tests/packs.rs` (6), `tests/writer.rs` (3), `tests/read.rs`, `tests/limits_rss_store.rs`
- `crates/crosswire-tools/tests/repeater.rs` (2)
- [ ] Step 1: `cargo update -p ulid@1.2.1` (ulid 1.2.1 stays locked under `utoipa-gen` only). Apply the rename (`grep -rl 'Ulid::new()' crates | xargs sed -i '' 's/Ulid::new()/Ulid::generate()/g'`); `grep -rn 'Ulid::new\b' crates` prints nothing. `cargo tree -i rand@0.10.4` shows ulid under it (one rand 0.10).
- [ ] Step 2: `cargo test --locked -p crosswire-model`, `cargo test --locked -p crosswire-store --test compat --test control --test writer --test packs`, `cargo test --locked -p crosswire-api --test openapi` (no `UPDATE_OPENAPI`), `cargo test --locked -p crosswire-proxy --test intercept`, `just features`, `just lint`, `cargo deny check`.
- [ ] **Commit** `build(deps): upgrade ulid to 3 (Ulid::new is now generate)`. Body: the text form is pinned by `ulid_text.rs` and the 0.1.0 fixture; `utoipa-gen` keeps ulid 1 for schema generation and the OpenAPI snapshot is unchanged.

### Task 9: rusqlite 0.41 → 0.43 (opus)
**Files:** `Cargo.toml` (`rusqlite = { version = "0.43", features = ["bundled", "fallible_uint"] }`), `Cargo.lock`. Expected source change: none (trial).
- [ ] Step 1: G3 static check. List every `.prepare(`, `.prepare_cached(`, `.execute(`, `.query_row(` in `crates/crosswire-store` whose SQL argument is not a non-empty literal; each must be a `format!` with a fixed SQL prefix (today: `writer.rs:802`, `read.rs:364`, `read.rs:379`, `read.rs:416`). Paste the list in the commit body.
- [ ] Step 2: `cargo update -p rusqlite`. `cargo tree -i libsqlite3-sys` shows `0.40.0`; `grep 'define SQLITE_VERSION ' $(cargo metadata --format-version 1 --locked | jq -r '.packages[]|select(.name=="libsqlite3-sys").manifest_path' | xargs dirname)/sqlite3/sqlite3.h` shows `3.54.1`; `cargo metadata` lists no package with `rust_version` above 1.89.
- [ ] Step 3: `cargo test --locked -p crosswire-store --all-features` (compat, packs incl. G4, indexes, tags_index, path_filter, durability, hostile, writer, read, project), `just features`, `just lint`, `cargo deny check`.
- [ ] **Commit** `build(deps): upgrade rusqlite to 0.43 (same SQLite 3.54.1)`. Body: SQLite unchanged, MSRV unchanged, the 0.43 `Error::Empty` refusal and the G3 list, the UAF fixes are in features we do not enable. If the maintainer answers spec §8 Q5 "wait", this commit is dropped alone.

### Task 10: lockfile-only minors (sonnet)
**Files:** `Cargo.lock` only.
- [ ] Step 1: `cargo update -p rust-embed -p rustls-pki-types` (8.13.0 → 8.14.0, 1.15.1 → 1.16.0). `git diff --stat` shows only `Cargo.lock`.
- [ ] Step 2: `cargo test --locked -p crosswire-server --test ui`, `cargo test --locked -p crosswire-client`, `cargo test --locked -p crosswire-ca`, `just ui-dist-check`, `cargo deny check`.
- [ ] **Commit** `build(deps): take rust-embed 8.14 and rustls-pki-types 1.16`. Body: semver-compatible minors that Dependabot's patch-only policy skips.

### Task 11: docs (sonnet; parallel worktree allowed, no cargo)
**Files:**
- `SECURITY.md`: a short `## Upstream trust store` section. Crosswire verifies upstream TLS (Repeater, proxy upstream) against the `webpki-roots` Mozilla root list compiled into the binary, not the OS store; Dependabot patch updates follow upstream root additions and removals, so a removed root can make a site fail verification after an upgrade; the `crosswire-client/tests/roots.rs` count changes in the same commit.
- `CHANGELOG.md`, under `[Unreleased]` → `### Changed`: one bullet per crate, old → new; "Bundled SQLite stays 3.54.1. No project-format, API or ULID change; 0.1.0 and 0.1.1 projects open unchanged."
- `docs/format.md` under `## Database`: one sentence that the format does not depend on the bundled SQLite version, and that newer-to-older compatibility is not promised (spec §8 Q2).
- [ ] **Commit** `docs: record the 0.1.2 dependency refresh and root-store policy`.

### Task 12: full gate and review (opus)
- [ ] Step 1: run each gate **sequentially**, each to its own scratchpad log, reading the exit code and summary (never `| tail`):
  - `just lint` (clippy `-D warnings` + `cargo fmt --check`)
  - `just doc`
  - `cargo deny check`
  - `cargo +1.89 check --workspace --locked --all-features` (CI `msrv` job; `rustup toolchain install 1.89 --profile minimal` if absent)
  - `just features`
  - `just bench`
  - `just package`
  - `just corpus-check`
  - `just ui-dist-check` (no UI change expected)
  - `just perf-smoke` only if Task 7 found a synthetic digest change; otherwise not needed (the bundled SQLite is unchanged and the EXPLAIN tests run in the workspace test).
- [ ] Step 2: start the workspace test once, in the background: `cargo test --workspace --locked --all-features --no-fail-fast > <scratchpad>/deps-workspace-test.log 2>&1`. It takes about 40 minutes; before calling it hung, check `pgrep -P <cargo pid>`. Then `grep -E '^test result|FAILED|panicked' <log>` and require zero failures.
- [ ] Step 3: whole-branch review (opus) against Review Focus. PR body: before/after `cargo tree -d`, the G3 list, the synthetic digest result, the G9 review, and `git log --format=%B origin/main.. | grep -ci claude` = 0.
- [ ] Step 4: push and open the PR for maintainer review (spec §8 Q5 decides Task 9).
