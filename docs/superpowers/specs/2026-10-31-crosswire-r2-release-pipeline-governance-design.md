# R2: release pipeline and governance (0.1.0 release tail, part 2 of 4)

**Status:** approved 2026-10-31. **Parent:** the 0.1.0 release-tail decomposition (R1 schema freeze → **R2** → R3 client gate ranges → R4 docs/launch → tag `v0.1.0`). **Product spec:** `docs/superpowers/specs/2026-10-22-crosswire-design.md` §11.7 (supply chain and governance), §12 (platforms and distribution), §14 (0.1.0 DoD), §15 (governance, SLAs, escrow, cadence).

## 1. Goal

A tagged push builds, attests and publishes the `crosswire` binary for the tier-1 targets with one command, and the repository carries the governance documents the spec requires before the first public tag. R2 is proven by cutting `v0.1.0-rc.1` as a GitHub pre-release and installing it with `cargo binstall`.

## 2. What exists (from the pre-flight scan)

- Twelve crates, every one `version = "0.0.0"` hard-coded and `publish = false`; the only product binary is `crosswire` from `crates/crosswire-server`. `Cargo.lock` committed. Workspace `rust-version = "1.88"`.
- One workflow, `.github/workflows/ci.yml` (check on ubuntu+macos, msrv, deny, nightly fuzz-smoke). Actions SHA-pinned. No tags, no release workflow, no `release-plz.toml`, no dist config, no Dependabot, no CODEOWNERS.
- The UI bundle is committed under `crates/crosswire-server/assets/ui/`, embedded with `rust-embed`, kept honest by `just ui-dist-check` (fresh `npm ci && npm run build`, `diff -r`). `cargo build` is Node-free.
- Root docs: README (no install section, still says "backend without UI"), single Keep-a-Changelog `CHANGELOG.md` with `[Unreleased]` → `### alpha.2` / `### alpha.1`, CONTRIBUTING (DCO, AI-assisted rule, design-locked crates, "no SLA until v0.2"), SECURITY (GHSA, 72 h, 90 d), Code of Conduct 2.1, dual licence. ADRs 0001–0004 in `docs/adr/`.

## 3. Decisions

### D1 Versions
- Every crate moves to `version.workspace = true`; `[workspace.package] version = "0.1.0-rc.1"` (bumped to `0.1.0` by the final release PR, D7). Intra-workspace path deps drop their `version = "0.0.0"` pins (path-only; nothing is published in 0.1.0). One version for the whole workspace until the first crates.io publish (v0.2a), when `crosswire-core`/`h1`/`ca` get independent versions as §11.7 says. Rationale: a single binary release has one version; independent crate versions before anything is published is bookkeeping without a consumer.
- `publish = false` stays on every crate. `just package` keeps packaging `crosswire-core` as a dry run.

