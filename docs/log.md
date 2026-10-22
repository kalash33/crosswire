| date | task | hours | notes |
|------|------|-------|-------|
| 2026-10-22 | 0: workspace scaffold | n/m | workspace, crates, CI, licences; actions pinned by SHA |
| 2026-10-22 | 1: crosswire-core types | n/m | spans, 25-code diagnostic catalogue, profile, change/hint types |
| 2026-10-22 | 2: crosswire-h1 public types | n/m | limits, config, errors, message/head view, progress types; no parsing logic |
| 2026-10-22 | 3: request head parser | n/m | streaming head state machine, bounded head buffer, line/header limits; strict happy path only |
| 2026-10-22 | 4: lenient head diagnostics | n/m | 15 head-level codes, bare-LF/mixed terminators, leading-ws skip, obs-fold spans, parser poisoning, bounded head capacity |
| 2026-10-22 | 6: response status-line parsing | n/m | Parser::response + ResponseContext, shared parse_head, status_code(), start_target_range(); version_unusual catch-all for malformed status lines |
| 2026-10-22 | 5: body framing resolution | n/m | framing::resolve, 6 framing codes, Ambiguous with TE-wins/first-CL/HEAD/no-body rules, 101 and CONNECT upgrade |
| 2026-10-22 | 7: body streaming, chunked decoder, pipelining | n/m | BodyState Fixed/Chunked/UntilClose, BodyDecoder (size line, ext peek, trailers, bounded buffering), body_diagnostics, pipelined Done |
| 2026-10-22 | 8: write_exact and BodyEncoder | n/m | byte-exact write_exact (head + inline body), chunked BodyEncoder (lowercase hex, ext, deferred trailers, empty chunk skipped) |
| 2026-10-22 | 9: golden corpus and harness | n/m | corpus/h1 with 95 cases covering all 25 codes plus framing shapes, corpus_golden harness, scripts/corpus-check.py, large body generated from .gen.json |
| 2026-10-22 | 10: split-at-every-byte property test | n/m | tests/split_prop.rs: whole vs every 2-way split vs byte-wise vs proptest; found chunk Ext/Trailers span-clamp bug (5 cases, listed in KNOWN_SPLIT_BUGS) |
| 2026-10-22 | 10b: remove chunk span clamp | n/m | BodyDecoder holds no partial token bytes; incomplete size line / trailer block returns NeedMore (consumed 0), caller re-presents; KNOWN_SPLIT_BUGS removed, split_prop passes all 95 cases |
| 2026-10-22 | 11: fuzz targets, criterion bench, CI smoke | n/m | cargo-fuzz parse_request/parse_response (split vs whole oracle, write_exact round-trip, span bounds), 60 s smoke ~11-13k exec/s no crashes; bench ~977 ns Lenient / ~967 ns Strict vs httparse ~102 ns on a 10-header head, h1/httparse ratio ~9.5x; nightly fuzz-smoke CI job |
| 2026-10-22 | 12: optional http and serde features | n/m | compat_http (TryFrom<&RawMessage> for http::Request/Response<()>, HttpConvertError with spans; lossy rules documented), serde derives on plain-data types, http implies std; tests/compat_http.rs 17, tests/serde_roundtrip.rs 3 |
| 2026-10-22 | 13: docs, packaging checks, velocity log | n/m | h1/core READMEs (h1 README is the crate doctest), docs/h1-diagnostics.md with docs_catalogue test, ADR-0001, root README; cargo package of both crates passes with `publish` lifted temporarily; cargo-deny clean |
| 2026-10-22 | final fix wave (whole-branch review) | n/m | body diagnostics capped (max_body_diagnostics 64) and chunk count limited (max_chunks 1M, TooManyChunks); chunk-size BWS accepted with 26th code chunk_size_whitespace; invalid CL -> Ambiguous/InvalidContentLength; HEAD/304 plain None, 101/CONNECT-2xx with length -> ConnectSuccess; reset_response, is_idle; absolute body-diagnostic spans; linear trailer re-scan; CL candidates capped at 8; Span deserialize validated; one canonical feed loop for corpus and split tests; limit fixtures; fuzz contexts, small limits and corpus seeding; CI doc, package and fuzz artifact steps |
| 2026-10-22 | recorder foundations 0: workspace additions | n/m | model, ca, client, store, recorder-tests crates scaffolded; workspace deps pinned; justfile features covers crosswire-model; deny.toml gains Zlib and CDLA-Permissive-2.0; lru 0.19 (0.12 has RUSTSEC unsoundness advisories) |
| 2026-10-22 | recorder foundations 1: crosswire-model | n/m | frozen schema (Message/Head/HeadSpans/FramingRecord/Body/Diff/ConnectionInfo, Exchange, Marker, Event, ExchangeSink, RedactionPolicy); Bytes as base64 under std (own b64 module), Ulid serde adapter (ulid's serde feature is not no_std); redaction rebuilds head by span and re-parses; tests: 3 integration files (15 tests) + 11 unit |
| 2026-10-22 | recorder foundations 2: crosswire-ca | n/m | P-256 Ca (10y, pathlen 0, keyCertSign/cRLSign), FileKeyStore 0600/0644/0700 via temp+rename, LeafIssuer (one leaf key, LRU, token-bucket mint limit, random serials, rustls CertifiedKey via any_ecdsa_type), SPKI pin; tests: 10 issue + 3 keystore + 1 unit |

## Velocity summary, Tasks 0-13 (2026-10-22)

- Hours: not measured (`n/m` above). The values first logged in that column were a running index (0.1, 0.2, ... 0.12), not hours, and have been removed.
- Lines: 4827 lines of Rust code, from `tokei 15.0.0 crates corpus --exclude '*.msg' --exclude target -t Rust` (`src/`, `tests/`, `benches/`, `fuzz/`; blanks and comments excluded; the 95 `.expect.json` corpus files, 827 lines, are data and not counted).
- LoC/h: not measured.
- Against the spec: no LoC/h figure exists to compare with the 60 LoC/h assumption (section 5) or the 45 LoC/h re-dating threshold (section 16). The section 16 velocity check is not due: it starts after 100 logged hours, and none have been measured.
- Caveat: this work was done by AI-paired implementers, and the hours column first held a running index (0.1, 0.2, ... 0.12) rather than measurements. Nothing in this log is comparable to the maintainer's part-time hours or may be used to re-date the roadmap. The roadmap needs a real measurement of the maintainer's own hours against reviewed lines.
