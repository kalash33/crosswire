//! Split-invariance: the parser must produce the same resolved events,
//! diagnostics and consumed total however the input is chopped across
//! `feed` calls.
mod common;

use common::{load_cases, parser_for, Case};
use crosswire_h1::*;
use proptest::collection::vec as pvec;
use proptest::test_runner::{Config, TestCaseError, TestRunner};

/// One resolved parser event. Spans are resolved to bytes against the input
/// of the call that produced them.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ev {
    Head {
        head_bytes: Vec<u8>,
        framing_name: String,
        diag_codes: Vec<&'static str>,
    },
    Data(Vec<u8>),
    Ext(Vec<u8>),
    Trailers(Vec<(Vec<u8>, Vec<u8>)>),
    Done,
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Trace {
    events: Vec<Ev>,
    body_diag_codes: Vec<&'static str>,
    consumed_total: usize,
}

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

fn is_until_close(f: &BodyFraming) -> bool {
    match f {
        BodyFraming::UntilClose => true,
        BodyFraming::Ambiguous { resolved, .. } => is_until_close(resolved),
        _ => false,
    }
}

fn variant_name(e: &ParseError) -> String {
    format!("{e:?}")
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

/// Feeds a case's input in the given pieces and records the resolved trace.
struct Run {
    p: Parser,
    events: Vec<Ev>,
    consumed_total: usize,
    /// Bytes the parser left unconsumed at `NeedMore`; they lead the next piece.
    carry: Vec<u8>,
    until_close: bool,
    stopped: bool,
}

impl Run {
    fn push(&mut self, ev: Ev) {
        // A body split across feeds legitimately yields more `BodyData`
        // events: coalesce adjacent ones.
        if let (Ev::Data(new), Some(Ev::Data(old))) = (&ev, self.events.last_mut()) {
            old.extend_from_slice(new);
        } else {
            self.events.push(ev);
        }
    }

    fn piece(&mut self, piece: &[u8]) {
        if self.stopped {
            return;
        }
        let mut buf = std::mem::take(&mut self.carry);
        buf.extend_from_slice(piece);
        let mut off = 0;
        // Unconsumed bytes are re-presented (`buf[off..]`) with every call:
        // incomplete size lines and trailer blocks are never buffered by the
        // parser. Zero-consumption steps (`Ext` peeks) need a bounded re-feed.
        let mut settled = false;
        for _ in 0..(buf.len() * 2 + 100) {
            let rest = &buf[off..];
            let step = match self.p.feed(rest) {
                Ok(s) => s,
                Err(e) => {
                    self.push(Ev::Error(variant_name(&e)));
                    self.stopped = true;
                    return;
                }
            };
            self.consumed_total += step.consumed;
            let mut finished = false;
            match step.event {
                Progress::NeedMore => finished = true,
                Progress::Head(m) => {
                    let mut codes: Vec<&'static str> =
                        m.diagnostics().iter().map(|d| d.code().as_str()).collect();
                    codes.sort_unstable();
                    self.until_close |= is_until_close(m.head().framing());
                    self.push(Ev::Head {
                        head_bytes: m.bytes()[..].to_vec(),
                        framing_name: framing_name(m.head().framing()).to_owned(),
                        diag_codes: codes,
                    });
                }
                Progress::BodyData { data, .. } => {
                    self.push(Ev::Data(rest[data.range()].to_vec()));
                }
                Progress::ChunkExt(s) => self.push(Ev::Ext(rest[s.range()].to_vec())),
                Progress::Trailers(t) => {
                    let t = t
                        .iter()
                        .map(|h| {
                            (
                                rest[h.name().range()].to_vec(),
                                rest[h.value().range()].to_vec(),
                            )
                        })
                        .collect();
                    self.push(Ev::Trailers(t));
                }
                Progress::Done => {
                    self.push(Ev::Done);
                    // Pipelined cases: the tail is the next message.
                    self.stopped = true;
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
        if !self.stopped {
            self.carry = buf[off..].to_vec();
        }
    }

    fn finish(mut self) -> Trace {
        if !self.stopped && self.until_close {
            match self.p.feed_eof() {
                Ok(s) if matches!(s.event, Progress::Done) => self.push(Ev::Done),
                Ok(s) => panic!("feed_eof returned {:?}", s.event),
                Err(e) => self.push(Ev::Error(variant_name(&e))),
            }
        }
        let mut codes: Vec<&'static str> = self
            .p
            .body_diagnostics()
            .iter()
            .map(|d| d.code().as_str())
            .collect();
        codes.sort_unstable();
        codes.dedup();
        Trace {
            events: self.events,
            body_diag_codes: codes,
            consumed_total: self.consumed_total,
        }
    }
}

fn trace(case: &Case, pieces: &[&[u8]]) -> Trace {
    let cfg = ParserConfig::default();
    let mut run = Run {
        p: parser_for(case, &cfg),
        events: vec![],
        consumed_total: 0,
        carry: vec![],
        until_close: false,
        stopped: false,
    };
    for piece in pieces {
        run.piece(piece);
    }
    run.finish()
}

fn short(ev: &Ev) -> String {
    let s = format!("{ev:?}");
    if s.len() > 300 {
        format!("{}...({} chars)", &s[..300], s.len())
    } else {
        s
    }
}

/// Describes the first difference between two traces, or `None` if equal.
fn diff(whole: &Trace, got: &Trace) -> Option<String> {
    for i in 0..whole.events.len().max(got.events.len()) {
        if whole.events.get(i) != got.events.get(i) {
            return Some(format!(
                "event #{i} differs\n  whole: {}\n  split: {}",
                whole.events.get(i).map_or("<none>".into(), short),
                got.events.get(i).map_or("<none>".into(), short)
            ));
        }
    }
    if whole.body_diag_codes != got.body_diag_codes {
        return Some(format!(
            "body diagnostics differ: whole {:?}, split {:?}",
            whole.body_diag_codes, got.body_diag_codes
        ));
    }
    // After an error `feed` reports no `consumed`, and bytes a split run
    // buffered in an earlier call are counted there, so the total is only
    // defined for traces that end without an error.
    let errored = whole.events.iter().any(|e| matches!(e, Ev::Error(_)));
    if !errored && whole.consumed_total != got.consumed_total {
        return Some(format!(
            "consumed_total differs: whole {}, split {}",
            whole.consumed_total, got.consumed_total
        ));
    }
    None
}

/// Cases larger than this get a bounded sample of split points (see below).
const BIG: usize = 64 * 1024;

fn check_case(case: &Case, failures: &mut Vec<String>) {
    let name = &case.name;
    let input = &case.input[..];
    let len = input.len();
    let whole = trace(case, &[input]);
    let mut fail = |strategy: &str, d: String| {
        failures.push(format!("case {name}: strategy {strategy}: {d}"));
    };

    // (A) two-piece splits. For inputs over 64 KiB (the 1 MiB large-body
    // case) every byte would be O(n^2), so use 256 evenly spaced points plus
    // every point in the first 512 bytes (the head) instead.
    let points: Vec<usize> = if len > BIG {
        let mut v: Vec<usize> = (1..len.min(512)).collect();
        v.extend((1..=256).map(|k| (len * k / 257).max(1)));
        v.sort_unstable();
        v.dedup();
        v
    } else {
        (1..len).collect()
    };
    for i in points {
        let got = trace(case, &[&input[..i], &input[i..]]);
        if let Some(d) = diff(&whole, &got) {
            fail(&format!("A (split at {i})"), d);
            break;
        }
    }

    // (B) one byte at a time; 4 KiB pieces for big inputs.
    let step = if len > BIG { 4096 } else { 1 };
    let pieces: Vec<&[u8]> = input.chunks(step).collect();
    let got = trace(case, &pieces);
    if let Some(d) = diff(&whole, &got) {
        fail(&format!("B (pieces of {step})"), d);
    }

    // (C) random split vectors, shrinking enabled.
    if len > 1 {
        let mut runner = TestRunner::new(Config {
            cases: 64,
            failure_persistence: None,
            ..Config::default()
        });
        let strat = pvec(1..len, 0..8);
        let res = runner.run(&strat, |mut cuts| {
            cuts.sort_unstable();
            cuts.dedup();
            let mut pieces: Vec<&[u8]> = vec![];
            let mut prev = 0;
            for c in cuts {
                pieces.push(&input[prev..c]);
                prev = c;
            }
            pieces.push(&input[prev..]);
            let got = trace(case, &pieces);
            match diff(&whole, &got) {
                None => Ok(()),
                Some(d) => Err(TestCaseError::fail(d)),
            }
        });
        if let Err(e) = res {
            fail("C (proptest)", e.to_string());
        }
    }
}

#[test]
fn split_invariance() {
    let cases = load_cases();
    assert!(!cases.is_empty(), "corpus has no cases");

    // Sanity: the comparison must not pass vacuously.
    let base = cases.iter().find(|c| c.name == "baseline-1").unwrap();
    let t = trace(base, &[&base.input]);
    assert!(
        matches!(
            t.events.first(),
            Some(Ev::Head { head_bytes, framing_name, .. })
                if head_bytes.starts_with(b"GET / HTTP/1.1") && framing_name == "none"
        ) && t.events.last() == Some(&Ev::Done),
        "sanity: baseline-1 whole-feed trace lacks Head/Done: {:?}",
        t.events
    );
    assert_eq!(t.consumed_total, base.input.len());

    let mut failures = vec![];
    let mut ran = 0;
    let mut skipped_known_bug = 0;
    for case in &cases {
        if let Some(bug) = &case.expect.known_bug {
            println!("known_bug {}: {bug}", case.name);
            skipped_known_bug += 1;
            continue;
        }
        let w = trace(case, &[&case.input]);
        assert!(
            matches!(w.events.first(), Some(Ev::Head { .. })),
            "case {}: whole-feed trace has no Head: {:?}",
            case.name,
            w.events.iter().map(short).collect::<Vec<_>>()
        );
        check_case(case, &mut failures);
        ran += 1;
    }
    assert!(
        failures.is_empty(),
        "{} split-invariance failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(ran + skipped_known_bug, cases.len());
    assert!(ran >= 90, "only {ran} cases ran");
    println!("split_prop: {ran} cases checked");
}
