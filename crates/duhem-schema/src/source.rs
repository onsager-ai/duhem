//! Source locations retained alongside a parsed Verification Definition.
//!
//! `serde_yml::Value` intentionally contains values only.  Its libyaml
//! parser does expose an exact start mark for every YAML node, so the VD
//! parser records those marks in a sidecar map while the ordinary serde
//! representation remains unchanged.

use std::borrow::Cow;
use std::collections::BTreeMap;

use serde_yml::libyml::error::Mark;
use serde_yml::libyml::parser::{Event, Parser};

use crate::{Assertion, Expr, Step};

/// A one-based source position suitable for editor diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SourcePathSegment {
    Key(String),
    Index(usize),
}

impl SourcePathSegment {
    pub(crate) fn key(value: impl Into<String>) -> Self {
        Self::Key(value.into())
    }

    pub(crate) fn index(value: usize) -> Self {
        Self::Index(value)
    }
}

#[derive(Debug, Clone)]
struct SourceNode {
    location: SourceLocation,
    scalar: Option<String>,
}

/// Stable authored coordinate for a step after flow expansion.
#[derive(Debug, Clone)]
pub(crate) enum StepSourceOrigin {
    AuthoredCheck { step_index: usize },
    FlowDefinition { name: String, step_index: usize },
}

/// Exact YAML-node marks keyed by their structural path.
///
/// This is public only because `VerificationDefinition` is publicly
/// constructible with struct syntax. It is skipped by serde and JSON Schema,
/// and callers should leave it at `Default::default()` for definitions that
/// did not come from YAML source.
#[doc(hidden)]
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    nodes: BTreeMap<Vec<SourcePathSegment>, SourceNode>,
    expanded_step_origins: BTreeMap<(usize, usize, usize), StepSourceOrigin>,
}

// Source provenance is not part of a Verification Definition's semantic
// identity. Keeping it out of equality preserves existing round-trip and
// composition behavior.
impl PartialEq for SourceMap {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl SourceMap {
    pub(crate) fn from_yaml(source: &str) -> Self {
        let mut map = Self::default();
        let mut parser = Parser::new(Cow::Borrowed(source.as_bytes()));

        while let Ok((event, mark)) = parser.parse_next_event() {
            match event {
                Event::MappingStart(_) | Event::SequenceStart(_) | Event::Scalar(_) => {
                    map.record_node(event, mark, &mut parser, &mut Vec::new());
                    break;
                }
                Event::StreamEnd => break,
                _ => {}
            }
        }
        map
    }

    /// Return a mark only when both the structural path and decoded scalar
    /// agree. The value check is what makes locations safe after loaders have
    /// expanded or composed a definition: stale indices fall back instead of
    /// pointing at an unrelated node.
    pub(crate) fn scalar_location(
        &self,
        path: &[SourcePathSegment],
        expected: &str,
    ) -> Option<SourceLocation> {
        self.nodes
            .get(path)
            .and_then(|node| (node.scalar.as_deref() == Some(expected)).then_some(node.location))
    }

    pub(crate) fn check_context_matches(
        &self,
        criterion_index: usize,
        criterion_id: &str,
        check_index: usize,
        check_id: &str,
    ) -> bool {
        let criterion = vec![
            SourcePathSegment::key("criteria"),
            SourcePathSegment::index(criterion_index),
            SourcePathSegment::key("id"),
        ];
        let check = check_path(criterion_index, check_index, "id");
        self.scalar_location(&criterion, criterion_id).is_some()
            && self.scalar_location(&check, check_id).is_some()
    }

    pub(crate) fn check_scalar_location(
        &self,
        criterion_index: usize,
        check_index: usize,
        field: &str,
        expected: &str,
    ) -> Option<SourceLocation> {
        self.scalar_location(&check_path(criterion_index, check_index, field), expected)
    }

    pub(crate) fn record_expanded_step_origins(
        &mut self,
        criterion_index: usize,
        check_index: usize,
        origins: Vec<StepSourceOrigin>,
    ) {
        for (expanded_index, origin) in origins.into_iter().enumerate() {
            self.expanded_step_origins
                .insert((criterion_index, check_index, expanded_index), origin);
        }
    }

