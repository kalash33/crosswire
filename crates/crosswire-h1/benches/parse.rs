//! Parsing throughput on a browser-like request head: crosswire-h1 against
#![allow(missing_docs)]
//! `httparse`. The input is built once; only parsing is timed.
use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion};
use crosswire_core::Profile;
use crosswire_h1::{Parser, ParserConfig, Progress};

const HEAD: &[u8] = b"GET /search?q=rust HTTP/1.1\r\n\
Host: www.example.com\r\n\
User-Agent: Mozilla/5.0 Chrome/126.0\r\n\
Accept: text/html,*/*;q=0.8\r\n\
Accept-Language: en-US\r\n\
Accept-Encoding: gzip, br\r\n\
Connection: keep-alive\r\n\
Cookie: sid=abcdef01\r\n\
Upgrade-Insecure-Requests: 1\r\n\
Cache-Control: max-age=0\r\n\
Referer: https://example.com/\r\n\
\r\n";

fn bench(c: &mut Criterion) {
    assert!(
        (270..=330).contains(&HEAD.len()),
        "head is {} bytes",
        HEAD.len()
    );
    for (name, profile) in [
        ("crosswire-h1 Lenient", Profile::Lenient),
        ("crosswire-h1 Strict", Profile::Strict),
    ] {
        let cfg = ParserConfig::builder().profile(profile).build();
        // Sanity: the benchmark really parses a head with 10 headers.
        let step = Parser::request(&cfg).feed(HEAD).unwrap();
        assert!(matches!(step.event, Progress::Head(_)) && step.consumed == HEAD.len());
        c.bench_function(name, |b| {
            b.iter(|| {
                let mut p = Parser::request(&cfg);
                black_box(p.feed(black_box(HEAD)).unwrap())
            });
        });
    }
    c.bench_function("httparse", |b| {
        b.iter(|| {
            let mut headers = [httparse::EMPTY_HEADER; 16];
            let mut req = httparse::Request::new(&mut headers);
            black_box(req.parse(black_box(HEAD)).unwrap())
        });
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
