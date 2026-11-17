# 0.1.2 dependency refresh (the minor and major upgrades Dependabot does not take)

**Status:** revised 2026-11-18 for maintainer review. It replaces the 2026-11-04 draft, which was written against 0.1.0 and planned the rusqlite move that 0.1.1 has since shipped. **Parent:** 0.1.2. **Trigger:** grouped Dependabot PR #16 (closed 2026-11-02: it bundled 0.x-minor and major upgrades, and ulid 3 failed the build). `.github/dependabot.yml` now takes only patch updates for Rust crates (`ignore` on `version-update:semver-major` and `version-update:semver-minor`), so minor and major upgrades arrive through this planned refresh.

**Base:** branch `feat/dep-refresh` from `origin/main` at `2b4e2eb` (v0.1.1). Every file and line referenced below is as of that commit.

## 1. Goal
Bring every direct Rust dependency to its current release line in one reviewed branch. Each crate (or tightly coupled set) gets its own commit, so a regression can be bisected and reverted alone. Keep the project format (format 6, message `schema_version` 2), the ULID text form, the stored SHA-256 values and the TLS trust set as they are, and prove it with tests that are written and pass **before** any manifest changes. MSRV stays 1.89.

## 2. Already done in 0.1.1 (do not redo)
PR #33 (`b5f8b3d`, commits `da0b7bc`, `87905f1`, `8b07824`) shipped, for CVE-2026-119375 / RUSTSEC-2026-0459 and RUSTSEC-2026-0486:
- **rusqlite 0.32.1 → 0.41.1** with `features = ["bundled", "fallible_uint"]` (`libsqlite3-sys` 0.30.1 → 0.39.1, bundled SQLite 3.46.0 → **3.54.1**). This was the draft's D1, and its deferred D10 (0.41 and SQLite 3.54).
- **MSRV 1.88 → 1.89** in `Cargo.toml`, the CI `msrv` job (`.github/workflows/ci.yml:132`), `README.md:7,61,269`, `CONTRIBUTING.md:31` and the `capture.rs:43`/`atomic.rs` comments. This answers the draft's §7 Q1.
- **rustix 1.1.5 → 1.1.6**.
- The draft's Task 0 store tests: the committed fixture `crates/crosswire-store/tests/fixtures/project-0.1.0/` (written by SQLite 3.46.0, with `expected.json`), `tests/compat.rs::opens_0_1_0_project` (G1) and `tests/packs.rs::offset_above_i64_max_is_refused` (G4). The draft's G3 (no multi-statement `execute`/`prepare`) is enforced at runtime since 0.41 and passed the 0.1.1 workspace suite.