    pub(crate) fn step_with_location(
        &self,
        step: &Step,
        criterion_index: usize,
        check_index: usize,
        step_index: usize,
        value_path: &[SourcePathSegment],
        raw: &str,
    ) -> Option<SourceLocation> {
        let steps_index = value_path
            .iter()
            .position(|segment| matches!(segment, SourcePathSegment::Key(key) if key == "steps"))?;
        let with_index = steps_index + 2;
        if !matches!(value_path.get(with_index), Some(SourcePathSegment::Key(key)) if key == "with")
        {
            return None;
        }
        let mut identity_path = value_path[..with_index].to_vec();
        debug_assert_eq!(
            identity_path.last(),
            Some(&SourcePathSegment::index(step_index))
        );
        let mut authored_value_path = value_path.to_vec();
        let flow_definition =
            match self
                .expanded_step_origins
                .get(&(criterion_index, check_index, step_index))
            {
                Some(StepSourceOrigin::AuthoredCheck { step_index }) => {
                    *identity_path.last_mut()? = SourcePathSegment::index(*step_index);
                    authored_value_path[steps_index + 1] = SourcePathSegment::index(*step_index);
                    false
                }
                Some(StepSourceOrigin::FlowDefinition { name, step_index }) => {
                    identity_path = vec![
                        SourcePathSegment::key("flows"),
                        SourcePathSegment::key(name),
                        SourcePathSegment::key("steps"),
                        SourcePathSegment::index(*step_index),
                    ];
                    authored_value_path = identity_path.clone();
                    authored_value_path.extend_from_slice(&value_path[with_index..]);
                    true
                }
                None if step.flow.is_some() => return None,
                None => false,
            };
        let (field, expected) = if flow_definition {
            // Expanded ids are namespaced at each invocation, while `uses:`
            // remains byte-identical to the flow definition. The stable path
            // plus scalar equality proves the definition-site identity.
            ("uses", step.uses.as_deref()?)
        } else if let Some(id) = step.id.as_deref() {
            ("id", id)
        } else if let Some(uses) = step.uses.as_deref() {
            ("uses", uses)
        } else {
            ("call", step.call.as_deref()?)
        };
        identity_path.push(SourcePathSegment::key(field));
        self.scalar_location(&identity_path, expected)?;
        self.scalar_location(&authored_value_path, raw)
    }

    pub(crate) fn assertion_location(
        &self,
        criterion_index: usize,
        check_index: usize,
        assertion_index: usize,
        assertion: &Assertion,
        raw: &str,
    ) -> Option<SourceLocation> {
        let mut path = check_path(criterion_index, check_index, "assertions");
        path.push(SourcePathSegment::index(assertion_index));
        match assertion {
            Assertion::Expr(_) => {}
            Assertion::TypeCheck { .. } | Assertion::Matches { .. } | Assertion::In { .. } => {
                let operator = match assertion {
                    Assertion::TypeCheck { .. } => "type_check",
                    Assertion::Matches { .. } => "matches",
                    Assertion::In { .. } => "in",
                    _ => unreachable!(),
                };
                path.push(SourcePathSegment::key(operator));
                path.push(SourcePathSegment::key("value"));
            }
            Assertion::Exists { .. } => path.push(SourcePathSegment::key("exists")),
            Assertion::Equal { values } => {
                // `walk_exprs` cannot distinguish repeated identical values.
                let mut matches = values
                    .iter()
                    .enumerate()
                    .filter(|(_, value)| value.raw == raw);
                let (index, _) = matches.next()?;
                if matches.next().is_some() {
                    return None;
                }
                path.push(SourcePathSegment::key("equal"));
                path.push(SourcePathSegment::index(index));
            }
        }
        self.scalar_location(&path, raw)
    }

