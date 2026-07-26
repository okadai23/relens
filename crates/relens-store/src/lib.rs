//! Deterministic filesystem persistence for project metadata.
use relens_domain::{
    AnswerSet, AnswerValue, CommandResult, LiftSession, Question, QuestionKind, Questionnaire,
    RelensError, SourceMap,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub fn initialize(path: &Path) -> Result<CommandResult, RelensError> {
    if path.exists() {
        return Err(RelensError::AlreadyExists(path.display().to_string()));
    }
    fs::write(path, "# relens configuration\n").io(path)?;
    Ok(CommandResult::new(
        "initialized",
        path.display().to_string(),
    ))
}
pub fn inspect(path: &Path) -> Result<CommandResult, RelensError> {
    fs::read(path).io(path)?;
    Ok(CommandResult::new("inspected", path.display().to_string()))
}

#[derive(Debug, Deserialize)]
struct Config {
    questions: BTreeMap<String, RawQuestion>,
}
#[derive(Debug, Deserialize)]
struct RawQuestion {
    #[serde(rename = "type")]
    kind: String,
    default: Option<toml::Value>,
    #[serde(default)]
    choices: Vec<String>,
}
pub fn load_questionnaire(template: &Path) -> Result<Questionnaire, RelensError> {
    let path = template.join("relens.toml");
    let text = fs::read_to_string(&path).io(&path)?;
    let raw: Config = toml::from_str(&text).map_err(|e| validation("questionnaire", e))?;
    let questions = raw
        .questions
        .into_iter()
        .map(|(name, q)| {
            let kind = match q.kind.as_str() {
                "string" | "Str" => QuestionKind::String,
                "bool" | "Bool" => QuestionKind::Bool,
                "integer" | "Int" => QuestionKind::Integer,
                "choice" | "Choice" if !q.choices.is_empty() => QuestionKind::Choice(q.choices),
                _ => {
                    return Err(RelensError::Validation {
                        kind: "questionnaire",
                        message: format!("unknown type `{}` for `{name}`", q.kind),
                    });
                }
            };
            let default = q.default.map(answer_from_toml).transpose()?;
            Ok((name, Question { kind, default }))
        })
        .collect::<Result<_, _>>()?;
    Ok(Questionnaire { questions })
}
fn answer_from_toml(v: toml::Value) -> Result<AnswerValue, RelensError> {
    match v {
        toml::Value::String(v) => Ok(AnswerValue::String(v)),
        toml::Value::Boolean(v) => Ok(AnswerValue::Bool(v)),
        toml::Value::Integer(v) => Ok(AnswerValue::Integer(v)),
        _ => Err(RelensError::Validation {
            kind: "questionnaire",
            message: "defaults must be string, boolean, or integer".into(),
        }),
    }
}

pub fn template_files(root: &Path) -> Result<Vec<PathBuf>, RelensError> {
    let mut files = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().strip_prefix(root).unwrap().to_owned())
        .filter(|p| p != Path::new("relens.toml"))
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

/// Converts a project-relative path to the platform-independent form used in
/// templates and lock files.
pub fn portable_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LockFile {
    pub files: BTreeMap<String, LockedFile>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct LockedFile {
    pub sha256: String,
    pub source_map: SourceMap,
}
pub fn persist(
    project: &Path,
    answer_set: &AnswerSet,
    rendered: &BTreeMap<String, (Vec<u8>, SourceMap)>,
) -> Result<(), RelensError> {
    let meta = project.join(".relens");
    fs::create_dir_all(&meta).io(&meta)?;
    let answers = toml::to_string_pretty(answer_set).map_err(|e| validation("answers", e))?;
    fs::write(meta.join("answers.toml"), answers).io(&meta)?;
    let files = rendered
        .iter()
        .map(|(path, (bytes, map))| {
            (
                path.clone(),
                LockedFile {
                    sha256: digest(bytes),
                    source_map: map.clone(),
                },
            )
        })
        .collect();
    let json = serde_json::to_vec_pretty(&LockFile { files }).map_err(|e| validation("lock", e))?;
    fs::write(meta.join("lock.json"), json).io(&meta)
}
pub fn load_answers(project: &Path) -> Result<AnswerSet, RelensError> {
    let path = project.join(".relens/answers.toml");
    toml::from_str(&fs::read_to_string(&path).io(&path)?).map_err(|e| validation("answers", e))
}
pub fn load_lock(project: &Path) -> Result<LockFile, RelensError> {
    let path = project.join(".relens/lock.json");
    serde_json::from_slice(&fs::read(&path).io(&path)?).map_err(|e| validation("lock", e))
}

pub fn save_session(project: &Path, session: &LiftSession) -> Result<(), RelensError> {
    let directory = project.join(".relens/sessions");
    fs::create_dir_all(&directory).io(&directory)?;
    let path = directory.join(format!("{}.json", session.id));
    let json = serde_json::to_vec_pretty(session).map_err(|e| validation("lift session", e))?;
    fs::write(&path, json).io(&path)
}

pub fn load_session(project: &Path, id: Option<&str>) -> Result<LiftSession, RelensError> {
    let directory = project.join(".relens/sessions");
    let path = if let Some(id) = id {
        directory.join(format!("{id}.json"))
    } else {
        let mut paths = fs::read_dir(&directory)
            .io(&directory)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect::<Vec<_>>();
        paths.sort();
        paths.pop().ok_or_else(|| RelensError::Validation {
            kind: "lift session",
            message: "no resumable session exists".into(),
        })?
    };
    serde_json::from_slice(&fs::read(&path).io(&path)?).map_err(|e| validation("lift session", e))
}
pub fn drift(project: &Path) -> Result<Vec<String>, RelensError> {
    let lock = load_lock(project)?;
    let locked_paths = lock
        .files
        .keys()
        .map(|relative| portable_path(Path::new(relative)))
        .collect::<std::collections::BTreeSet<_>>();
    let mut changed = Vec::new();
    for (relative, file) in &lock.files {
        match fs::read(project.join(relative)) {
            Ok(bytes) if digest(&bytes) == file.sha256 => {}
            _ => changed.push(relative.clone()),
        }
    }
    for entry in walkdir::WalkDir::new(project)
        .into_iter()
        .filter_entry(|entry| entry.file_name() != ".relens")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let relative = portable_path(
            entry
                .path()
                .strip_prefix(project)
                .expect("walked below project"),
        );
        if !locked_paths.contains(&relative) && !changed.contains(&relative) {
            changed.push(relative);
        }
    }
    changed.sort();
    Ok(changed)
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn validation(kind: &'static str, e: impl std::fmt::Display) -> RelensError {
    RelensError::Validation {
        kind,
        message: e.to_string(),
    }
}
trait Io<T> {
    fn io(self, path: &Path) -> Result<T, RelensError>;
}
impl<T> Io<T> for std::io::Result<T> {
    fn io(self, path: &Path) -> Result<T, RelensError> {
        self.map_err(|source| RelensError::Io {
            path: path.display().to_string(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use relens_domain::TemplateRef;
    #[test]
    fn questionnaire_is_typed() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join("relens.toml"),
            "[questions.name]\ntype='string'\ndefault='app'\n",
        )
        .unwrap();
        assert_eq!(
            load_questionnaire(d.path()).unwrap().questions["name"].default,
            Some(AnswerValue::String("app".into()))
        );
    }
    #[test]
    fn persistence_detects_drift() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a"), "old").unwrap();
        let set = AnswerSet {
            template: TemplateRef::new("x", "1").unwrap(),
            answers: BTreeMap::new(),
        };
        persist(
            d.path(),
            &set,
            &BTreeMap::from([("a".into(), (b"old".to_vec(), SourceMap::default()))]),
        )
        .unwrap();
        assert!(drift(d.path()).unwrap().is_empty());
        fs::write(d.path().join("a"), "new").unwrap();
        assert_eq!(drift(d.path()).unwrap(), ["a"]);
    }

    #[test]
    fn portable_paths_are_stable_across_platforms() {
        assert_eq!(
            portable_path(Path::new(r"package\main.py")),
            "package/main.py"
        );
        assert_eq!(
            portable_path(Path::new("package/main.py")),
            "package/main.py"
        );
    }

    #[test]
    fn lift_session_round_trips() {
        use relens_domain::{LiftSession, LiftSessionState};
        let d = tempfile::tempdir().unwrap();
        let session = LiftSession {
            id: "0001".into(),
            project: "app".into(),
            template: TemplateRef::new("repo", "abc").unwrap(),
            state: LiftSessionState::Verified,
            edits: vec![relens_domain::SessionEdit {
                project_path: "removed.txt".into(),
                template_path: Some("removed.txt.j2".into()),
                literal: String::new(),
                substituted: None,
                decision: relens_domain::ReviewDecision::Automatic,
                deleted: true,
            }],
            divergences: vec![],
        };
        save_session(d.path(), &session).unwrap();
        assert_eq!(load_session(d.path(), None).unwrap(), session);
    }
}
