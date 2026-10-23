//! The binary's error type and its exit codes.

/// Why a command failed.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// Invalid arguments or configuration; exit code 2.
    #[error("{0}")]
    Config(String),
    /// A failure while running; exit code 1.
    #[error("{0}")]
    Runtime(String),
}

impl CliError {
    /// The process exit code for this error.
    pub fn exit_code(&self) -> u8 {
        match self {
            CliError::Config(_) => 2,
            CliError::Runtime(_) => 1,
        }
    }

    /// A runtime error with context.
    pub fn runtime(context: &str, err: impl std::fmt::Display) -> CliError {
        CliError::Runtime(format!("{context}: {err}"))
    }
}