### D2 release-plz (versions, changelog, tag)
- `release-plz.toml` at the root: `[workspace] publish = false`, `git_release_enable = false` (dist owns the GitHub release), `git_tag_enable = false`, `changelog_update = true`, `changelog_path = "CHANGELOG.md"` (one workspace changelog until v0.2a; §11.7's per-crate changelogs start when crates publish), `dependencies_update = false`, `semver_check = false` (nothing published). One package override: `[[package]] name = "crosswire-server"`, `git_tag_enable = true`, `git_tag_name = "v{{ version }}"`. Result: one tag `vX.Y.Z` per release, created when the release PR merges.
- `.github/workflows/release-plz.yml`: on push to `main`, runs `release-plz release-pr` and `release-plz release` using `MarcoIeni/release-plz-action` pinned by SHA, with `permissions: contents: write, pull-requests: write`. It uses a `RELEASE_PLZ_TOKEN` repository secret (a fine-grained PAT with contents+pull-requests on this repo) because PRs opened with `GITHUB_TOKEN` do not trigger `ci.yml`; the workflow fails fast with a clear message if the secret is missing. **User action:** create the secret.
- Changelog: restructure `CHANGELOG.md` so `[Unreleased]` holds only post-alpha.2 entries (beta-1 tags/notes/decoder, preview zero-egress, R1 schema freeze, R2) under Keep-a-Changelog headings, and the alpha.1/alpha.2 blocks become `## [0.1.0-alpha.1]` / `## [0.1.0-alpha.2]` historical sections (no tags exist for them; say so in one line). release-plz's `git-cliff` config (`[changelog]` in `release-plz.toml`) produces Conventional-Commit groups matching those headings.

### D3 cargo-dist (build, attest, publish)
- `dist init` writes `dist-workspace.toml` (committed) and `.github/workflows/release.yml` (committed, generated; a CI step `dist generate --check` guards drift). Settings: `ci = "github"`, `installers = ["shell"]` (the shell installer is what `cargo binstall` and the README one-liner use), `targets = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "aarch64-apple-darwin"]` (the §12 tier-1 set; tier-2 macOS x86_64 and Windows are added non-blocking in 0.1.x), `github-attestations = true` (SLSA provenance via GitHub's Sigstore-backed `actions/attest-build-provenance`, satisfying §11.7 "Sigstore-signed releases with SLSA provenance" without any long-lived key), `pr-run-mode = "plan"`, `allow-dirty = []`, `min-glibc-version` default. Linux builds are `gnu` on native runners (`ubuntu-22.04`, `ubuntu-22.04-arm`); the spec's "single static binary" is met in the sense that matters (UI embedded, no runtime files); a fully static musl build is a 0.1.x follow-up and is written down as such.
- `dist` builds `crates/crosswire-server` only (`[workspace.metadata.dist] dist = true` on that package, `dist = false` elsewhere, or dist's package filter; the implementer uses whichever the installed dist version supports).
- The generated `release.yml` gets a **pre-build gate job** that runs `just ui-dist-check` (Node 24, `npm ci --ignore-scripts`) so a release can never ship a bundle that differs from `ui/` source. This is the control that reconciles §11.7's "UI built from source in release CI" with the committed bundle (see D6).
- Binstall: `[package.metadata.binstall]` in `crates/crosswire-server/Cargo.toml` with `pkg-url` matching dist's artifact naming (`{ repo }/releases/download/v{ version }/crosswire-{ target }{ archive-suffix }`), `pkg-fmt = "tgz"`, and `bin-dir = "crosswire-{ target }/{ bin }{ binary-ext }`. Signature verification (binstall `signing` with a minisign key) is deferred: GitHub attestations already give verifiable provenance (`gh attestation verify`), and a maintainer-held signing key contradicts the escrow rule "no long-lived secrets". Recorded in the README install section and ADR-0005.
- SBOM: a `cargo cyclonedx` step is NOT added in R2 (tool install cost on every release, no consumer yet); listed as a 0.1.x item. cosign-signed Docker image: out of scope (no Docker image in 0.1.0).

### D4 Supply-chain hygiene in CI (cheap, spec'd)
- `.github/dependabot.yml`: `cargo` (weekly, grouped), `npm` in `/ui` (weekly, grouped), `github-actions` (weekly).
- `npm ci` → `npm ci --ignore-scripts` in `justfile` (`ui-build`, `ui-dist-check`) and `ci.yml`; verified by `just ui-dist-check` still passing (the UI deps need no install scripts; if one does, R2 reports it instead of dropping the flag).
- `cargo deny check` already covers advisories; cargo-audit/osv-scanner are duplicates and are NOT added. cargo-semver-checks starts at the first publish (v0.2a).
- `.github/CODEOWNERS`: `* @kalash33`.

### D5 Governance documents
- `GOVERNANCE.md`: BDFL (the maintainer) until two maintainers are listed; then lazy consensus, RFC issues with a 14-day timebox and a maintainer vote; contributor ladder triager → committer → maintainer by vote; crates.io ownership via a GitHub team of ≥ 2 before the first publish; named backup owner and second crates.io owner; release cadence (tagged release or pre-release every 4–6 weeks); archival plan (six months without a tag → "unmaintained" banner, pin last release, hand over or archive); how decisions are recorded (`docs/adr/`).
- `MAINTAINERS.md`: the maintainer, and a **Backup owner** row that reads "to be named before `v0.1.0`" (user action; R4 checks it is filled before the final tag).
- `CONTRIBUTING.md`: add the SLA block as spec'd ("Before v0.2: best-effort except security 72 h. From v0.2: issue first response 7 days, PR review 14 days, security 72 h."), point to GOVERNANCE.md, and state the DCO rule in the form release-plz PRs can satisfy (bot PRs are exempt from DCO; the maintainer's merge is the sign-off).
- `SECURITY.md`: add "Releases carry GitHub build-provenance attestations; verify with `gh attestation verify <file> --repo kalash33/crosswire`."
- Issue/PR templates and labels: deferred to R4 (launch assets).

### D6 ADR-0005: committed UI bundle
`docs/adr/0005-committed-ui-bundle.md` records the deviation from §11.7: the minified bundle is committed so `cargo build` is Node-free (install-to-first-capture and `cargo binstall`/`cargo install` paths), with the compensating controls: `just ui-dist-check` drift gate in CI and as a release pre-build gate, `npm ci --ignore-scripts` with a lockfile, and the bundle treated as a build artefact in review (never hand-edited). The product spec §11.7 sentence is amended in the same commit to "no generated artefacts committed except the UI bundle under the ADR-0005 drift gate".

### D7 Validation: `v0.1.0-rc.1`
After R2 merges to `main`, the controller tags `v0.1.0-rc.1` by hand (release-plz is not asked to cut a pre-release) and pushes the tag. `release.yml` must go green, the GitHub pre-release must carry three archives, checksums, the shell installer and attestations, and `cargo binstall crosswire --git https://github.com/kalash33/crosswire` (or the dist shell installer) must install a binary whose `crosswire --version` prints `0.1.0-rc.1`. dist requires the tag to equal the package version, so the R2 branch sets `[workspace.package] version = "0.1.0-rc.1"` (D1 reads `0.1.0-rc.1`, not `0.1.0`); R4's final release PR (release-plz) bumps to `0.1.0` and tags `v0.1.0`. Failures are fixed forward on `main` with new rc tags (`rc.2`, …).

## 4. Out of scope (0.1.x or later, recorded)
musl static Linux builds; tier-2 targets (macOS x86_64, Windows); binstall signature verification; SBOM; Docker image + cosign; Homebrew cask; per-crate changelogs and crates.io trusted publishing (v0.2a); cargo-semver-checks; issue/PR templates (R4); the "Coming from Burp" guide, video and README rewrite (R4, but R2 adds the install section because R2 produces the installable artefacts).

## 5. User actions (cannot be done by the agent)
1. Create the `RELEASE_PLZ_TOKEN` fine-grained PAT secret (contents: write, pull requests: write on `kalash33/crosswire`).
2. Name the backup owner (GitHub successor + future second crates.io owner) and fill `MAINTAINERS.md`.
3. Reserve names (GitHub org, crates.io `crosswire*`, domain) per §16; not blocking R2.

## 6. Constraints carried
No new Rust dependencies (dist and release-plz are CI tools, not deps); `unsafe_code = "forbid"`; MSRV 1.88; `cargo build` stays Node-free; `cargo deny` clean; every action SHA-pinned; no Claude attribution anywhere; Conventional Commits ≤ 72-char subjects; full CI gate before push.

## 7. Risks
- `dist init` output is version-specific; the plan pins the dist version `dist init` writes and the `release.yml` is regenerated, never hand-edited except via the documented pre-build-gate insertion (which dist supports through `[dist.github-custom-job-permissions]`/custom jobs; if the installed dist cannot host a custom pre-build job, the gate becomes a required status check from `ci.yml` on the tag instead, and the plan says which).
- `ubuntu-22.04-arm` runner availability on the free tier for public repos (available since 2025 for public repos; if the job cannot schedule, aarch64-linux is cross-built with `cargo-zigbuild` via dist's `cross` support, or dropped to tier 2 for rc.1 with a note).
- release-plz's first run on a repo with no tags proposes `0.1.0-rc.1 → ?`; verified in the plan by `release-plz update --dry-run` locally before the workflow is enabled.
