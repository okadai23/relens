use anyhow::{Context, Result, bail};
use relens_domain::{
    AnswerSet, AnswerValue, CommandResult, TemplateRef, TemplateSource, TemplateTree,
};
use std::{
    collections::BTreeMap,
    fs, io,
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
        Command::Lift {
            project,
            resume,
            decisions,
            export,
        } => lift(&project, resume, export, &decisions),
        Command::Update { project } => update(&project),
        Command::Init { path } => {
            relens_store::initialize(&path).context("failed to initialize relens")
        }
        Command::Run { path } => relens_store::inspect(&path).context("failed to run relens"),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("update has conflicts")]
pub struct UpdateConflict {
    pub files: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("lift verification failed")]
pub struct ExportVerification {
    pub locations: String,
}

fn render_tree(
    tree: &TemplateTree,
    answers: &BTreeMap<String, AnswerValue>,
) -> Result<BTreeMap<String, (Vec<u8>, relens_domain::SourceMap)>> {
    let mut rendered = BTreeMap::new();
    for (path, bytes) in tree {
        if path == "relens.toml" {
            continue;
        }
        let source =
            std::str::from_utf8(bytes).with_context(|| format!("template {path} is not UTF-8"))?;
        let output_path = relens_engine::render(path, answers).context("failed to render path")?;
        let output_path =
            String::from_utf8(output_path.bytes).context("rendered path is not UTF-8")?;
        let output_path = output_path
            .strip_suffix(".j2")
            .unwrap_or(&output_path)
            .replace('\\', "/");
        let output_path = relens_store::portable_path(&safe_relative_path(&output_path)?);
        let output = relens_engine::render(source, answers)
            .with_context(|| format!("failed to render {path}"))?;
        if !output.bytes.is_empty() {
            rendered.insert(output_path, (output.bytes, output.source_map));
        }
    }
    Ok(rendered)
}

fn update(project: &Path) -> Result<CommandResult> {
    let mut answers =
        relens_store::load_answers(project).context("failed to load project answers")?;
    let source = relens_vcs::GitTemplateSource;
    let old_tree = source
        .fetch(&answers.template)
        .context("failed to fetch recorded template")?;
    let latest = source
        .latest(&answers.template.locator)
        .context("failed to resolve latest template")?;
    let new_tree = source
        .fetch(&latest)
        .context("failed to fetch latest template")?;
    let base = render_tree(&old_tree, &answers.answers)?;
    let updated = render_tree(&new_tree, &answers.answers)?;
    let paths = base
        .keys()
        .chain(updated.keys())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    // Check every target before changing any files so a malicious tree cannot
    // leave the project partially updated before an unsafe path is discovered.
    for path in &paths {
        reject_symlinked_path(project, Path::new(path))?;
    }
    let mut merged_metadata = BTreeMap::new();
    let mut conflicts = Vec::new();
    for path in paths {
        let old = base
            .get(&path)
            .map(|value| value.0.as_slice())
            .unwrap_or_default();
        let new = updated
            .get(&path)
            .map(|value| value.0.as_slice())
            .unwrap_or_default();
        let project_path = project.join(&path);
        let local = match fs::read(&project_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", project_path.display()));
            }
        };
        let result = relens_engine::three_way_merge(old, &local, new);
        let (bytes, conflict) = match result {
            relens_engine::MergeResult::Merged(bytes) => (bytes, false),
            relens_engine::MergeResult::Conflict(bytes) => (bytes, true),
        };
        if let Some(parent) = project.join(&path).parent() {
            fs::create_dir_all(parent)?;
        }
        if new.is_empty() && !conflict && bytes.is_empty() {
            let _ = fs::remove_file(project.join(&path));
        } else {
            fs::write(project.join(&path), &bytes)?;
        }
        if conflict {
            conflicts.push(path.clone());
        }
        if let Some((rendered, map)) = updated.get(&path) {
            merged_metadata.insert(path, (rendered.clone(), map.clone()));
        }
    }
    if !conflicts.is_empty() {
        return Err(UpdateConflict { files: conflicts }.into());
    }
    answers.template = latest;
    relens_store::persist(project, &answers, &merged_metadata)
        .context("failed to persist updated metadata")?;
    Ok(CommandResult::new("updated", project.display().to_string()))
}

