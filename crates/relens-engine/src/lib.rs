//! Deterministic renderer for relens' deliberately small Jinja subset.
use relens_domain::{AnswerValue, Origin, SourceMap, SourceSpan};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RenderError {
    #[error("unsupported or malformed template syntax at byte {offset}: {message}")]
    Syntax { offset: usize, message: String },
    #[error("unknown variable `{0}`")]
    UnknownVariable(String),
    #[error("condition `{0}` is not boolean")]
    NonBoolean(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedText {
    pub bytes: Vec<u8>,
    pub source_map: SourceMap,
}

pub fn render(
    source: &str,
    answers: &BTreeMap<String, AnswerValue>,
) -> Result<RenderedText, RenderError> {
    let mut node = 0;
    render_range(source, 0, answers, &mut node)
}

fn render_range(
    source: &str,
    base: usize,
    answers: &BTreeMap<String, AnswerValue>,
    node: &mut u64,
) -> Result<RenderedText, RenderError> {
    let mut output = Vec::new();
    let mut spans = Vec::new();
    let mut cursor = 0;
    while cursor < source.len() {
        let expr = source[cursor..].find("{{").map(|v| cursor + v);
        let block = source[cursor..].find("{%").map(|v| cursor + v);
        let next = match (expr, block) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => source.len(),
        };
        push(
            &mut output,
            &mut spans,
            &source.as_bytes()[cursor..next],
            Origin::Literal {
                template_start: base + cursor,
                template_end: base + next,
            },
        );
        if next == source.len() {
            break;
        }
        if source[next..].starts_with("{{") {
            let close = source[next + 2..]
                .find("}}")
                .ok_or_else(|| syntax(base + next, "unclosed expression"))?
                + next
                + 2;
            let expression = source[next + 2..close].trim();
            let mut parts = expression.split('|').map(str::trim);
            let variable = parts.next().unwrap_or_default();
            if !valid_name(variable) {
                return Err(syntax(
                    base + next,
                    "only variable expressions are supported",
                ));
            }
            let mut value = answers
                .get(variable)
                .ok_or_else(|| RenderError::UnknownVariable(variable.into()))?
                .display();
            for filter in parts {
                value = match filter {
                    "lower" => value.to_lowercase(),
                    "upper" => value.to_uppercase(),
                    _ => {
                        return Err(syntax(
                            base + next,
                            &format!("unsupported filter `{filter}`"),
                        ));
                    }
                };
            }
            *node += 1;
            push(
                &mut output,
                &mut spans,
                value.as_bytes(),
                Origin::Expr {
                    variable: variable.into(),
                    node_id: *node,
                },
            );
            cursor = close + 2;
        } else {
            let tag_end = source[next + 2..]
                .find("%}")
                .ok_or_else(|| syntax(base + next, "unclosed block"))?
                + next
                + 2;
            let tag = source[next + 2..tag_end].trim();
            if let Some(condition) = tag.strip_prefix("if ") {
                if !valid_name(condition.trim()) {
                    return Err(syntax(
                        base + next,
                        "only a boolean variable is allowed in if",
                    ));
                }
                let body_start = tag_end + 2;
                let (end_marker, branches) = if_boundaries(source, body_start)
                    .ok_or_else(|| syntax(base + next, "missing endif"))?;
                let mut start = body_start;
                let mut selected = None;
                let mut branch_condition = Some(condition.trim());
                for (boundary, next_condition, tag_length) in branches
                    .into_iter()
                    .chain(std::iter::once((end_marker, None, 0)))
                {
                    let enabled = if selected.is_some() {
                        false
                    } else {
                        match branch_condition {
                            Some(name) => answers
                                .get(name)
                                .ok_or_else(|| RenderError::UnknownVariable(name.into()))?
                                .as_bool()
                                .ok_or_else(|| RenderError::NonBoolean(name.into()))?,
                            None => true,
                        }
                    };
                    if selected.is_none() && enabled {
                        selected = Some((start, boundary));
                    }
                    start = boundary + tag_length;
                    branch_condition = next_condition;
                }
                let (start, end) = selected.unwrap_or((body_start, body_start));
                *node += 1;
                let block_node = *node;
                let child = render_range(&source[start..end], base + start, answers, node)?;
                if child.bytes.is_empty() {
                    push(
                        &mut output,
                        &mut spans,
                        &[],
                        Origin::Block {
                            node_id: block_node,
                        },
                    );
                } else {
                    append(&mut output, &mut spans, child);
                }
                cursor = end_marker + 11;
            } else if let Some(loop_spec) = tag.strip_prefix("for ") {
                let (variable, iterable) = loop_spec
                    .split_once(" in ")
                    .filter(|(variable, iterable)| {
                        valid_name(variable.trim()) && valid_name(iterable.trim())
                    })
                    .ok_or_else(|| syntax(base + next, "expected `for name in variable`"))?;
                let body_start = tag_end + 2;
                let end_marker = matching_end(source, body_start, "for", "endfor")
                    .ok_or_else(|| syntax(base + next, "missing endfor"))?;
                let value = answers
                    .get(iterable.trim())
                    .ok_or_else(|| RenderError::UnknownVariable(iterable.trim().into()))?;
                let values: Vec<AnswerValue> = match value {
                    AnswerValue::String(value) => value
                        .chars()
                        .map(|character| AnswerValue::String(character.to_string()))
                        .collect(),
                    _ => {
                        return Err(syntax(
                            base + next,
                            "for iterable must be a string in the current answer model",
                        ));
                    }
                };
                *node += 1;
                for value in values {
                    let mut local_answers = answers.clone();
                    local_answers.insert(variable.trim().into(), value);
                    let child = render_range(
                        &source[body_start..end_marker],
                        base + body_start,
                        &local_answers,
                        node,
                    )?;
                    append(&mut output, &mut spans, child);
                }
                cursor = end_marker + 12;
            } else if tag == "raw" {
                let body_start = tag_end + 2;
                let end = source[body_start..]
                    .find("{% endraw %}")
                    .ok_or_else(|| syntax(base + next, "missing endraw"))?
                    + body_start;
                push(
                    &mut output,
                    &mut spans,
                    &source.as_bytes()[body_start..end],
                    Origin::Literal {
                        template_start: base + body_start,
                        template_end: base + end,
                    },
                );
                cursor = end + 12;
            } else {
                return Err(syntax(base + next, &format!("unsupported block `{tag}`")));
            }
        }
    }
    let map = SourceMap { spans };
    map.validate_coverage(output.len())
        .map_err(|e| syntax(base, &e.to_string()))?;
    Ok(RenderedText {
        bytes: output,
        source_map: map,
    })
}

/// Finds top-level elif/else boundaries and the matching endif.
type BranchBoundary<'a> = (usize, Option<&'a str>, usize);

