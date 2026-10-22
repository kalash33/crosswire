| date | task | hours | notes |
|------|------|-------|-------|
| 2026-10-22 | 0: workspace scaffold | 0.1 | workspace, crates, CI, licences; actions pinned by SHA |
| 2026-10-22 | 1: crosswire-core types | 0.2 | spans, 25-code diagnostic catalogue, profile, change/hint types |
| 2026-10-22 | 2: crosswire-h1 public types | 0.2 | limits, config, errors, message/head view, progress types; no parsing logic |
| 2026-10-22 | 3: request head parser | 0.3 | streaming head state machine, bounded head buffer, line/header limits; strict happy path only |
| 2026-10-22 | 4: lenient head diagnostics | 0.4 | 15 head-level codes, bare-LF/mixed terminators, leading-ws skip, obs-fold spans, parser poisoning, bounded head capacity |
| 2026-10-22 | 6: response status-line parsing | 0.5 | Parser::response + ResponseContext, shared parse_head, status_code(), start_target_range(); version_unusual catch-all for malformed status lines |
| 2026-10-22 | 5: body framing resolution | 0.6 | framing::resolve, 6 framing codes, Ambiguous with TE-wins/first-CL/HEAD/no-body rules, 101 and CONNECT upgrade |
| 2026-10-22 | 7: body streaming, chunked decoder, pipelining | 0.7 | BodyState Fixed/Chunked/UntilClose, BodyDecoder (size line, ext peek, trailers, bounded buffering), body_diagnostics, pipelined Done |
| 2026-10-22 | 8: write_exact and BodyEncoder | 0.8 | byte-exact write_exact (head + inline body), chunked BodyEncoder (lowercase hex, ext, deferred trailers, empty chunk skipped) |
| 2026-10-22 | 9: golden corpus and harness | 0.9 | corpus/h1 with 95 cases covering all 25 codes plus framing shapes, corpus_golden harness, scripts/corpus-check.py, large body generated from .gen.json |
| 2026-10-22 | 10: split-at-every-byte property test | 0.10 | tests/split_prop.rs: whole vs every 2-way split vs byte-wise vs proptest; found chunk Ext/Trailers span-clamp bug (5 cases, listed in KNOWN_SPLIT_BUGS) |
| 2026-10-22 | 10b: remove chunk span clamp | 0.10b | BodyDecoder holds no partial token bytes; incomplete size line / trailer block returns NeedMore (consumed 0), caller re-presents; KNOWN_SPLIT_BUGS removed, split_prop passes all 95 cases |
| 2026-10-22 | 11: fuzz targets, criterion bench, CI smoke | 0.11 | cargo-fuzz parse_request/parse_response (split vs whole oracle, write_exact round-trip, span bounds), 60 s smoke ~11-13k exec/s no crashes; bench ~977 ns Lenient / ~967 ns Strict vs httparse ~102 ns on a 10-header head, h1/httparse ratio ~9.5x; nightly fuzz-smoke CI job |
