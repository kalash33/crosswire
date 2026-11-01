# R4 Docs and Launch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every document a first visitor reads accurate for the 0.1.0 product, record the performance decisions, add the repository templates, and produce the definition-of-done checklist that becomes the `v0.1.0` release PR.

**Architecture:** Docs-only sub-plan on top of U2's merged product. Three parallel documentation tasks (README + guide; docs truth pass + ADR; templates + checklist), then a link-check gate, review, merge, and the release-plz release PR.

**Spec:** `docs/superpowers/specs/2026-11-01-crosswire-r4-docs-launch-design.md`

## Global Constraints
Docs only (no code, no bundle, no manifest change); every claim verifiable against the code or a CI run (cite the test or run URL); no "alpha" wording left in user-facing docs; relative links resolve; Conventional Commits ≤72; NO AI attribution trailer; `just doc` and the UI dist check unchanged.

## Review Focus
1. A reader following README Install on Linux x86_64 with only the release page → every command works as written against the `v0.1.0-rc.2` assets (T0 verifies the four commands).
2. A Burp user looking for Intruder/Extensions → the guide says "not yet" with the roadmap pointer, not silence (T0).
3. A reader checking `docs/security-gates.md` gate 3 → the cited specs exist (T1 runs `ls` on every cited test path).
4. The DoD checklist links → every CI run URL and test name resolves (T2).

---

### Task 0: README + screenshot + Coming-from-Burp guide (sonnet)
**Files:** Rewrite `README.md` per spec D1 (keep the Developer/Contributing sections' facts; replace Quick start with `crosswire app`, keep the manual path); add `docs/images/crosswire-history-dark.png` (from the U2 final screenshots, ≤300 KB, optimised with `pngquant` if available else as is); create `docs/coming-from-burp.md` per D2; link both from README.
- [ ] Step 1: verify the four install commands against the live `v0.1.0-rc.2` release (download URL 200s; `gh attestation verify` on one archive; `cargo install --git … --locked` is syntactically valid). Step 2: write. Step 3: link check (`grep -o '](\.[^)]*)'` + `test -e`). Commit `docs: README for 0.1.0 and the Coming-from-Burp guide`.

### Task 1: docs truth pass + ADR-0006 (sonnet)
**Files:** `docs/api.md` (diff route list and fields against `docs/api/openapi.json`; add anything missing: `proxy_addr`, `project`, `enforce_strict`, `ChangeDto`), `docs/ca-install.md` (matches the Start panel), `crates/*/README.md` key-types rows (client gate variants; store `analyze_threshold`; api DTOs), `docs/security-gates.md` (gate 3 cites real specs; perf rows REPORT-ONLY with numbers; gate 7 current), new `docs/adr/0006-performance-gates-0-1-0.md` per D4 (numbers from the backlog: ingest ~700/s at the 60/30/9/1 mix vs 2000/s; UI max frame ~117 ms vs 32 ms at 1M; filter 163 ms; the harnesses `crosswire-store/tests/perf_ingest.rs`, `ui/e2e/perf-scroll.spec.ts`, `just ui-perf`; decision: report-only for 0.1.0, re-spec ingest as MB/s + lighter mix and virtualise the table in 0.1.x).
- [ ] Verify each cited test path exists; commit `docs: truth pass over API, CA, crate READMEs, gates; ADR-0006`.

### Task 2: templates, checklist, maintainers (sonnet)
**Files:** `.github/ISSUE_TEMPLATE/bug_report.yml`, `feature_request.yml`, `config.yml` (security → GHSA link, no blank issues), `.github/PULL_REQUEST_TEMPLATE.md` (DCO sign-off, tests run, docs updated, no AI-attribution trailers per CONTRIBUTING), `CONTRIBUTING.md` labels paragraph, `MAINTAINERS.md` backup owner (fill from the maintainer's answer; if none, leave "to be named before v0.1.0" and flag in the checklist), `docs/release-checklist-0.1.0.md` per D6 with evidence links (CI run URLs for the gates from PRs #7–#19 and the R3/U2 PRs, test names, rc.2 release URL).
- [ ] Commit `docs: issue and PR templates; 0.1.0 release checklist`.

### Task 3: gate, review, merge, release PR
- [ ] Link check over all touched Markdown; `just doc`; `npm --prefix ui run lint` (unchanged); whole-branch review (sonnet is enough: docs); PR; CI; merge. Then: watch release-plz open the release PR (requires `RELEASE_PLZ_TOKEN`), fold `[Unreleased]` into `## [0.1.0] - <date>` in that PR, confirm version `0.1.0`, merge → tag `v0.1.0` → release workflow → verify install + attestation → update the backlog memory and the release checklist with the final URLs.