fn if_boundaries(source: &str, start: usize) -> Option<(usize, Vec<BranchBoundary<'_>>)> {
    let mut boundaries = Vec::new();
    let mut depth = 1;
    let mut cursor = start;
    while let Some(relative) = source[cursor..].find("{%") {
        let position = cursor + relative;
        let close = source[position + 2..].find("%}")? + position + 2;
        let tag = source[position + 2..close].trim();
        if tag.starts_with("if ") {
            depth += 1;
        } else if tag == "endif" {
            depth -= 1;
            if depth == 0 {
                return Some((position, boundaries));
            }
        } else if depth == 1 && tag == "else" {
            boundaries.push((position, None, close + 2 - position));
        } else if depth == 1 {
            if let Some(condition) = tag.strip_prefix("elif ") {
                boundaries.push((position, Some(condition.trim()), close + 2 - position));
            }
        }
        cursor = close + 2;
    }
    None
}

fn matching_end(source: &str, start: usize, open: &str, close_name: &str) -> Option<usize> {
    let mut depth = 1;
    let mut cursor = start;
    while let Some(relative) = source[cursor..].find("{%") {
        let position = cursor + relative;
        let close = source[position + 2..].find("%}")? + position + 2;
        let tag = source[position + 2..close].trim();
        if tag.starts_with(&format!("{open} ")) {
            depth += 1;
        } else if tag == close_name {
            depth -= 1;
            if depth == 0 {
                return Some(position);
            }
        }
        cursor = close + 2;
    }
    None
}
fn valid_name(v: &str) -> bool {
    !v.is_empty()
        && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !v.as_bytes()[0].is_ascii_digit()
}
fn syntax(offset: usize, message: &str) -> RenderError {
    RenderError::Syntax {
        offset,
        message: message.into(),
    }
}
fn push(out: &mut Vec<u8>, spans: &mut Vec<SourceSpan>, bytes: &[u8], origin: Origin) {
    let start = out.len();
    out.extend_from_slice(bytes);
    let end = out.len();
    if start != end {
        spans.push(SourceSpan { start, end, origin });
    }
}
fn append(out: &mut Vec<u8>, spans: &mut Vec<SourceSpan>, child: RenderedText) {
    let offset = out.len();
    out.extend(child.bytes);
    spans.extend(child.source_map.spans.into_iter().map(|mut s| {
        s.start += offset;
        s.end += offset;
        s
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn answers() -> BTreeMap<String, AnswerValue> {
        BTreeMap::from([
            ("name".into(), AnswerValue::String("App".into())),
            ("on".into(), AnswerValue::Bool(true)),
        ])
    }
    #[test]
    fn renders_and_maps_every_byte() {
        let r = render("Hi {{ name|lower }}!", &answers()).unwrap();
        assert_eq!(r.bytes, b"Hi app!");
        r.source_map.validate_coverage(r.bytes.len()).unwrap();
    }
    #[test]
    fn renders_boolean_branch() {
        assert_eq!(
            render("{% if on %}yes{% else %}no{% endif %}", &answers())
                .unwrap()
                .bytes,
            b"yes"
        );
    }
    #[test]
    fn keeps_node_ids_unique_across_a_branch() {
        let rendered = render("{% if on %}{{ name }}{% endif %}{{ name }}", &answers()).unwrap();
        let ids: Vec<_> = rendered
            .source_map
            .spans
            .iter()
            .filter_map(|span| match span.origin {
                Origin::Expr { node_id, .. } => Some(node_id),
                _ => None,
            })
            .collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
    }
    #[test]
    fn renders_elif_and_for_blocks() {
        let mut values = answers();
        values.insert("other".into(), AnswerValue::Bool(true));
        assert_eq!(
            render(
                "{% if missing %}a{% elif other %}b{% else %}c{% endif %}",
                &BTreeMap::from([
                    ("missing".into(), AnswerValue::Bool(false)),
                    ("other".into(), AnswerValue::Bool(true)),
                ]),
            )
            .unwrap()
            .bytes,
            b"b"
        );
        assert_eq!(
            render("{% for letter in name %}{{ letter }}{% endfor %}", &values)
                .unwrap()
                .bytes,
            b"App"
        );
    }
    #[test]
    fn rejects_unsupported_construct() {
        assert!(
            render("{% include 'x' %}", &answers())
                .unwrap_err()
                .to_string()
                .contains("unsupported")
        );
    }
}
