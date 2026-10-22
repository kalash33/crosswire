//! Shared test helpers: the golden corpus loader and the canonical feed loop
//! ([`drive`]) used by both `corpus.rs` and `split_prop.rs`. The fuzz targets
//! keep a copy of the same loop in `fuzz/common.rs` because they cannot
//! depend on this test crate.
#![allow(dead_code)]

use std::fs;
use std::ops::Range;
use std::path::PathBuf;

use crosswire_core::Diagnostic;
use crosswire_h1::{ParseError, ParseLimits, Parser, ParserConfig, Progress, RawMessage};
use serde::Deserialize;

/// The `ctx` block of a response case.
#[derive(Debug, Deserialize)]
pub struct Ctx {
    pub method: String,
    #[serde(default)]
    pub is_connect: bool,
}

/// Optional `limits` block: overrides of the default [`ParseLimits`].
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub head_size: Option<u32>,
    pub max_headers: Option<u16>,
    pub max_line: Option<u32>,
    pub max_chunk_size_line: Option<u32>,
    pub max_trailer_size: Option<u32>,
    pub max_chunks: Option<u64>,
    pub max_body_diagnostics: Option<u16>,
}

/// The expectations stored in `<name>.expect.json`.
#[derive(Debug, Deserialize)]
pub struct Expect {
    pub kind: String,
    pub ctx: Option<Ctx>,
    /// Required unless `error` is set and the error comes before the head.
    pub framing: Option<String>,
    pub resolved: Option<String>,
    pub rule: Option<String>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    #[serde(default)]
    pub pipelined: bool,
    pub body: Option<String>,
    pub body_hex: Option<String>,
    pub error: Option<String>,
    pub trailers: Option<Vec<[String; 2]>>,
    pub known_bug: Option<String>,
    pub limits: Option<Limits>,
    pub source: Option<String>,
}

/// The `<name>.gen.json` descriptor of a generated case.
#[derive(Debug, Deserialize)]
struct Gen {
    kind: String,
    head: String,
    body_byte: u8,
    body_len: usize,
    framing: String,
    #[serde(default)]
    diagnostics: Vec<String>,
}

/// One corpus case.
#[derive(Debug)]
pub struct Case {
    pub name: String,
    pub input: Vec<u8>,
    pub expect: Expect,
}

pub fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/h1/cases")
}

/// Loads every case, sorted by name.
pub fn load_cases() -> Vec<Case> {
    let dir = corpus_dir();
    let mut cases = vec![];
    let entries = fs::read_dir(&dir).unwrap_or_else(|e| panic!("cannot read {dir:?}: {e}"));
    for entry in entries {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        if let Some(name) = file.strip_suffix(".msg") {
            let input = fs::read(&path).unwrap();
            let json_path = dir.join(format!("{name}.expect.json"));
            let json = fs::read_to_string(&json_path)
                .unwrap_or_else(|e| panic!("case {name}: missing expect.json: {e}"));
            let expect: Expect = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("case {name}: bad expect.json: {e}"));
            cases.push(Case {
                name: name.to_owned(),
                input,
                expect,
            });
        } else if let Some(name) = file.strip_suffix(".gen.json") {
            let g: Gen = serde_json::from_str(&fs::read_to_string(&path).unwrap())
                .unwrap_or_else(|e| panic!("case {name}: bad gen.json: {e}"));
            let mut input = g.head.into_bytes();
            input.resize(input.len() + g.body_len, g.body_byte);
            cases.push(Case {
                name: name.to_owned(),
                input,
                expect: Expect {
                    kind: g.kind,
                    ctx: None,
                    framing: Some(g.framing),
                    resolved: None,
                    rule: None,
                    diagnostics: g.diagnostics,
                    pipelined: false,
                    body: None,
                    body_hex: None,
                    error: None,
                    trailers: None,
                    known_bug: None,
                    limits: None,
                    source: None,
                },
            });
        }
    }
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    cases
}

