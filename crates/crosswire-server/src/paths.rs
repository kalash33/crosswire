//! Default locations.

use std::path::PathBuf;

use crate::error::CliError;

/// The default CA directory: `<config dir>/crosswire/ca` (for example
/// `~/.config/crosswire/ca` on Linux).
///
/// # Errors
/// [`CliError::Config`] when the platform has no config directory (no home
/// directory, typically); pass `--ca-dir` then.
pub fn default_ca_dir() -> Result<PathBuf, CliError> {
    dirs::config_dir()
        .map(|d| d.join("crosswire").join("ca"))
        .ok_or_else(|| {
            CliError::Config(
                "no config directory found for the default CA location; pass --ca-dir".into(),
            )
        })
}
