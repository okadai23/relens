//! Git-backed template source adapter.

pub use relens_domain as domain;
use relens_domain::{TemplateRef, TemplateSource, TemplateTree};
use std::{path::Path, process::Command};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("git command failed in {repository}: {message}")]
    Command { repository: String, message: String },
    #[error("invalid UTF-8 path returned by git")]
    Path,
    #[error(transparent)]
    Reference(#[from] relens_domain::RelensError),
}

#[derive(Debug, Default)]
pub struct GitTemplateSource;

impl GitTemplateSource {
    fn git(repository: &Path, args: &[&str]) -> Result<Vec<u8>, GitError> {
        let output = Command::new("git")
            .args(args)
            .current_dir(repository)
            .output()
            .map_err(|error| GitError::Command {
                repository: repository.display().to_string(),
                message: error.to_string(),
            })?;
        if !output.status.success() {
            return Err(GitError::Command {
                repository: repository.display().to_string(),
                message: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }
        Ok(output.stdout)
    }
}

impl TemplateSource for GitTemplateSource {
    type Error = GitError;
    fn fetch(&self, reference: &TemplateRef) -> Result<TemplateTree, Self::Error> {
        let repository = Path::new(&reference.locator);
        let listing = Self::git(
            repository,
            &["ls-tree", "-r", "--name-only", "-z", &reference.revision],
        )?;
        let mut tree = TemplateTree::new();
        for raw in listing
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            let path = std::str::from_utf8(raw).map_err(|_| GitError::Path)?;
            let object = format!("{}:{path}", reference.revision);
            tree.insert(
                path.replace('\\', "/"),
                Self::git(repository, &["show", &object])?,
            );
        }
        Ok(tree)
    }
    fn latest(&self, locator: &str) -> Result<TemplateRef, Self::Error> {
        let revision = Self::git(Path::new(locator), &["rev-parse", "HEAD"])?;
        TemplateRef::new(locator, String::from_utf8_lossy(&revision).trim()).map_err(Into::into)
    }
}
