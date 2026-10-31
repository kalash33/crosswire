# ADR-0005: Committed UI bundle

Status: Accepted

## Context

The web UI is a single-page app in `ui/`, built with Node (`npm run build`). It
ships embedded in the `crosswire` binary: `crates/crosswire-server/src/ui.rs`
embeds the directory `crates/crosswire-server/assets/ui/` at compile time with
`rust_embed`, and serves it from the API origin.

Product spec section 11.7 says no generated, minified or vendored artefacts are
committed and the UI is built from source in release CI. Following that rule
literally means `cargo build` and `cargo install` need a Node toolchain, or the
crate cannot be built from crates.io at all, because a published crate cannot
run `npm` safely. Crosswire is a security tool. People will build it from
source, and many will do so on a machine that has no Node.

The cost of committing a minified bundle is that a reviewer cannot read it, so a
change to the bundle could hide something that the source does not contain.

## Decision

The minified bundle under `crates/crosswire-server/assets/ui/` is committed, so
`cargo build` and `cargo install` are Node-free. Three controls make that safe:

1. **A drift gate.** `just ui-dist-check` runs `npm ci` and `npm run build` in
   `ui/`, then diffs `ui/dist` against the committed bundle. It fails if they
   differ. CI runs it on every change. The release workflow runs it again as a
   pre-build gate, so a release is refused when the committed bundle is not what
   the source produces.
2. **A locked, script-free install.** The UI dependencies are installed with
   `npm ci --ignore-scripts` from the committed lockfile (justfile and CI), so
   dependency install scripts do not run and the dependency set is the reviewed
   one.
3. **The bundle is a build artefact in review.** It is never edited by hand.
   Reviewers read the source and the lockfile diff; the bundle diff is checked by
   the gate, not by eye. Regenerate it with `just ui-build`.

This deviates from product spec section 11.7. The spec is amended to say no
generated, minified or vendored artefacts are committed, except the UI bundle
under this ADR's drift gate, and that release CI rebuilds the UI from source and
refuses a release when the committed bundle differs.

## Alternatives considered

- **Build the UI in `build.rs` with Node.** This keeps the repository free of
  generated files, but every `cargo build` and `cargo install` then needs Node
  and the network, and a build script that runs `npm` is a larger supply-chain
  surface than a diffable bundle. A crates.io package could not build without it.
  Rejected.
- **Download a prebuilt bundle at build time.** The build would trust a download
  whose contents the source tree does not pin, and it breaks offline and
  reproducible builds. Rejected.
- **A git submodule or separate repository for the bundle.** It moves the same
  committed artefact somewhere else and adds a second repository to keep in step,
  without removing the need for a drift check. Rejected.

## Consequences

- **`cargo install` and `cargo build` need only Rust.** A contributor who does not
  touch the UI never installs Node.
- **A UI change commits two things:** the source and the regenerated bundle. A
  pull request that changes only one fails `just ui-dist-check`.
- **The check depends on a reproducible build.** If the toolchain or a dependency
  makes the build vary between machines, the gate fails for no real reason. The
  lockfile and a pinned Node version in CI are what prevent that.
- **Bundle diffs are noisy.** Reviewers rely on the gate, so the gate must stay
  mandatory. Weakening it needs a new ADR.
- **The repository is larger** by the size of the bundle, and each UI change adds
  to the history.
