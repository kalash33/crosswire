//! Split-invariance: the parser must produce the same resolved events,
//! diagnostics and consumed total however the input is chopped across
//! `feed` calls.
mod common;

use common::{drive, load_cases, parser_for, Case, Ev, Trace};
use crosswire_h1::BodyFraming;
use proptest::collection::vec as pvec;
use proptest::test_runner::{Config, TestCaseError, TestRunner};

/// Feeds a case's input in the given pieces through the canonical loop.
fn trace(case: &Case, pieces: &[&[u8]]) -> Trace {
    drive(&mut parser_for(case), pieces)
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
    // Body diagnostics: every code with its count (a multiset, not a
    // deduplicated set) and its absolute span, in order.
    if whole.body_diags != got.body_diags {
        return Some(format!(
            "body diagnostics differ: whole {:?}, split {:?}",
            whole.body_diags, got.body_diags
        ));
    }
    if whole.body_diags_dropped != got.body_diags_dropped {
        return Some(format!(
            "dropped body diagnostics differ: whole {}, split {}",
            whole.body_diags_dropped, got.body_diags_dropped
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
            Some(Ev::Head { msg, .. })
                if msg.bytes().starts_with(b"GET / HTTP/1.1")
                    && *msg.head().framing() == BodyFraming::None
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
        // Only an error case may stop before the head.
        assert!(
            matches!(w.events.first(), Some(Ev::Head { .. }))
                || (case.expect.error.is_some() && matches!(w.events.as_slice(), [Ev::Error(_)])),
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
