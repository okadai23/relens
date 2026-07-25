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
    render_range(source, 0, answers, 0)
}

fn render_range(
    source: &str,
    base: usize,
    answers: &BTreeMap<String, AnswerValue>,
    node_seed: u64,
) -> Result<RenderedText, RenderError> {
    let mut output = Vec::new();
    let mut spans = Vec::new();
    let mut cursor = 0;
    let mut node = node_seed;
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
            node += 1;
            push(
                &mut output,
                &mut spans,
                value.as_bytes(),
                Origin::Expr {
                    variable: variable.into(),
                    node_id: node,
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
                let end_marker = source[body_start..]
                    .find("{% endif %}")
                    .ok_or_else(|| syntax(base + next, "missing endif"))?
                    + body_start;
                let else_marker = source[body_start..end_marker]
                    .find("{% else %}")
                    .map(|v| body_start + v);
                let enabled = answers
                    .get(condition.trim())
                    .ok_or_else(|| RenderError::UnknownVariable(condition.trim().into()))?
                    .as_bool()
                    .ok_or_else(|| RenderError::NonBoolean(condition.trim().into()))?;
                let (start, end) = match (enabled, else_marker) {
                    (true, Some(e)) => (body_start, e),
                    (false, Some(e)) => (e + 10, end_marker),
                    (true, None) => (body_start, end_marker),
                    (false, None) => (body_start, body_start),
                };
                let child = render_range(&source[start..end], base + start, answers, node + 1)?;
                node += 1;
                if child.bytes.is_empty() {
                    push(
                        &mut output,
                        &mut spans,
                        &[],
                        Origin::Block { node_id: node },
                    );
                } else {
                    append(&mut output, &mut spans, child);
                }
                cursor = end_marker + 11;
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
    fn rejects_unsupported_construct() {
        assert!(
            render("{% include 'x' %}", &answers())
                .unwrap_err()
                .to_string()
                .contains("unsupported")
        );
    }
}
