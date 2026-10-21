test:
    cargo test --locked --workspace --all-features

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

fuzz-smoke:
    @echo "no targets yet"
