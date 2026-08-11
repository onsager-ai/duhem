//! Static validation for the closed `$runtime` helper catalog.

use crate::SourceLocation;
use crate::expr::{Expr, Path, PathRoot, RuntimeHelper, authored_runtime_helper_names};
use crate::validate::ValidationError;

pub(crate) fn check_runtime_path(
    path: &Path,
    arity: Option<usize>,
    raw: &str,
    site: &str,
    location: Option<SourceLocation>,
    errors: &mut Vec<ValidationError>,
) {
    let helper_name = path.segments().join(".");
    let Some(helper) = RuntimeHelper::named(&helper_name) else {
        let candidates: Vec<&str> = authored_runtime_helper_names().collect();
        let help = crate::validate_pages::nearest_name(&helper_name, &candidates)
            .map(|name| format!(" — did you mean `{name}`?"))
            .unwrap_or_default();
        errors.push(ValidationError::UnknownRuntimeHelper {
            site: site.to_string(),
            raw: raw.to_string(),
            helper: helper_name,
            help,
            location,
        });
        return;
    };
    let Some(given) = arity else {
        let args = if helper.arity().accepts(0) { "" } else { "..." };
        errors.push(ValidationError::UncalledRuntimeHelper {
            site: site.to_string(),
            raw: raw.to_string(),
            helper: helper_name,
            call_form: format!("$runtime.{}({args})", helper.name()),
            location,
        });
        return;
    };
    if !helper.arity().accepts(given) {
        errors.push(ValidationError::WrongRuntimeHelperArity {
            site: site.to_string(),
            raw: raw.to_string(),
            helper: helper.name().to_string(),
            given,
            argument_word: if given == 1 { "argument" } else { "arguments" },
            expected: helper.arity().expected(),
            location,
        });
    }
}

/// Walk references while preserving `default()`'s missing-value carve-out.
pub(crate) fn walk_checkable_paths<F: FnMut(&Path, Option<usize>)>(expr: &Expr, visit: &mut F) {
    match expr {
        Expr::Lit(_) => {}
        Expr::Path(path) => visit(path, None),
        Expr::Call { path, args } => {
            visit(path, Some(args.len()));
            let skip_first = path.root == PathRoot::Runtime
                && path.segments().len() == 1
                && path.segments()[0] == "default";
            for (index, arg) in args.iter().enumerate() {
                if skip_first && index == 0 {
                    walk_runtime_calls(arg, visit);
                } else {
                    walk_checkable_paths(arg, visit);
                }
            }
        }
        Expr::BinOp { lhs, rhs, .. } => {
            walk_checkable_paths(lhs, visit);
            walk_checkable_paths(rhs, visit);
        }
        Expr::UnaryOp { expr, .. } => walk_checkable_paths(expr, visit),
    }
}

fn walk_runtime_calls<F: FnMut(&Path, Option<usize>)>(expr: &Expr, visit: &mut F) {
    match expr {
        Expr::Lit(_) | Expr::Path(_) => {}
        Expr::Call { path, args } => {
            visit(path, Some(args.len()));
            for arg in args {
                walk_runtime_calls(arg, visit);
            }
        }
        Expr::BinOp { lhs, rhs, .. } => {
            walk_runtime_calls(lhs, visit);
            walk_runtime_calls(rhs, visit);
        }
        Expr::UnaryOp { expr, .. } => walk_runtime_calls(expr, visit),
    }
}
