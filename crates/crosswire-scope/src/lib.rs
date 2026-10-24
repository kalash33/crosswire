//! Scope rules: which hosts, ports and paths Crosswire is allowed to touch.
//!
//! A [`Scope`] is an ordered list of include [`Rule`]s and exclude [`Rule`]s.
//! [`Scope::compile`] builds every regex once into a [`Compiled`] matcher, and
//! [`Compiled::evaluate`] returns a [`Verdict`] for a single request identity
//! ([`Input`]).
//!
//! # Precedence (spec §4.7)
//!
//! Include rules are tried first, then exclude rules. A matching exclude wins
//! over any matching include. With no include match a request is
//! [`Verdict::Out`]; with no rules at all the scope is [`Verdict::Empty`].
//!
//! # Two-identity evaluation
//!
//! [`Compiled::evaluate_pair`] tests a connection target and a message (Host
//! header) identity together and returns a [`PairVerdict`]; when the two
//! single verdicts disagree it is [`PairVerdict::HostMismatch`].
//!
//! # Importing
//!
//! [`parse_program_list`] turns a pasted scope list (bare hosts, `*.`
//! wildcards, full URLs, CIDR blocks and IP literals) into a [`Scope`].

mod import;
mod input;
mod rule;
mod scope;

pub use import::{parse_program_list, ImportError};
pub use input::{Input, Scheme};
pub use rule::{HostPattern, PathMatch, Rule, ScopeError};
pub use scope::{Compiled, PairVerdict, Scope, Verdict};