    fn record_node<'input>(
        &mut self,
        event: Event<'input>,
        mark: Mark,
        parser: &mut Parser<'input>,
        path: &mut Vec<SourcePathSegment>,
    ) {
        let location = SourceLocation {
            line: mark.line() as usize + 1,
            column: mark.column() as usize + 1,
        };
        match event {
            Event::Scalar(scalar) => {
                let scalar = String::from_utf8(scalar.value.into_vec()).ok();
                self.nodes
                    .insert(path.clone(), SourceNode { location, scalar });
            }
            Event::SequenceStart(_) => {
                self.nodes.insert(
                    path.clone(),
                    SourceNode {
                        location,
                        scalar: None,
                    },
                );
                let mut index = 0;
                loop {
                    let Ok((child, child_mark)) = parser.parse_next_event() else {
                        return;
                    };
                    if matches!(child, Event::SequenceEnd | Event::StreamEnd) {
                        return;
                    }
                    path.push(SourcePathSegment::Index(index));
                    self.record_node(child, child_mark, parser, path);
                    path.pop();
                    index += 1;
                }
            }
            Event::MappingStart(_) => {
                self.nodes.insert(
                    path.clone(),
                    SourceNode {
                        location,
                        scalar: None,
                    },
                );
                loop {
                    let Ok((key_event, key_mark)) = parser.parse_next_event() else {
                        return;
                    };
                    if matches!(key_event, Event::MappingEnd | Event::StreamEnd) {
                        return;
                    }
                    let key = match key_event {
                        Event::Scalar(key) => String::from_utf8(key.value.into_vec()).ok(),
                        other => {
                            // Complex mapping keys are not part of the VD schema. Consume
                            // the node so parsing stays aligned, but record no descendants
                            // under a fabricated key.
                            self.record_node(other, key_mark, parser, &mut Vec::new());
                            None
                        }
                    };
                    let Ok((value, value_mark)) = parser.parse_next_event() else {
                        return;
                    };
                    if let Some(key) = key {
                        path.push(SourcePathSegment::Key(key));
                        self.record_node(value, value_mark, parser, path);
                        path.pop();
                    } else {
                        self.record_node(value, value_mark, parser, &mut Vec::new());
                    }
                }
            }
            // Aliases do not expose the aliased scalar through this event API.
            // Omitting a mark is safer than pointing at the alias or anchor when
            // the precise offending value cannot be proven.
            Event::Alias(_)
            | Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart
            | Event::DocumentEnd
            | Event::SequenceEnd
            | Event::MappingEnd => {}
        }
    }
}

pub(crate) fn check_path(
    criterion_index: usize,
    check_index: usize,
    field: &str,
) -> Vec<SourcePathSegment> {
    vec![
        SourcePathSegment::key("criteria"),
        SourcePathSegment::index(criterion_index),
        SourcePathSegment::key("checks"),
        SourcePathSegment::index(check_index),
        SourcePathSegment::key(field),
    ]
}

/// Walk substitutable expression strings inside an untyped `with:` value
/// while retaining the structural YAML path to each scalar.
pub(crate) fn walk_with_refs<F: FnMut(&Expr, &str, &[SourcePathSegment])>(
    with: &serde_yml::Value,
    source_path: &mut Vec<SourcePathSegment>,
    visit: &mut F,
) {
    match with {
        serde_yml::Value::String(raw) => {
            if raw.trim_start().starts_with('$')
                && let Ok(expression) = crate::expr::parse(raw)
                && matches!(expression, Expr::Path(_) | Expr::Call { .. })
            {
                visit(&expression, raw, source_path);
            }
        }
        serde_yml::Value::Sequence(values) => {
            for (index, value) in values.iter().enumerate() {
                source_path.push(SourcePathSegment::index(index));
                walk_with_refs(value, source_path, visit);
                source_path.pop();
            }
        }
        serde_yml::Value::Mapping(values) => {
            for (key, value) in values {
                let Some(key) = key.as_str() else {
                    continue;
                };
                source_path.push(SourcePathSegment::key(key));
                walk_with_refs(value, source_path, visit);
                source_path.pop();
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_exact_nested_scalar_mark() {
        let map = SourceMap::from_yaml("root:\n  - with:\n      url: $inputs.MISSING\n");
        let path = [
            SourcePathSegment::key("root"),
            SourcePathSegment::index(0),
            SourcePathSegment::key("with"),
            SourcePathSegment::key("url"),
        ];
        assert_eq!(
            map.scalar_location(&path, "$inputs.MISSING"),
            Some(SourceLocation {
                line: 3,
                column: 12,
            })
        );
        assert_eq!(map.scalar_location(&path, "$inputs.OTHER"), None);
    }
}
