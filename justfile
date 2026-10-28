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

# UI frame-time / filter-latency trace (§13) against a real `serve` on a seeded
# synthetic project. CROSSWIRE_UI_PERF_ROWS sets the size (default 100000; the
# maintainer run is 1000000 and takes minutes to seed). REPORT-ONLY (gross
# floors only; the 32 ms / 300 ms targets are a known gap). Needs
# Playwright's chromium (`npx playwright install chromium`).
ui-perf:
    scripts/ui-perf.sh

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

# Pairing, schema and per-DiagnosticCode coverage of the golden corpus
# (stdlib-only; -I keeps the interpreter isolated from the cwd).
corpus-check:
    python3 -I scripts/corpus-check.py corpus/h1

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

# Maintainer-only full gate (spec §13): 1 000 000 rows, structured p95 < 50 ms
# warm / < 500 ms cold, peak anonymous RSS < 500 MB. Takes minutes (the load
# alone is ~3 min) and the numbers depend on the machine, so CI never runs it;
# compare the output with the committed baseline in bench/README.md.
#
# Also runs the proxy gates ASSERTING (CROSSWIRE_PERF_ASSERT=1, --release):
# added-latency p50 < 2 ms and MITM throughput >= 5000 req/s. The store ingest
# and UI frame/filter gates run REPORT-ONLY (known gaps vs the spec §13
# targets, tracked in the pre-0.1.0 backlog); read their SUMMARY lines against
# bench/README.md. The UI step needs Playwright's chromium and defaults to
# 100k rows (CROSSWIRE_UI_PERF_ROWS=1000000 for the full size).
perf:
    scripts/perf-gate.sh 1000000 50 500 500
    CROSSWIRE_PERF_ASSERT=1 cargo test --locked --release -p crosswire-proxy --test perf_added_latency -- --nocapture
    CROSSWIRE_PERF_ASSERT=1 cargo test --locked --release -p crosswire-proxy --test perf_throughput -- --nocapture --test-threads=1
    cargo test --locked --release -p crosswire-store --test perf_ingest -- --nocapture
    scripts/ui-perf.sh
    @echo "Compare the SUMMARY lines above with the committed baselines and ledger: bench/README.md"

# Authoritative added-latency p99 / throughput cross-check with `oha` (a
# separate process, so no in-process harness contention). Maintainer-only:
# `cargo install oha` yourself; it is not a repo or CI dependency. See the
# "oha cross-check" section of bench/README.md for the full recipe.
perf-oha:
    @echo "See bench/README.md, section 'oha cross-check (authoritative p99 and throughput)'."

# CI regression guard, hardware-independent. (1) The EXPLAIN QUERY PLAN tests
# fail if any named history query stops using its index (or the status query
# regains a temp sort): a dropped index flips the plan to SCAN, on any machine.
# (2) A 100k-row run with bounds ~4x/2x looser than the 1M gate, a sanity check
# for gross regressions that CI hardware noise cannot trip.
#
# (3) The proxy perf tests (added-latency, throughput) in --release and
# REPORT-ONLY (no CROSSWIRE_PERF_ASSERT): proves they build and run and that
# their hard invariants hold; the tight numbers are hardware-sensitive and are
# asserted maintainer-side by `just perf`. The 1M ingest and UI runs are
# maintainer-only (slow, GBs of disk).
perf-smoke:
    cargo test --locked -p crosswire-store --features synthetic --test indexes --test tags_index --test path_filter
    scripts/perf-gate.sh 100000 200 1000 250
    cargo test --locked --release -p crosswire-proxy --test perf_added_latency -- --nocapture
    cargo test --locked --release -p crosswire-proxy --test perf_throughput -- --nocapture --test-threads=1
