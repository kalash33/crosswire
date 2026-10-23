//! Test helper for `tests/durability.rs`: records exchanges until SIGKILLed.
//!
//! Usage: `killtest-child <project dir> <count>`. Opens (or creates) the
//! project, submits `count` exchanges with 20 KiB request and response
//! bodies whose bytes are derived from the exchange index (stored in the
//! exchange notes), prints `accepted <ulid>` for every submit that returned
//! `Ok` and every `Committed` event as `committed <ulid> <ulid> ...` on
//! stdout, and never closes the store.

use bytes::Bytes;
use crosswire_model::{
    Body, ConnectionInfo, Event, Exchange, Fidelity, FramingRecord, Head, Message, MessageKind,
};
use crosswire_store::{Project, Store, StoreConfig};
use std::io::Write;
use std::path::PathBuf;
use ulid::Ulid;

const BODY_LEN: usize = 20 * 1024;

/// Must match `pattern` in `tests/durability.rs`.
fn pattern(index: u64, side: u64, len: usize) -> Vec<u8> {
    let mut x = (index * 2 + side + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn message(body: Vec<u8>) -> Message {
    Message::new(
        MessageKind::Normalized,
        Fidelity::Normalized,
        Head::Normalized { map: Vec::new() },
        FramingRecord::ContentLength {
            len: body.len() as u64,
        },
        Body::inline(Bytes::from(body)),
        ConnectionInfo::new("example.test", 443),
    )
}

fn main() {
    let mut args = std::env::args_os().skip(1);
    let (Some(dir), Some(count)) = (args.next(), args.next()) else {
        eprintln!("usage: killtest-child <project dir> <count>");
        std::process::exit(2);
    };
    let dir = PathBuf::from(dir);
    let count: u64 = count.to_string_lossy().parse().unwrap_or(0);
    let project = Project::open_or_create(&dir).unwrap_or_else(|e| {
        eprintln!("open: {e}");
        std::process::exit(1);
    });
    let store = Store::open(project, StoreConfig::default()).unwrap_or_else(|e| {
        eprintln!("store: {e}");
        std::process::exit(1);
    });
    let mut events = store.subscribe();
    let sink = store.sink();
    std::thread::spawn(move || {
        for i in 0..count {
            let (req, resp) = (
                message(pattern(i, 0, BODY_LEN)),
                message(pattern(i, 1, BODY_LEN)),
            );
            // The id is minted just before the submit, as a proxy would.
            let mut ex = Exchange::with_id(Ulid::new(), req);
            ex.set_response(resp);
            ex.set_notes(i.to_string());
            let id = ex.id();
            match sink.submit(ex) {
                Ok(()) => {
                    let mut out = std::io::stdout().lock();
                    let _ = writeln!(out, "accepted {id}").and_then(|()| out.flush());
                }
                Err(e) => eprintln!("submit {i}: {e}"),
            }
        }
    });
    let stdout = std::io::stdout();
    while let Some(ev) = events.blocking_recv() {
        if let Event::Committed { ids, .. } = ev {
            let mut line = String::from("committed");
            for id in ids {
                line.push(' ');
                line.push_str(&id.to_string());
            }
            let mut out = stdout.lock();
            // A closed pipe is ignored: the store must keep running until killed.
            let _ = writeln!(out, "{line}").and_then(|()| out.flush());
        }
    }
    // Never close or drop the store: the parent kills this process.
    loop {
        std::thread::park();
    }
}
