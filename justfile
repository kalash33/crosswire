test:
    cargo test --locked --workspace --all-features

serve *ARGS:
    cargo run --locked -p crosswire-server -- serve {{ARGS}}

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
