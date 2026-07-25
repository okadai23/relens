//! Pure drift lifting and PutGet verification.

use relens_domain::{AnswerValue, Origin, SourceMap};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Classification {
    Auto,
    Unmappable { suggestion: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiftedFile {
    pub project_path: String,
    pub template_path: Option<String>,
    pub classification: Classification,
    pub content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiftResult {
    pub files: Vec<LiftedFile>,
    pub verification: Verification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verification {
    Pass,
    Fail(Vec<Divergence>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    pub path: String,
    pub expected: Vec<u8>,
    pub actual: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum LiftError {
    #[error("template `{0}` is not UTF-8")]
    NonUtf8(String),
    #[error("could not render lifted template `{path}`: {source}")]
    Render {
        path: String,
        #[source]
        source: relens_engine::RenderError,
    },
}

/// Lifts changed generated files into complete replacement template files.
///
/// Complete replacements make patch application deterministic. Source maps identify
/// which answer expressions must be restored; newly introduced Jinja delimiters are
/// protected with raw blocks before those expressions are put back.
pub fn lift(
    changed: &BTreeSet<String>,
    templates: &BTreeMap<String, (String, String, SourceMap)>,
    project: &BTreeMap<String, Vec<u8>>,
    answers: &BTreeMap<String, AnswerValue>,
) -> Result<LiftResult, LiftError> {
    let mut files = Vec::new();
    let mut patched = BTreeMap::new();
    for path in changed {
        let Some((template_path, _, map)) = templates.get(path) else {
            files.push(LiftedFile {
                project_path: path.clone(),
                template_path: None,
                classification: Classification::Unmappable {
                    suggestion: "テンプレートへ新規ファイルとして追加する".into(),
                },
                content: None,
            });
            continue;
        };
        let bytes = project.get(path).cloned().unwrap_or_default();
        let rendered = String::from_utf8(bytes).map_err(|_| LiftError::NonUtf8(path.clone()))?;
        let variables = map
            .spans
            .iter()
            .filter_map(|span| match &span.origin {
                Origin::Expr { variable, .. } => Some(variable.clone()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let content = inverse_render(&rendered, &variables, answers);
        patched.insert(path.clone(), content.clone());
        files.push(LiftedFile {
            project_path: path.clone(),
            template_path: Some(template_path.clone()),
            classification: Classification::Auto,
            content: Some(content),
        });
    }

    let mut divergences = Vec::new();
    for (path, content) in patched {
        let actual = relens_engine::render(&content, answers)
            .map_err(|source| LiftError::Render {
                path: path.clone(),
                source,
            })?
            .bytes;
        let expected = project.get(&path).cloned().unwrap_or_default();
        if actual != expected {
            divergences.push(Divergence {
                path,
                expected,
                actual,
            });
        }
    }
    Ok(LiftResult {
        files,
        verification: if divergences.is_empty() {
            Verification::Pass
        } else {
            Verification::Fail(divergences)
        },
    })
}

fn inverse_render(
    rendered: &str,
    variables: &BTreeSet<String>,
    answers: &BTreeMap<String, AnswerValue>,
) -> String {
    let mut substitutions = variables
        .iter()
        .filter_map(|name| answers.get(name).map(|value| (name, value.display())))
        .filter(|(_, value)| !value.is_empty())
        .collect::<Vec<_>>();
    substitutions
        .sort_by(|(a_name, a), (b_name, b)| b.len().cmp(&a.len()).then_with(|| a_name.cmp(b_name)));
    let mut protected = rendered.to_owned();
    let mut markers = Vec::new();
    for (index, (name, value)) in substitutions.iter().enumerate() {
        let marker = format!("\u{e000}{index}\u{e001}");
        protected = protected.replace(value, &marker);
        markers.push((marker, format!("{{{{ {name} }}}}")));
    }
    protected = protected.replace("{%", "{% raw %}{%{% endraw %}");
    protected = protected.replace("{{", "{% raw %}{{{% endraw %}");
    for (marker, expression) in markers {
        protected = protected.replace(&marker, &expression);
    }
    protected
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use relens_domain::SourceSpan;

    fn expression_map(variable: &str, len: usize) -> SourceMap {
        SourceMap {
            spans: vec![SourceSpan {
                start: 0,
                end: len,
                origin: Origin::Expr {
                    variable: variable.into(),
                    node_id: 1,
                },
            }],
        }
    }

    #[test]
    fn reverses_answer_values_and_protects_new_jinja() {
        let answers = BTreeMap::from([("name".into(), AnswerValue::String("myapp".into()))]);
        let variables = BTreeSet::from(["name".into()]);
        let lifted = inverse_render("myapp {{ example }}", &variables, &answers);
        assert!(lifted.contains("{{ name }}"));
        assert!(lifted.contains("{% raw %}{{{% endraw %} example }}"));
        assert_eq!(
            relens_engine::render(&lifted, &answers).unwrap().bytes,
            b"myapp {{ example }}"
        );
    }

    #[test]
    fn reports_added_files_as_unmappable() {
        let result = lift(
            &BTreeSet::from(["notes/private.md".into()]),
            &BTreeMap::new(),
            &BTreeMap::from([("notes/private.md".into(), b"private".to_vec())]),
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(matches!(
            result.files[0].classification,
            Classification::Unmappable { .. }
        ));
        assert_eq!(result.verification, Verification::Pass);
    }

    proptest! {
        #[test]
        fn put_get_for_variable_adjacent_literal_edits(
            value in "[a-z]{1,12}", suffix in "[ A-Za-z0-9]{0,24}"
        ) {
            let answers = BTreeMap::from([("name".into(), AnswerValue::String(value.clone()))]);
            let project_text = format!("{value}{suffix}");
            let result = lift(
                &BTreeSet::from(["file".into()]),
                &BTreeMap::from([("file".into(), ("file.j2".into(), "{{ name }}".into(), expression_map("name", value.len())))]),
                &BTreeMap::from([("file".into(), project_text.as_bytes().to_vec())]),
                &answers,
            ).unwrap();
            prop_assert_eq!(result.verification, Verification::Pass);
        }

        #[test]
        fn get_put_has_no_patch(value in "[a-z]{1,12}") {
            let result = lift(
                &BTreeSet::new(),
                &BTreeMap::new(),
                &BTreeMap::from([("file".into(), value.into_bytes())]),
                &BTreeMap::new(),
            ).unwrap();
            prop_assert!(result.files.is_empty());
            prop_assert_eq!(result.verification, Verification::Pass);
        }
    }
}
