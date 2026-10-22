//! Collects diagnostics, assigning each its severity from the active profile.

use alloc::vec::Vec;

use crosswire_core::{Diagnostic, DiagnosticCode, Profile, Span};

/// An ordered collection of diagnostics for one message, optionally capped.
///
/// With a cap, at most `cap` diagnostics are stored and the backing `Vec`
/// never grows beyond `cap` entries of capacity; later findings are only
/// counted in `dropped`.
#[derive(Debug)]
pub(crate) struct Diagnostics {
    profile: Profile,
    items: Vec<Diagnostic>,
    cap: usize,
    dropped: usize,
}

impl Diagnostics {
    /// An uncapped collection (used for the head, which `head_size` bounds).
    pub(crate) fn new(profile: Profile) -> Self {
        Self::with_cap(profile, usize::MAX)
    }

    /// A collection that keeps at most `cap` diagnostics.
    pub(crate) fn with_cap(profile: Profile, cap: usize) -> Self {
        Diagnostics {
            profile,
            items: Vec::new(),
            cap,
            dropped: 0,
        }
    }

    /// Records a finding. The severity is the code's default under the profile;
    /// no profile turns a diagnostic into a fatal error. Past the cap the
    /// finding is counted, not stored.
    pub(crate) fn push(&mut self, code: DiagnosticCode, span: Span) {
        let len = self.items.len();
        if len >= self.cap {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        if len == self.items.capacity() {
            // Grow by doubling, but never past the cap.
            let grow = self.items.capacity().max(4).min(self.cap - len);
            self.items.reserve_exact(grow);
        }
        let severity = code.default_severity(self.profile);
        self.items.push(Diagnostic::new(code, span, severity));
    }

    pub(crate) fn as_slice(&self) -> &[Diagnostic] {
        &self.items
    }

    /// How many findings were counted but not stored because of the cap.
    pub(crate) fn dropped(&self) -> usize {
        self.dropped
    }

    #[cfg(test)]
    pub(crate) fn items_capacity(&self) -> usize {
        self.items.capacity()
    }

    pub(crate) fn into_vec(self) -> Vec<Diagnostic> {
        self.items
    }
}
