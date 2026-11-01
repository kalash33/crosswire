# R4: docs and launch (0.1.0 release tail, 4 of 4)

**Status:** approved 2026-11-01. **Parent:** release tail (R1 ✓, R2 ✓, U1 ✓, R3, U2 → **R4** → `v0.1.0`). **Product spec:** §14 (0.1.0 DoD), §15 (launch table), §16 (name reservation), §13 (performance gates still report-only).

## 1. Goal
Everything a first visitor to the repository or the release page needs is accurate and complete for the final product (post U1/U2), the §14 definition of done is walked line by line with evidence, the two open performance decisions are recorded as an ADR, and the release PR that becomes `v0.1.0` is cut.

## 2. Decisions
- **D1 README rewrite** for 0.1.0: what Crosswire is (one paragraph, honest scope: HTTP/1 proxy, history, repeater, intercept, scope, decoder; no intruder, no HTTP/2 yet), a screenshot of the new UI (dark) committed under `docs/images/` (PNG from the U2 screenshot set, ≤ 300 KB), Install (binstall / installer / tarball + attestation / from source), Quick start using `crosswire app` (U2) with the manual `serve` + `browser` path kept, Features table with status, Security model summary (§11 bullets with links to `docs/security-gates.md`), Project status + roadmap table (v0.1.0 shipped, 0.1.x, v0.2 items), Contributing/Governance/Security links, licence. Remove every "alpha" wording.
- **D2 "Coming from Burp" guide** (`docs/coming-from-burp.md`): a mapping table (Proxy → History/Intercept, Repeater → Repeater, Target/Scope → Scope, Decoder → Decoder, Intruder → not yet, Extensions → not yet), the CA install flow, the three things that differ (byte-exact raw editing, scope-first design, single binary with embedded UI), shortcut cheat sheet.
- **D3 Docs refresh:** `docs/api.md` against the OpenAPI document (every route and field present, incl. `proxy_addr`, `project`, `enforce_strict`, `ChangeDto`); `docs/format.md` already current (R1/R3); `docs/ca-install.md` checked against the Start panel wording; `crates/*/README.md` key-types rows; `docs/security-gates.md` status column truthful (gate 3 XSS row: cite the real specs instead of the nonexistent `just ui-e2e`; perf rows marked REPORT-ONLY with the measured numbers and the ADR link).
- **D4 ADR-0006 performance gates:** record the two §13 misses with numbers (ingest ~700/s vs 2000/s at the 60/30/9/1 mix; UI frame 117 ms vs 32 ms at 1M rows, filter 163 ms) and the decision for 0.1.0: ship with REPORT-ONLY gates, re-specify ingest as sustained MB/s plus the lighter mix for 0.1.x, virtualise the history table in 0.1.x; list the measurement harnesses.
- **D5 Repository hygiene for launch:** `.github/ISSUE_TEMPLATE/{bug_report,feature_request,security}.yml` (security points to GHSA), `.github/PULL_REQUEST_TEMPLATE.md` (DCO, tests, docs checklist), labels documented in CONTRIBUTING, `CODE_OF_CONDUCT.md` contact checked, `MAINTAINERS.md` backup owner filled from the maintainer's answer (or the tag waits).
- **D6 §14 DoD walk-through** (`docs/release-checklist-0.1.0.md`): each DoD line with the evidence (test name, CI run URL, measurement) and the three user actions (RELEASE_PLZ_TOKEN, backup owner, name reservations) with their status; the §15 launch items (video, guide) listed with owner and date; the checklist is the PR description of the release.
- **D7 Cutting `v0.1.0`:** merge R4 → release-plz opens the release PR (`0.1.0-rc.2 → 0.1.0`, folds curated `[Unreleased]` into `## [0.1.0]`); the maintainer merges it → tag `v0.1.0` → `release.yml` builds the attested release; install from the GitHub release verified on macOS (controller) and the Linux archive checked by attestation; announce per §15 is the maintainer's.

## 3. Out of scope
The 90-second video (maintainer-produced; the checklist lists it), crates.io publishing (v0.2a), Homebrew, Docker, docs site.

## 4. Tasks (plan to follow on approval)
T0 README + screenshot + guide (sonnet) → T1 docs refresh + security-gates truthing + ADR-0006 (sonnet) → T2 templates + checklist + MAINTAINERS (sonnet) → T3 gate (docs-only: link check, `just doc`, UI dist unchanged), review, PR, merge → release PR → tag.
