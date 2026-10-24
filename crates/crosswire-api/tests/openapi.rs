//! Keeps the committed OpenAPI document in sync with the annotated routes.
//!
//! Run `UPDATE_OPENAPI=1 cargo test -p crosswire-api --test openapi` to
//! rewrite `docs/api/openapi.json` after a route change.

use std::path::PathBuf;

fn snapshot_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/api/openapi.json")
}

#[test]
fn openapi_snapshot_is_current() {
    let current = serde_json::to_string_pretty(&crosswire_api::document()).unwrap() + "\n";
    let path = snapshot_path();

    if std::env::var_os("UPDATE_OPENAPI").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, current.as_bytes()).unwrap();
        return;
    }

    let committed = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "{} is missing; run `UPDATE_OPENAPI=1 cargo test -p crosswire-api --test openapi`",
            path.display()
        )
    });
    assert_eq!(
        committed, current,
        "the OpenAPI document drifted from the routes; run \
         `UPDATE_OPENAPI=1 cargo test -p crosswire-api --test openapi` to update it"
    );
}