/// The parser configuration a case asks for: defaults plus its `limits`.
pub fn config_for(case: &Case) -> ParserConfig {
    let mut b = ParseLimits::builder();
    if let Some(l) = &case.expect.limits {
        if let Some(v) = l.head_size {
            b = b.head_size(v);
        }
        if let Some(v) = l.max_headers {
            b = b.max_headers(v);
        }
        if let Some(v) = l.max_line {
            b = b.max_line(v);
        }
        if let Some(v) = l.max_chunk_size_line {
            b = b.max_chunk_size_line(v);
        }
        if let Some(v) = l.max_trailer_size {
            b = b.max_trailer_size(v);
        }
        if let Some(v) = l.max_chunks {
            b = b.max_chunks(v);
        }
        if let Some(v) = l.max_body_diagnostics {
            b = b.max_body_diagnostics(v);
        }
    }
    ParserConfig::builder().limits(b.build()).build()
}

/// Builds the parser a case expects: a request parser, or a response parser
/// with the case's `ctx`, configured with [`config_for`].
pub fn parser_for(case: &Case) -> Parser {
    use crosswire_h1::ResponseContext;
    let cfg = config_for(case);
    let name = &case.name;
    match case.expect.kind.as_str() {
        "request" => Parser::request(&cfg),
        "response" => {
            let c = case
                .expect
                .ctx
                .as_ref()
                .unwrap_or_else(|| panic!("case {name}: response needs ctx"));
            let mut ctx = ResponseContext::for_method(c.method.as_bytes());
            ctx.is_connect = c.is_connect;
            Parser::response(&cfg, ctx)
        }
        k => panic!("case {name}: bad kind {k}"),
    }
}

/// The snake_case name of a parse error, as used in `expect.json`.
pub fn error_name(e: &ParseError) -> &'static str {
    match e {
        ParseError::HeadTooLarge => "head_too_large",
        ParseError::LineTooLong => "line_too_long",
        ParseError::TooManyHeaders => "too_many_headers",
        ParseError::NoTerminator => "no_terminator",
        ParseError::UnexpectedEof => "unexpected_eof",
        ParseError::InvalidChunkSize { .. } => "invalid_chunk_size",
        ParseError::TrailerTooLarge => "trailer_too_large",
        ParseError::TooManyChunks => "too_many_chunks",
        _ => "unknown",
    }
}

/// One resolved parser event. Body-phase spans are resolved to bytes (or
/// absolute input offsets) at the call that produced them, so events do not
/// depend on how the input was split.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ev {
    /// The head; `end` is its absolute end offset in the input.
    Head {
        msg: RawMessage,
        end: usize,
    },
    /// Body bytes: `raw` is the absolute input range of the framed bytes,
    /// `data` the decoded payload. Adjacent pieces are coalesced.
    Data {
        raw: Range<usize>,
        data: Vec<u8>,
    },
    /// A chunk extension's text.
    Ext(Vec<u8>),
    /// Trailer fields as (name, value) bytes.
    Trailers(Vec<(Vec<u8>, Vec<u8>)>),
    Done,
    /// A fatal error, by name (its span, if any, is call-relative).
    Error(&'static str),
}

/// Everything one [`drive`] run observed.
#[derive(Debug, Clone)]
pub struct Trace {
    pub events: Vec<Ev>,
    /// The parser's body diagnostics at the end (absolute spans).
    pub body_diags: Vec<Diagnostic>,
    pub body_diags_dropped: usize,
    /// Total bytes consumed: the absolute offset of the first byte the
    /// parser did not take (for a pipelined input, the start of the tail).
    pub consumed_total: usize,
    pub error: Option<ParseError>,
}

impl Trace {
    pub fn head(&self) -> Option<(&RawMessage, usize)> {
        self.events.iter().find_map(|e| match e {
            Ev::Head { msg, end } => Some((msg, *end)),
            _ => None,
        })
    }

