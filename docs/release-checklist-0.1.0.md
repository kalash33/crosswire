# Release checklist: v0.1.0

This is the walk-through of the spec §14 "Definition of done" for `v0.1.0`
(`docs/superpowers/specs/2026-10-22-crosswire-design.md`) and the PR description
of the release. Evidence was gathered on 2026-11-02 against `main` at
`526a1bd` ([PR #20](https://github.com/kalash33/crosswire/pull/20)); the CI run
on that merge is
[ci #41338947529](https://github.com/kalash33/crosswire/actions/runs/41338947529) (success).

Status values: **Done** (proven by a named test or run), **Report-only** (measured,
below target, accepted for 0.1.0 by an ADR), **Pending** (not yet done).

## Definition of done

| # | §14 line (quoted) | Status | Evidence |
|---|---|---|---|
| 1 | "golden HTTP/1 suite green on ≥ 200 captures covering every `DiagnosticCode`" | Done | `crates/crosswire-h1/tests/corpus.rs::corpus_golden` over the 207 files in [`corpus/h1/cases`](../corpus/h1/cases) (`.msg` plus `.expect.json` pairs, each case run against its write patterns); `corpus.rs::every_diagnostic_code_has_a_corpus_case` fails if a code lacks a case; `scripts/corpus-check.py`; CI [#41338947529](https://github.com/kalash33/crosswire/actions/runs/41338947529) |
| 2 | "split proptest green" | Done | `crates/crosswire-h1/tests/split_prop.rs::split_invariance` (every corpus message split at every byte boundary); `crates/crosswire-h1/tests/body.rs::chunked_split_at_every_byte_yields_same_payload`; same CI run |
| 3 | "kill -9 test green as specified in §13" | Done | `crates/crosswire-store/tests/durability.rs::kill_minus_nine` with the `killtest-child` binary; same CI run |
| 4 | "anonymous RSS < 500 MB at 1M synthetic exchanges" | Done | Measured peak anonymous RSS 21 MB at N = 1 000 000 (bound 500 MB), after loading, running every named query and scrolling: [`bench/README.md`](../bench/README.md), run through `scripts/perf-gate.sh` (maintainer-run at 1M; asserted at 100k in CI) |
| 5 | "install-to-first-capture < 2 min scripted on tier-1 with an empty scope" | Done | `crates/crosswire-server/tests/first_capture.rs::first_capture_under_two_minutes` (120 s limit, real binary, empty scope); `scripts/first-capture.sh` (`just first-capture`); through the UI: `scripts/first-capture-ui.sh` and `ui/e2e/first-capture-ui.spec.ts` |
| 6 | "all §13 security gates green" | Done, with report-only performance rows | Gates 1 to 18 are tabulated with test names in [`security-gates.md`](security-gates.md); all are `proven` (UI gates 3 and 4 are Playwright specs run by the CI `check` job on ubuntu); performance gates are rows 6a to 6c |
| 6a | §13 performance: ingest "≥ 2k exchanges/s" at the 60/30/9/1 body mix | Report-only | Measured about 700/s, `crates/crosswire-store/tests/perf_ingest.rs` (report-only, 300/s gross floor asserted); decision in [ADR-0006](adr/0006-performance-gates-0-1-0.md) |
| 6b | §13 performance: UI "no frame > 32 ms" over a 10 s scroll and "filter applied < 300 ms" at 1M rows | Report-only | Measured 200 ms max frame, p99 150 ms at 100k rows with real paging (an earlier 1M run: 117 ms); filter 190 ms at 100k, 163 ms at 1M (filter is inside the target); `ui/e2e/perf-scroll.spec.ts` via `just ui-perf`; decision in [ADR-0006](adr/0006-performance-gates-0-1-0.md) |
| 6c | §13 performance: history queries p95 < 50 ms warm and < 500 ms cold at 1M rows | Done | All seven named queries pass at 1M (warm p95 at most 9.7 ms, cold at most 10.4 ms): [`bench/README.md`](../bench/README.md) |
| 7 | "one-command release pipeline (release-plz + cargo-dist) green" | Done, final tag pending | Built in [PR #10](https://github.com/kalash33/crosswire/pull/10) and fixed in [PR #17](https://github.com/kalash33/crosswire/pull/17); rehearsed with the pre-release [`v0.1.0-rc.2`](https://github.com/kalash33/crosswire/releases/tag/v0.1.0-rc.2) built by [Release run #41211813487](https://github.com/kalash33/crosswire/actions/runs/41211813487) (success): macOS arm64, Linux x86_64 and aarch64 archives, installer script, checksums. Attestation checked on 2026-11-02: `gh attestation verify crosswire-server-aarch64-apple-darwin.tar.xz --repo kalash33/crosswire` exited 0. The release-plz half cannot open a release PR until the token exists (see Maintainer actions) |
| 8 | "`docs/format.md` and the `Message` schema published" | Done | [`format.md`](format.md); schema freeze in [PR #9](https://github.com/kalash33/crosswire/pull/9); every gated API response carries `Crosswire-Schema-Version: 2` ([`CHANGELOG.md`](../CHANGELOG.md)) |
| 9 | "name, org and crate names reserved (§16)" | Pending | The `kalash33/crosswire` repository exists; the GitHub organisation, crates.io names, domain and trademark search are a maintainer action (see below) |

## Maintainer actions

These cannot be done from a pull request.

| Action | Status | Why it blocks |
|---|---|---|
| Add the `RELEASE_PLZ_TOKEN` repository secret (fine-grained PAT, contents and pull-requests write; see [GOVERNANCE.md](../GOVERNANCE.md), Releases) | pending | Every `release-plz` run on `main` fails at its first step, for example [run #41338947551](https://github.com/kalash33/crosswire/actions/runs/41338947551): "RELEASE_PLZ_TOKEN secret is not set". Without it no release PR opens and the `v0.1.0` tag cannot be cut the one-command way |
| Name the backup owner in [`MAINTAINERS.md`](../MAINTAINERS.md); set as GitHub successor and second crates.io owner (see [GOVERNANCE.md](../GOVERNANCE.md), Ownership and escrow) | pending | Spec §15: "a specific person agreed before the first public tag". `MAINTAINERS.md` still reads "To be named before v0.1.0". **Blocks the tag.** |
| Reserve the GitHub org, crates.io names (`crosswire-core`, `crosswire-h1`, `crosswire-ca` and the rest), domain and run the trademark search (§16) | pending | DoD line 9 |

## Sub-plans still in flight

| Sub-plan | Status | Branch |
|---|---|---|
| U2, super app (viewer, repeater bench, history navigation, app identity, shortcuts) | merged ([PR #21](https://github.com/kalash33/crosswire/pull/21)) | `feat/u2-super-app` |
| R4, docs and launch (this checklist, README, API docs, ADR-0006, templates) | this pull request | `feat/r4-docs-launch` |

## Launch items (spec §15)

| Item | Owner | Status |
|---|---|---|
| 90-second video built from the §2.3 corpus | maintainer | pending (not part of R4) |
| "Coming from Burp" guide | R4 | done ([`coming-from-burp.md`](coming-from-burp.md)) |
| Public release (`v0.1.0` and its announcement) | maintainer | pending; follows the procedure below |

The success signal for the release is at least 3 weekly-active early-access
creators at +6 weeks. A follow-up at +4 weeks (crates.io 0.1 of `core`, `h1`,
`ca`, r/rust, This Week in Rust, NLnet) is planned separately.

## Merged work behind the evidence

| PR | Subject |
|---|---|
| [#2](https://github.com/kalash33/crosswire/pull/2) | store performance and history indexes |
| [#3](https://github.com/kalash33/crosswire/pull/3) | redaction golden, copy-as-curl, copy-raw (gate 14) |
| [#4](https://github.com/kalash33/crosswire/pull/4) | security-gate hardening (gates 5, 15, 16) |
| [#5](https://github.com/kalash33/crosswire/pull/5) | Limits flat-RSS matrix (gate 18) |
| [#6](https://github.com/kalash33/crosswire/pull/6) | performance harness |
| [#7](https://github.com/kalash33/crosswire/pull/7) | preview zero-egress (gate 4) |
| [#9](https://github.com/kalash33/crosswire/pull/9) | schema freeze (R1) |
| [#10](https://github.com/kalash33/crosswire/pull/10), [#17](https://github.com/kalash33/crosswire/pull/17) | release pipeline and governance (R2) |
| [#20](https://github.com/kalash33/crosswire/pull/20) | pre-tag hardening (R3) |

## Cutting the tag

Do these in order; stop at the first failure.

1. Both pending maintainer actions that block the tag are done: `RELEASE_PLZ_TOKEN` is set and the backup owner is named in `MAINTAINERS.md`.
2. Merge R4 (`feat/r4-docs-launch`) after U2 has merged. Confirm the `ci` run on the merge commit is green.
3. release-plz opens the release pull request. It ignores pre-release tags, so it may propose a stale version: edit the version to `0.1.0` in the pull request ([GOVERNANCE.md](../GOVERNANCE.md), Releases). Per [GOVERNANCE.md](../GOVERNANCE.md), fold the curated `## [Unreleased]` entries of `CHANGELOG.md` into the generated `## [0.1.0]` block so none are stranded beneath it.
4. Merge the release pull request. The merge bumps the version and creates the `v0.1.0` tag.
5. The tag triggers `release.yml` (cargo-dist). Wait for it to finish; if the UI drift gate fails, delete the release, fix `main` and tag again.
6. Verify the release: install from the GitHub release on macOS and run `crosswire --version`; for every archive run `gh attestation verify <file> --repo kalash33/crosswire` (see [SECURITY.md](../SECURITY.md)).
7. Announce, per the launch table above.

**Manual fallback if `RELEASE_PLZ_TOKEN` is not set.** Set `[workspace.package] version` to `0.1.0`, run `cargo update -w` so `Cargo.lock` matches, fold `## [Unreleased]` into a `## [0.1.0] - <date>` block in `CHANGELOG.md`, merge that change through a normal pull request with green CI, then on the merge commit run `git tag -a v0.1.0 -m "crosswire 0.1.0" && git push origin v0.1.0` and continue at step 5.
