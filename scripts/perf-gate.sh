#!/usr/bin/env bash
# Run the history bench and assert the SUMMARY against thresholds. The bench
# only reports (crates/crosswire-store/benches/history.rs); this is the gate.
#   usage: scripts/perf-gate.sh <rows> <warm_ms> <cold_ms> <rss_mb>
# Asserts, for every `kind=structured` query: warm p95 < warm_ms and cold p95 <
# cold_ms; and peak anonymous RSS < rss_mb. Path scans are reported, not gated
# (they are bounded, not indexed). Exits non-zero on any breach or if the
# SUMMARY is missing or incomplete.
set -euo pipefail

if [ "$#" -ne 4 ]; then
  echo "usage: $0 <rows> <warm_ms> <cold_ms> <rss_mb>" >&2
  exit 2
fi
N="$1" WARM_MS="$2" COLD_MS="$3" RSS_MB="$4"
cd "$(dirname "$0")/.."

OUT="$(mktemp)"
trap 'rm -f "$OUT"' EXIT

CROSSWIRE_BENCH_N="$N" CROSSWIRE_BENCH_QUICK=1 \
  cargo bench --locked -p crosswire-store --features synthetic --bench history | tee "$OUT"

awk -v n="$N" -v warm="$WARM_MS" -v cold="$COLD_MS" -v rss="$RSS_MB" '
  function field(key,   i, kv) {
    for (i = 2; i <= NF; i++) {
      split($i, kv, "=")
      if (kv[1] == key) return kv[2]
    }
    return ""
  }
  $1 == "SUMMARY" && $2 ~ /^n=/ { if (field("n") != n) { printf "FAIL: summary n=%s, expected %s\n", field("n"), n; bad = 1 } seen_n = 1 }
  $1 == "SUMMARY" && $2 ~ /^query=/ && $0 ~ /kind=structured/ {
    q = $2; w = field("warm_p95_ms") + 0; c = field("cold_p95_ms") + 0; structured++
    if (w >= warm) { printf "FAIL: %s warm p95 %.3f ms >= %s ms\n", q, w, warm; bad = 1 }
    if (c >= cold) { printf "FAIL: %s cold p95 %.3f ms >= %s ms\n", q, c, cold; bad = 1 }
  }
  $1 == "SUMMARY" && $0 ~ /anon_rss_peak_mb=/ {
    peak = field("anon_rss_peak_mb"); have_rss = 1
    if (peak == "NaN" || peak + 0 >= rss) { printf "FAIL: peak anon RSS %s MB >= %s MB\n", peak, rss; bad = 1 }
  }
  END {
    if (!seen_n || !structured || !have_rss) { print "FAIL: incomplete SUMMARY (n/structured/rss missing)"; exit 1 }
    if (bad) exit 1
    printf "OK: %d structured queries under %s ms warm / %s ms cold, peak anon RSS %s MB < %s MB (n=%s)\n", structured, warm, cold, peak, rss, n
  }
' "$OUT"
