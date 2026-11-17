# ADR-0006: Performance gates at 0.1.0

Status: Accepted. The UI frame-time half is closed in 0.1.1 (see Update); the
ingest half stands.

## Context

Spec section 13 sets performance targets for v0.1.0. Most are met and asserted
(history queries at 1M rows, peak memory, proxy added latency and throughput,
first capture in the UI under two minutes). Two are not:

- **Store ingest.** The target is at least 2,000 exchanges/s committed through
  the real writer path at the section 13 body mix (60 % under 4 KiB, 30 %
  4 to 64 KiB, 9 % 64 KiB to 1 MiB, 1 % 1 to 8 MiB). The harness is
  `crates/crosswire-store/tests/perf_ingest.rs` (`just perf`). Measured on the
  reference machine (MacBook Air M2, 8 GiB, release build): about 700
  exchanges/s, three runs at 698, 708 and 722, which is about 65 MB/s of body
  bytes.
- **UI frame time.** The target is no animation frame over 32 ms during a 10 s
  scroll of the History view. The harness is `ui/e2e/perf-scroll.spec.ts`,
  driven by `just ui-perf` (`scripts/ui-perf.sh`), which seeds a synthetic
  project with `crates/crosswire-store/examples/seed_project.rs`, serves it with
  the real `crosswire serve`, and samples frames passively in headless
  Chromium. Measured at 100k rows with the real paging the UI ships: max frame
  200 ms, p99 150 ms, 76 of 356 frames over 32 ms. An earlier maintainer run at
  1M rows measured a 117 ms max frame.

The UI filter target (under 300 ms) is met: 190 ms at 100k rows and 163 ms at
1M. Peak anonymous RSS at 1M rows is 21 MB against a 500 MB bound.

Root causes:

- **Ingest** is bound by body volume, not by exchange count. The mean exchange
  at this mix carries about 100 KB, because the 9 % and 1 % buckets dominate, so
  2,000/s would need about 200 MB/s through one writer thread. That thread does
  SHA-256, zstd level 3, the pack append and `fsync` serially and is CPU-bound
  at about 65 MB/s. The disk is not the limit (a full-fsync write sustains about
  350 MB/s on the same machine), and the same harness with bodies capped at
  65 KiB commits about 2,400 exchanges/s.
- **Frame time** is the DOM. The History table is not virtualised: every loaded
  row stays in the document, growing to about 3.8k rows (about 48k nodes) at 100k
  rows, so layout and style cost grows with how far you have scrolled.

## Decision

Ship 0.1.0 with both gates **REPORT-ONLY**. Each harness prints a `SUMMARY`
line, and the numbers are recorded in [bench/README.md](../../bench/README.md)
and [security-gates.md](../security-gates.md). Only gross-regression floors
assert, so a collapse still fails: ingest below 300 exchanges/s (release builds),
a max frame of 2 s or more, and a filter of 5 s or more. The section 13 targets
themselves stay as written in the spec; this ADR records that they are missed,
and does not change them.

Commitments for 0.1.1:

1. **Virtualise the History table**, rendering only the rows in view, and turn
   `perf-scroll` into an asserting CI gate at 100k rows against the 32 ms frame
   target. A spec for this is drafted.
2. **Re-specify ingest** as sustained MB/s with a lighter, more typical mix,
   and/or raise the writer's throughput with parallel hashing and compression
   (more than one core), then assert the new figure.

## Alternatives considered

- **Block the release until both targets are met.** Virtualising a table and
  re-architecting the writer are each larger than a pre-tag fix, and neither
  miss loses data or breaks a security property: the ingest queue spills to a
  journal and, past the journal cap, drops with a gap marker rather than silently, and the UI
  stays usable (it is slow to scroll at tens of thousands of rows, not wrong).
  Holding the tag would delay every other 0.1.0 deliverable for a gap that is
  documented and measured.
- **Lower the targets until the numbers pass.** This would make the gate green
  without changing anything a user feels. It would also hide the gap, and the
  number we lowered it to would be arbitrary. The targets were chosen from what
  a busy session needs, and the ingest one will be re-specified in 0.1.1 from
  measured behaviour, in the open, not tuned to the current result.
- **Assert the current numbers as the new gate.** Rejected for the same reason
  as lowering the targets, and the figures are machine-dependent enough (the
  authoritative run is on the reference machines) that CI hardware would make
  such a gate flaky.

## Consequences

- A user who scrolls History past a few thousand rows will see dropped frames
  and a sluggish list on this class of machine. Filtering and searching stay
  fast, so narrowing the list is the workaround.
- A sustained burst of large bodies (several MB each, many per second) can
  outrun the writer. The bounded queue, the journal spill and the gap marker
  keep this visible and bounded, but capture falls behind real time.
- The gross-regression floors keep catching collapses, but a gradual slowdown
  between the measured figures and the floors will not fail CI. Maintainers
  read the `SUMMARY` lines in `just perf` against the ledger.
- The 0.1.1 work has a concrete harness and a number to hit for each gate, so
  turning them into asserting gates is a matter of the changes above, not of
  new measurement tooling.

## Update (0.1.1): UI gate asserting

Commitment 1 is done. The History list is virtualised (only the rows in view
plus `OVERSCAN` = 8 above and below are in the DOM) and auto-loads keyset pages
as the window nears the loaded end. `perf-scroll` is now an asserting gate at
<= 100k rows (`CROSSWIRE_UI_PERF_ASSERT`, defaulted by `scripts/ui-perf.sh`):
max frame <= 32 ms, filter < 300 ms, DOM rows <= 2 x visible + OVERSCAN
(the spec's acceptance bound), at least one auto-loaded page and at least
1,000 rows loaded. The section 13 targets are unchanged. CI runs it at 100k in
its own `ui-perf` job on ubuntu with one Playwright retry for shared-runner
noise. The retry re-runs the whole measurement in a fresh page on the same
serve and session (the single-use bootstrap token is exchanged once in a global
setup and the session reused from a 0600 temp file the script deletes). A
retried pass is reported as `flaky`, with the first failure in the log, not
hidden; two failed attempts fail the job. The 1M run stays maintainer-only and
report-only.

Measured on the reference machine at 100k rows, 5 runs: max frame 16.8 ms
(one 60 Hz frame) in every run, no frame over 32 ms, p99 16.8 ms; 37 DOM rows
against a bound of 50; filter 10 to 27 ms. An earlier round, before frames were
sampled only after the page settled, had one run of ten with a single 33.3 ms
frame. Frame deltas are quantised to the display refresh, so the 32 ms target
in practice means no dropped frame in about 600: one missed vsync fails an
attempt, and the single retry is the agreed mitigation. Whether that holds on
CI runners is for the maintainer to judge once CI history exists.

Ingest (commitment 2) is still REPORT-ONLY.