    pub fn done(&self) -> bool {
        self.events.last() == Some(&Ev::Done)
    }
}

/// The reference feed loop. Feeds `pieces` in order with the re-present
/// contract: the bytes the parser leaves unconsumed at `NeedMore` lead the
/// next piece. Within a piece it loops until `NeedMore` (zero-consumption
/// steps such as `ChunkExt` are re-fed with the same bytes) and stops for
/// good at `Done` (the tail is the next pipelined message) or an error.
/// After the last piece it signals end of input with `feed_eof`.
pub fn drive(p: &mut Parser, pieces: &[&[u8]]) -> Trace {
    let mut t = Trace {
        events: vec![],
        body_diags: vec![],
        body_diags_dropped: 0,
        consumed_total: 0,
        error: None,
    };
    let mut carry: Vec<u8> = vec![];
    let mut stopped = false;
    for piece in pieces {
        let mut buf = std::mem::take(&mut carry);
        buf.extend_from_slice(piece);
        let mut off = 0;
        let mut settled = false;
        for _ in 0..(buf.len() * 2 + 100) {
            let rest = &buf[off..];
            // `rest[0]` is input byte `base`: every consumed byte was counted.
            let base = t.consumed_total;
            let step = match p.feed(rest) {
                Ok(s) => s,
                Err(e) => {
                    t.events.push(Ev::Error(error_name(&e)));
                    t.error = Some(e);
                    stopped = true;
                    settled = true;
                    break;
                }
            };
            assert!(step.consumed <= rest.len(), "consumed > input");
            t.consumed_total += step.consumed;
            let mut finished = false;
            match step.event {
                Progress::NeedMore => finished = true,
                Progress::Head(msg) => push(
                    &mut t.events,
                    Ev::Head {
                        msg,
                        end: base + step.consumed,
                    },
                ),
                Progress::BodyData { raw, data } => push(
                    &mut t.events,
                    Ev::Data {
                        raw: base + raw.range().start..base + raw.range().end,
                        data: rest[data.range()].to_vec(),
                    },
                ),
                Progress::ChunkExt(s) => push(&mut t.events, Ev::Ext(rest[s.range()].to_vec())),
                Progress::Trailers(tr) => {
                    let tr = tr
                        .iter()
                        .map(|h| {
                            (
                                rest[h.name().range()].to_vec(),
                                rest[h.value().range()].to_vec(),
                            )
                        })
                        .collect();
                    push(&mut t.events, Ev::Trailers(tr));
                }
                Progress::Done => {
                    push(&mut t.events, Ev::Done);
                    stopped = true;
                    finished = true;
                }
                _ => {}
            }
            off += step.consumed;
            if finished {
                settled = true;
                break;
            }
        }
        assert!(settled, "feed loop did not settle");
        if stopped {
            break;
        }
        carry = buf[off..].to_vec();
    }
    if !stopped {
        match p.feed_eof() {
            Ok(s) if matches!(s.event, Progress::Done) => push(&mut t.events, Ev::Done),
            Ok(s) => panic!("feed_eof returned {:?}", s.event),
            Err(e) => {
                t.events.push(Ev::Error(error_name(&e)));
                t.error = Some(e);
            }
        }
    }
    t.body_diags = p.body_diagnostics().to_vec();
    t.body_diags_dropped = p.body_diagnostics_dropped();
    t
}

/// Appends `ev`, coalescing contiguous `Data` pieces (a body split across
/// feeds legitimately yields more of them).
fn push(events: &mut Vec<Ev>, ev: Ev) {
    if let (
        Ev::Data { raw, data },
        Some(Ev::Data {
            raw: old_raw,
            data: old_data,
        }),
    ) = (&ev, events.last_mut())
    {
        if old_raw.end == raw.start {
            old_raw.end = raw.end;
            old_data.extend_from_slice(data);
            return;
        }
    }
    events.push(ev);
}
