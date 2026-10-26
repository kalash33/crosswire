test:
    cargo test --locked --workspace --all-features

# Needs a Chromium-based browser: set CROSSWIRE_BROWSER or have google-chrome
# or chromium on PATH.
browser-test:
    #!/usr/bin/env bash
    set -euo pipefail
    export CROSSWIRE_BROWSER="${CROSSWIRE_BROWSER:-$(command -v google-chrome || command -v chromium || command -v chromium-browser || true)}"
    : "${CROSSWIRE_BROWSER:?no Chromium-based browser found; set CROSSWIRE_BROWSER}"
    cargo test --locked -p crosswire-proxy --test browser -- --ignored --nocapture
    cargo test --locked -p crosswire-server --test browser -- --ignored --nocapture

# Real headless browser to https://example.com/ through a fresh serve; needs
# network and a Chromium-based browser (CROSSWIRE_BROWSER or PATH).
first-capture:
    scripts/first-capture.sh

# Install-to-first-capture THROUGH THE UI (§12 DoD): builds and starts a real
# `crosswire serve` (embedded production bundle), then drives the served SPA
# with Playwright to prove the example.com capture appears in the history view
# under 120 s. Needs network, a Chromium-based browser (CROSSWIRE_BROWSER or
# PATH), and Playwright's chromium (`npx playwright install chromium`).
first-capture-ui:
    scripts/first-capture-ui.sh

# Build the Svelte UI bundle and stage it into the server crate for embedding.
# Run this after changing ui/src; `cargo build` stays Node-free because the
# built bundle is committed under crates/crosswire-server/assets/ui/ and
# rust-embed embeds it at compile time.
ui-build:
    #!/usr/bin/env bash
    set -euo pipefail
    cd ui
    npm ci
    npm run build
    dest=../crates/crosswire-server/assets/ui
    rm -rf "$dest"
    mkdir -p "$dest"
    cp -R dist/. "$dest"/
    # Keep the directory tracked even if the bundle is ever emptied.
    touch "$dest/.gitkeep"

# ESLint over ui/src. Assumes `npm ci` has already populated ui/node_modules
# (CI runs it once before the ui-* recipes; locally `just ui-build` does too).
ui-lint:
    cd ui && npm run lint

# Vitest unit tests only; Playwright e2e is wired separately. Assumes deps are
# installed (see ui-lint).
ui-test:
    cd ui && npm run test

# Drift check: rebuild ui/ from scratch and byte-compare ui/dist against the
# committed bundle, like the openapi_snapshot_is_current test. .gitkeep is
# excluded because ui-build adds it to the committed dir but it is not in dist.
# Installs deps itself so it is correct standalone. Fix drift with `just ui-build`.
ui-dist-check:
    #!/usr/bin/env bash
    set -euo pipefail
    (cd ui && npm ci && npm run build)
    if ! diff -r --exclude=.gitkeep ui/dist crates/crosswire-server/assets/ui; then
      echo "error: committed UI bundle is stale; run 'just ui-build' and commit" >&2
      exit 1
    fi

serve *ARGS:
    cargo run --locked -p crosswire-server -- serve {{ARGS}}

lint:
    cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
    cargo fmt --check

features:
    cargo check --locked -p crosswire-core --no-default-features
    cargo check --locked -p crosswire-core --no-default-features --features serde
    cargo check --locked -p crosswire-h1 --no-default-features
    cargo check --locked -p crosswire-h1 --no-default-features --features std
    cargo check --locked -p crosswire-h1 --no-default-features --features http
    cargo check --locked -p crosswire-h1 --no-default-features --features serde
    cargo check --locked -p crosswire-model --no-default-features
    cargo check --locked -p crosswire-model --no-default-features --features std
    cargo check --locked -p crosswire-scope --no-default-features
    cargo check --locked -p crosswire-proxy

# Seeds the libFuzzer corpora from the golden corpus. The targets read their
# configuration from the first two bytes, so each case gets a 2-byte prefix
# (0x10: Lenient, pieces of 9 bytes; 0x06: GET context, default limits).
fuzz-seed:
    #!/usr/bin/env bash
    set -euo pipefail
    for t in parse_request parse_response; do
      d=crates/crosswire-h1/fuzz/corpus/$t
      mkdir -p "$d"
      for f in corpus/h1/cases/*.msg; do
        { printf '\x10\x06'; cat "$f"; } > "$d/seed-$(basename "$f" .msg)"
      done
    done

fuzz-smoke: fuzz-seed
    cd crates/crosswire-h1/fuzz && cargo +nightly fuzz run parse_request -- -max_total_time=60 && cargo +nightly fuzz run parse_response -- -max_total_time=60

doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --all-features

# crosswire-h1 is packaged locally only until crosswire-core is published:
# `cargo package` resolves path dependencies from the registry.
package:
    cargo package -p crosswire-core --locked

bench:
    cargo bench -p crosswire-h1 --locked --no-run
    cargo bench -p crosswire-store --locked --features synthetic --no-run
