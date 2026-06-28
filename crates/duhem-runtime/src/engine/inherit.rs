//! Pre-flight guard for inherited inputs (spec #135).
//!
//! A leaf may declare `inherits: [name, ...]` — input names it pulls
//! from the parent manifest's resolution chain (#68) instead of
//! redeclaring under `inputs:`. When such a name is *referenced* by a
//! `$inputs.<name>` but the chain bound nothing for it (no manifest
//! environment selected, no `--inputs`), the run must fail loudly and
//! specifically rather than as a generic deep failure. This module
//! collects the referenced inherited names and reports the first one
//! that is unbound; the engine turns that into an
//! `EngineError::UnresolvedInheritedInput` before any check runs.

use std::collections::{BTreeMap, HashSet};

use duhem_schema::{Expr, PathRoot, VerificationDefinition};

use crate::eval::Value;

/// Return the first `inherits:` name that the VD references via
/// `$inputs.<name>` but that the resolution chain left unbound
/// (`bound` is the run's resolved input map). `None` when every
/// referenced inherited name is bound — the common path when a manifest
/// environment or `--inputs` supplied it.
///
/// References are read from assertions and step `with:` payloads, with
/// the same `$runtime.default(value, fallback)` carve-out the validator
/// uses: a missing ref in `default()`'s first argument is the author's
/// explicit "may be absent" escape hatch, so it does not trip the guard.
pub(crate) fn first_unbound_inherited(
    def: &VerificationDefinition,
    inherited: &HashSet<String>,
    bound: &BTreeMap<String, Value>,
) -> Option<String> {
    let mut hit: Option<String> = None;
    let mut consider = |name: &str| {
        if hit.is_none() && inherited.contains(name) && !bound.contains_key(name) {
            hit = Some(name.to_string());
        }
    };
    for criterion in &def.criteria {
        for check in &criterion.checks {
            for assertion in &check.assertions {
                assertion.walk_exprs(|e| visit_input_refs(&e.parsed, &mut consider));
            }
            for step in &check.steps {
                walk_with_input_refs(&step.with, &mut consider);
            }
        }
    }
    hit
}

/// Walk an `Expr`, invoking `visit(name)` for every `$inputs.<name>`
/// reference, skipping `$runtime.default(...)`'s first argument.
fn visit_input_refs<F: FnMut(&str)>(expr: &Expr, visit: &mut F) {
    match expr {
        Expr::Lit(_) => {}
        Expr::Path(p) => {
            if p.root == PathRoot::Inputs
                && let Some(name) = p.segments().first()
            {
                visit(name);
            }
        }
        Expr::Call { path, args } => {
            if path.root == PathRoot::Inputs
                && let Some(name) = path.segments().first()
            {
                visit(name);
            }
            let skip_first = path.root == PathRoot::Runtime
                && path.segments().len() == 1
                && path.segments()[0] == "default";
            for (i, a) in args.iter().enumerate() {
                if skip_first && i == 0 {
                    continue;
                }
                visit_input_refs(a, visit);
            }
        }
        Expr::BinOp { lhs, rhs, .. } => {
            visit_input_refs(lhs, visit);
            visit_input_refs(rhs, visit);
        }
        Expr::UnaryOp { expr, .. } => visit_input_refs(expr, visit),
    }
}

/// Walk a step's untyped `with:` tree for substitutable `$inputs.*`
/// references — mirrors the validator's `walk_with_refs` so the guard
/// sees the same refs the runtime would later try to substitute.
fn walk_with_input_refs<F: FnMut(&str)>(with: &serde_yml::Value, visit: &mut F) {
    match with {
        serde_yml::Value::String(s) => {
            if s.trim_start().starts_with('$')
                && let Ok(expr) = duhem_schema::expr::parse(s)
                && matches!(expr, Expr::Path(_) | Expr::Call { .. })
            {
                visit_input_refs(&expr, visit);
            }
        }
        serde_yml::Value::Sequence(seq) => {
            for v in seq {
                walk_with_input_refs(v, visit);
            }
        }
        serde_yml::Value::Mapping(map) => {
            for (_k, v) in map {
                walk_with_input_refs(v, visit);
            }
        }
        _ => {}
    }
}
