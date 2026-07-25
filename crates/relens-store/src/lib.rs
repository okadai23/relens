//! Filesystem-backed persistence used by the CLI scaffold.

use std::{fs, path::Path};

use relens_domain::{CommandResult, RelensError};

pub fn initialize(path: &Path) -> Result<CommandResult, RelensError> {
    if path.exists() {
        return Err(RelensError::AlreadyExists(path.display().to_string()));
    }
    fs::write(path, "# relens configuration\n").map_err(|source| RelensError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(CommandResult::new(
        "initialized",
        path.display().to_string(),
    ))
}

pub fn inspect(path: &Path) -> Result<CommandResult, RelensError> {
    fs::read(path).map_err(|source| RelensError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(CommandResult::new("inspected", path.display().to_string()))
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
    fn initialize_does_not_replace_an_existing_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("relens.toml");
        fs::write(&path, "keep me").unwrap();
        assert!(matches!(
            initialize(&path),
            Err(RelensError::AlreadyExists(_))
        ));
        assert_eq!(fs::read_to_string(path).unwrap(), "keep me");
    }
}
