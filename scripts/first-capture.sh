#!/usr/bin/env bash
# Install-to-first-capture: build, serve, drive a real headless browser through
# the proxy, and wait for the first recorded exchange. Fails over 120 s.
# Real network egress happens here (https://example.com/), never in a Rust test.
set -euo pipefail

LIMIT=120
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

# The clock starts at launch: install-to-first-capture excludes the build.
START=$(date +%s)

# Ports are OS-assigned (:0); the real addresses are parsed from stdout. The
# CA lives outside the project (serve refuses a CA dir inside it). The
# bootstrap token goes to a 0600 file, never to argv or stdout.
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
COOKIE=$(curl -fsS -D - -o /dev/null -X POST "http://$API/api/v1/session" \
  -H 'Content-Type: application/json' -d "{\"bootstrap\":\"$TOKEN\"}" |
  tr -d '\r' | sed -n 's/^[Ss]et-[Cc]ookie: \([^;]*\).*/\1/p' | head -n1)
[ -n "$COOKIE" ] || { echo "no session cookie from the API" >&2; exit 1; }

# serve holds the project's exclusive lock, so the launcher is pointed at the
# running proxy with --proxy rather than opening the project's bind setting.
"$BIN" browser --project "$TMP/proj" --ca-dir "$TMP/ca" --proxy "$PROXY" \
  --headless --url https://example.com/ -- \
  --no-sandbox --disable-gpu --disable-dev-shm-usage --disable-extensions \
  --disable-background-networking --disable-component-update --disable-sync \
  --disable-default-apps --dump-dom --timeout=15000 >"$TMP/browser.out" 2>&1 || {
  echo "browser launch failed:" >&2
  cat "$TMP/browser.out" >&2
  exit 1
}

while :; do
  ELAPSED=$(($(date +%s) - START))
  BODY=$(curl -fsS "http://$API/api/v1/exchanges?host_contains=example.com" \
    -H "Cookie: $COOKIE" -H 'Sec-Fetch-Site: same-origin' || true)
  case "$BODY" in
    *'"items":[{'*) break ;;
  esac
  if [ "$ELAPSED" -ge "$LIMIT" ]; then
    echo "FAIL: no example.com exchange within ${LIMIT}s" >&2
    exit 1
  fi
  sleep 0.5
done

echo "first capture in ${ELAPSED}s (limit ${LIMIT}s)"
