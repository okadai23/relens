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

/// A deterministic snapshot of the files in a template revision.
pub type TemplateTree = BTreeMap<String, Vec<u8>>;

/// Port used by update operations to obtain immutable template revisions.
pub trait TemplateSource {
    type Error: std::error::Error + Send + Sync + 'static;
    fn fetch(&self, reference: &TemplateRef) -> Result<TemplateTree, Self::Error>;
    fn latest(&self, locator: &str) -> Result<TemplateRef, Self::Error>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuestionKind {
    String,
    Bool,
    Integer,
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
            let valid = matches!(
                (&question.kind, &value),
                (QuestionKind::String, AnswerValue::String(_))
                    | (QuestionKind::Bool, AnswerValue::Bool(_))
                    | (QuestionKind::Integer, AnswerValue::Integer(_))
            );
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
}
