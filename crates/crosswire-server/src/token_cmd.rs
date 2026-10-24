//! `crosswire token`: mint, list, revoke and rotate a project's API tokens.
//!
//! Every command opens the project directly (taking its exclusive lock), so it
//! can only run while `crosswire serve` is not holding the same project.

use std::io::Write;
use std::path::Path;

use crosswire_store::{Project, Store, StoreConfig, TokenRecord, TokenScope};
use ulid::Ulid;

use crate::cli::{TokenCommand, TokenScopeArg};
use crate::error::CliError;

/// The actor recorded in the audit log for token commands.
const ACTOR: &str = "cli";

/// Runs a `crosswire token` subcommand.
///
/// # Errors
/// [`CliError::Config`] for a bad token id; [`CliError::Runtime`] when the
/// project cannot be opened (for example it is locked by a running `serve`),
/// written or closed.
pub fn run(cmd: TokenCommand) -> Result<(), CliError> {
    match cmd {
        TokenCommand::Create {
            project,
            name,
            scopes,
        } => create(&project, &name, &scopes),
        TokenCommand::List { project } => list(&project),
        TokenCommand::Revoke { project, id } => revoke(&project, &id),
        TokenCommand::Rotate { project, id } => rotate(&project, &id),
    }
}

/// Opens the project's store.
fn open(project: &Path) -> Result<Store, CliError> {
    let project = Project::open_or_create(project).map_err(|e| {
        CliError::runtime(
            &format!(
                "opening project {} (is `crosswire serve` running on it?)",
                project.display()
            ),
            e,
        )
    })?;
    Store::open(project, StoreConfig::default())
        .map_err(|e| CliError::runtime("opening the store", e))
}

/// Closes the store, surfacing a close failure.
fn close(store: Store) -> Result<(), CliError> {
    store
        .close()
        .map_err(|e| CliError::runtime("closing the store", e))
}

/// Maps the CLI scope enum to the store's.
fn to_store_scope(scope: TokenScopeArg) -> TokenScope {
    match scope {
        TokenScopeArg::Read => TokenScope::Read,
        TokenScopeArg::Send => TokenScope::Send,
        TokenScopeArg::Admin => TokenScope::Admin,
    }
}

/// The wire name of a stored scope.
fn scope_name(scope: &TokenScope) -> &'static str {
    match scope {
        TokenScope::Read => "read",
        TokenScope::Send => "send",
        TokenScope::Admin => "admin",
        _ => "unknown",
    }
}

/// `read,send` from a record's scopes.
fn scopes_csv(record: &TokenRecord) -> String {
    record
        .scopes
        .iter()
        .map(scope_name)
        .collect::<Vec<_>>()
        .join(",")
}

fn create(project: &Path, name: &str, scopes: &[TokenScopeArg]) -> Result<(), CliError> {
    let store = open(project)?;
    let result = (|| {
        let scopes: Vec<TokenScope> = scopes.iter().copied().map(to_store_scope).collect();
        let (record, secret) = store
            .control()
            .map_err(|e| CliError::runtime("opening a control handle", e))?
            .token_create(name, &scopes)
            .map_err(|e| CliError::runtime("creating the token", e))?;
        store
            .audit(ACTOR, "token.create", &record.id.to_string(), "ok")
            .map_err(|e| CliError::runtime("writing the audit row", e))?;
        print_created(&record, &secret)
    })();
    result.and(close(store))
}

/// Prints a created (or rotated) token and its one-time secret.
fn print_created(record: &TokenRecord, secret: &str) -> Result<(), CliError> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "id: {}", record.id)
        .and_then(|()| writeln!(out, "name: {}", record.name))
        .and_then(|()| writeln!(out, "scopes: {}", scopes_csv(record)))
        .and_then(|()| writeln!(out, "secret: {secret}"))
        .and_then(|()| writeln!(out, "(the secret is shown once and cannot be recovered)"))
        .and_then(|()| out.flush())
        .map_err(|e| CliError::runtime("writing stdout", e))
}

fn list(project: &Path) -> Result<(), CliError> {
    let store = open(project)?;
    let result = (|| {
        let tokens = store
            .control()
            .map_err(|e| CliError::runtime("opening a control handle", e))?
            .token_list()
            .map_err(|e| CliError::runtime("listing tokens", e))?;
        let mut out = std::io::stdout().lock();
        if tokens.is_empty() {
            writeln!(out, "no tokens").map_err(|e| CliError::runtime("writing stdout", e))?;
        }
        for t in &tokens {
            let revoked = if t.revoked_at.is_some() {
                "revoked"
            } else {
                "active"
            };
            writeln!(
                out,
                "{}  {}  [{}]  {}",
                t.id,
                t.name,
                scopes_csv(t),
                revoked
            )
            .map_err(|e| CliError::runtime("writing stdout", e))?;
        }
        out.flush()
            .map_err(|e| CliError::runtime("writing stdout", e))
    })();
    result.and(close(store))
}

/// Parses a token id, mapping a bad value to a config error.
fn parse_id(id: &str) -> Result<Ulid, CliError> {
    Ulid::from_string(id).map_err(|e| CliError::Config(format!("invalid token id {id:?}: {e}")))
}

fn revoke(project: &Path, id: &str) -> Result<(), CliError> {
    let id = parse_id(id)?;
    let store = open(project)?;
    let result = (|| {
        let revoked = store
            .control()
            .map_err(|e| CliError::runtime("opening a control handle", e))?
            .token_revoke(id)
            .map_err(|e| CliError::runtime("revoking the token", e))?;
        store
            .audit(
                ACTOR,
                "token.revoke",
                &id.to_string(),
                if revoked { "ok" } else { "noop" },
            )
            .map_err(|e| CliError::runtime("writing the audit row", e))?;
        let mut out = std::io::stdout().lock();
        let line = if revoked {
            format!("revoked {id}")
        } else {
            format!("no such active token {id}")
        };
        writeln!(out, "{line}")
            .and_then(|()| out.flush())
            .map_err(|e| CliError::runtime("writing stdout", e))
    })();
    result.and(close(store))
}

fn rotate(project: &Path, id: &str) -> Result<(), CliError> {
    let id = parse_id(id)?;
    let store = open(project)?;
    let result = (|| {
        let control = store
            .control()
            .map_err(|e| CliError::runtime("opening a control handle", e))?;
        // Find the token's name and scopes so the replacement matches.
        let existing = control
            .token_list()
            .map_err(|e| CliError::runtime("listing tokens", e))?
            .into_iter()
            .find(|t| t.id == id)
            .ok_or_else(|| CliError::Config(format!("no such token {id}")))?;
        let revoked = control
            .token_revoke(id)
            .map_err(|e| CliError::runtime("revoking the old token", e))?;
        store
            .audit(
                ACTOR,
                "token.revoke",
                &id.to_string(),
                if revoked { "ok" } else { "noop" },
            )
            .map_err(|e| CliError::runtime("writing the audit row", e))?;
        let (record, secret) = control
            .token_create(&existing.name, &existing.scopes)
            .map_err(|e| CliError::runtime("creating the replacement token", e))?;
        store
            .audit(ACTOR, "token.create", &record.id.to_string(), "ok")
            .map_err(|e| CliError::runtime("writing the audit row", e))?;
        print_created(&record, &secret)?;
        let mut out = std::io::stdout().lock();
        writeln!(out, "invalidated: {id}")
            .and_then(|()| out.flush())
            .map_err(|e| CliError::runtime("writing stdout", e))
    })();
    result.and(close(store))
}
