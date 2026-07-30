//! Static validation for the named locator catalog (spec #366).

use crate::expr::{Path, PathRoot};
use crate::validate::ValidationError;
use crate::{PageCatalog, VerificationDefinition};

pub(crate) fn validate_catalog_inputs(
    definition: &VerificationDefinition,
    errors: &mut Vec<ValidationError>,
) {
    for (page, elements) in &definition.pages {
        for (element, locator) in elements {
            walk_strings(locator, &mut |raw| {
                let Ok(expression) = crate::expr::parse(raw) else {
                    return;
                };
                expression.walk_paths(|path| {
                    if path.root == PathRoot::Inputs
                        && let Some(input) = path.segments().first()
                        && !definition.inputs.contains_key(input)
                    {
                        errors.push(ValidationError::PageInputUndeclared {
                            verification: definition.verification.clone(),
                            page: page.clone(),
                            element: element.clone(),
                            input: input.clone(),
                        });
                    }
                });
            });
        }
    }
}

pub(crate) fn check_page_path(
    pages: &PageCatalog,
    path: &Path,
    raw: &str,
    site: &str,
    errors: &mut Vec<ValidationError>,
) {
    let segments = path.segments();
    if segments.is_empty() || segments.len() > 2 {
        errors.push(ValidationError::MalformedPageRef {
            site: site.to_string(),
            raw: raw.to_string(),
        });
        return;
    }
    let page = &segments[0];
    let element = segments.get(1);
    if element.is_some_and(|element| {
        pages
            .get(page)
            .is_some_and(|entries| entries.contains_key(element))
    }) {
        return;
    }

    let candidates: Vec<&str> = match (pages.get(page), element) {
        (Some(entries), Some(_)) => entries.keys().map(String::as_str).collect(),
        _ => pages.keys().map(String::as_str).collect(),
    };
    let sought = if pages.contains_key(page) {
        element.unwrap_or(page)
    } else {
        page
    };
    let help = nearest_name(sought, &candidates)
        .map(|name| format!(" — did you mean `{name}`?"))
        .unwrap_or_default();
    errors.push(ValidationError::UnresolvedPageRef {
        site: site.to_string(),
        raw: raw.to_string(),
        entry: element
            .map(|element| format!("$pages.{page}.{element}"))
            .unwrap_or_else(|| format!("$pages.{page}")),
        help,
    });
}

fn walk_strings<F: FnMut(&str)>(value: &serde_yml::Value, visit: &mut F) {
    match value {
        serde_yml::Value::String(raw) if raw.trim_start().starts_with('$') => visit(raw),
        serde_yml::Value::Sequence(values) => {
            for value in values {
                walk_strings(value, visit);
            }
        }
        serde_yml::Value::Mapping(values) => {
            for value in values.values() {
                walk_strings(value, visit);
            }
        }
        _ => {}
    }
}

fn nearest_name<'a>(sought: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let limit = if sought.chars().count() >= 5 { 2 } else { 1 };
    candidates
        .iter()
        .map(|candidate| (*candidate, levenshtein(sought, candidate)))
        .filter(|(_, distance)| *distance <= limit)
        .min_by_key(|(candidate, distance)| (*distance, *candidate))
        .map(|(candidate, _)| candidate)
}

fn levenshtein(left: &str, right: &str) -> usize {
    let mut prior: Vec<usize> = (0..=right.chars().count()).collect();
    let mut current = vec![0; prior.len()];
    for (i, a) in left.chars().enumerate() {
        current[0] = i + 1;
        for (j, b) in right.chars().enumerate() {
            current[j + 1] = (current[j] + 1)
                .min(prior[j + 1] + 1)
                .min(prior[j] + usize::from(a != b));
        }
        std::mem::swap(&mut prior, &mut current);
    }
    prior[right.chars().count()]
}
