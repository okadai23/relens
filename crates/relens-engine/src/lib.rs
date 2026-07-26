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

#[derive(Debug)]
enum Token<'a> {
    Expression {
        start: usize,
        end: usize,
        body: &'a str,
    },
    Block {
        start: usize,
        end: usize,
        body: &'a str,
    },
    Raw {
        start: usize,
        end: usize,
        body_start: usize,
        body_end: usize,
    },
}

impl Token<'_> {
    fn start(&self) -> usize {
        match self {
            Self::Expression { start, .. }
            | Self::Block { start, .. }
            | Self::Raw { start, .. } => *start,
        }
    }

    fn end(&self) -> usize {
        match self {
            Self::Expression { end, .. } | Self::Block { end, .. } | Self::Raw { end, .. } => *end,
        }
    }
}

/// Returns the next template token. A raw block is deliberately one token, so every
/// consumer applies the same rule that tag-looking text inside it is literal.
fn next_token(source: &str, cursor: usize) -> Result<Option<Token<'_>>, (usize, &'static str)> {
    let expr = source[cursor..].find("{{").map(|at| cursor + at);
    let block = source[cursor..].find("{%").map(|at| cursor + at);
    let Some(start) = (match (expr, block) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }) else {
        return Ok(None);
    };
    if source[start..].starts_with("{{") {
        let close = source[start + 2..]
            .find("}}")
            .ok_or((start, "unclosed expression"))?
            + start
            + 2;
        return Ok(Some(Token::Expression {
            start,
            end: close + 2,
            body: &source[start + 2..close],
        }));
    }
    let close = source[start + 2..]
        .find("%}")
        .ok_or((start, "unclosed block"))?
        + start
        + 2;
    let body = source[start + 2..close].trim();
    if body == "raw" {
        let body_start = close + 2;
        let (body_end, end) = find_endraw(source, body_start).ok_or((start, "missing endraw"))?;
        return Ok(Some(Token::Raw {
            start,
            end,
            body_start,
            body_end,
        }));
    }
    Ok(Some(Token::Block {
        start,
        end: close + 2,
        body,
    }))
}

