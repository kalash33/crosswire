//! The golden fidelity corpus: every case is parsed, diagnosed and written
//! back byte-identical.
mod common;

use bytes::BytesMut;
use common::{load_cases, Case};
use crosswire_h1::*;

fn framing_name(f: &BodyFraming) -> &'static str {
    match f {
        BodyFraming::None => "none",
        BodyFraming::ContentLength(_) => "content_length",
        BodyFraming::Chunked => "chunked",
        BodyFraming::UntilClose => "until_close",
        BodyFraming::Upgrade => "upgrade",
        BodyFraming::Ambiguous { .. } => "ambiguous",
        _ => "unknown",
    }
}

fn rule_name(r: &ResolutionRule) -> &'static str {
    match r {
        ResolutionRule::TransferEncodingWins => "transfer_encoding_wins",
        ResolutionRule::FirstContentLength => "first_content_length",
        ResolutionRule::NoBodyStatus => "no_body_status",
        ResolutionRule::HeadMethod => "head_method",
        ResolutionRule::ConnectSuccess => "connect_success",
        _ => "unknown",
    }
}

fn hex(name: &str, s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .unwrap_or_else(|_| panic!("case {name}: bad body_hex {s:?}"))
        })
        .collect()
}

fn run(case: &Case) {
    let name = &case.name;
    let e = &case.expect;
    let input = &case.input[..];
    let cfg = ParserConfig::default();
    let mut p = match e.kind.as_str() {
        "request" => Parser::request(&cfg),
        "response" => {
            let c = e
                .ctx
                .as_ref()
                .unwrap_or_else(|| panic!("case {name}: response needs ctx"));
            let mut ctx = ResponseContext::for_method(c.method.as_bytes());
            ctx.is_connect = c.is_connect;
            Parser::response(&cfg, ctx)
        }
        k => panic!("case {name}: bad kind {k}"),
    };

    let mut off = 0;
    let mut head: Option<RawMessage> = None;
    let mut head_len = 0;
    let mut payload = vec![];
    let mut raw_spans: Vec<(usize, usize)> = vec![];
    let mut trailers: Option<Vec<(Vec<u8>, Vec<u8>)>> = None;
    let mut done = false;
    let mut err = None;
    for _ in 0..(input.len() + 100) {
        let rest = &input[off..];
        let step = if rest.is_empty() && head.is_some() {
            p.feed_eof()
        } else {
            p.feed(rest)
        };
        let step = match step {
            Ok(s) => s,
            Err(er) => {
                err = Some(er);
                break;
            }
        };
        match step.event {
            Progress::Head(m) => {
                head_len = step.consumed;
                assert_eq!(
                    m.bytes()[..],
                    rest[..step.consumed],
                    "case {name}: head bytes differ from input"
                );
                head = Some(m);
            }
            Progress::BodyData { raw, data } => {
                raw_spans.push((off + raw.range().start, off + raw.range().end));
                payload.extend_from_slice(&rest[data.range()]);
            }
            Progress::Trailers(t) => {
                trailers = Some(
                    t.iter()
                        .map(|h| {
                            (
                                rest[h.name().range()].to_vec(),
                                rest[h.value().range()].to_vec(),
                            )
                        })
                        .collect(),
                );
            }
            Progress::Done => done = true,
            _ => {}
        }
        off += step.consumed;
        if done {
            break;
        }
    }

    if let Some(code) = &e.error {
        let er = err.unwrap_or_else(|| panic!("case {name}: expected error {code}"));
        match (code.as_str(), &er) {
            ("invalid_chunk_size", ParseError::InvalidChunkSize { .. }) => {}
            _ => panic!("case {name}: expected error {code}, got {er:?}"),
        }
    } else {
        assert!(err.is_none(), "case {name}: unexpected error {err:?}");
        assert!(done, "case {name}: parser never reported Done");
    }
    let head = head.unwrap_or_else(|| panic!("case {name}: no head parsed"));

    // (1) framing
    let framing = head.head().framing();
    assert_eq!(framing_name(framing), e.framing, "case {name}: framing");
    if let BodyFraming::Ambiguous { resolved, rule, .. } = framing {
        if let Some(r) = &e.resolved {
            assert_eq!(framing_name(resolved), r, "case {name}: resolved");
        }
        if let Some(r) = &e.rule {
            assert_eq!(rule_name(rule), r, "case {name}: rule");
        }
    } else {
        assert!(
            e.resolved.is_none() && e.rule.is_none(),
            "case {name}: resolved/rule given for non-ambiguous framing"
        );
    }

    // (2) diagnostics: head + body, sorted, deduplicated
    let mut codes: Vec<&str> = head
        .diagnostics()
        .iter()
        .chain(p.body_diagnostics())
        .map(|d| d.code().as_str())
        .collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes, e.diagnostics, "case {name}: diagnostics");

    // (3) byte-exact round trip
    let mut out = BytesMut::new();
    write_exact(&head, &mut out);
    assert_eq!(
        &out[..],
        &input[..head_len],
        "case {name}: write_exact differs from the head bytes"
    );
    // Raw body spans tile the body bytes contiguously, in absolute offsets.
    let mut pos = head_len;
    for &(start, end) in &raw_spans {
        assert!(
            start == pos && end >= start,
            "case {name}: raw spans are not contiguous at offset {pos} (got {start}..{end})"
        );
        pos = end;
    }
    let resolved = match framing {
        BodyFraming::Ambiguous { resolved, .. } => &**resolved,
        f => f,
    };
    if matches!(
        resolved,
        BodyFraming::ContentLength(_) | BodyFraming::UntilClose
    ) {
        assert_eq!(
            pos, off,
            "case {name}: raw spans do not cover the whole body"
        );
    } else if matches!(resolved, BodyFraming::Chunked) && e.error.is_none() {
        let term = &input[pos..off];
        assert!(
            term.first() == Some(&b'0'),
            "case {name}: terminator section does not start with 0: {term:?}"
        );
        assert!(
            term.ends_with(b"\r\n\r\n") || term.ends_with(b"\n\n") || term.ends_with(b"\n\r\n"),
            "case {name}: terminator section does not end with an empty line: {term:?}"
        );
    }
    if let Some(want) = &e.trailers {
        let want: Vec<(Vec<u8>, Vec<u8>)> = want
            .iter()
            .map(|[n, v]| (n.clone().into_bytes(), v.clone().into_bytes()))
            .collect();
        assert_eq!(trailers.as_ref(), Some(&want), "case {name}: trailers");
    }

    // (4) consumed
    if e.pipelined {
        assert!(off < input.len(), "case {name}: no tail after message");
        let mut next = match e.kind.as_str() {
            "request" => Parser::request(&cfg),
            _ => unreachable!("pipelined responses are not seeded"),
        };
        let s = next.feed(&input[off..]).unwrap();
        assert!(
            matches!(s.event, Progress::Head(_)),
            "case {name}: tail does not start with a second message"
        );
    } else if e.error.is_none() {
        assert_eq!(off, input.len(), "case {name}: consumed != input length");
    }

    // (5) payload
    if let Some(b) = &e.body {
        assert_eq!(payload, b.as_bytes(), "case {name}: body");
    } else if let Some(h) = &e.body_hex {
        assert_eq!(payload, hex(name, h), "case {name}: body_hex");
    }
}

#[test]
fn corpus_golden() {
    let cases = load_cases();
    assert!(!cases.is_empty(), "corpus has no cases");
    let mut ran = 0;
    let mut failures = vec![];
    for case in &cases {
        if let Some(bug) = &case.expect.known_bug {
            println!("known_bug {}: {bug}", case.name);
            continue;
        }
        match std::panic::catch_unwind(|| run(case)) {
            Ok(()) => ran += 1,
            Err(p) => {
                let msg = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                    .unwrap_or_default();
                failures.push(msg);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases failed:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
    println!("corpus: {ran} cases run");
}
