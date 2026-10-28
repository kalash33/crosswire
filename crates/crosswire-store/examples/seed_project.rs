//! Builds a synthetic project directory for the UI frame/filter perf trace
//! (`scripts/ui-perf.sh`): `cargo run --release -p crosswire-store --features
//! synthetic --example seed_project -- <dir> <rows>`. A dev helper, never part
//! of a default build.

use crosswire_store::synthetic::generate;
use crosswire_store::{Project, Store, StoreConfig};
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("usage: seed_project <dir> <rows>"));
    let rows: usize = args
        .next()
        .and_then(|v| v.parse().ok())
        .expect("usage: seed_project <dir> <rows>");
    let store = Store::open(
        Project::create(&dir).expect("create the project"),
        StoreConfig::default(),
    )
    .expect("open the store");
    generate(&store, rows, 0xC055_1417);
    // Dropping the store releases the project lock so `serve` can open it.
    drop(store);
    println!("seeded {rows} rows into {}", dir.display());
}
