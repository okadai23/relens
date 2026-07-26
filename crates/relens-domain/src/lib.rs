//! Pure domain types shared by relens use cases and adapters.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RelensError {
    #[error("configuration already exists at {0}")]
    AlreadyExists(String),
    #[error("could not access {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid {kind}: {message}")]
    Validation { kind: &'static str, message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateRef {
    pub locator: String,
    pub revision: String,
}

impl TemplateRef {
    pub fn new(
        locator: impl Into<String>,
        revision: impl Into<String>,
    ) -> Result<Self, RelensError> {
        let value = Self {
            locator: locator.into(),
            revision: revision.into(),
        };
        if value.locator.trim().is_empty() || value.revision.trim().is_empty() {
            return Err(RelensError::Validation {
                kind: "template reference",
                message: "locator and revision must not be empty".into(),
            });
        }
        Ok(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnswerValue {
    String(String),
    Bool(bool),
    Integer(i64),
}

impl AnswerValue {
    pub fn display(&self) -> String {
        match self {
            Self::String(v) => v.clone(),
            Self::Bool(v) => v.to_string(),
            Self::Integer(v) => v.to_string(),
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        if let Self::Bool(v) = self {
            Some(*v)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerSet {
    pub template: TemplateRef,
    pub answers: BTreeMap<String, AnswerValue>,
}

/// Durable state machine for a reviewed lift operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiftSession {
    pub id: String,
    pub project: String,
    pub template: TemplateRef,
    pub state: LiftSessionState,
    pub edits: Vec<SessionEdit>,
    #[serde(default)]
    pub divergences: Vec<SessionDivergence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LiftSessionState {
    Reviewing,
    Verified,
    Exported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEdit {
    pub project_path: String,
    pub template_path: Option<String>,
    pub literal: String,
    pub substituted: Option<String>,
    pub decision: ReviewDecision,
    /// Whether applying this edit removes the template (and therefore generated) file.
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewDecision {
    Automatic,
    Pending,
    KeepLiteral,
    Substitute,
    Unmappable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDivergence {
    pub path: String,
    pub start: usize,
    pub end: usize,
}

impl LiftSession {
    pub fn resolve(&mut self, index: usize, decision: ReviewDecision) -> Result<(), RelensError> {
        if self.state != LiftSessionState::Reviewing
            || !matches!(
                decision,
                ReviewDecision::KeepLiteral | ReviewDecision::Substitute
            )
        {
            return Err(session_transition(
                "only a reviewing session can resolve a candidate",
            ));
        }
        let edit = self
            .edits
            .get_mut(index)
            .ok_or_else(|| RelensError::Validation {
                kind: "lift session",
                message: format!("unknown edit {index}"),
            })?;
        if edit.decision != ReviewDecision::Pending {
            return Err(session_transition("only a pending edit can be resolved"));
        }
        edit.decision = decision;
        Ok(())
    }

    pub fn verify(&mut self, divergences: Vec<SessionDivergence>) -> Result<(), RelensError> {
        if self.state != LiftSessionState::Reviewing
            || self
                .edits
                .iter()
                .any(|edit| edit.decision == ReviewDecision::Pending)
        {
            return Err(session_transition(
                "all review decisions are required before verification",
            ));
        }
        self.divergences = divergences;
        if self.divergences.is_empty() {
            self.state = LiftSessionState::Verified;
        }
        Ok(())
    }

    pub fn export(&mut self) -> Result<(), RelensError> {
        if self.state != LiftSessionState::Verified || !self.divergences.is_empty() {
            return Err(session_transition(
                "a session must be verified before export",
            ));
        }
        self.state = LiftSessionState::Exported;
        Ok(())
    }
}

fn session_transition(message: &str) -> RelensError {
    RelensError::Validation {
        kind: "lift session",
        message: message.into(),
    }
}

/// A deterministic snapshot of the files in a template revision.
pub type TemplateTree = BTreeMap<String, Vec<u8>>;

/// Port used by update operations to obtain immutable template revisions.
pub trait TemplateSource {
    type Error: std::error::Error + Send + Sync + 'static;
    fn fetch(&self, reference: &TemplateRef) -> Result<TemplateTree, Self::Error>;
    fn latest(&self, locator: &str) -> Result<TemplateRef, Self::Error>;
}

/// Optional extension point for producing lift candidates. Suggestions are
/// deliberately unverified; callers must always pass them through the normal
/// lift verification gate before they can be exported.
pub trait LiftSuggester {
    type Error: std::error::Error + Send + Sync + 'static;
    fn suggest(
        &self,
        request: &LiftSuggestionRequest,
    ) -> Result<Vec<TemplateSuggestion>, Self::Error>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiftSuggestionRequest {
    pub template: TemplateTree,
    pub project: TemplateTree,
    pub answers: BTreeMap<String, AnswerValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateSuggestion {
    pub path: String,
    pub content: Vec<u8>,
}

/// Declarative, side-effect-free answer migration. Applying a migration is
/// atomic because validation is performed on a cloned answer map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Migration {
    #[serde(default)]
    pub rename: BTreeMap<String, String>,
    #[serde(default)]
    pub set: BTreeMap<String, AnswerValue>,
    #[serde(default)]
    pub remove: Vec<String>,
}

impl Migration {
    pub fn apply(
        &self,
        answers: &BTreeMap<String, AnswerValue>,
    ) -> Result<BTreeMap<String, AnswerValue>, RelensError> {
        let mut migrated = answers.clone();
        for (from, to) in &self.rename {
            if migrated.contains_key(to) {
                return Err(RelensError::Validation {
                    kind: "migration",
                    message: format!("cannot rename `{from}` to existing answer `{to}`"),
                });
            }
            let value = migrated
                .remove(from)
                .ok_or_else(|| RelensError::Validation {
                    kind: "migration",
                    message: format!("answer `{from}` does not exist"),
                })?;
            migrated.insert(to.clone(), value);
        }
        for name in &self.remove {
            migrated.remove(name);
        }
        migrated.extend(self.set.clone());
        Ok(migrated)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestionKind {
    String,
    Bool,
    Integer,
    Choice(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub kind: QuestionKind,
    pub default: Option<AnswerValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Questionnaire {
    pub questions: BTreeMap<String, Question>,
}

impl Questionnaire {
    pub fn validate(
        &self,
        supplied: &BTreeMap<String, AnswerValue>,
    ) -> Result<BTreeMap<String, AnswerValue>, RelensError> {
        let mut result = BTreeMap::new();
        for (name, question) in &self.questions {
            let value = supplied
                .get(name)
                .cloned()
                .or_else(|| question.default.clone())
                .ok_or_else(|| RelensError::Validation {
                    kind: "answers",
                    message: format!("missing answer `{name}`"),
                })?;
            let valid = match (&question.kind, &value) {
                (QuestionKind::String, AnswerValue::String(_))
                | (QuestionKind::Bool, AnswerValue::Bool(_))
                | (QuestionKind::Integer, AnswerValue::Integer(_)) => true,
                (QuestionKind::Choice(options), AnswerValue::String(value)) => {
                    options.contains(value)
                }
                _ => false,
            };
            if !valid {
                return Err(RelensError::Validation {
                    kind: "answers",
                    message: format!("answer `{name}` has the wrong type"),
                });
            }
            result.insert(name.clone(), value);
        }
        if let Some(name) = supplied
            .keys()
            .find(|name| !self.questions.contains_key(*name))
        {
            return Err(RelensError::Validation {
                kind: "answers",
                message: format!("unknown answer `{name}`"),
            });
        }
        Ok(result)
    }
}

/// Generates a deterministic greedy pairwise covering array.
pub fn pairwise_answers(
    questionnaire: &Questionnaire,
) -> Result<Vec<BTreeMap<String, AnswerValue>>, RelensError> {
    let dimensions = questionnaire
        .questions
        .iter()
        .map(|(name, question)| {
            let values = match &question.kind {
                QuestionKind::Bool => vec![AnswerValue::Bool(false), AnswerValue::Bool(true)],
                QuestionKind::Choice(values) if !values.is_empty() => {
                    values.iter().cloned().map(AnswerValue::String).collect()
                }
                _ => question.default.clone().map(|v| vec![v]).ok_or_else(|| {
                    RelensError::Validation {
                        kind: "matrix",
                        message: format!("question `{name}` needs a finite choice set or default"),
                    }
                })?,
            };
            Ok((name.clone(), values))
        })
        .collect::<Result<Vec<_>, RelensError>>()?;
    if dimensions.is_empty() {
        return Ok(vec![BTreeMap::new()]);
    }
    if dimensions.len() == 1 {
        return Ok(dimensions[0]
            .1
            .iter()
            .cloned()
            .map(|value| BTreeMap::from([(dimensions[0].0.clone(), value)]))
            .collect());
    }

    // Start with the first pair (all of these combinations are required), then
    // extend the covering array one dimension at a time. This keeps both work
    // and storage proportional to the required pair coverage rather than to
    // the Cartesian product of every question.
    let mut rows = Vec::new();
    for left in &dimensions[0].1 {
        for right in &dimensions[1].1 {
            rows.push(BTreeMap::from([
                (dimensions[0].0.clone(), left.clone()),
                (dimensions[1].0.clone(), right.clone()),
            ]));
        }
    }
    for current in 2..dimensions.len() {
        let (name, values) = &dimensions[current];
        let mut uncovered = pairs_for_dimension(&dimensions, current);
        for row in &mut rows {
            let value = values
                .iter()
                .max_by_key(|value| {
                    dimensions[..current]
                        .iter()
                        .filter(|(previous, _)| {
                            let pair = (
                                previous.clone(),
                                row[previous].display(),
                                name.clone(),
                                value.display(),
                            );
                            uncovered.contains(&pair)
                        })
                        .count()
                })
                .expect("finite dimensions are non-empty")
                .clone();
            row.insert(name.clone(), value);
            uncovered.retain(|pair| !row_covers(row, pair));
        }
        while let Some(pair) = uncovered.iter().next().cloned() {
            let mut row = dimensions[..current]
                .iter()
                .map(|(previous, choices)| (previous.clone(), choices[0].clone()))
                .collect::<BTreeMap<_, _>>();
            let new_value = values
                .iter()
                .find(|value| value.display() == pair.3)
                .expect("pair value belongs to dimension")
                .clone();
            row.insert(
                pair.0.clone(),
                dimensions[..current]
                    .iter()
                    .find(|(previous, _)| previous == &pair.0)
                    .and_then(|(_, choices)| choices.iter().find(|value| value.display() == pair.1))
                    .expect("pair value belongs to dimension")
                    .clone(),
            );
            row.insert(name.clone(), new_value);
            uncovered.retain(|candidate| !row_covers(&row, candidate));
            rows.push(row);
        }
    }
    Ok(rows)
}

type Pair = (String, String, String, String);
fn row_covers(row: &BTreeMap<String, AnswerValue>, pair: &Pair) -> bool {
    row.get(&pair.0).is_some_and(|v| v.display() == pair.1)
        && row.get(&pair.2).is_some_and(|v| v.display() == pair.3)
}
fn pairs_for_dimension(
    dimensions: &[(String, Vec<AnswerValue>)],
    current: usize,
) -> std::collections::BTreeSet<Pair> {
    let mut pairs = std::collections::BTreeSet::new();
    for (previous, previous_values) in &dimensions[..current] {
        for left in previous_values {
            for right in &dimensions[current].1 {
                pairs.insert((
                    previous.clone(),
                    left.display(),
                    dimensions[current].0.clone(),
                    right.display(),
                ));
            }
        }
    }
    pairs
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    Literal {
        template_start: usize,
        template_end: usize,
    },
    Expr {
        variable: String,
        node_id: u64,
    },
    Block {
        node_id: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SourceMap {
    pub spans: Vec<SourceSpan>,
}

impl SourceMap {
    pub fn validate_coverage(&self, length: usize) -> Result<(), RelensError> {
        let mut cursor = 0;
        for span in &self.spans {
            if span.start != cursor || span.end < span.start {
                return Err(RelensError::Validation {
                    kind: "source map",
                    message: format!("gap or overlap at byte {cursor}"),
                });
            }
            cursor = span.end;
        }
        if cursor != length {
            return Err(RelensError::Validation {
                kind: "source map",
                message: format!("coverage ends at {cursor}, expected {length}"),
            });
        }
        Ok(())
    }
}

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
    fn rejects_incomplete_template_reference() {
        assert!(TemplateRef::new("repo", "").is_err());
    }
    #[test]
    fn validates_answer_types_and_defaults() {
        let q = Questionnaire {
            questions: BTreeMap::from([(
                "enabled".into(),
                Question {
                    kind: QuestionKind::Bool,
                    default: Some(AnswerValue::Bool(true)),
                },
            )]),
        };
        assert_eq!(
            q.validate(&BTreeMap::new()).unwrap()["enabled"],
            AnswerValue::Bool(true)
        );
        assert!(
            q.validate(&BTreeMap::from([(
                "enabled".into(),
                AnswerValue::String("yes".into())
            )]))
            .is_err()
        );
    }
    #[test]
    fn source_map_requires_gapless_coverage() {
        let good = SourceMap {
            spans: vec![SourceSpan {
                start: 0,
                end: 2,
                origin: Origin::Block { node_id: 1 },
            }],
        };
        assert!(good.validate_coverage(2).is_ok());
        assert!(good.validate_coverage(3).is_err());
    }

    #[test]
    fn lift_session_requires_review_and_verification_before_export() {
        let mut session = LiftSession {
            id: "one".into(),
            project: "app".into(),
            template: TemplateRef::new("repo", "abc").unwrap(),
            state: LiftSessionState::Reviewing,
            edits: vec![SessionEdit {
                project_path: "a".into(),
                template_path: Some("a.j2".into()),
                literal: "main".into(),
                substituted: Some("{{ name }}".into()),
                decision: ReviewDecision::Pending,
                deleted: false,
            }],
            divergences: vec![],
        };
        assert!(session.export().is_err());
        session.resolve(0, ReviewDecision::KeepLiteral).unwrap();
        session.verify(vec![]).unwrap();
        assert_eq!(session.state, LiftSessionState::Verified);
        session.export().unwrap();
        assert_eq!(session.state, LiftSessionState::Exported);
    }
}

#[cfg(test)]
mod matrix_tests {
    use super::*;

    #[test]
    fn pairwise_plan_covers_every_value_pair_without_full_product() {
        let questionnaire = Questionnaire {
            questions: BTreeMap::from([
                (
                    "a".into(),
                    Question {
                        kind: QuestionKind::Bool,
                        default: None,
                    },
                ),
                (
                    "b".into(),
                    Question {
                        kind: QuestionKind::Bool,
                        default: None,
                    },
                ),
                (
                    "c".into(),
                    Question {
                        kind: QuestionKind::Bool,
                        default: None,
                    },
                ),
                (
                    "flavor".into(),
                    Question {
                        kind: QuestionKind::Choice(vec!["x".into(), "y".into(), "z".into()]),
                        default: None,
                    },
                ),
            ]),
        };
        let rows = pairwise_answers(&questionnaire).unwrap();
        assert!(rows.len() < 24);
        let dimensions = [
            ("a", vec!["false", "true"]),
            ("b", vec!["false", "true"]),
            ("c", vec!["false", "true"]),
            ("flavor", vec!["x", "y", "z"]),
        ];
        for left in 0..dimensions.len() {
            for right in left + 1..dimensions.len() {
                for a in &dimensions[left].1 {
                    for b in &dimensions[right].1 {
                        assert!(
                            rows.iter()
                                .any(|row| row[dimensions[left].0].display() == *a
                                    && row[dimensions[right].0].display() == *b)
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn pairwise_plan_scales_to_many_boolean_questions() {
        let questionnaire = Questionnaire {
            questions: (0..25)
                .map(|index| {
                    (
                        format!("q{index:02}"),
                        Question {
                            kind: QuestionKind::Bool,
                            default: None,
                        },
                    )
                })
                .collect(),
        };

        let rows = pairwise_answers(&questionnaire).unwrap();

        assert!(rows.len() < 100);
        assert!(rows.iter().all(|row| row.len() == 25));
    }

    #[test]
    fn choice_validation_rejects_values_outside_options() {
        let q = Questionnaire {
            questions: BTreeMap::from([(
                "color".into(),
                Question {
                    kind: QuestionKind::Choice(vec!["red".into()]),
                    default: None,
                },
            )]),
        };
        assert!(
            q.validate(&BTreeMap::from([(
                "color".into(),
                AnswerValue::String("blue".into())
            )]))
            .is_err()
        );
    }

    #[test]
    fn failed_migration_leaves_original_answers_unchanged() {
        let answers = BTreeMap::from([
            ("old".into(), AnswerValue::String("value".into())),
            ("new".into(), AnswerValue::String("existing".into())),
        ]);
        let migration = Migration {
            rename: BTreeMap::from([("old".into(), "new".into())]),
            ..Migration::default()
        };
        assert!(migration.apply(&answers).is_err());
        assert_eq!(answers["old"], AnswerValue::String("value".into()));
    }
}
