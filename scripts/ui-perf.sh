#!/usr/bin/env bash
# UI frame-time / filter-latency trace (spec §13): seed a synthetic project of
# CROSSWIRE_UI_PERF_ROWS rows (default 100000; the maintainer run is 1000000),
# serve it with the real `crosswire serve` (embedded production bundle), and
# drive the history view with Playwright. REPORT-ONLY: prints the SUMMARY
# ui_perf line and asserts only gross-regression floors (the 32 ms / 300 ms
# targets are a known gap, see bench/README.md).
# Mirrors scripts/first-capture-ui.sh's serve lifecycle; no network egress.
set -euo pipefail

cd "$(dirname "$0")/.."
ROWS="${CROSSWIRE_UI_PERF_ROWS:-100000}"

TMP="$(mktemp -d)"
SERVE_PID=""
cleanup() {
  if [ -n "$SERVE_PID" ] && kill -0 "$SERVE_PID" 2>/dev/null; then
    kill -INT "$SERVE_PID" 2>/dev/null || true
    wait "$SERVE_PID" 2>/dev/null || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT

cargo build --release --locked -p crosswire-server
cargo build --release --locked -p crosswire-store --features synthetic --example seed_project
BIN="$PWD/target/release/crosswire"

"$PWD/target/release/examples/seed_project" "$TMP/proj" "$ROWS"

# The CA lives outside the project; ports are OS-assigned; the bootstrap token
# goes to a 0600 file (see first-capture-ui.sh).
"$BIN" serve --project "$TMP/proj" --ca-dir "$TMP/ca" \
  --api-bind 127.0.0.1:0 --proxy-bind 127.0.0.1:0 \
  --bootstrap-file "$TMP/boot" >"$TMP/serve.out" 2>"$TMP/serve.err" &
SERVE_PID=$!

API=""
# A freshly seeded project is opened as an unclean shutdown, and serve's
# recovery scan over 1M rows takes ~2 min; allow up to 10 min.
for _ in $(seq 1 6000); do
  API=$(sed -n 's/^api listening on //p' "$TMP/serve.out" | head -n1)
  if [ -n "$API" ] && [ -s "$TMP/boot" ]; then break; fi
  kill -0 "$SERVE_PID" 2>/dev/null || { echo "serve exited early" >&2; cat "$TMP/serve.err" >&2; exit 1; }
  sleep 0.1
done
if [ -z "$API" ] || [ ! -s "$TMP/boot" ]; then
  echo "serve did not report its address in time" >&2
  exit 1
fi
TOKEN=$(sed -n 's/.*#bootstrap=\([^[:space:]]*\).*/\1/p' "$TMP/boot" | head -n1)
[ -n "$TOKEN" ] || { echo "no bootstrap token in the bootstrap file" >&2; exit 1; }

export CROSSWIRE_API_URL="http://$API"
export CROSSWIRE_BOOTSTRAP_TOKEN="$TOKEN"
export CROSSWIRE_UI_PERF_ROWS="$ROWS"

cd ui
npx playwright test --config playwright.perf-scroll.config.ts
