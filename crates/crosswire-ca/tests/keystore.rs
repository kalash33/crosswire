//! File key store round trip and permissions.

use crosswire_ca::{Ca, CaError, FileKeyStore, KeyStore};

#[test]
fn file_keystore_roundtrip_and_modes() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("ca");
    let store = FileKeyStore::new(&dir);
    assert!(store.load().unwrap().is_none());

    let first = Ca::load_or_generate(&store, "test").unwrap();
    let second = Ca::load_or_generate(&store, "test").unwrap();
    assert_eq!(first.cert_der(), second.cert_der());
    assert_eq!(first.spki_sha256_base64(), second.spki_sha256_base64());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.join("ca.key")), 0o600);
        assert_eq!(mode(&dir.join("ca.crt")), 0o644);
        assert_eq!(mode(&dir), 0o700);
    }
}

#[test]
fn mismatched_key_is_rejected() {
    let a = Ca::generate("a").unwrap();
    let b = Ca::generate("b").unwrap();
    let err = Ca::from_pem(&a.cert_pem(), &b.key_pem()).unwrap_err();
    assert!(matches!(err, CaError::Pem(_)));
}

#[test]
fn debug_never_prints_key() {
    let ca = Ca::generate("a").unwrap();
    let dbg = format!("{ca:?}");
    assert!(!dbg.contains("PRIVATE"));
}
