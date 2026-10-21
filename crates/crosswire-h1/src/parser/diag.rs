//! Collects diagnostics, assigning each its severity from the active profile.

use alloc::vec::Vec;

use crosswire_core::{Diagnostic, DiagnosticCode, Profile, Span};

/// An ordered collection of diagnostics for one message.
#[derive(Debug)]
pub(crate) struct Diagnostics {
    profile: Profile,
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub(crate) fn new(profile: Profile) -> Self {
        Diagnostics {
            profile,
            items: Vec::new(),
        }
    }

    /// Records a finding. The severity is the code's default under the profile;
    /// no profile turns a diagnostic into a fatal error.
    pub(crate) fn push(&mut self, code: DiagnosticCode, span: Span) {
        let severity = code.default_severity(self.profile);
        self.items.push(Diagnostic::new(code, span, severity));
    }

    pub(crate) fn into_vec(self) -> Vec<Diagnostic> {
        self.items
    }
}
