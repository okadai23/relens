use anyhow::{Context, Result, bail};
use relens_domain::{AnswerSet, AnswerValue, CommandResult, TemplateRef};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};

use crate::cli::Command;

pub fn execute(command: Command) -> Result<CommandResult> {
    match command {
        Command::New {
            template,
            destination,
            answers,
        } => new_project(&template, &destination, &answers),
        Command::Drift { project } => drift(&project, false),
        Command::Lift { project } => drift(&project, true),
        Command::Init { path } => {
            relens_store::initialize(&path).context("failed to initialize relens")
        }
        Command::Run { path } => relens_store::inspect(&path).context("failed to run relens"),
    }
}

fn new_project(
    template: &Path,
    destination: &Path,
    raw_answers: &[String],
) -> Result<CommandResult> {
    let questionnaire =
        relens_store::load_questionnaire(template).context("failed to load questionnaire")?;
    let supplied = parse_answers(raw_answers)?;
    let answers = questionnaire
        .validate(&supplied)
        .context("invalid answers")?;
    ensure_clean_template(template)?;
    let revision = git_revision(template).context("template repository has no HEAD commit")?;
    let reference = TemplateRef::new(template.display().to_string(), revision)
        .context("invalid template reference")?;
    fs::create_dir_all(destination)
        .with_context(|| format!("failed to create {}", destination.display()))?;
    let mut rendered = BTreeMap::new();
    for relative in relens_store::template_files(template)? {
        let source = fs::read_to_string(template.join(&relative))
            .with_context(|| format!("template {} is not UTF-8", relative.display()))?;
        let path_template = relens_store::portable_path(&relative);
        let output_path =
            relens_engine::render(&path_template, &answers).context("failed to render path")?;
        let output_path =
            String::from_utf8(output_path.bytes).context("rendered path is not UTF-8")?;
        let output_path = output_path
            .strip_suffix(".j2")
            .unwrap_or(&output_path)
            .replace('\\', "/");
        let output_path = safe_relative_path(&output_path)?;
        let output = relens_engine::render(&source, &answers)
            .with_context(|| format!("failed to render {}", relative.display()))?;
        if output.bytes.is_empty() {
            continue;
        }
        let target = prepare_output_path(destination, &output_path)?;
        fs::write(&target, &output.bytes)?;
        rendered.insert(
            relens_store::portable_path(&output_path),
            (output.bytes, output.source_map),
        );
    }
    relens_store::persist(
        destination,
        &AnswerSet {
            template: reference,
            answers,
        },
        &rendered,
    )
    .context("failed to persist project metadata")?;
    Ok(CommandResult::new(
        "generated",
        destination.display().to_string(),
    ))
}
fn parse_answers(raw: &[String]) -> Result<BTreeMap<String, AnswerValue>> {
    let mut values = BTreeMap::new();
    for item in raw {
        let (name, value) = item
            .split_once('=')
            .with_context(|| format!("answer must be NAME=VALUE: `{item}`"))?;
        let value = match value {
            "true" => AnswerValue::Bool(true),
            "false" => AnswerValue::Bool(false),
            v if v.parse::<i64>().is_ok() => AnswerValue::Integer(v.parse().unwrap()),
            v => AnswerValue::String(v.into()),
        };
        values.insert(name.into(), value);
    }
    Ok(values)
}
fn git_revision(template: &Path) -> Option<String> {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(template)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().into())
}

fn ensure_clean_template(template: &Path) -> Result<()> {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(template)
        .output()
        .context("failed to inspect template repository")?;
    if !output.status.success() {
        bail!("template must be a Git repository with a committed HEAD");
    }
    if !output.stdout.is_empty() {
        bail!("template repository has uncommitted or untracked files");
    }
    Ok(())
}

fn safe_relative_path(rendered: &str) -> Result<PathBuf> {
    let path = Path::new(rendered);
    let windows_drive = rendered
        .as_bytes()
        .get(1)
        .is_some_and(|separator| *separator == b':');
    if rendered.is_empty() || path.is_absolute() || windows_drive {
        bail!("rendered path must be a non-empty relative path: `{rendered}`");
    }
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("rendered path escapes the destination: `{rendered}`");
            }
        }
    }
    if safe.as_os_str().is_empty() {
        bail!("rendered path must name a file: `{rendered}`");
    }
    Ok(safe)
}

/// Creates missing parent directories without ever traversing a symlink already
/// present below the destination. The final component is checked too because
/// `fs::write` would otherwise follow a symlink in place of the output file.
fn prepare_output_path(destination: &Path, relative: &Path) -> Result<PathBuf> {
    let mut target = destination.to_path_buf();
    let component_count = relative.components().count();

    for (index, component) in relative.components().enumerate() {
        let Component::Normal(component) = component else {
            bail!("output path is not a normalized relative path");
        };
        target.push(component);
        match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("refusing to write through symlink `{}`", target.display());
            }
            Ok(metadata) if index + 1 < component_count && !metadata.is_dir() => {
                bail!("output parent is not a directory: `{}`", target.display());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if index + 1 < component_count {
                    fs::create_dir(&target).with_context(|| {
                        format!("failed to create output directory {}", target.display())
                    })?;
                }
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to inspect {}", target.display()));
            }
        }
    }

    Ok(target)
}

fn drift(project: &Path, lift: bool) -> Result<CommandResult> {
    let changed = relens_store::drift(project).context("failed to inspect drift")?;
    if changed.is_empty() {
        Ok(CommandResult::new(
            if lift { "no-patch" } else { "clean" },
            project.display().to_string(),
        ))
    } else if lift {
        bail!("automatic lifting of non-empty drift is not available in M1")
    } else {
        Ok(CommandResult::new("drift", changed.join(",")))
    }
}

#[cfg(test)]
mod tests {
    use super::{prepare_output_path, safe_relative_path};
    use std::{fs, path::PathBuf};

    #[test]
    fn accepts_only_paths_confined_to_the_destination() {
        assert_eq!(
            safe_relative_path("src/./main.rs").unwrap(),
            PathBuf::from("src/main.rs")
        );
        for unsafe_path in ["", "/tmp/file", "../file", "src/../../file", "C:/file"] {
            assert!(safe_relative_path(unsafe_path).is_err(), "{unsafe_path}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_below_the_destination() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("link")).unwrap();

        let error = prepare_output_path(root.path(), &PathBuf::from("link/file")).unwrap_err();
        assert!(error.to_string().contains("symlink"));
        assert!(!outside.path().join("file").exists());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symlink_in_place_of_the_output_file() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = root.path().join("outside");
        fs::write(&outside, "unchanged").unwrap();
        symlink(&outside, root.path().join("output")).unwrap();

        assert!(prepare_output_path(root.path(), &PathBuf::from("output")).is_err());
        assert_eq!(fs::read_to_string(outside).unwrap(), "unchanged");
    }
}
