//! Pure drift lifting and PutGet verification.

use relens_domain::{AnswerValue, LiftSession, Origin, ReviewDecision, SourceMap, SourceSpan};
use similar::{Algorithm, DiffOp, capture_diff_slices};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Classification {
    Auto,
    Ambiguous {
        literal: String,
        substituted: String,
    },
    Unmappable {
        suggestion: String,
    },
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

/// Renders every applicable decision in a reviewed session and compares it with the project.
pub fn verify_session(
    session: &LiftSession,
    project: &BTreeMap<String, Vec<u8>>,
    answers: &BTreeMap<String, AnswerValue>,
) -> Result<Vec<Divergence>, LiftError> {
    let mut divergences = Vec::new();
    for edit in &session.edits {
        if matches!(
            edit.decision,
            ReviewDecision::Pending | ReviewDecision::Unmappable
        ) {
            continue;
        }
        let content = if edit.decision == ReviewDecision::Substitute {
            edit.substituted.as_deref().unwrap_or(&edit.literal)
        } else {
            &edit.literal
        };
        let actual = relens_engine::render(content, answers)
            .map_err(|source| LiftError::Render {
                path: edit.project_path.clone(),
                source,
            })?
            .bytes;
        let expected = project.get(&edit.project_path).cloned().unwrap_or_default();
        if actual != expected {
            divergences.push(Divergence {
                path: edit.project_path.clone(),
                expected,
                actual,
            });
        }
    }
    Ok(divergences)
}

/// Lifts changed generated files into complete replacement template files.
///
/// Edits are projected onto the original template through its source map. This keeps
/// control-flow tags and the exact spelling of expressions (including filters) intact.
pub fn lift(
    changed: &BTreeSet<String>,
    templates: &BTreeMap<String, (String, String, SourceMap)>,
    project: &BTreeMap<String, Vec<u8>>,
    answers: &BTreeMap<String, AnswerValue>,
) -> Result<LiftResult, LiftError> {
    let mut files = Vec::new();
    let mut patched = BTreeMap::new();
    for path in changed {
        let Some((template_path, template, map)) = templates.get(path) else {
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
        let pristine =
            relens_engine::render(template, answers).map_err(|source| LiftError::Render {
                path: path.clone(),
                source,
            })?;
        let content = apply_rendered_edits(template, &pristine.bytes, rendered.as_bytes(), map);
        let ambiguous = answers.iter().find_map(|(name, value)| {
            let value = value.display();
            (!value.is_empty()
                && rendered.matches(&value).count()
                    > String::from_utf8_lossy(&pristine.bytes)
                        .matches(&value)
                        .count())
            .then(|| {
                (
                    value.clone(),
                    content.replace(&value, &format!("{{{{ {name} }}}}")),
                )
            })
        });
        if ambiguous.is_none() {
            patched.insert(path.clone(), content.clone());
        }
        files.push(LiftedFile {
            project_path: path.clone(),
            template_path: Some(template_path.clone()),
            classification: ambiguous.map_or(Classification::Auto, |(_, substituted)| {
                Classification::Ambiguous {
                    literal: content.clone(),
                    substituted,
                }
            }),
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

fn apply_rendered_edits(template: &str, pristine: &[u8], edited: &[u8], map: &SourceMap) -> String {
    let mut replacements = capture_diff_slices(Algorithm::Myers, pristine, edited)
        .into_iter()
        .filter_map(|operation| match operation {
            DiffOp::Equal { .. } => None,
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => Some((old_index, old_index + old_len, new_index, new_index)),
            DiffOp::Insert {
                old_index,
                new_index,
                new_len,
            } => Some((old_index, old_index, new_index, new_index + new_len)),
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => Some((
                old_index,
                old_index + old_len,
                new_index,
                new_index + new_len,
            )),
        })
        .filter_map(|(old_start, old_end, new_start, new_end)| {
            template_range(map, template.len(), pristine.len(), old_start, old_end)
                .map(|range| (range, &edited[new_start..new_end]))
        })
        .collect::<Vec<_>>();

    replacements.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut lifted = template.as_bytes().to_vec();
    for (range, replacement) in replacements {
        let replacement = protect_jinja(replacement);
        lifted.splice(range, replacement);
    }
    String::from_utf8(lifted).expect("a UTF-8 template with UTF-8 edits remains UTF-8")
}

fn template_range(
    map: &SourceMap,
    template_len: usize,
    rendered_len: usize,
    rendered_start: usize,
    rendered_end: usize,
) -> Option<std::ops::Range<usize>> {
    if rendered_start == rendered_end {
        let offset = map
            .spans
            .iter()
            .find(|span| {
                matches!(span.origin, Origin::Literal { .. })
                    && span.start <= rendered_start
                    && rendered_start <= span.end
            })
            .and_then(|span| literal_offset(span, rendered_start))
            .or({
                // With no adjacent literal span (for example `{{ value }}` followed by
                // an inserted suffix), the rendered file boundaries map to the template
                // boundaries without consuming the expression itself.
                match rendered_start {
                    0 => Some(0),
                    position if position == rendered_len => Some(template_len),
                    _ => None,
                }
            })?;
        return Some(offset..offset);
    }
    let first = map.spans.iter().find(|span| {
        matches!(span.origin, Origin::Literal { .. })
            && span.start <= rendered_start
            && rendered_start < span.end
    })?;
    let last_position = rendered_end - 1;
    let last = map.spans.iter().find(|span| {
        matches!(span.origin, Origin::Literal { .. })
            && span.start <= last_position
            && last_position < span.end
    })?;
    // A range crossing an expression or block would also remove its template syntax.
    // Leave such a change for manual mapping rather than corrupting the template.
    if !std::ptr::eq(first, last) {
        return None;
    }
    Some(literal_offset(first, rendered_start)?..literal_offset(last, rendered_end)?)
}

fn literal_offset(span: &SourceSpan, rendered_offset: usize) -> Option<usize> {
    match span.origin {
        Origin::Literal { template_start, .. } => {
            Some(template_start + rendered_offset.saturating_sub(span.start))
        }
        _ => None,
    }
}

fn protect_jinja(bytes: &[u8]) -> Vec<u8> {
    let mut protected = Vec::with_capacity(bytes.len());
    let mut offset = 0;
    while offset < bytes.len() {
        let remaining = &bytes[offset..];
        if remaining.starts_with(b"{%") {
            protected.extend_from_slice(b"{% raw %}{%{% endraw %}");
            offset += 2;
        } else if remaining.starts_with(b"{{") {
            protected.extend_from_slice(b"{% raw %}{{{% endraw %}");
            offset += 2;
        } else {
            protected.push(bytes[offset]);
            offset += 1;
        }
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
    fn preserves_blocks_and_filtered_expressions() {
        let answers = BTreeMap::from([
            ("enabled".into(), AnswerValue::Bool(true)),
            ("name".into(), AnswerValue::String("myapp".into())),
        ]);
        let template = "{% if enabled %}project = {{ name | upper }}\nold text\n{% endif %}";
        let pristine = relens_engine::render(template, &answers).unwrap();
        let lifted = apply_rendered_edits(
            template,
            &pristine.bytes,
            b"project = MYAPP\nnew text\n",
            &pristine.source_map,
        );

        assert_eq!(
            lifted,
            "{% if enabled %}project = {{ name | upper }}\nnew text\n{% endif %}"
        );
        assert_eq!(
            relens_engine::render(&lifted, &answers).unwrap().bytes,
            b"project = MYAPP\nnew text\n"
        );
    }

    #[test]
    fn preserves_loops_when_editing_their_literal_body() {
        let answers = BTreeMap::from([("letters".into(), AnswerValue::String("ab".into()))]);
        let template = "{% for letter in letters %}item={{ letter }}\n{% endfor %}";
        let pristine = relens_engine::render(template, &answers).unwrap();
        let lifted = apply_rendered_edits(
            template,
            &pristine.bytes,
            b"entry=a\nitem=b\n",
            &pristine.source_map,
        );

        assert!(lifted.contains("{% for letter in letters %}"));
        assert!(lifted.contains("{% endfor %}"));
    }

    #[test]
    fn preserves_utf8_when_diff_replaces_a_continuation_byte() {
        let template = "café\n";
        let pristine = relens_engine::render(template, &BTreeMap::new()).unwrap();
        let lifted = apply_rendered_edits(
            template,
            &pristine.bytes,
            "cafê\n".as_bytes(),
            &pristine.source_map,
        );

        assert_eq!(lifted, "cafê\n");
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

    #[test]
    fn accidental_answer_match_has_literal_and_substituted_candidates() {
        let answers = BTreeMap::from([("project_name".into(), AnswerValue::String("main".into()))]);
        let pristine = relens_engine::render("header\n", &answers).unwrap();
        let result = lift(
            &BTreeSet::from(["file".into()]),
            &BTreeMap::from([(
                "file".into(),
                ("file.j2".into(), "header\n".into(), pristine.source_map),
            )]),
            &BTreeMap::from([("file".into(), b"header\nrun main here\n".to_vec())]),
            &answers,
        )
        .unwrap();
        assert!(matches!(&result.files[0].classification,
            Classification::Ambiguous { literal, substituted }
            if literal.contains("run main here") && substituted.contains("run {{ project_name }} here")));
    }

    #[test]
    fn reviewed_choice_is_rendered_before_verification() {
        let answers = BTreeMap::from([("project_name".into(), AnswerValue::String("main".into()))]);
        let mut session = LiftSession {
            id: "session".into(),
            project: "project".into(),
            template: relens_domain::TemplateRef::new("template", "revision").unwrap(),
            state: relens_domain::LiftSessionState::Reviewing,
            edits: vec![relens_domain::SessionEdit {
                project_path: "README.md".into(),
                template_path: Some("README.md.j2".into()),
                literal: "run main here\n".into(),
                substituted: Some("run {{ project_name }} here\n".into()),
                decision: ReviewDecision::Substitute,
            }],
            divergences: vec![],
        };

        let matching = BTreeMap::from([("README.md".into(), b"run main here\n".to_vec())]);
        assert!(
            verify_session(&session, &matching, &answers)
                .unwrap()
                .is_empty()
        );

        session.edits[0].decision = ReviewDecision::KeepLiteral;
        let changed_answers =
            BTreeMap::from([("project_name".into(), AnswerValue::String("other".into()))]);
        let divergences = verify_session(&session, &matching, &changed_answers).unwrap();
        assert!(divergences.is_empty(), "literal choice must remain literal");

        session.edits[0].decision = ReviewDecision::Substitute;
        let divergences = verify_session(&session, &matching, &changed_answers).unwrap();
        assert_eq!(divergences[0].path, "README.md");
        assert_eq!(divergences[0].actual, b"run other here\n");
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
