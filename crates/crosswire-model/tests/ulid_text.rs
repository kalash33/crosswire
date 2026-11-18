//! The ULID text form is frozen (spec G6).
//!
//! Exchange ids are stored and exchanged as 26-character Crockford base32
//! ULIDs, and keyset pagination orders by that text (`docs/format.md`). This
//! test pins the encoding so a `ulid` upgrade cannot change it unnoticed. It
//! uses only `from_parts`, `from_string`, `nil`, `ULID_LEN` and `Display`,
//! none of which needs ulid's `std` feature, so it compiles unchanged on
//! ulid 1 and 3 and under `--no-default-features`.
use ulid::Ulid;

/// The timestamp of the ULID specification's example id.
const T: u64 = 1_469_918_176_385;
/// The largest 80-bit random part.
const MAX_RANDOM: u128 = u128::MAX >> 48;

#[test]
fn ulid_text_is_frozen() {
    // The example from the ULID specification, with a zero random part.
    assert_eq!(
        Ulid::from_parts(T, 0).to_string(),
        "01ARYZ6S410000000000000000"
    );
    // Pinned by running this test once on ulid 1.2.1 and copying the value.
    assert_eq!(
        Ulid::from_parts(T, MAX_RANDOM).to_string(),
        "01ARYZ6S41ZZZZZZZZZZZZZZZZ"
    );
    assert_eq!(Ulid::nil().to_string(), "00000000000000000000000000");
    assert_eq!(ulid::ULID_LEN, 26);
}

#[test]
fn ulid_parsing_rules_are_pinned() {
    let want = Ulid::from_parts(T, 0);
    assert_eq!(Ulid::from_string("01ARYZ6S410000000000000000"), Ok(want));
    assert_eq!(Ulid::from_string("01aryz6s410000000000000000"), Ok(want));
    for bad in ['I', 'L', 'O', 'U'] {
        let s = format!("01ARYZ6S41000000000000000{bad}");
        assert!(Ulid::from_string(&s).is_err(), "{s} must be refused");
    }
    // One character short and one character long.
    assert!(Ulid::from_string(&"0".repeat(25)).is_err());
    assert!(Ulid::from_string(&"0".repeat(27)).is_err());
    // The largest ULID. A first character above `7` sets bits past 128:
    // ulid 1.2.1 and 3.0.0 both accept it and drop those bits (`8` aliases
    // `0`) rather than refusing it. Pinned as found, so a change in either
    // direction is noticed.
    assert_eq!(
        Ulid::from_string("7ZZZZZZZZZZZZZZZZZZZZZZZZZ"),
        Ok(Ulid::from_parts(u64::MAX >> 16, MAX_RANDOM))
    );
    assert_eq!(
        Ulid::from_string("80000000000000000000000000"),
        Ok(Ulid::nil())
    );
}

#[test]
fn ulid_order_is_timestamp_first() {
    // Keyset pagination relies on the next millisecond sorting after every
    // id of the previous one, in both the value and the text form.
    let late = Ulid::from_parts(T + 1, 0);
    let early = Ulid::from_parts(T, MAX_RANDOM);
    assert!(late > early);
    assert!(late.to_string() > early.to_string());
}
