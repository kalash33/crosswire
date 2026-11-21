//! The upstream TLS trust set (spec G7).
//!
//! Crosswire verifies upstream certificates against the Mozilla root list
//! that `webpki-roots` compiles into the binary. This test pins the size of
//! that list and the presence of one well-known anchor, so a dependency bump
//! that silently changes the trust set fails here instead of in the field.

/// `TLS_SERVER_ROOTS` holds 120 anchors in webpki-roots 1.0.10 (re-exported
/// unchanged by the 0.26.11 shim), and Let's Encrypt's ISRG Root X1 is one.
///
/// A webpki-roots patch release may change the count legitimately when
/// Mozilla adds or removes a root. Whoever takes that update reads the
/// upstream diff, then bumps the number below in the same commit and names
/// the added or removed roots in its body.
#[test]
fn webpki_root_count_is_unchanged() {
    let roots = webpki_roots::TLS_SERVER_ROOTS;
    assert_eq!(roots.len(), 120, "webpki-roots trust set changed size");
    let needle: &[u8] = b"ISRG Root X1";
    assert!(
        roots.iter().any(|a| a
            .subject
            .as_ref()
            .windows(needle.len())
            .any(|w| w == needle)),
        "ISRG Root X1 is missing from the trust set"
    );
}
