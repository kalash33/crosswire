//! The upstream TLS trust set (spec G7).
//!
//! Crosswire verifies upstream certificates against the Mozilla root list
//! that `webpki-roots` compiles into the binary. This test pins the size of
//! that list and the presence of one well-known anchor, so a dependency bump
//! that silently changes the trust set fails here instead of in the field.

/// `TLS_SERVER_ROOTS` holds 112 anchors in webpki-roots 1.0.12 (re-exported
/// unchanged by the 0.26.11 shim), and Let's Encrypt's ISRG Root X1 is one.
/// It was 120 in 1.0.10: 1.0.11 and 1.0.12 dropped eight roots that Mozilla
/// stopped trusting for TLS (Buypass Class 2 and 3, Certigna Root CA,
/// DigiCert Trusted Root G4, QuoVadis Root CA 1 G3 and 3 G3, emSign Root
/// CA - C1 and emSign ECC Root CA - C3) and added none.
///
/// A webpki-roots patch release may change the count legitimately when
/// Mozilla adds or removes a root. Whoever takes that update reads the
/// upstream diff, then bumps the number below in the same commit and names
/// the added or removed roots in its body.
#[test]
fn webpki_root_count_is_unchanged() {
    let roots = webpki_roots::TLS_SERVER_ROOTS;
    assert_eq!(roots.len(), 112, "webpki-roots trust set changed size");
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
