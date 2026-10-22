//! Tests for `Span`.
use crosswire_core::Span;
#[test]
fn span_len_and_range() {
    let s = Span::new(3, 7);
    assert_eq!(s.len(), 4);
    assert!(!s.is_empty());
    assert_eq!(s.range(), 3..7);
}
#[test]
fn empty_span() {
    assert!(Span::new(5, 5).is_empty());
}
#[test]
fn empty_span_len_is_zero() {
    assert_eq!(Span::new(5, 5).len(), 0);
}
