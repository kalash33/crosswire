| date | task | hours | notes |
|------|------|-------|-------|
| 2026-10-22 | 0: workspace scaffold | 0.1 | workspace, crates, CI, licences; actions pinned by SHA |
| 2026-10-22 | 1: crosswire-core types | 0.2 | spans, 25-code diagnostic catalogue, profile, change/hint types |
| 2026-10-22 | 2: crosswire-h1 public types | 0.2 | limits, config, errors, message/head view, progress types; no parsing logic |
| 2026-10-22 | 3: request head parser | 0.3 | streaming head state machine, bounded head buffer, line/header limits; strict happy path only |
| 2026-10-22 | 4: lenient head diagnostics | 0.4 | 15 head-level codes, bare-LF/mixed terminators, leading-ws skip, obs-fold spans, parser poisoning, bounded head capacity |
| 2026-10-22 | 6: response status-line parsing | 0.5 | Parser::response + ResponseContext, shared parse_head, status_code(), start_target_range(); version_unusual catch-all for malformed status lines |
| 2026-10-22 | 5: body framing resolution | 0.6 | framing::resolve, 6 framing codes, Ambiguous with TE-wins/first-CL/HEAD/no-body rules, 101 and CONNECT upgrade |
