//! Core types shared by Crosswire crates.
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod change;
mod diagnostic;
mod hint;
mod profile;
mod span;

pub use change::{Change, ChangeOp, ChangeReason};
pub use diagnostic::{Diagnostic, DiagnosticCode, Severity};
pub use hint::{EncodingHint, FieldHint};
pub use profile::Profile;
pub use span::Span;
