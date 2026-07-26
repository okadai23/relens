//! Git-backed template source adapter.

pub use relens_domain as domain;
use relens_domain::{LiftSession, ReviewDecision, TemplateRef, TemplateSource, TemplateTree};
use std::{
    fs,
    path::{Component, Path},
    process::Command,
};
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

pub fn read_worktree(repository: &Path) -> Result<TemplateTree, GitError> {
    let mut tree = TemplateTree::new();
    for entry in walkdir::WalkDir::new(repository)
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".git")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let relative = entry
            .path()
            .strip_prefix(repository)
            .map_err(|_| GitError::Path)?;
        let path = relative.to_str().ok_or(GitError::Path)?.replace('\\', "/");
        tree.insert(
            path,
            fs::read(entry.path()).map_err(|error| GitError::Command {
                repository: repository.display().to_string(),
                message: error.to_string(),
            })?,
        );
    }
    Ok(tree)
}

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

/// Applies a verified session on a dedicated branch and commits it.
pub fn export_lift(session: &LiftSession) -> Result<String, GitError> {
    let repository = Path::new(&session.template.locator);
    if !GitTemplateSource::git(
        repository,
        &["status", "--porcelain", "--untracked-files=all"],
    )?
    .is_empty()
    {
        return Err(GitError::Command {
            repository: repository.display().to_string(),
            message: "template repository has uncommitted or untracked files".into(),
        });
    }
    let project = Path::new(&session.project)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project");
    let branch = format!("lift/{project}-{}", session.id);
    GitTemplateSource::git(
        repository,
        &["checkout", "-b", &branch, &session.template.revision],
    )?;
    let mut written = Vec::new();
    for edit in &session.edits {
        let Some(relative) = &edit.template_path else {
            continue;
        };
        let path = Path::new(relative);
        if path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(GitError::Path);
        }
        let content = if edit.decision == ReviewDecision::Substitute {
            edit.substituted.as_deref().unwrap_or(&edit.literal)
        } else {
            &edit.literal
        };
        let target = repository.join(path);
        if edit.deleted {
            fs::remove_file(&target).map_err(|error| GitError::Command {
                repository: repository.display().to_string(),
                message: error.to_string(),
            })?;
            written.push(relative.as_str());
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| GitError::Command {
                repository: repository.display().to_string(),
                message: error.to_string(),
            })?;
        }
        fs::write(&target, content).map_err(|error| GitError::Command {
            repository: repository.display().to_string(),
            message: error.to_string(),
        })?;
        written.push(relative.as_str());
    }
    for path in written {
        GitTemplateSource::git(repository, &["add", "--", path])?;
    }
    let message = format!(
        "relens lift from {}\n\nSource-Commit: {}",
        session.project, session.template.revision
    );
    GitTemplateSource::git(
        repository,
        &[
            "-c",
            "user.name=Relens",
            "-c",
            "user.email=relens@example.invalid",
            "commit",
            "-m",
            &message,
        ],
    )?;
    Ok(branch)
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

#[cfg(test)]
mod tests {
    use super::*;
    use relens_domain::{LiftSessionState, SessionEdit};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn export_rejects_unrelated_worktree_changes() {
        let repository = std::env::temp_dir().join(format!(
            "relens-vcs-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&repository).unwrap();
        GitTemplateSource::git(&repository, &["init"]).unwrap();
        fs::write(repository.join("template.txt"), "old").unwrap();
        GitTemplateSource::git(&repository, &["add", "template.txt"]).unwrap();
        GitTemplateSource::git(
            &repository,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-m",
                "initial",
            ],
        )
        .unwrap();
        let revision =
            String::from_utf8(GitTemplateSource::git(&repository, &["rev-parse", "HEAD"]).unwrap())
                .unwrap()
                .trim()
                .to_string();
        fs::write(repository.join("private.txt"), "do not commit").unwrap();
        let session = LiftSession {
            id: "session".into(),
            project: "project".into(),
            template: TemplateRef::new(repository.to_string_lossy(), revision).unwrap(),
            state: LiftSessionState::Verified,
            edits: vec![SessionEdit {
                project_path: "template.txt".into(),
                template_path: Some("template.txt".into()),
                literal: "new".into(),
                substituted: None,
                decision: ReviewDecision::Automatic,
                deleted: false,
            }],
            divergences: vec![],
        };

        let error = export_lift(&session).unwrap_err().to_string();
        assert!(error.contains("uncommitted or untracked"));
        assert!(
            GitTemplateSource::git(&repository, &["branch", "--list", "lift/*"])
                .unwrap()
                .is_empty()
        );
        fs::remove_dir_all(repository).unwrap();
    }
}