## 3. Facts (recon 2026-11-18 on `2b4e2eb`; crates.io API; trial in a scratch clone)
- Workspace `rust-version = "1.89"`; the CI `msrv` job runs `cargo check --workspace --locked --all-features` on 1.89. `deny.toml` allows MIT, Apache-2.0, BSD-2/3, Unicode-3.0, ISC, Zlib and CDLA-Permissive-2.0 (plus MPL-2.0 for `option-ext` only), and `multiple-versions = "allow"`.
- **Already on the latest release (no action):** tokio 1.54.1, rustls 0.23.46 (0.24 is `-dev` only), tokio-rustls 0.26.6, rcgen 0.14.10, axum 0.8.9, tower 0.5.3, utoipa 6.0.0, utoipa-axum 0.3.0, x509-parser 0.18.1, lru 0.19.0, time 0.3.55, tracing 0.1.44, tracing-subscriber 0.3.23, thiserror 2.0.21, serde 1.0.229, serde_json 1.0.154, regex 1.13.1, ipnet 2.12.2, subtle 2.6.1, http 1.5.0, url 2.5.8, bytes 1.12.1, proptest 1.12.0, httparse 1.10.1, tempfile 3.27.0, rustix 1.1.6, mime_guess, memory-stats. hyper, hyper-util and h2 are not direct dependencies (hyper-util 0.1.21 is locked through axum and is the latest; h2 is not in the graph). Pending **patches** that Dependabot takes: clap 4.6.7 → 4.6.8, zstd 0.14.3 → 0.14.4.
- **Behind** (locked → latest, latest's declared `rust-version`; all ≤ 1.89):

| crate | locked | latest | latest MSRV | where used |
|---|---|---|---|---|
| ulid | 1.2.1 | 3.0.0 | none | model, store, proxy, api, tools, server, recorder-tests (33 `Ulid::new()` sites in 15 files) |
| sha2 | 0.10.9 | 0.11.1 | 1.88 | ca, store, proxy, api, tools, server(dev) |
| webpki-roots | 0.26.11 (shim over 1.0.10) | 1.0.12 | 1.71 | client, proxy(dev) |
| rand | 0.9.5 | 0.10.4 | 1.85 | api, store, server |
| tower-http | 0.6.11 | 0.7.1 | 1.65 | api (`TraceLayer` only, `app.rs:239`) |
| base64 | 0.22.1 | 0.23.1 | 1.71 | api, proxy, store, server |
| dirs | 6.0.0 | 7.0.0 | none | server (`paths.rs:14`, `config_dir` only) |
| criterion (dev) | 0.5.1 | 0.8.2 | 1.86 | h1 and store benches |
| rusqlite | 0.41.1 | 0.43.0 (2026-11-15) | 1.89.0 | store |
| rust-embed | 8.13.0 | 8.14.0 | 1.80 | server |
| rustls-pki-types | 1.15.1 | 1.16.0 | 1.60 | ca, client, proxy, server, tools |

- New since the 2026-11-04 draft: **rusqlite 0.42.0 and 0.43.0** (both MSRV 1.89, both on `libsqlite3-sys 0.40.0`, which bundles the **same SQLite 3.54.1** as our 0.39.1), rand 0.10.4, webpki-roots 1.0.12, rust-embed 8.14.0, rustls-pki-types 1.16.0. rust-embed and rustls-pki-types are semver-compatible minors that Dependabot's `semver-minor` ignore also skips.
- Four of the new versions are already in our lockfile through other crates: webpki-roots 1.0.10 (under the 0.26.11 shim), sha2 0.11.1 (`rust-embed-utils`), base64 0.23.1 (`rcgen → pem`, `alloc` only) and rand 0.10.3 (`proptest`).
- **Trial (2026-11-18, scratch clone of `2b4e2eb`, all bumps at once including rusqlite 0.43):** `cargo check --workspace --all-targets --all-features --locked` passes with **no warnings**, and `cargo deny check` (cargo-deny 0.20.2) reports `advisories ok, bans ok, licenses ok, sources ok`. `cargo check -p crosswire-model --no-default-features` (with and without `std`) passes, and so does `cargo +1.89 check --workspace --locked --all-features`. No locked package declares a `rust-version` above 1.89. A second pass adding D10 and rand 0.10.4 (`cargo update -p rust-embed -p rustls-pki-types -p rand@0.10.3`) also checks clean with no warnings, and `cargo deny check` stays clean. The only source changes needed:
  - `Ulid::new()` → `Ulid::generate()`, 33 sites in 15 files.
  - rand: `use rand::{Rng, SeedableRng}` → `use rand::{RngExt, SeedableRng}` in `crates/crosswire-store/src/synthetic.rs:33` and `tests/perf_ingest.rs:33`; `use rand::RngCore` → `use rand::Rng` in `crates/crosswire-server/src/ui.rs:27` (`fill_bytes` on `rand::rng()`).
  - sha2, webpki-roots, tower-http, base64, dirs, criterion and rusqlite 0.43 needed none.
- Trial lockfile delta: `sha2 0.10.9` and `webpki-roots 0.26.11` leave the graph; `ulid 3.0.0`, `alloca`, `page_size` and `winapi*` (criterion 0.8) are added; `libsqlite3-sys 0.39.1 → 0.40.0`; `hermit-abi` and `is-terminal` leave. `cargo tree -e features -i base64@0.23.1` shows no `simd-unsafe`.
- Duplicates left after the refresh (allowed): `ulid 1.2.1` only inside the `utoipa-gen` proc-macro (its `ulid` feature needs `ulid ^1`); `base64 0.22` through axum; `rand 0.9` through tungstenite; `digest`/`block-buffer`/`crypto-common 0.10` through `sha1` (axum ws).
- Proofs that already exist: `opens_0_1_0_project` (it pins every exchange's ULID text, JSON SHA-256 and body SHA-256 written by 0.1.0, including the relay capture inside the stored JSON); `offset_above_i64_max_is_refused`; `preview_token.rs:286,295` (HMAC-SHA-256 RFC 4231 vectors); `crosswire-model/tests/fixtures/exchange.v2.json`; `crosswire-api/tests/openapi.rs::openapi_snapshot_is_current`; the store EXPLAIN-QUERY-PLAN tests. **Missing:** a plain SHA-256 known-answer test (`capture.rs` unit tests compare against `sha2` itself), a ULID text test against its parts, and any check on the root store.

## 4. Decisions

| # | crate | decision | breaking changes that touch us | risk | size |
|---|---|---|---|---|---|
| D1 | webpki-roots | **Upgrade to `"1"`**, staying on locked 1.0.10 in this commit | None. 0.26.11 already re-exports 1.0.10's `TLS_SERVER_ROOTS`, so the trust set is identical (120 anchors). Removes the shim; 1.0.12 then arrives as a Dependabot patch, which updates the G7 count. | Low | XS |
| D2 | sha2 | **Upgrade to 0.11.1** | `digest 0.11`/`hybrid-array` output; `.into()` to `[u8; 32]` and `.as_slice()` still compile. SHA-256 is FIPS 180-4; the KATs prove the output. | Low | XS |
| D3 | tower-http | **Upgrade to 0.7.1** | None for `TraceLayer` (0.7 changes `fs`, `cors`, `decompression`, `timeout`, adds `csrf`). | Low | XS |
| D4 | dirs | **Upgrade to 7.0.0** | Only Windows `preference_dir` changed; we call `config_dir()`. | Low | XS |
| D5 | criterion (dev) | **Upgrade to 0.8.2** in the h1 and store dev-deps | None at compile time; benches already use `std::hint::black_box`. | Low | XS |
| D6 | base64 | **Upgrade to 0.23.1 as `default-features = false, features = ["std"]`** | None in the `Engine` API we use. The new default `simd-unsafe` stays off (§8 Q3). | Low | XS |
| D7 | rand | **Upgrade to 0.10 (lock 0.10.4)** | `Rng` → `RngExt`, `RngCore` → `Rng`. `rand::random()`/`rand::rng()` stay `ThreadRng` (ChaCha12, OS-reseeded). | Low–Med (security randomness) | S |
| D8 | ulid | **Upgrade to 3.0.0, after D7** | `Ulid::new` → `Ulid::generate`; the Crockford encoding is unchanged. Pulls `rand 0.10`, so it lands after D7 to share one copy. `utoipa-gen` keeps ulid 1 for schema generation only. | Low | M (15 files, 1 line each) |
| D9 | rusqlite | **Upgrade 0.41.1 → 0.43.0**, last among the code-bearing bumps | Same bundled SQLite 3.54.1, same MSRV 1.89, no compile change. **Runtime:** `prepare` of empty or comment-only SQL now returns `Error::Empty` (0.43). Our dynamic SQL (`writer.rs:802`, `read.rs:364,379,416`) is always a non-empty template. The 0.42/0.43 use-after-free fixes are in `session`, `column_metadata` and vtab `array`, which we do not enable. | Low | XS code, S verification |
| D10 | rust-embed, rustls-pki-types | **Lockfile-only `cargo update`** to 8.14.0 and 1.16.0 | Semver-compatible; manifests unchanged. | Low | XS |
| D11 | everything in §3 "already latest", clap/zstd patches | **No action.** Dependabot patch updates. | — | — | — |

**Order.** Compatibility tests on unchanged dependencies first (Task 0). Then the low-risk bumps, one commit each. Then rand before ulid. Then rusqlite 0.43. Then the lockfile-only minors. Every bump edits `Cargo.toml` and `Cargo.lock`, so `git revert` of one commit undoes one crate.

## 5. Format and compatibility guarantees, and the tests that prove them
New tests land in **Task 0** and pass on today's dependencies, so a failure after a bump is the bump's fault.

- **G1. A 0.1.0 project opens and reads identically** (D2, D6, D8, D9). Exists: `opens_0_1_0_project`. Re-run after each of those bumps; it must pass unchanged.
- **G2. The query plans stay on their indexes** (D9). Exists: `tests/indexes.rs`, `tests/tags_index.rs`, `tests/path_filter.rs`, `src/read.rs:416`. The SQLite version does not change, so no perf run is required.
- **G3. No empty SQL reaches `prepare`** (D9). Static check in Task 9: every `prepare`/`prepare_cached`/`execute`/`query_row` in `crates/crosswire-store` takes a non-empty literal or a `format!` with a fixed SQL prefix. The workspace suite exercises them at runtime.
- **G4. u64 bounds** (D9). Exists: `offset_above_i64_max_is_refused`. Re-run after D9.
- **G5. SHA-256 values are byte-identical** (D2). New:
  - `crates/crosswire-store/tests/packs.rs::blob_hash_is_fips_sha256` stores a body and checks `blobs.hash` against a pinned FIPS 180-4 value. Pack bodies are ≥ 16 KiB, so use 20 KiB of `0x61` and pin its digest, computed once on sha2 0.10 and cross-checked with `shasum -a 256`. The `abc` (`ba7816bf…f20015ad`) and empty (`e3b0c442…7852b855`) vectors go in the next test.
  - `crates/crosswire-proxy/src/capture.rs` `mod tests` gains `relay_sha256_matches_kat`: `CaptureResult::empty().relay.sha256` equals the pinned empty-string digest, and a capture of `abc` equals `ba7816bf…f20015ad`, both as hex literals, not through `sha2`.
  - Existing: RFC 4231 vectors in `preview_token.rs`, and G1's pinned hashes.
- **G6. The ULID text form is frozen** (D8). New `crates/crosswire-model/tests/ulid_text.rs`:
  - `Ulid::from_parts(1_469_918_176_385, 0).to_string() == "01ARYZ6S410000000000000000"`, and `from_parts(…, u128::MAX >> 48)` formats to a pinned string (computed on ulid 1).
  - `from_string` accepts lowercase and refuses `I`, `L`, `O`, `U`, 25 and 27 characters, and a first character above `7`.
  - `Ulid::nil().to_string() == "00000000000000000000000000"` and `ulid::ULID_LEN == 26`.
  - `from_parts(t + 1, 0) > from_parts(t, u128::MAX >> 48)` (keyset ordering, `docs/format.md`).
  - It uses only `from_parts`/`from_string`/`nil`, none of which needs ulid's `std` feature, so it compiles unchanged on ulid 1 and 3.
- **G7. TLS trust is unchanged** (D1). New `crates/crosswire-client/tests/roots.rs::webpki_root_count_is_unchanged`: `webpki_roots::TLS_SERVER_ROOTS.len() == 120` (1.0.10) and an anchor whose `subject` DER contains `ISRG Root X1` is present. Its doc comment says a webpki-roots patch may change the count legitimately, and that the updater bumps the number in the same commit after reading the upstream diff.
- **G8. The API schema is unchanged** (D3, D8). Exists: `openapi_snapshot_is_current`, run without `UPDATE_OPENAPI`.
- **G9. The randomness sources are unchanged in kind** (D7). Review check, recorded in the rand commit body: `crates/crosswire-store/src/control.rs:136`, `crates/crosswire-api/src/session.rs:37`, `tokens.rs:43`, `preview_token.rs:45` still call `rand::random()`, and `crates/crosswire-server/src/ui.rs:157` still calls `rand::rng().fill_bytes`. None moves to a seeded or `StdRng` source. The existing token-uniqueness and CSP nonce tests (`crates/crosswire-server/tests/ui.rs`) keep passing.

## 6. Out of scope
- Removing the leftover duplicates (§3). Upstream's to move.
- A cargo-deny `[graph] targets` restriction for wasm-only crates (`sqlite-wasm-rs`, `rsqlite-vfs`).
- Any schema, migration, `user_version` or `schema_version` change.
- UI (npm) updates (Dependabot `ui-deps`) and the Dependabot policy itself (it also skips 1.x minors; D10 catches the two that exist today).
- The 1M-row `just perf` gate: the bundled SQLite does not change.

## 7. Risks
- **Synthetic data drift (D7).** If rand 0.10 changes `random_range` sampling, the seeded synthetic project changes and bench numbers stop being comparable with `bench/README.md`. Task 7 compares a digest of a 1k-row seeded project before and after.
- **Runtime-only rusqlite change (D9).** `Error::Empty` is not a compile error. G3's grep plus the workspace suite cover it.
- **A fresh release (D9).** rusqlite 0.43.0 is three days old at recon. It is last among the code bumps so it can be dropped alone (§8 Q5).
- **webpki-roots patch cadence (D1).** Mozilla removals arrive as patch updates without review, as they already did through the shim. Task 11 documents this in `SECURITY.md`.

## 8. Open questions (maintainer)
1. **MSRV.** Resolved in 0.1.1: 1.89 (PR #33). Nothing in this refresh needs more.
2. **Downgrade compatibility.** Not promised. A project touched by a newer build is not guaranteed to open in an older one; `docs/format.md` already refuses newer formats, and the format does not change here. No CI job.
3. **base64 `simd-unsafe`.** Kept off (D6): we decode no bulk base64 on hot paths, and it would add unsafe code we do not call.
4. **Root-store policy.** Becomes Task 11: `SECURITY.md` says the upstream trust store follows `webpki-roots` patch releases (Mozilla's root list), so a root removal can make a site fail verification in Repeater or the proxy upstream.
5. **rusqlite 0.43 now or after it ages?** Recommended: take it in this branch (same SQLite, no code change); if the maintainer prefers to wait, drop Task 9 alone.
