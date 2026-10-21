test:
    cargo test --workspace --all-features

lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo fmt --check

features:
    cargo check -p crosswire-core --no-default-features
    cargo check -p crosswire-core --no-default-features --features serde
    cargo check -p crosswire-h1 --no-default-features
    cargo check -p crosswire-h1 --no-default-features --features std
    cargo check -p crosswire-h1 --no-default-features --features http
    cargo check -p crosswire-h1 --no-default-features --features serde

fuzz-smoke:
    @echo "no targets yet"
