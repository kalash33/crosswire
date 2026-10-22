//! Persistence of the CA key pair.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::CaError;

/// Storage for the CA certificate and private key (both PEM).
pub trait KeyStore {
    /// Load `(cert_pem, key_pem)`, or `None` when nothing has been saved.
    fn load(&self) -> Result<Option<(String, String)>, CaError>;
    /// Persist the certificate and key, replacing any existing pair.
    fn save(&self, cert_pem: &str, key_pem: &str) -> Result<(), CaError>;
}

/// A key store of two files, `ca.crt` (0644) and `ca.key` (0600), in a 0700
/// directory. On Windows no mode bits are applied.
///
/// `save` writes `ca.crt` first, then `ca.key`. A crash between the two leaves
/// an orphan `ca.crt`, which `load` reports as an error; recover by deleting
/// the orphan and regenerating.
#[derive(Debug, Clone)]
pub struct FileKeyStore {
    dir: PathBuf,
}

impl FileKeyStore {
    /// A store rooted at `dir` (created on first save).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        FileKeyStore { dir: dir.into() }
    }

    /// The directory holding the files.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

fn read_opt(path: &Path) -> Result<Option<String>, CaError> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

impl KeyStore for FileKeyStore {
    fn load(&self) -> Result<Option<(String, String)>, CaError> {
        let cert = read_opt(&self.dir.join("ca.crt"))?;
        let key = read_opt(&self.dir.join("ca.key"))?;
        match (cert, key) {
            (None, None) => Ok(None),
            (Some(c), Some(k)) => Ok(Some((c, k))),
            _ => Err(CaError::Pem("ca.crt and ca.key must both exist".into())),
        }
    }

    fn save(&self, cert_pem: &str, key_pem: &str) -> Result<(), CaError> {
        make_dir(&self.dir)?;
        write_file(&self.dir.join("ca.crt"), cert_pem, 0o644)?;
        write_file(&self.dir.join("ca.key"), key_pem, 0o600)?;
        Ok(())
    }
}

#[cfg(unix)]
fn make_dir(dir: &Path) -> Result<(), CaError> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_dir(dir: &Path) -> Result<(), CaError> {
    fs::create_dir_all(dir)?;
    Ok(())
}

/// Write via a temp file created with its final mode, then rename, so the
/// secret is never visible with looser permissions.
fn write_file(path: &Path, data: &str, mode: u32) -> Result<(), CaError> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    let tmp = path.with_file_name(name);
    let _ = fs::remove_file(&tmp);
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut f = opts.open(&tmp)?;
    f.write_all(data.as_bytes())?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, path)?;
    Ok(())
}
