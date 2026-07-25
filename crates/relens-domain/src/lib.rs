//! Pure domain types shared by relens use cases and adapters.

use serde::Serialize;
use thiserror::Error;

/// Errors returned by reusable relens operations.
#[derive(Debug, Error)]
pub enum RelensError {
    #[error("configuration already exists at {0}")]
    AlreadyExists(String),
    #[error("could not access configuration {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// A machine-serializable command outcome.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct CommandResult {
    pub action: &'static str,
    pub path: String,
}

impl CommandResult {
    pub fn new(action: &'static str, path: impl Into<String>) -> Self {
        Self {
            action,
            path: path.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_result_keeps_the_stable_machine_contract() {
        let result = CommandResult::new("inspected", "relens.toml");
        assert_eq!(result.action, "inspected");
        assert_eq!(result.path, "relens.toml");
    }
}
