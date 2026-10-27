//! The anonymous-RSS probe for the §13 memory bound.
//!
//! The bound is on **anonymous** memory: it excludes the mmap'd database
//! file, whose pages are file-backed and reclaimable. `VmRSS` (and a crate's
//! "physical memory" on Linux) includes those pages and would wrongly count
//! the database, so the probe reads `RssAnon` on Linux and `phys_footprint`
//! on macOS.
//!
//! This file is deliberately not part of the library's module tree. The macOS
//! branch needs the `memory-stats` crate, which is a macOS-only
//! dev-dependency (the library stays free of it and of `unsafe`), so the
//! integration test and the bench both include it with `#[path]`.
#![allow(dead_code)]

/// Parses the `RssAnon:` line of a `/proc/<pid>/status` text into bytes.
///
/// Only `RssAnon:` is read; `VmRSS:` and `RssFile:` are ignored on purpose.
/// Returns `None` when the line is absent or its value is not a number.
#[must_use]
pub fn parse_rss_anon(status: &str) -> Option<u64> {
    let rest = status
        .lines()
        .find_map(|l| l.strip_prefix("RssAnon:"))?
        .trim();
    let kb: u64 = rest
        .strip_suffix("kB")
        .unwrap_or(rest)
        .trim()
        .parse()
        .ok()?;
    kb.checked_mul(1024)
}

/// The process's current anonymous resident memory in bytes, excluding
/// file-backed (mmap'd) pages.
///
/// Linux: the `RssAnon:` line of `/proc/self/status`. macOS: `phys_footprint`
/// from `memory-stats`. `None` on other targets or if the read fails.
#[must_use]
pub fn anon_rss_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        parse_rss_anon(&std::fs::read_to_string("/proc/self/status").ok()?)
    }
    #[cfg(target_os = "macos")]
    {
        // `physical_mem` is `phys_footprint` on macOS (the Activity Monitor
        // "Memory" figure); the crate wraps the `task_info` call.
        memory_stats::memory_stats().map(|m| m.physical_mem as u64)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}