fn find_endraw(source: &str, mut cursor: usize) -> Option<(usize, usize)> {
    while let Some(relative) = source[cursor..].find("{%").map(|at| cursor + at) {
        let close = source[relative + 2..].find("%}")? + relative + 2;
        if source[relative + 2..close].trim() == "endraw" {
            return Some((relative, close + 2));
        }
        // Raw contents are opaque, including malformed tag-looking text. Move
        // past only this opener so a later, valid endraw can still be found.
        cursor = relative + 2;
    }
    None
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
        let token = next_token(source, cursor)
            .map_err(|(offset, message)| syntax(base + offset, message))?;
        let next = token.as_ref().map_or(source.len(), Token::start);
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
        if let Some(Token::Expression { end, body, .. }) = token {
            let expression = body.trim();
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
            cursor = end;
        } else if let Some(Token::Raw {
            end,
            body_start,
            body_end,
            ..
        }) = token
        {
            push(
                &mut output,
                &mut spans,
                &source.as_bytes()[body_start..body_end],
                Origin::Literal {
                    template_start: base + body_start,
                    template_end: base + body_end,
                },
            );
            cursor = end;
        } else if let Some(Token::Block {
            end: tag_end,
            body: tag,
            ..
        }) = token
        {
            if let Some(condition) = tag.strip_prefix("if ") {
                if !valid_name(condition.trim()) {
                    return Err(syntax(
                        base + next,
                        "only a boolean variable is allowed in if",
                    ));
                }
                let body_start = tag_end;
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
                let body_start = tag_end;
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
            } else {
                return Err(syntax(base + next, &format!("unsupported block `{tag}`")));
            }
        } else {
            unreachable!("a token exists when next is before the source end");
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
    while let Some(token) = next_token(source, cursor).ok()? {
        cursor = token.end();
        let Token::Block {
            start: position,
            end,
            body: tag,
        } = token
        else {
            continue;
        };
        if tag.starts_with("if ") {
            depth += 1;
        } else if tag == "endif" {
            depth -= 1;
            if depth == 0 {
                return Some((position, boundaries));
            }
        } else if depth == 1 && tag == "else" {
            boundaries.push((position, None, end - position));
        } else if depth == 1 {
            if let Some(condition) = tag.strip_prefix("elif ") {
                boundaries.push((position, Some(condition.trim()), end - position));
            }
        }
    }
    None
}

fn matching_end(source: &str, start: usize, open: &str, close_name: &str) -> Option<usize> {
    let mut depth = 1;
    let mut cursor = start;
    while let Some(token) = next_token(source, cursor).ok()? {
        cursor = token.end();
        let Token::Block {
            start: position,
            body: tag,
            ..
        } = token
        else {
            continue;
        };
        if tag.starts_with(&format!("{open} ")) {
            depth += 1;
        } else if tag == close_name {
            depth -= 1;
            if depth == 0 {
                return Some(position);
            }
        }
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

/// Merge a pristine render, a user's project file, and a newly rendered file.
///
/// UTF-8 inputs are diffed as newline-inclusive lines, which preserves both UTF-8
/// text and the presence or absence of the final newline. Every hunk is expressed
/// in base coordinates. Deletions and replacements conflict only when their base
/// ranges overlap; adjacent hunks and insertions at a replacement boundary do not.
/// Identical insertions at the same point are deduplicated, while different
/// insertions at that point conflict. Binary inputs retain the conservative policy:
/// an edit on only one side is accepted, but competing edits conflict.
pub fn three_way_merge(base: &[u8], project: &[u8], updated: &[u8]) -> MergeResult {
    if project == base || project == updated {
        return MergeResult::Merged(updated.to_vec());
    }
    if updated == base {
        return MergeResult::Merged(project.to_vec());
    }
    if let (Ok(base), Ok(project), Ok(updated)) = (
        std::str::from_utf8(base),
        std::str::from_utf8(project),
        std::str::from_utf8(updated),
    ) {
        let base_lines = base.split_inclusive('\n').collect::<Vec<_>>();
        let local_edits = diff_hunks(
            &base_lines,
            &project.split_inclusive('\n').collect::<Vec<_>>(),
        );
        let template_edits = diff_hunks(
            &base_lines,
            &updated.split_inclusive('\n').collect::<Vec<_>>(),
        );
        if local_edits.iter().all(|local| {
            template_edits
                .iter()
                .all(|template| compatible(local, template))
        }) {
            let mut lines = base_lines;
            let mut edits = local_edits;
            for edit in template_edits {
                if !edits.iter().any(|existing| existing == &edit) {
                    edits.push(edit);
                }
            }
            // Apply edits from the end of the base towards the beginning so their
            // coordinates remain valid. At an equal start, apply a replacement
            // before an insertion: the insertion is then placed in front of the
            // replacement instead of being overwritten by it.
            edits.sort_by_key(|edit| std::cmp::Reverse((edit.start, edit.end)));
            for edit in edits {
                lines.splice(edit.start..edit.end, edit.replacement);
            }
            return MergeResult::Merged(lines.concat().into_bytes());
        }
    }
    let mut marked = b"<<<<<<< project\n".to_vec();
    marked.extend_from_slice(project);
    if !project.ends_with(b"\n") {
        marked.push(b'\n');
    }
    marked.extend_from_slice(b"=======\n");
    marked.extend_from_slice(updated);
    if !updated.ends_with(b"\n") {
        marked.push(b'\n');
    }
    marked.extend_from_slice(b">>>>>>> template\n");
    MergeResult::Conflict(marked)
}

#[derive(PartialEq, Eq)]
struct LineEdit<'a> {
    start: usize,
    end: usize,
    replacement: Vec<&'a str>,
}

fn compatible(left: &LineEdit<'_>, right: &LineEdit<'_>) -> bool {
    let left_insertion = left.start == left.end;
    let right_insertion = right.start == right.end;
    if left_insertion && right_insertion && left.start == right.start {
        return left.replacement == right.replacement;
    }
    if left_insertion || right_insertion {
        let (insertion, changed) = if left_insertion {
            (left, right)
        } else {
            (right, left)
        };
        return insertion.start <= changed.start || insertion.start >= changed.end;
    }
    left.end <= right.start || right.end <= left.start
}

/// Computes all line hunks using an LCS table. Equal runs terminate a hunk, so
/// distinct edits remain independently mergeable instead of becoming one span.
fn diff_hunks<'a>(base: &[&str], changed: &[&'a str]) -> Vec<LineEdit<'a>> {
    let mut lcs = vec![vec![0; changed.len() + 1]; base.len() + 1];
    for i in (0..base.len()).rev() {
        for j in (0..changed.len()).rev() {
            lcs[i][j] = if base[i] == changed[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut hunks = Vec::new();
    while i < base.len() || j < changed.len() {
        if i < base.len() && j < changed.len() && base[i] == changed[j] {
            i += 1;
            j += 1;
            continue;
        }
        let start = i;
        let replacement_start = j;
        while i < base.len() || j < changed.len() {
            if i < base.len() && j < changed.len() && base[i] == changed[j] {
                break;
            }
            if j < changed.len() && (i == base.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
                j += 1;
            } else {
                i += 1;
            }
        }
        hunks.push(LineEdit {
            start,
            end: i,
            replacement: changed[replacement_start..j].to_vec(),
        });
    }
    hunks
}

#[derive(Debug, PartialEq, Eq)]
pub enum MergeResult {
    Merged(Vec<u8>),
    Conflict(Vec<u8>),
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
    fn rejects_an_unterminated_raw_block() {
        let error = render("{% raw %}{{ ignored }}", &answers()).unwrap_err();
        assert!(error.to_string().contains("missing endraw"));
    }

    #[test]
    fn ignores_endif_inside_raw_blocks() {
        let rendered = render(
            "{% if on %}before{% raw %}{% endif %}{{ ignored }}{% endraw %}after{% endif %}",
            &answers(),
        )
        .unwrap();
        assert_eq!(rendered.bytes, b"before{% endif %}{{ ignored }}after");
    }

    #[test]
    fn ignores_endfor_inside_raw_blocks() {
        let rendered = render(
            "{% for letter in name %}{% raw %}{% endfor %}{{ ignored }}{% endraw %}{{ letter }}{% endfor %}",
            &answers(),
        )
        .unwrap();
        assert_eq!(
            rendered.bytes,
            b"{% endfor %}{{ ignored }}A{% endfor %}{{ ignored }}p{% endfor %}{{ ignored }}p"
        );
    }

    #[test]
    fn matches_nested_if_and_for_around_raw_blocks() {
        let rendered = render(
            "{% if on %}{% for letter in name %}{% if on %}{% raw %}{% endif %}{% endfor %}{% endraw %}{{ letter }}{% endif %}{% endfor %}{% endif %}",
            &answers(),
        )
        .unwrap();
        assert_eq!(
            rendered.bytes,
            b"{% endif %}{% endfor %}A{% endif %}{% endfor %}p{% endif %}{% endfor %}p"
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

    #[test]
    fn three_way_merge_preserves_the_only_changed_side() {
        assert_eq!(
            three_way_merge(b"base", b"local", b"base"),
            MergeResult::Merged(b"local".to_vec())
        );
        assert_eq!(
            three_way_merge(b"base", b"base", b"new"),
            MergeResult::Merged(b"new".to_vec())
        );
    }

    #[test]
    fn three_way_merge_marks_competing_changes() {
        let MergeResult::Conflict(bytes) = three_way_merge(b"old\n", b"local\n", b"new\n") else {
            panic!("expected conflict")
        };
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("<<<<<<< project\nlocal\n=======\nnew\n>>>>>>> template"));
    }

    #[test]
    fn three_way_merge_combines_disjoint_edits_in_one_file() {
        assert_eq!(
            three_way_merge(b"one\ntwo\n", b"one\ntwo\nlocal\n", b"ONE\ntwo\n"),
            MergeResult::Merged(b"ONE\ntwo\nlocal\n".to_vec())
        );
    }

    #[test]
    fn three_way_merge_keeps_an_insertion_before_a_replaced_line() {
        assert_eq!(
            three_way_merge(b"a\n", b"x\na\n", b"A\n"),
            MergeResult::Merged(b"x\nA\n".to_vec())
        );
    }

    #[test]
    fn three_way_merge_combines_multiple_local_hunks_around_a_template_hunk() {
        assert_eq!(
            three_way_merge(
                b"one\ntwo\nthree\nfour\nfive\n",
                b"ONE\ntwo\nthree\nfour\nFIVE\n",
                b"one\ntwo\nTHREE\nfour\nfive\n",
            ),
            MergeResult::Merged(b"ONE\ntwo\nTHREE\nfour\nFIVE\n".to_vec())
        );
    }

    #[test]
    fn three_way_merge_conflicts_when_only_one_of_multiple_hunks_overlaps() {
        assert!(matches!(
            three_way_merge(
                b"one\ntwo\nthree\nfour\n",
                b"ONE\ntwo\nlocal three\nfour\n",
                b"one\ntwo\ntemplate three\nFOUR\n",
            ),
            MergeResult::Conflict(_)
        ));
    }

    #[test]
    fn three_way_merge_deduplicates_identical_insertions_and_preserves_utf8_and_eof() {
        assert_eq!(
            three_way_merge(
                "甲\n乙".as_bytes(),
                "甲\n追加\n乙".as_bytes(),
                "甲\n追加\n乙".as_bytes()
            ),
            MergeResult::Merged("甲\n追加\n乙".as_bytes().to_vec())
        );
        assert!(matches!(
            three_way_merge(b"a\n", b"a\nlocal\n", b"a\ntemplate\n"),
            MergeResult::Conflict(_)
        ));
    }

    #[test]
    fn three_way_merge_handles_deletion_and_adjacent_hunks() {
        assert_eq!(
            three_way_merge(b"a\nb\nc\n", b"a\nc\n", b"a\nb\nC\n"),
            MergeResult::Merged(b"a\nC\n".to_vec())
        );
    }

    #[test]
    fn three_way_merge_treats_competing_binary_changes_as_a_conflict() {
        assert!(matches!(
            three_way_merge(&[0, 1], &[0, 2], &[0, 3]),
            MergeResult::Conflict(_)
        ));
    }
}
