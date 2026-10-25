#!/usr/bin/env bash
# Install-to-first-capture THROUGH THE UI: build, serve the embedded production
# bundle, then drive the real served SPA with Playwright — open it with the
# bootstrap fragment, cause one real capture through the proxy, and assert the
# exchange shows in the history view within 120 s. Mirrors scripts/first-capture.sh
# (same serve lifecycle) but drives the UI instead of curl-ing the API.
# Real network egress happens here (https://example.com/), never in a Rust test.
set -euo pipefail

CROSSWIRE_BROWSER="${CROSSWIRE_BROWSER:-$(command -v google-chrome || command -v chromium || command -v chromium-browser || true)}"
: "${CROSSWIRE_BROWSER:?no Chromium-based browser found; set CROSSWIRE_BROWSER}"
export CROSSWIRE_BROWSER
cd "$(dirname "$0")/.."

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
BIN="$PWD/target/release/crosswire"

# The clock starts at launch: install-to-first-capture excludes the build. The
# spec reads CROSSWIRE_START and enforces the 120 s budget against it.
START=$(date +%s)

# Ports are OS-assigned (:0); real addresses are parsed from stdout. The CA
# lives outside the project (serve refuses a CA dir inside it). The bootstrap
# token goes to a 0600 file, never to argv or stdout.
"$BIN" serve --project "$TMP/proj" --ca-dir "$TMP/ca" \
  --api-bind 127.0.0.1:0 --proxy-bind 127.0.0.1:0 \
  --bootstrap-file "$TMP/boot" >"$TMP/serve.out" 2>"$TMP/serve.err" &
SERVE_PID=$!

API="" PROXY=""
for _ in $(seq 1 300); do
  API=$(sed -n 's/^api listening on //p' "$TMP/serve.out" | head -n1)
  PROXY=$(sed -n 's/^proxy listening on //p' "$TMP/serve.out" | head -n1)
  if [ -n "$API" ] && [ -n "$PROXY" ] && [ -s "$TMP/boot" ]; then break; fi
  kill -0 "$SERVE_PID" 2>/dev/null || { echo "serve exited early" >&2; cat "$TMP/serve.err" >&2; exit 1; }
  sleep 0.1
done
if [ -z "$API" ] || [ -z "$PROXY" ] || [ ! -s "$TMP/boot" ]; then
  echo "serve did not report its addresses in time" >&2
  exit 1
fi

# The file holds http://localhost:<port>/#bootstrap=<token>.
TOKEN=$(sed -n 's/.*#bootstrap=\([^[:space:]]*\).*/\1/p' "$TMP/boot" | head -n1)
[ -n "$TOKEN" ] || { echo "no bootstrap token in the bootstrap file" >&2; exit 1; }

# Hand the serve addresses, the single-use token, and the capture inputs to the
# spec. baseURL is the serve API origin; the spec spawns `crosswire browser`
# itself (via Node child_process) to generate the capture through the proxy.
export CROSSWIRE_API_URL="http://$API"
export CROSSWIRE_BOOTSTRAP_TOKEN="$TOKEN"
export CROSSWIRE_PROXY="$PROXY"
export CROSSWIRE_BIN="$BIN"
export CROSSWIRE_PROJECT="$TMP/proj"
export CROSSWIRE_CA="$TMP/ca"
export CROSSWIRE_START="$START"

cd ui
npx playwright test --config playwright.first-capture.config.ts
