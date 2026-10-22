//! Tests for body streaming, chunked decoding and pipelining.
use crosswire_core::{DiagnosticCode, Span};
use crosswire_h1::*;

const CHUNKED_HEAD: &[u8] = b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n";

fn chunked() -> Parser {
    let mut p = Parser::request(&ParserConfig::default());
    p.feed(CHUNKED_HEAD).unwrap();
    p
}

fn has(p: &Parser, c: DiagnosticCode) -> usize {
    p.body_diagnostics()
        .iter()
        .filter(|d| d.code() == c)
        .count()
}

/// Feeds `body` in pieces of `step` bytes, re-feeding unconsumed bytes, and
/// returns the concatenated payload.
fn payload(body: &[u8], step: usize) -> Vec<u8> {
    let mut p = chunked();
    let mut out = vec![];
    let mut off = 0;
    let mut end = step.min(body.len());
    for _ in 0..10_000 {
        let input = &body[off..end];
        let s = p.feed(input).unwrap();
        assert!(s.consumed <= input.len());
        if let Progress::BodyData { data, .. } = &s.event {
            out.extend_from_slice(&input[data.range()]);
        }
        off += s.consumed;
        if matches!(s.event, Progress::Done) {
            assert_eq!(off, body.len(), "step {step}");
            return out;
        }
        let peek = matches!(s.event, Progress::ChunkExt(_));
        if !peek && (s.consumed == 0 || off == end) && end < body.len() {
            end = (end + step).min(body.len());
        }
    }
    panic!("no progress")
}

