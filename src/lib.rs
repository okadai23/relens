//! Testable application logic for `relens`.

use std::{fs, io, path::Path};

use serde::Serialize;
use thiserror::Error;

pub mod cli;
pub mod commands;
pub mod output;

/// Errors exposed by the reusable library layer.
#[derive(Debug, Error)]
pub enum RelensError {
    #[error("configuration already exists at {0}")]
    AlreadyExists(String),
    #[error("could not access configuration {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: io::Error,
    },
    #[error("invalid configuration at {path}: {source}")]
    InvalidConfiguration {
        path: String,
        #[source]
        source: toml::de::Error,
    },
}

/// A machine-serializable summary produced by a command.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct CommandResult {
    pub action: &'static str,
    pub path: String,
}

pub fn initialize(path: &Path) -> Result<CommandResult, RelensError> {
    if path.exists() {
        return Err(RelensError::AlreadyExists(path.display().to_string()));
    }
    fs::write(path, "# relens configuration\n").map_err(|source| RelensError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(CommandResult {
        action: "initialized",
        path: path.display().to_string(),
    })
}

pub fn inspect(path: &Path) -> Result<CommandResult, RelensError> {
    let contents = fs::read_to_string(path).map_err(|source| RelensError::Io {
        path: path.display().to_string(),
        source,
    })?;
    toml::from_str::<toml::Table>(&contents).map_err(|source| {
        RelensError::InvalidConfiguration {
            path: path.display().to_string(),
            source,
        }
    })?;
    Ok(CommandResult {
        action: "inspected",
        path: path.display().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_creates_a_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("relens.toml");
        let result = initialize(&path).unwrap();
        assert_eq!(result.action, "initialized");
        assert!(path.is_file());
    }

    #[test]
    fn inspect_rejects_malformed_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("relens.toml");
        fs::write(&path, "invalid = [").unwrap();

        let error = inspect(&path).unwrap_err();

        assert!(matches!(error, RelensError::InvalidConfiguration { .. }));
    }
}
