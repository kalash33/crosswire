//! The edit engine: `apply_edits`, `content_length_edit` and `crlf_edits`.
use bytes::{Bytes, BytesMut};
use crosswire_core::{ChangeOp, ChangeReason, DiagnosticCode, Span};
use crosswire_h1::*;

fn request(raw: &[u8]) -> RawMessage {
    let mut p = Parser::request(&ParserConfig::default());
    let step = p.feed(raw).unwrap();
    let head_len = step.consumed;
    match step.event {
        Progress::Head(m) if head_len < raw.len() => {
            m.with_inline_body(Bytes::copy_from_slice(&raw[head_len..]))
        }
        Progress::Head(m) => m,
        other => panic!("expected head, got {other:?}"),
    }
}

fn apply(msg: &RawMessage, edits: &[Edit]) -> Result<Edited, EditError> {
    apply_edits(
        msg,
        edits,
        &ParserConfig::default(),
        None,
        ChangeReason::InterceptEdit,
    )
}

fn written(msg: &RawMessage) -> Vec<u8> {
    let mut out = BytesMut::new();
    write_exact(msg, &mut out);
    out.to_vec()
}

fn sp(a: usize, b: usize) -> Span {
    Span::new(a as u32, b as u32)
}

fn edit(a: usize, b: usize, with: &'static [u8]) -> Edit {
    Edit {
        span: sp(a, b),
        replacement: Bytes::from_static(with),
    }
}

#[test]
fn replace_one_header_keeps_every_body_byte() {
    let body: Vec<u8> = (0..=255u8).collect();
    let head = b"POST /up HTTP/1.1\r\nHost: x\r\nUser-Agent: curl/8\r\nContent-Length: 256\r\n\r\n";
    let mut raw = head.to_vec();
    raw.extend_from_slice(&body);
    let msg = request(&raw);
    let ua = msg.head().header(msg.bytes(), "user-agent").unwrap();
    let e = Edit {
        span: ua,
        replacement: Bytes::from_static(b"crosswire/1"),
    };
    let out = apply(&msg, &[e]).unwrap();

    let new_head =
        b"POST /up HTTP/1.1\r\nHost: x\r\nUser-Agent: crosswire/1\r\nContent-Length: 256\r\n\r\n";
    let mut expected = new_head.to_vec();
    expected.extend_from_slice(&body);
    assert_eq!(written(&out.message), expected);
    assert_eq!(out.message.bytes().as_ref(), &new_head[..]);
    assert_eq!(out.message.body().unwrap().as_ref(), &body[..]);

    assert_eq!(out.changes.len(), 1);
    let c = &out.changes[0];
    assert_eq!(c.op(), ChangeOp::Replace);
    assert_eq!(c.reason(), &ChangeReason::InterceptEdit);
    assert_eq!(c.before().as_ref(), b"curl/8");
    assert_eq!(c.after().as_ref(), b"crosswire/1");
    assert_eq!(c.span(), Span::new(ua.start(), ua.start() + 11));
    assert_eq!(&written(&out.message)[c.span().range()], b"crosswire/1");
    assert!(out.new_diagnostics.is_empty());
}

#[test]
fn insert_header_at_end_of_head() {
    let raw = b"GET / HTTP/1.1\r\nHost: x\r\n\r\n";
    let msg = request(raw);
    let at = raw.len() - 2;
    let out = apply(&msg, &[edit(at, at, b"X-Test: 1\r\n")]).unwrap();
    assert_eq!(
        written(&out.message),
        b"GET / HTTP/1.1\r\nHost: x\r\nX-Test: 1\r\n\r\n"
    );
    assert_eq!(out.message.head().headers().len(), 2);
    let c = &out.changes[0];
    assert_eq!(c.op(), ChangeOp::Insert);
    assert!(c.before().is_empty());
    assert_eq!(c.span(), sp(at, at + 11));
}

#[test]
fn delete_header_line() {
    let raw = b"GET / HTTP/1.1\r\nHost: x\r\nCookie: a=b\r\nAccept: */*\r\n\r\n";
    let msg = request(raw);
    let line = msg.head().headers()[1].line();
    let e = Edit {
        span: line,
        replacement: Bytes::new(),
    };
    let out = apply(&msg, &[e]).unwrap();
    assert_eq!(
        written(&out.message),
        b"GET / HTTP/1.1\r\nHost: x\r\nAccept: */*\r\n\r\n"
    );
    let c = &out.changes[0];
    assert_eq!(c.op(), ChangeOp::Delete);
    assert_eq!(c.before().as_ref(), b"Cookie: a=b\r\n");
    assert_eq!(c.span(), Span::new(line.start(), line.start()));
}

#[test]
fn overlapping_edits_are_rejected() {
    let msg = request(b"GET / HTTP/1.1\r\nHost: abcdef\r\n\r\n");
    let err = apply(&msg, &[edit(24, 27, b"x"), edit(22, 25, b"y")]).unwrap_err();
    assert_eq!(err, EditError::Overlap);
    // Adjacent edits and an insert at the start of a replacement are fine.
    let out = apply(
        &msg,
        &[edit(22, 25, b"X"), edit(25, 28, b"Y"), edit(22, 22, b"<")],
    )
    .unwrap();
    assert_eq!(
        written(&out.message),
        b"GET / HTTP/1.1\r\nHost: <XY\r\n\r\n"
    );
    // Changes are in ascending order, in after-coordinates.
    let spans: Vec<Span> = out.changes.iter().map(|c| c.span()).collect();
    assert_eq!(spans, [sp(22, 23), sp(23, 24), sp(24, 25)]);
}

