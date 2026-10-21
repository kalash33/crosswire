//! Byte-exact output: `write_exact` and the chunked `BodyEncoder`.
use bytes::{Bytes, BytesMut};
use crosswire_h1::*;

const REQUESTS: &[&[u8]] = &[
    b"GET / HTTP/1.1\r\nHost: x\r\n\r\n",
    b"GET / HTTP/1.1\nHost: x\n\n",
    b"GET / HTTP/1.1\r\nHost: x\r\n\n",
    b"GET / HTTP/1.1\r\nHost: x\n\r\n",
    b"GET / HTTP/1.1\r\nHost: a\rb\r\n\r\n",
    b"GET / HTTP/1.1\r\nHost: x\r\nX-A: 1\r\n 2\r\n\r\n",
    b"GET / HTTP/1.1\r\nHost : x\r\n\r\n",
    b"\r\n \tGET / HTTP/1.1\r\nHost: x\r\n\r\n",
    b"GET / HTTP/1.1\r\nhost: x\r\nX-a: 1\r\nContent-Length: 0\r\n\r\n",
    b"GET / HTTP/1.1\r\nHo\x7fst: x\x01\r\nX-B: \xff\xfe\r\n\r\n",
    b"GET / HTTP/1.1\r\nHost: x\r\nnocolon\r\n\r\n",
    b"GET / HTTP/1.1\r\n\r\n",
    b"GET / HTTP/1.1\r\nHost: a\r\nhost: b\r\n\r\n",
    b"GET http://x/ HTTP/1.1\r\nHost: x\r\n\r\n",
    b"GET /\r\n\r\n",
    b"GET /\r\nHost: x\r\n\r\n",
    b"GET\r\nHost: x\r\n\r\n",
    b"GET / HTTP/1.7\r\nHost: x\r\n\r\n",
    b"POST / HTTP/1.1\r\nHost: x\r\nExpect: 100-Continue\r\n\r\n",
    b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length : 5\r\n\r\n",
    b"GET / HTTP/1.1\r\nHost: x\r\nX-A: 1\r\n \x01\r\n\r\n",
    b"\x00\r\n\r\n",
    b"GET / HTTP/1.1\r\n: v\r\n\r\n",
];

const RESPONSES: &[&[u8]] = &[
    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
    b"HTTP/1.1 404\nServer: x\n\n",
    b"HTTP/1.1 204 No Content\r\nX-A: 1\r\n 2\r\n\r\n",
];

fn head(p: &mut Parser, raw: &[u8]) -> RawMessage {
    match p.feed(raw).unwrap().event {
        Progress::Head(m) => m,
        other => panic!("expected head for {raw:?}, got {other:?}"),
    }
}

#[test]
fn write_exact_round_trips_every_head_diagnostics_input() {
    for raw in REQUESTS {
        let m = head(&mut Parser::request(&ParserConfig::default()), raw);
        let mut out = BytesMut::new();
        write_exact(&m, &mut out);
        assert_eq!(&out[..], *raw, "{raw:?}");
    }
}

#[test]
fn write_exact_round_trips_responses() {
    for raw in RESPONSES {
        let mut p = Parser::response(
            &ParserConfig::default(),
            ResponseContext::for_method(b"GET"),
        );
        let m = head(&mut p, raw);
        let mut out = BytesMut::new();
        write_exact(&m, &mut out);
        assert_eq!(&out[..], *raw, "{raw:?}");
    }
}

#[test]
fn write_exact_includes_inline_body() {
    let raw = b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\n";
    let m = head(&mut Parser::request(&ParserConfig::default()), raw)
        .with_inline_body(Bytes::from_static(b"hello"));
    let mut out = BytesMut::new();
    write_exact(&m, &mut out);
    let mut want = raw.to_vec();
    want.extend_from_slice(b"hello");
    assert_eq!(&out[..], &want[..]);
}

#[test]
fn body_encoder_matches_task7_example() {
    let mut e = BodyEncoder::chunked();
    let mut out = BytesMut::new();
    e.chunk(b"hello", Some(b"x=y"), &mut out);
    e.trailers(&[(b"X-T", b"1")], &mut out);
    e.finish(&mut out);
    assert_eq!(&out[..], b"5;x=y\r\nhello\r\n0\r\nX-T: 1\r\n\r\n");
}

#[test]
fn body_encoder_finish_without_trailers() {
    let mut e = BodyEncoder::chunked();
    let mut out = BytesMut::new();
    e.finish(&mut out);
    assert_eq!(&out[..], b"0\r\n\r\n");
}

#[test]
fn body_encoder_hex_is_lowercase() {
    let mut e = BodyEncoder::chunked();
    let mut out = BytesMut::new();
    e.chunk(&[b'a'; 255], None, &mut out);
    assert!(out.starts_with(b"ff\r\n"));
    assert_eq!(out.len(), 4 + 255 + 2);
}

#[test]
fn body_encoder_skips_empty_chunk() {
    let mut e = BodyEncoder::chunked();
    let mut out = BytesMut::new();
    e.chunk(b"", Some(b"x"), &mut out);
    assert!(out.is_empty());
}

#[test]
fn encoder_output_decodes_with_body_decoder() {
    let mut e = BodyEncoder::chunked();
    let mut out = BytesMut::new();
    e.chunk(b"hello", Some(b"x=y"), &mut out);
    e.chunk(b" world", None, &mut out);
    e.trailers(&[(b"X-T", b"1"), (b"X-U", b"two")], &mut out);
    e.finish(&mut out);

    let mut d = BodyDecoder::chunked(&ParseLimits::default());
    let mut pos = 0;
    let mut payload = Vec::new();
    let mut trailers = Vec::new();
    let mut steps = 0;
    loop {
        let input = &out[pos..];
        let (n, ev) = d.feed(input).unwrap();
        match ev {
            BodyEvent::Ext(_) | BodyEvent::NeedMore => {}
            BodyEvent::Data { data, .. } => payload.extend_from_slice(&input[data.range()]),
            BodyEvent::Trailers(ts) => {
                for t in ts {
                    trailers.push((
                        input[t.name().range()].to_vec(),
                        input[t.value().range()].to_vec(),
                    ));
                }
            }
            BodyEvent::Done => break,
            _ => unreachable!(),
        }
        pos += n;
        assert!(pos <= out.len());
        steps += 1;
        assert!(steps < 64, "decoder made no progress");
    }
    assert_eq!(payload, b"hello world");
    assert_eq!(
        trailers,
        vec![
            (b"X-T".to_vec(), b"1".to_vec()),
            (b"X-U".to_vec(), b"two".to_vec())
        ]
    );
}