fn reject_symlinked_path(project: &Path, relative: &Path) -> Result<()> {
    let mut candidate = project.to_path_buf();
    for component in relative.components() {
        candidate.push(component);
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!(
                    "refusing to access through symbolic link: {}",
                    candidate.display()
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to inspect {}", candidate.display()));
            }
        }
    }
    Ok(())
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
        let target = destination.join(&output_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
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

fn drift(project: &Path, lift: bool) -> Result<CommandResult> {
    let changed = relens_store::drift(project).context("failed to inspect drift")?;
    if changed.is_empty() {
        Ok(CommandResult::new(
            if lift { "no-patch" } else { "clean" },
            project.display().to_string(),
        ))
    } else if lift {
        unreachable!("lift uses the dedicated command path")
    } else {
        Ok(CommandResult::new("drift", changed.join(",")))
    }
}

fn lift(
    project: &Path,
    resume: bool,
    export: bool,
    decisions: &[crate::cli::ReviewResolution],
) -> Result<CommandResult> {
    reject_unsafe_lift_paths(project)?;
    if !decisions.is_empty() && !resume {
        bail!("--decision requires --resume");
    }
    if resume || export {
        return continue_lift(project, export, decisions);
    }
    let changed = relens_store::drift(project)
        .context("failed to inspect drift")?
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    if changed.is_empty() {
        return Ok(CommandResult::new(
            "no-patch",
            project.display().to_string(),
        ));
    }
    let lock = relens_store::load_lock(project).context("failed to load source maps")?;
    let answer_set = relens_store::load_answers(project).context("failed to load answers")?;
    let source = relens_vcs::GitTemplateSource;
    let tree = source
        .fetch(&answer_set.template)
        .context("failed to fetch recorded template")?;
    let rendered = render_tree(&tree, &answer_set.answers)?;
    let mut templates = BTreeMap::new();
    for (template_path, bytes) in &tree {
        if template_path == "relens.toml" {
            continue;
        }
        let rendered_path = relens_engine::render(template_path, &answer_set.answers)
            .with_context(|| format!("failed to render template path {template_path}"))?;
        let rendered_path = String::from_utf8(rendered_path.bytes)
            .context("rendered template path is not UTF-8")?;
        let rendered_path = rendered_path.strip_suffix(".j2").unwrap_or(&rendered_path);
        let portable = relens_store::portable_path(&safe_relative_path(rendered_path)?);
        if let Some(locked) = lock.files.get(&portable) {
            templates.insert(
                portable,
                (
                    template_path.clone(),
                    String::from_utf8(bytes.clone())
                        .with_context(|| format!("template {template_path} is not UTF-8"))?,
                    locked.source_map.clone(),
                ),
            );
        }
    }
    let mut project_files = BTreeMap::new();
    for path in &changed {
        if let Ok(bytes) = fs::read(project.join(path)) {
            project_files.insert(path.clone(), bytes);
        }
    }
    // Include pristine mapped files for consistent lookup and future multi-file verification.
    for (path, (bytes, _)) in rendered {
        project_files.entry(path).or_insert(bytes);
    }
    let result = relens_lift::lift(&changed, &templates, &project_files, &answer_set.answers)
        .context("failed to lift drift")?;
    let id = format!(
        "{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let mut session = relens_domain::LiftSession {
        id: id.clone(),
        project: answer_set
            .answers
            .get("project_name")
            .map(AnswerValue::display)
            .unwrap_or_else(|| project.display().to_string()),
        template: answer_set.template.clone(),
        state: relens_domain::LiftSessionState::Reviewing,
        edits: result
            .files
            .iter()
            .map(|file| {
                let (literal, substituted, decision) = match &file.classification {
                    relens_lift::Classification::Auto => (
                        file.content.clone().unwrap_or_default(),
                        None,
                        relens_domain::ReviewDecision::Automatic,
                    ),
                    relens_lift::Classification::Ambiguous {
                        literal,
                        substituted,
                    } => (
                        literal.clone(),
                        Some(substituted.clone()),
                        relens_domain::ReviewDecision::Pending,
                    ),
                    relens_lift::Classification::Unmappable { .. } => (
                        String::new(),
                        None,
                        relens_domain::ReviewDecision::Unmappable,
                    ),
                };
                relens_domain::SessionEdit {
                    project_path: file.project_path.clone(),
                    template_path: file.template_path.clone(),
                    literal,
                    substituted,
                    decision,
                }
            })
            .collect(),
        divergences: match &result.verification {
            relens_lift::Verification::Pass => vec![],
            relens_lift::Verification::Fail(items) => items
                .iter()
                .map(|item| relens_domain::SessionDivergence {
                    path: item.path.clone(),
                    start: 0,
                    end: item.expected.len().max(item.actual.len()),
                })
                .collect(),
        },
    };
    if session.divergences.is_empty()
        && !session
            .edits
            .iter()
            .any(|edit| edit.decision == relens_domain::ReviewDecision::Pending)
    {
        session
            .verify(vec![])
            .context("failed to verify lift session")?;
    }
    relens_store::save_session(project, &session).context("failed to persist lift session")?;
    let patch_path = project.join(".relens/template.patch");
    let mut patch = String::new();
    let mut reports = Vec::new();
    for file in &result.files {
        match (&file.classification, &file.template_path, &file.content) {
            (relens_lift::Classification::Auto, Some(path), Some(content)) => {
                patch.push_str(&format!(
                    "--- a/{path}\n+++ b/{path}\n@@ replacement @@\n{content}"
                ));
                if !content.ends_with('\n') {
                    patch.push('\n');
                }
                reports.push(format!("{}:Auto", file.project_path));
            }
            (
                relens_lift::Classification::Ambiguous {
                    literal,
                    substituted,
                },
                _,
                _,
            ) => {
                reports.push(format!(
                    "{}:Ambiguous (candidates: {:?}, {:?})",
                    file.project_path, substituted, literal
                ));
            }
            (relens_lift::Classification::Unmappable { suggestion }, _, _) => {
                reports.push(format!("{}:Unmappable ({suggestion})", file.project_path));
            }
            _ => {}
        }
    }
    if !patch.is_empty() {
        fs::write(&patch_path, patch).context("failed to write template patch")?;
    }
    reports.push(format!("session:{id}"));
    reports.push(format!("state:{:?}", session.state));
    if session.state == relens_domain::LiftSessionState::Verified {
        reports.push("verification:Pass".into());
    }
    Ok(CommandResult::new("lifted", reports.join(", ")))
}

fn reject_unsafe_lift_paths(project: &Path) -> Result<()> {
    for metadata_path in [
        Path::new(".relens/answers.toml"),
        Path::new(".relens/lock.json"),
        Path::new(".relens/template.patch"),
        Path::new(".relens/sessions"),
    ] {
        reject_symlinked_path(project, metadata_path)?;
    }

    let lock = relens_store::load_lock(project).context("failed to load source maps")?;
    for path in lock.files.keys() {
        let path = safe_relative_path(path)
            .with_context(|| format!("unsafe path in project lock: {path}"))?;
        reject_symlinked_path(project, &path)?;
    }

    let sessions = project.join(".relens/sessions");
    if sessions.is_dir() {
        for entry in fs::read_dir(&sessions).context("failed to inspect lift sessions")? {
            let entry = entry.context("failed to inspect lift session")?;
            let relative = entry
                .path()
                .strip_prefix(project)
                .context("lift session escaped project")?
                .to_path_buf();
            reject_symlinked_path(project, &relative)?;
        }
    }
    Ok(())
}

fn continue_lift(
    project: &Path,
    export: bool,
    decisions: &[crate::cli::ReviewResolution],
) -> Result<CommandResult> {
    let mut session =
        relens_store::load_session(project, None).context("failed to load lift session")?;
    for edit in &session.edits {
        let path = safe_relative_path(&edit.project_path).with_context(|| {
            format!("unsafe project path in lift session: {}", edit.project_path)
        })?;
        reject_symlinked_path(project, &path)?;
    }
    if export {
        if !decisions.is_empty() {
            bail!("--decision can only be used with --resume");
        }
        if !session.divergences.is_empty() {
            let locations = session
                .divergences
                .iter()
                .map(|d| format!("{}:{}..{}", d.path, d.start, d.end))
                .collect::<Vec<_>>()
                .join(",");
            return Err(ExportVerification { locations }.into());
        }
        session.export().context("cannot export lift session")?;
        let branch = relens_vcs::export_lift(&session).context("failed to export lift session")?;
        relens_store::save_session(project, &session)?;
        return Ok(CommandResult::new("exported", branch));
    }
    for resolution in decisions {
        let decision = match resolution.choice {
            crate::cli::ReviewChoice::KeepLiteral => relens_domain::ReviewDecision::KeepLiteral,
            crate::cli::ReviewChoice::Substitute => relens_domain::ReviewDecision::Substitute,
        };
        session.resolve(resolution.edit, decision)?;
    }
    let answers = relens_store::load_answers(project).context("failed to load answers")?;
    let project_files = session
        .edits
        .iter()
        .filter_map(|edit| {
            fs::read(project.join(&edit.project_path))
                .ok()
                .map(|bytes| (edit.project_path.clone(), bytes))
        })
        .collect();
    let divergences = relens_lift::verify_session(&session, &project_files, &answers.answers)
        .context("failed to verify reviewed lift")?
        .into_iter()
        .map(|item| relens_domain::SessionDivergence {
            path: item.path,
            start: 0,
            end: item.expected.len().max(item.actual.len()),
        })
        .collect();
    session.verify(divergences)?;
    write_session_patch(project, &session)?;
    relens_store::save_session(project, &session)?;
    Ok(CommandResult::new(
        "resumed",
        format!("session:{}, state:{:?}", session.id, session.state),
    ))
}

fn write_session_patch(project: &Path, session: &relens_domain::LiftSession) -> Result<()> {
    let mut patch = String::new();
    for edit in &session.edits {
        let Some(path) = &edit.template_path else {
            continue;
        };
        let content = if edit.decision == relens_domain::ReviewDecision::Substitute {
            edit.substituted.as_deref().unwrap_or(&edit.literal)
        } else {
            &edit.literal
        };
        patch.push_str(&format!(
            "--- a/{path}\n+++ b/{path}\n@@ replacement @@\n{content}"
        ));
        if !content.ends_with('\n') {
            patch.push('\n');
        }
    }
    fs::write(project.join(".relens/template.patch"), patch)
        .context("failed to write template patch")
}

#[cfg(test)]
mod tests {
    use super::{reject_symlinked_path, render_tree, safe_relative_path};
    use relens_domain::{AnswerValue, TemplateTree};
    use std::{
        collections::BTreeMap,
        path::{Path, PathBuf},
    };

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

    fn answers(name: &str) -> BTreeMap<String, AnswerValue> {
        BTreeMap::from([("name".to_string(), AnswerValue::String(name.into()))])
    }

    #[test]
    fn render_tree_normalizes_paths_confined_to_the_project() {
        let tree = TemplateTree::from([("src/./{{ name }}.py.j2".to_string(), b"pass".to_vec())]);
        let rendered = render_tree(&tree, &answers("main")).unwrap();
        assert_eq!(rendered.keys().collect::<Vec<_>>(), ["src/main.py"]);
    }

    #[test]
    fn render_tree_rejects_paths_escaping_the_project() {
        let tree = TemplateTree::from([("{{ name }}/file.txt.j2".to_string(), b"data".to_vec())]);
        for escaping in ["../..", "/tmp"] {
            assert!(
                render_tree(&tree, &answers(escaping)).is_err(),
                "{escaping}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_in_project_paths() {
        use std::{fs, os::unix::fs::symlink};

        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let outside = root.path().join("outside");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, project.join("output")).unwrap();
        fs::write(outside.join("victim"), "unchanged").unwrap();
        symlink(outside.join("victim"), project.join("patch")).unwrap();

        assert!(
            reject_symlinked_path(&project, PathBuf::from("output/file.txt").as_path()).is_err()
        );
        assert!(reject_symlinked_path(&project, Path::new("patch")).is_err());
        assert!(reject_symlinked_path(&project, PathBuf::from("safe/file.txt").as_path()).is_ok());
    }
}