#[test]
fn fixed_body_across_feeds() {
    let mut p = Parser::request(&ParserConfig::default());
    p.feed(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\n")
        .unwrap();
    let s = p.feed(b"he").unwrap();
    assert_eq!(s.consumed, 2);
    let Progress::BodyData { raw, data } = s.event else {
        panic!()
    };
    assert_eq!(raw, Span::new(0, 2));
    assert_eq!(data, raw);
    let s = p.feed(b"llo!!").unwrap();
    assert_eq!(s.consumed, 3);
    assert!(matches!(p.feed(b"").unwrap().event, Progress::Done));
}

#[test]
fn fixed_empty_input_needs_more() {
    let mut p = Parser::request(&ParserConfig::default());
    p.feed(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\n")
        .unwrap();
    let s = p.feed(b"").unwrap();
    assert_eq!(s.consumed, 0);
    assert!(matches!(s.event, Progress::NeedMore));
    assert!(matches!(p.feed_eof(), Err(ParseError::UnexpectedEof)));
}

#[test]
fn fixed_complete_at_eof() {
    let mut p = Parser::request(&ParserConfig::default());
    p.feed(b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 1\r\n\r\n")
        .unwrap();
    p.feed(b"a").unwrap();
    assert!(matches!(p.feed_eof().unwrap().event, Progress::Done));
}

#[test]
fn pipelined_two_requests() {
    let two = b"GET /1 HTTP/1.1\r\nHost: x\r\nContent-Length: 0\r\n\r\nGET /2 HTTP/1.1\r\nHost: x\r\n\r\n";
    let mut p = Parser::request(&ParserConfig::default());
    let s = p.feed(two).unwrap();
    let first_len = s.consumed;
    assert!(matches!(s.event, Progress::Head(_)));
    assert!(matches!(
        p.feed(&two[first_len..]).unwrap(),
        Step {
            consumed: 0,
            event: Progress::Done
        }
    ));
    p.reset();
    let s = p.feed(&two[first_len..]).unwrap();
    assert_eq!(s.consumed, two.len() - first_len);
    let Progress::Head(m) = s.event else { panic!() };
    assert_eq!(&m.bytes()[m.head().start_target_range().unwrap()], b"/2");
}

#[test]
fn chunked_with_ext_and_trailers() {
    let body = b"5;x=y\r\nhello\r\n0\r\nX-T: 1\r\n\r\n";
    let mut p = chunked();
    let mut events = vec![];
    let mut off = 0;
    loop {
        let s = p.feed(&body[off..]).unwrap();
        events.push((off, s.event));
        off += s.consumed;
        if matches!(events.last().unwrap().1, Progress::Done) {
            break;
        }
    }
    // Spans index the input of the call that produced them.
    let at = |i: usize| &body[events[i].0..];
    let Progress::ChunkExt(e) = &events[0].1 else {
        panic!()
    };
    assert_eq!(&at(0)[e.range()], b"x=y");
    let Progress::BodyData { raw, data } = &events[1].1 else {
        panic!()
    };
    assert_eq!(&body[raw.range()], b"5;x=y\r\nhello\r\n");
    assert_eq!(&body[data.range()], b"hello");
    let Progress::Trailers(t) = &events[2].1 else {
        panic!()
    };
    assert_eq!(t.len(), 1);
    assert_eq!(&at(2)[t[0].name().range()], b"X-T");
    assert_eq!(&at(2)[t[0].value().range()], b"1");
    assert_eq!(off, body.len());
    assert_eq!(has(&p, DiagnosticCode::ChunkExtensionPresent), 1);
    assert_eq!(has(&p, DiagnosticCode::TrailerPresent), 1);
}

#[test]
fn chunked_no_ext_no_trailers_done_directly() {
    let body = b"3\r\nabc\r\n0\r\n\r\n";
    let mut p = chunked();
    let s = p.feed(body).unwrap();
    assert!(matches!(s.event, Progress::BodyData { .. }));
    assert_eq!(s.consumed, 8);
    let s = p.feed(&body[8..]).unwrap();
    assert_eq!(s.consumed, body.len() - 8);
    assert!(matches!(s.event, Progress::Done));
    assert!(p.body_diagnostics().is_empty());
}

#[test]
fn chunk_size_leading_zeros_and_invalid() {
    let mut p = chunked();
    p.feed(b"005\r\nhello\r\n").unwrap();
    assert_eq!(has(&p, DiagnosticCode::ChunkSizeLeadingZeros), 1);
    let Err(ParseError::InvalidChunkSize { span }) = p.feed(b"zz\r\n") else {
        panic!()
    };
    assert_eq!(span, Span::new(0, 2));
    assert!(matches!(
        p.feed(b"0\r\n\r\n"),
        Err(ParseError::InvalidChunkSize { .. })
    ));
}

#[test]
fn invalid_sizes() {
    for bad in [
        &b"\r\n"[..],
        b";x\r\n",
        b"-1\r\n",
        b"1 \r\n",
        b"fffffffffffffffff\r\n",
    ] {
        let mut p = chunked();
        assert!(
            matches!(p.feed(bad), Err(ParseError::InvalidChunkSize { .. })),
            "{bad:?}"
        );
    }
}

#[test]
fn hex_case_insensitive_and_bare_lf() {
    let body = b"A\nabcdefghij\n0\n\n";
    let mut p = chunked();
    let mut off = 0;
    let mut data = vec![];
    loop {
        let s = p.feed(&body[off..]).unwrap();
        if let Progress::BodyData { data: d, .. } = &s.event {
            data.extend_from_slice(&body[off..][d.range()]);
        }
        off += s.consumed;
        if matches!(s.event, Progress::Done) {
            break;
        }
    }
    assert_eq!(data, b"abcdefghij");
    assert!(has(&p, DiagnosticCode::BareLf) >= 3);
}

#[test]
fn chunk_size_line_too_long() {
    let cfg = ParserConfig::builder()
        .limits(ParseLimits::builder().max_chunk_size_line(8).build())
        .build();
    let mut p = Parser::request(&cfg);
    p.feed(CHUNKED_HEAD).unwrap();
    assert!(matches!(
        p.feed(b"5;aaaaaaaaaaaaaaaaaaaaa\r\n"),
        Err(ParseError::LineTooLong)
    ));
    let mut p = Parser::request(&cfg);
    p.feed(CHUNKED_HEAD).unwrap();
    // An incomplete line is not buffered; the caller re-presents it, and it
    // errors as soon as the bytes present exceed the limit.
    let s = p.feed(b"5;aaaa").unwrap();
    assert_eq!(s.consumed, 0);
    assert!(matches!(s.event, Progress::NeedMore));
    assert!(matches!(
        p.feed(b"5;aaaaaaaaaa"),
        Err(ParseError::LineTooLong)
    ));
}

#[test]
fn trailer_too_large() {
    let cfg = ParserConfig::builder()
        .limits(ParseLimits::builder().max_trailer_size(16).build())
        .build();
    let mut p = Parser::request(&cfg);
    p.feed(CHUNKED_HEAD).unwrap();
    assert!(matches!(
        p.feed(b"0\r\nX-A: aaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n\r\n"),
        Err(ParseError::TrailerTooLarge)
    ));
    let mut p = Parser::request(&cfg);
    p.feed(CHUNKED_HEAD).unwrap();
    let s = p.feed(b"0\r\nX-A: aaaa").unwrap();
    assert_eq!(s.consumed, 0);
    assert!(matches!(s.event, Progress::NeedMore));
    assert!(matches!(
        p.feed(b"0\r\nX-A: aaaaaaaaaaaaaaaaaaaaaaa"),
        Err(ParseError::TrailerTooLarge)
    ));
}

#[test]
fn chunked_split_at_every_byte_yields_same_payload() {
    let body = b"5;x=y\r\nhello\r\n00A\r\n0123456789\r\n0\r\nX-T: 1\r\nY: 2\r\n\r\n";
    for step in 1..=body.len() {
        assert_eq!(payload(body, step), b"hello0123456789", "step {step}");
    }
}

#[test]
fn chunked_split_trailers_and_eof_midchunk() {
    let mut p = chunked();
    let s = p.feed(b"0\r\nX-").unwrap();
    assert_eq!(s.consumed, 0);
    // The caller re-presents the unconsumed bytes with the new data.
    let s = p.feed(b"0\r\nX-T: 1\r\n\r\nGET").unwrap();
    assert_eq!(s.consumed, 13);
    let Progress::Trailers(t) = s.event else {
        panic!()
    };
    assert_eq!(t.len(), 1);
    let mut p = chunked();
    p.feed(b"5\r\nhe").unwrap();
    assert!(matches!(p.feed_eof(), Err(ParseError::UnexpectedEof)));
}

#[test]
fn chunk_data_split_reports_size_line_once() {
    let mut p = chunked();
    let s = p.feed(b"5\r\nhe").unwrap();
    assert_eq!(s.consumed, 5);
    let Progress::BodyData { raw, data } = s.event else {
        panic!()
    };
    assert_eq!(raw, Span::new(0, 5));
    assert_eq!(data, Span::new(3, 5));
    let s = p.feed(b"llo\r\n").unwrap();
    assert_eq!(s.consumed, 5);
    let Progress::BodyData { raw, data } = s.event else {
        panic!()
    };
    assert_eq!(raw, Span::new(0, 5));
    assert_eq!(data, Span::new(0, 3));
}

#[test]
fn chunked_pipelined_tail_untouched() {
    let mut p = chunked();
    let s = p.feed(b"0\r\n\r\nGET / HTTP/1.1\r\n\r\n").unwrap();
    assert_eq!(s.consumed, 5);
    assert!(matches!(s.event, Progress::Done));
    assert!(matches!(p.feed(b"GET").unwrap().event, Progress::Done));
}

#[test]
fn until_close_needs_eof() {
    let mut p = Parser::response(
        &ParserConfig::default(),
        ResponseContext::for_method(b"GET"),
    );
    p.feed(b"HTTP/1.1 200 OK\r\n\r\n").unwrap();
    let s = p.feed(b"abc").unwrap();
    assert_eq!(s.consumed, 3);
    assert!(matches!(s.event, Progress::BodyData { .. }));
    assert!(matches!(p.feed_eof().unwrap().event, Progress::Done));
}

#[test]
fn upgrade_is_done_immediately() {
    let mut p = Parser::response(
        &ParserConfig::default(),
        ResponseContext::for_method(b"GET"),
    );
    let s = p
        .feed(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\nrawbytes")
        .unwrap();
    assert_eq!(
        s.consumed,
        b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n".len()
    );
    assert!(matches!(
        p.feed(b"rawbytes").unwrap(),
        Step {
            consumed: 0,
            event: Progress::Done
        }
    ));
}

#[test]
fn reset_clears_body_diagnostics() {
    let mut p = chunked();
    p.feed(b"005\r\nhello\r\n").unwrap();
    assert_eq!(p.body_diagnostics().len(), 1);
    p.reset();
    assert!(p.body_diagnostics().is_empty());
    p.feed(CHUNKED_HEAD).unwrap();
    assert!(matches!(
        p.feed(b"0\r\n\r\n").unwrap().event,
        Progress::Done
    ));
}

#[test]
fn decoder_standalone() {
    let mut d = BodyDecoder::chunked(&ParseLimits::default());
    let (n, ev) = d.feed(b"2\r\nok\r\n0\r\n\r\n").unwrap();
    assert_eq!(n, 7);
    assert!(matches!(ev, BodyEvent::Data { .. }));
    let (n, ev) = d.feed(b"0\r\n\r\n").unwrap();
    assert_eq!(n, 5);
    assert!(matches!(ev, BodyEvent::Done));
    assert!(d.diagnostics().is_empty());
}

#[test]
fn garbage_never_panics() {
    let junk: &[&[u8]] = &[
        b"\xff\x00\r\r\n\n",
        b"5\r\nhelloXX",
        b"0\r\nbad line\r\n\r\n",
        b"\r",
        b"5\r",
    ];
    for j in junk {
        let mut p = chunked();
        let _ = p.feed(j);
        let _ = p.feed(j);
        let _ = p.feed_eof();
    }
}

mod fuzz {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn arbitrary_bytes_never_panic_or_overconsume(
            data in proptest::collection::vec(
                prop_oneof![Just(b'\r'), Just(b'\n'), Just(b';'), Just(b'0'), Just(b'5'), Just(b'a'), any::<u8>()],
                0..200,
            ),
            cut in 0usize..200,
        ) {
            let mut p = chunked();
            let cut = cut.min(data.len());
            for part in [&data[..cut], &data[cut..]] {
                for _ in 0..4 {
                    match p.feed(part) {
                        Ok(s) => prop_assert!(s.consumed <= part.len()),
                        Err(_) => break,
                    }
                }
            }
            let _ = p.feed_eof();
        }
    }
}

#[test]
fn size_line_split_requires_represent() {
    let mut p = chunked();
    let s = p.feed(b"5").unwrap();
    assert_eq!(s.consumed, 0);
    assert!(matches!(s.event, Progress::NeedMore));
    // The caller re-presents the unconsumed bytes together with new data.
    let input = b"5\r\nhello\r\n";
    let s = p.feed(input).unwrap();
    assert_eq!(s.consumed, input.len());
    let Progress::BodyData { raw, data } = s.event else {
        panic!()
    };
    assert_eq!(raw, Span::new(0, 10));
    assert_eq!(&input[data.range()], b"hello");
}

#[test]
fn ext_split_across_calls_round_trips() {
    let mut p = chunked();
    let s = p.feed(b"5;x=").unwrap();
    assert_eq!(s.consumed, 0);
    assert!(matches!(s.event, Progress::NeedMore));
    let input = b"5;x=y\r\nhello\r\n";
    let s = p.feed(input).unwrap();
    assert_eq!(s.consumed, 0);
    let Progress::ChunkExt(e) = s.event else {
        panic!()
    };
    assert_eq!(&input[e.range()], b"x=y");
    let s = p.feed(input).unwrap();
    assert_eq!(s.consumed, input.len());
    assert!(matches!(s.event, Progress::BodyData { .. }));
    assert_eq!(has(&p, DiagnosticCode::ChunkExtensionPresent), 1);
}

#[test]
fn trailers_split_across_calls_round_trip() {
    let mut p = chunked();
    let s = p.feed(b"0\r\nX-T").unwrap();
    assert_eq!(s.consumed, 0);
    assert!(matches!(s.event, Progress::NeedMore));
    let input = b"0\r\nX-T: 1\r\n\r\n";
    let s = p.feed(input).unwrap();
    assert_eq!(s.consumed, input.len());
    let Progress::Trailers(t) = s.event else {
        panic!()
    };
    assert_eq!(t.len(), 1);
    assert_eq!(&input[t[0].name().range()], b"X-T");
    assert_eq!(&input[t[0].value().range()], b"1");
}

#[test]
fn incomplete_size_line_over_limit_errors() {
    let cfg = ParserConfig::builder()
        .limits(ParseLimits::builder().max_chunk_size_line(4).build())
        .build();
    let mut p = Parser::request(&cfg);
    p.feed(CHUNKED_HEAD).unwrap();
    assert!(matches!(p.feed(b"12345"), Err(ParseError::LineTooLong)));
}