#[test]
fn out_of_range_is_rejected() {
    let raw = b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\n\r\nhi";
    let msg = request(raw);
    assert_eq!(
        apply(&msg, &[edit(raw.len(), raw.len() + 1, b"")]).unwrap_err(),
        EditError::OutOfRange
    );
    // An insert at the very end of the body is in range.
    let out = apply(&msg, &[edit(raw.len(), raw.len(), b"!")]).unwrap();
    assert_eq!(out.message.body().unwrap().as_ref(), b"hi!");
}

#[test]
fn body_edit_without_inline_body_is_rejected() {
    let raw = b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\n\r\n";
    let msg = request(raw);
    assert!(msg.body().is_none());
    assert_eq!(
        apply(&msg, &[edit(raw.len(), raw.len() + 1, b"z")]).unwrap_err(),
        EditError::BodyNotInline
    );
}

#[test]
fn reparse_error_surfaces() {
    let raw = b"GET / HTTP/1.1\r\nHost: x\r\n\r\n";
    let msg = request(raw);
    let err = apply(&msg, &[edit(raw.len() - 2, raw.len(), b"")]).unwrap_err();
    assert_eq!(err, EditError::Reparse(ParseError::UnexpectedEof));
}

#[test]
fn content_length_edit_updates_and_skips_te() {
    let raw = b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 3\r\n\r\nhello";
    let msg = request(&raw[..raw.len() - 2]).with_inline_body(Bytes::from_static(b"hello"));
    let e = content_length_edit(&msg).unwrap();
    assert_eq!(e.replacement.as_ref(), b"5");
    assert_eq!(&msg.bytes()[e.span.range()], b"3");
    let out = apply_edits(
        &msg,
        &[e],
        &ParserConfig::default(),
        None,
        ChangeReason::ContentLength,
    )
    .unwrap();
    assert_eq!(out.message.head().framing(), &BodyFraming::ContentLength(5));
    assert_eq!(content_length_edit(&out.message), None);

    let te = request(
        b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nContent-Length: 3\r\n\r\n",
    )
    .with_inline_body(Bytes::from_static(b"0\r\n\r\n"));
    assert_eq!(content_length_edit(&te), None);

    // No header but a body: the header is inserted before the blank line.
    let bare =
        request(b"POST / HTTP/1.1\r\nHost: x\r\n\r\n").with_inline_body(Bytes::from_static(b"ab"));
    let e = content_length_edit(&bare).unwrap();
    assert!(e.span.is_empty());
    let out = apply(&bare, &[e]).unwrap();
    assert_eq!(
        written(&out.message),
        b"POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\n\r\nab"
    );

    // No inline body: nothing to measure.
    assert_eq!(
        content_length_edit(&request(b"POST / HTTP/1.1\r\nContent-Length: 3\r\n\r\n")),
        None
    );
}

#[test]
fn crlf_edits_normalise_bare_lf() {
    let raw = b"GET / HTTP/1.1\nHost: x\r\nAccept: */*\n\n";
    let msg = request(raw);
    let edits = crlf_edits(&msg);
    assert_eq!(edits.len(), 3);
    assert!(edits
        .iter()
        .all(|e| e.span.is_empty() && e.replacement.as_ref() == b"\r"));
    let out = apply(&msg, &edits).unwrap();
    assert_eq!(
        written(&out.message),
        b"GET / HTTP/1.1\r\nHost: x\r\nAccept: */*\r\n\r\n"
    );
    assert!(out
        .message
        .diagnostics()
        .iter()
        .all(|d| d.code() != DiagnosticCode::BareLf));
    assert!(crlf_edits(&out.message).is_empty());
}

#[test]
fn new_diagnostics_reports_only_added_codes() {
    // The bare LF after `X-A: 1` exists before the edit and only moves.
    let raw = b"GET / HTTP/1.1\r\nHost: x\r\nX-A: 1\n\r\n";
    let msg = request(raw);
    assert!(msg
        .diagnostics()
        .iter()
        .any(|d| d.code() == DiagnosticCode::BareLf));
    let host = msg.head().header(msg.bytes(), "host").unwrap();
    let e = Edit {
        span: host,
        replacement: Bytes::from_static(b"a\rb"),
    };
    let out = apply(&msg, &[e]).unwrap();
    let codes: Vec<DiagnosticCode> = out.new_diagnostics.iter().map(|d| d.code()).collect();
    assert_eq!(codes, [DiagnosticCode::BareCr]);
    assert!(out
        .message
        .diagnostics()
        .iter()
        .any(|d| d.code() == DiagnosticCode::BareLf));
}

#[test]
fn response_edit_reparses_with_the_given_context() {
    let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\n";
    let ctx = ResponseContext::for_method(b"HEAD");
    let mut p = Parser::response(&ParserConfig::default(), ctx);
    let msg = match p.feed(raw).unwrap().event {
        Progress::Head(m) => m,
        other => panic!("expected head, got {other:?}"),
    };
    let out = apply_edits(
        &msg,
        &[edit(9, 15, b"201 Created")],
        &ParserConfig::default(),
        Some(ctx),
        ChangeReason::Custom("test".into()),
    )
    .unwrap();
    assert_eq!(out.message.kind(), Kind::Response);
    assert_eq!(
        out.message.head().status_code(out.message.bytes()),
        Some(201)
    );
    assert_eq!(out.message.head().framing(), msg.head().framing());
    assert!(out.message.body().is_none());
}
