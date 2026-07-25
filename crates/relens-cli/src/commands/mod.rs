use anyhow::{Context, Result, bail};
use relens_domain::{
    AnswerSet, AnswerValue, CommandResult, TemplateRef, TemplateSource, TemplateTree,
};
use std::{collections::BTreeMap, fs, path::Path};

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
        let local = fs::read(project.join(&path)).unwrap_or_default();
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
        if let Some((_, map)) = updated.get(&path) {
            merged_metadata.insert(path, (bytes, map.clone()));
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
    let revision = git_revision(template).unwrap_or_else(|| "working-tree".into());
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
        rendered.insert(output_path, (output.bytes, output.source_map));
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
