# Contributing to Crosswire

Thanks for your interest. Crosswire is a pure community open-source project; there is no company behind it and nothing is gated.

## Before v0.2: expectations

The project is pre-alpha and maintained part-time. Until v0.2 there is no response-time promise except for security reports (see [SECURITY.md](SECURITY.md)). Issues and pull requests are labelled best-effort.

## What is open for contribution now

- **Corpus cases** (`corpus/h1/cases`): real-world HTTP/1 shapes that normalising tools alter. Every case must round-trip byte-for-byte; see [corpus/h1/README.md](corpus/h1/README.md).
- **Tests and fuzzing**: new targets, harness improvements, limit fixtures.
- **Documentation**: anything in `docs/` or a crate README that is wrong, unclear or missing.

`crosswire-core` and `crosswire-h1` are **design-locked** until their 0.1 publication: public API changes need an RFC issue first; fixes, docs, corpus and tests are welcome without one.

## Development

```
just test        # all tests, all features
just features    # no-default-features + each feature alone
just lint        # clippy -D warnings, fmt --check
just fuzz-smoke  # 60 s per fuzz target (nightly + cargo-fuzz)
just bench       # build the criterion bench
```

Rust stable; the MSRV is 1.88, declared in `Cargo.toml` (`rust-version`) and checked by the `msrv` CI job (`cargo check --workspace --locked --all-features` on 1.88). Dependencies in the lockfile (`rcgen`, `time`) already require 1.88. CI runs on Linux and macOS.

## Pull requests

- One logical change per PR; tests for behaviour changes; `just lint` and `just test` green.
- **DCO sign-off** required: add `Signed-off-by: Your Name <you@example.com>` to each commit (`git commit -s`).
- Wire-layer changes (`crosswire-h1`) need golden corpus coverage for the shape they touch.
- **AI-assisted contributions are welcome** under one rule: the author must be able to explain every line. The reviewer may ask; "the model wrote it" is not an answer. Tests are still required.
- Commits follow the existing style: `feat(h1): …`, `fix(core): …`, `test(h1): …`, `docs: …`.

## Decisions

Architecture decisions are recorded in `docs/adr/`. The design spec is `docs/superpowers/specs/2026-10-22-crosswire-design.md`; when the code and the spec disagree, open an issue rather than silently changing either.

## Code of conduct

This project follows the [Contributor Covenant](https://www.contributor-covenant.org/version/2/1/code_of_conduct/), version 2.1. Report conduct concerns privately through the same channel as security reports.

## Licence

By contributing you agree that your contribution is licensed under Apache-2.0 OR MIT, like the rest of the project.
