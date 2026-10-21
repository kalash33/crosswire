//! Tests for the streaming request head parser.
use crosswire_h1::*;

const REQ: &[u8] = b"GET /a?b=1 HTTP/1.1\r\nHost: x.io\r\nX-A: 1\r\n\r\n";

#[test]
fn head_in_one_feed() {
    let mut p = Parser::request(&ParserConfig::default());
    let s = p.feed(REQ).unwrap();
    assert_eq!(s.consumed, REQ.len());
    let Progress::Head(m) = s.event else { panic!() };
    assert_eq!(m.bytes().as_ref(), REQ);
    let StartLine::Request {
        method,
        target,
        version,
    } = m.head().start()
    else {
        panic!()
    };
    assert_eq!(&REQ[method.range()], b"GET");
    assert_eq!(&REQ[target.range()], b"/a?b=1");
    assert_eq!(&REQ[version.range()], b"HTTP/1.1");
    assert_eq!(m.head().headers().len(), 2);
    assert_eq!(&REQ[m.head().headers()[1].name().range()], b"X-A");
    assert_eq!(&REQ[m.head().headers()[1].value().range()], b"1");
    assert!(m.diagnostics().is_empty());
    assert!(matches!(m.head().framing(), BodyFraming::None));
    assert!(matches!(p.feed(b"").unwrap().event, Progress::Done));
}

#[test]
fn needmore_across_terminator() {
    let mut p = Parser::request(&ParserConfig::default());
    let (a, b) = REQ.split_at(REQ.len() - 2); // split inside "\r\n\r\n"
    let s = p.feed(a).unwrap();
    assert_eq!(s.consumed, a.len());
    assert!(matches!(s.event, Progress::NeedMore));
    let s = p.feed(b).unwrap();
    assert_eq!(s.consumed, 2);
    let Progress::Head(m) = s.event else { panic!() };
    assert_eq!(m.bytes().as_ref(), REQ);
}

#[test]
fn head_consumed_stops_at_terminator() {
    let mut p = Parser::request(&ParserConfig::default());
    let mut v = REQ.to_vec();
    v.extend_from_slice(b"GET / HTTP/1.1\r\n\r\n");
    let s = p.feed(&v).unwrap();
    assert_eq!(s.consumed, REQ.len());
}

#[test]
fn head_size_boundary() {
    let cfg = ParserConfig::builder()
        .limits(ParseLimits::builder().head_size(64).build())
        .build();
    let ok = b"GET / HTTP/1.1\r\nHost: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n\r\n"; // exactly 64 bytes
    assert_eq!(ok.len(), 64);
    assert!(matches!(
        Parser::request(&cfg).feed(ok).unwrap().event,
        Progress::Head(_)
    ));
    let mut big = ok.to_vec();
    big.insert(5, b'/');
    assert!(matches!(
        Parser::request(&cfg).feed(&big),
        Err(ParseError::HeadTooLarge)
    ));
}

#[test]
fn too_many_headers_and_long_line() {
    let cfg = ParserConfig::builder()
        .limits(ParseLimits::builder().max_headers(1).build())
        .build();
    assert!(matches!(
        Parser::request(&cfg).feed(REQ),
        Err(ParseError::TooManyHeaders)
    ));
    let cfg = ParserConfig::builder()
        .limits(ParseLimits::builder().max_line(8).build())
        .build();
    assert!(matches!(
        Parser::request(&cfg).feed(REQ),
        Err(ParseError::LineTooLong)
    ));
}

#[test]
fn eof_before_empty_line() {
    let mut p = Parser::request(&ParserConfig::default());
    p.feed(b"GET / HTTP/1.1\r\nHost: x\r\n").unwrap();
    assert!(matches!(p.feed_eof(), Err(ParseError::UnexpectedEof)));
}

#[test]
fn byte_at_a_time_and_reset() {
    let mut p = Parser::request(&ParserConfig::default());
    for round in 0..2 {
        let mut head = None;
        for b in REQ {
            let s = p.feed(core::slice::from_ref(b)).unwrap();
            assert_eq!(s.consumed, 1);
            if let Progress::Head(m) = s.event {
                head = Some(m);
            }
        }
        assert_eq!(head.expect("head").bytes().as_ref(), REQ, "round {round}");
        assert!(matches!(p.feed_eof().unwrap().event, Progress::Done));
        p.reset();
    }
}
