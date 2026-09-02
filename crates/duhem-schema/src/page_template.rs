//! Brace-template grammar for parameterized page locators (spec #495).
//!
//! `{}` consumes one positional argument; `{{` and `}}` emit literal
//! braces. Any other brace is rejected so a selector typo cannot pass
//! validation as literal text.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct TemplateError {
    message: String,
}

impl TemplateError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Count unescaped `{}` placeholders in one string while validating
/// literal-brace escaping.
pub fn string_placeholder_count(template: &str) -> Result<usize, TemplateError> {
    let mut chars = template.char_indices().peekable();
    let mut placeholders = 0usize;
    while let Some((offset, ch)) = chars.next() {
        match ch {
            '{' => match chars.next() {
                Some((_, '{')) => {}
                Some((_, '}')) => placeholders += 1,
                _ => {
                    return Err(TemplateError::new(format!(
                        "unescaped `{{` at byte {offset}; use `{{{{` for a literal brace"
                    )));
                }
            },
            '}' => match chars.next() {
                Some((_, '}')) => {}
                _ => {
                    return Err(TemplateError::new(format!(
                        "unescaped `}}` at byte {offset}; use `}}}}` for a literal brace"
                    )));
                }
            },
            _ => {}
        }
    }
    Ok(placeholders)
}

/// Count placeholders across every string value in a YAML node.
/// Mapping keys are structural names, not locator string values.
pub fn value_placeholder_count(value: &serde_yml::Value) -> Result<usize, TemplateError> {
    match value {
        serde_yml::Value::String(template) => string_placeholder_count(template),
        serde_yml::Value::Sequence(values) => values.iter().try_fold(0usize, |total, value| {
            Ok(total + value_placeholder_count(value)?)
        }),
        serde_yml::Value::Mapping(values) => values.values().try_fold(0usize, |total, value| {
            Ok(total + value_placeholder_count(value)?)
        }),
        _ => Ok(0),
    }
}

/// Fill and unescape one string using the next entries in `parts`.
fn fill_string<'a>(
    template: &str,
    parts: &mut impl Iterator<Item = &'a str>,
) -> Result<String, TemplateError> {
    let mut chars = template.char_indices().peekable();
    let mut out = String::with_capacity(template.len());
    while let Some((offset, ch)) = chars.next() {
        match ch {
            '{' => match chars.next() {
                Some((_, '{')) => out.push('{'),
                Some((_, '}')) => out.push_str(
                    parts
                        .next()
                        .ok_or_else(|| TemplateError::new("not enough placeholder arguments"))?,
                ),
                _ => {
                    return Err(TemplateError::new(format!(
                        "unescaped `{{` at byte {offset}; use `{{{{` for a literal brace"
                    )));
                }
            },
            '}' => match chars.next() {
                Some((_, '}')) => out.push('}'),
                _ => {
                    return Err(TemplateError::new(format!(
                        "unescaped `}}` at byte {offset}; use `}}}}` for a literal brace"
                    )));
                }
            },
            _ => out.push(ch),
        }
    }
    Ok(out)
}

/// Fill one string after enforcing exact arity.
pub fn fill_string_placeholders(template: &str, parts: &[String]) -> Result<String, TemplateError> {
    let placeholders = string_placeholder_count(template)?;
    if placeholders != parts.len() {
        return Err(TemplateError::new(format!(
            "template has {placeholders} `{{}}` placeholder(s) but {} argument(s) were given",
            parts.len()
        )));
    }
    fill_string(template, &mut parts.iter().map(String::as_str))
}

/// Fill placeholders across every string value in a YAML node. Arguments
/// are consumed in deterministic YAML traversal order (spec #495).
pub fn fill_value_placeholders(
    value: &mut serde_yml::Value,
    parts: &[String],
) -> Result<(), TemplateError> {
    let placeholders = value_placeholder_count(value)?;
    if placeholders != parts.len() {
        return Err(TemplateError::new(format!(
            "template has {placeholders} `{{}}` placeholder(s) but {} argument(s) were given",
            parts.len()
        )));
    }

    fn fill<'a>(
        value: &mut serde_yml::Value,
        parts: &mut impl Iterator<Item = &'a str>,
    ) -> Result<(), TemplateError> {
        match value {
            serde_yml::Value::String(template) => {
                *template = fill_string(template, parts)?;
            }
            serde_yml::Value::Sequence(values) => {
                for value in values {
                    fill(value, parts)?;
                }
            }
            serde_yml::Value::Mapping(values) => {
                for value in values.values_mut() {
                    fill(value, parts)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fill(value, &mut parts.iter().map(String::as_str))
}

/// Whether filling can change a string, used by the runtime to keep an
/// inserted argument from becoming a second expression evaluation.
pub fn has_template_syntax(value: &str) -> bool {
    value.contains(['{', '}'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_placeholders_without_counting_escaped_braces() {
        assert_eq!(string_placeholder_count("{{row}}[{}]").unwrap(), 1);
    }

    #[test]
    fn fills_and_unescapes_braces() {
        assert_eq!(
            fill_string_placeholders("{{row}}[{}]", &["2".into()]).unwrap(),
            "{row}[2]"
        );
    }

    #[test]
    fn fills_every_string_in_yaml_order() {
        let mut value: serde_yml::Value =
            serde_yml::from_str("first: 'row {}'\nnested:\n  - '{{literal}}'\n  - 'item {}'\n")
                .unwrap();
        fill_value_placeholders(&mut value, &["one".into(), "two".into()]).unwrap();
        assert_eq!(value["first"].as_str(), Some("row one"));
        assert_eq!(value["nested"][0].as_str(), Some("{literal}"));
        assert_eq!(value["nested"][1].as_str(), Some("item two"));
    }

    #[test]
    fn rejects_unescaped_literal_braces() {
        assert!(string_placeholder_count("[data={value}]").is_err());
        assert!(string_placeholder_count("[data=value}]").is_err());
    }
}
