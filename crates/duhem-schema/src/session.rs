//! Preserve an explicit signed-out declaration separately from omission.
use serde::{Deserialize, Deserializer};

pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

use crate::source::SourcePathSegment as S;
use crate::{Expr, PathRoot, Step, ValidationError, VerificationDefinition};

pub(crate) fn validate_scopes(
    v: &VerificationDefinition,
    outputs_for: &dyn Fn(&str) -> Vec<String>,
    errors: &mut Vec<ValidationError>,
) {
    validate_scalar(
        v,
        &v.session,
        &[],
        &[S::key("session")],
        outputs_for,
        errors,
    );
    for (ci, criterion) in v.criteria.iter().enumerate() {
        validate_scalar(
            v,
            &criterion.session,
            &v.setup,
            &[S::key("criteria"), S::index(ci), S::key("session")],
            outputs_for,
            errors,
        );
    }
}

fn validate_scalar(
    v: &VerificationDefinition,
    declaration: &Option<Option<String>>,
    enclosing_setup: &[Step],
    location_path: &[S],
    outputs_for: &dyn Fn(&str) -> Vec<String>,
    errors: &mut Vec<ValidationError>,
) {
    let Some(Some(raw)) = declaration else { return };
    let valid = match crate::expr::parse(raw) {
        Ok(Expr::Path(path)) => match path.root {
            PathRoot::Inputs => path
                .segments
                .first()
                .is_some_and(|name| v.inputs.contains_key(name)),
            PathRoot::Setup if location_path.len() > 1 => {
                let s = &path.segments;
                s.len() >= 3
                    && s[1] == "outputs"
                    && enclosing_setup.iter().any(|step| {
                        step.id.as_ref() == s.first()
                            && (step.outputs.contains_key(&s[2])
                                || step
                                    .uses
                                    .as_deref()
                                    .map(outputs_for)
                                    .unwrap_or_default()
                                    .contains(&s[2]))
                    })
            }
            _ => false,
        },
        _ => false,
    };
    if !valid {
        errors.push(ValidationError::InvalidNamedSession {
            message: format!("session `{raw}` must reference a declared input{}; a scope cannot read its own setup outputs", if location_path.len() > 1 { " or a leaf setup output" } else { "" }),
            location: v.source_map.scalar_location(location_path, raw),
        });
    }
}
