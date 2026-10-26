//! The anonymous-RSS probe (§13 names `RssAnon` / `phys_footprint`, not
//! `VmRSS`, because the memory bound excludes the mmap'd database file).
//!
//! The probe lives in `src/rss.rs` and is compiled into this test and the
//! bench by path rather than into the library: its macOS branch needs the
//! `memory-stats` crate, which is a dev-dependency only.

#[path = "../src/rss.rs"]
mod rss;

use rss::{anon_rss_bytes, parse_rss_anon};

#[test]
fn reports_a_positive_value_on_supported_targets() {
    if cfg!(any(target_os = "linux", target_os = "macos")) {
        assert!(anon_rss_bytes().is_some_and(|x| x > 0));
    } else {
        assert_eq!(anon_rss_bytes(), None);
    }
}

#[test]
fn tracks_touched_anonymous_memory() {
    if !cfg!(any(target_os = "linux", target_os = "macos")) {
        return;
    }
    const SIZE: usize = 128 * 1024 * 1024;
    let before = anon_rss_bytes().unwrap();
    // Non-zero fill so the pages are really written (not lazily zeroed).
    let mut buf = vec![0u8; SIZE];
    for (i, b) in buf.iter_mut().enumerate().step_by(4096) {
        *b = (i >> 12) as u8 | 1;
    }
    let during = anon_rss_bytes().unwrap();
    std::hint::black_box(&buf);
    let grown = during.saturating_sub(before);
    assert!(
        grown > (SIZE as u64) * 3 / 4 && grown < (SIZE as u64) * 3 / 2,
        "128 MiB touched, probe rose by {grown}"
    );
    drop(buf);
}

/// The Linux reader must take `RssAnon:`, never `VmRSS:` (which includes
/// file-backed pages such as the mmap'd database). The fixture makes the two
/// differ by orders of magnitude so a wrong line cannot pass.
#[test]
fn parses_rss_anon_not_vmrss() {
    let status = "Name:\ttest\n\
                  VmPeak:\t  900000 kB\n\
                  VmRSS:\t  800000 kB\n\
                  RssAnon:\t   12345 kB\n\
                  RssFile:\t  787000 kB\n\
                  RssShmem:\t       0 kB\n";
    assert_eq!(parse_rss_anon(status), Some(12_345 * 1024));
}

#[test]
fn rss_anon_missing_or_malformed_is_none() {
    assert_eq!(parse_rss_anon("VmRSS:\t 100 kB\n"), None);
    assert_eq!(parse_rss_anon("RssAnon:\t abc kB\n"), None);
    assert_eq!(parse_rss_anon(""), None);
}
