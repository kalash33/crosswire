//! `crosswire ca`: export the CA certificate and its pin.

use std::io::Write;
use std::path::{Path, PathBuf};

use crosswire_ca::{Ca, FileKeyStore};

use crate::cli::{CaCommand, CertFormat};
use crate::error::CliError;
use crate::paths::default_ca_dir;

/// The subject suffix of a generated CA.
pub const CA_SUBJECT_SUFFIX: &str = "Crosswire CA";

/// A CA loaded from (or generated into) a directory.
pub struct LoadedCa {
    /// The CA.
    pub ca: Ca,
    /// Whether this call generated it.
    pub generated: bool,
}

/// Loads the CA in `dir`, generating and saving one when it holds none.
///
/// # Errors
/// [`CliError::Runtime`] if the files cannot be read, written or parsed.
pub fn load_or_generate(dir: &Path) -> Result<LoadedCa, CliError> {
    let store = FileKeyStore::new(dir);
    let existed = ["ca.crt", "ca.key"]
        .iter()
        .any(|f| store.dir().join(f).exists());
    let ca = Ca::load_or_generate(&store, CA_SUBJECT_SUFFIX)
        .map_err(|e| CliError::runtime(&format!("CA in {}", dir.display()), e))?;
    Ok(LoadedCa {
        ca,
        generated: !existed,
    })
}

/// `--ca-dir`, or the default.
pub fn resolve_ca_dir(ca_dir: Option<PathBuf>) -> Result<PathBuf, CliError> {
    match ca_dir {
        Some(d) => Ok(d),
        None => default_ca_dir(),
    }
}

/// Runs a `crosswire ca` subcommand.
///
/// # Errors
/// [`CliError::Config`] when no CA directory can be determined;
/// [`CliError::Runtime`] for CA or output failures.
pub fn run(cmd: CaCommand) -> Result<(), CliError> {
    match cmd {
        CaCommand::Export {
            format,
            out,
            ca_dir,
        } => {
            let loaded = load_or_generate(&resolve_ca_dir(ca_dir)?)?;
            let bytes = match format {
                CertFormat::Der => loaded.ca.cert_der(),
                CertFormat::Pem => loaded.ca.cert_pem().into_bytes(),
            };
            match out {
                Some(path) => std::fs::write(&path, &bytes)
                    .map_err(|e| CliError::runtime(&format!("writing {}", path.display()), e)),
                None => write_stdout(&bytes),
            }
        }
        CaCommand::Spki { ca_dir } => {
            let loaded = load_or_generate(&resolve_ca_dir(ca_dir)?)?;
            write_stdout(format!("{}\n", loaded.ca.spki_sha256_base64()).as_bytes())
        }
        CaCommand::Path => {
            let dir = default_ca_dir()?;
            write_stdout(format!("{}\n", dir.display()).as_bytes())
        }
    }
}

fn write_stdout(bytes: &[u8]) -> Result<(), CliError> {
    let mut out = std::io::stdout().lock();
    out.write_all(bytes)
        .and_then(|()| out.flush())
        .map_err(|e| CliError::runtime("writing stdout", e))
}
