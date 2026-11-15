//! Dry-run presentation, kept outside the execution dispatcher.
use crate::filter::{CliCheckFilter, native_selection};
use crate::resolve::render_input_value;
use crate::run_cmd::ResolvedLeaf;
use crate::run_scope::Scope;
use crate::shard_plan::Plan;
use std::io::Write;
use std::process::ExitCode;

pub(crate) fn render(
    resolved: &[ResolvedLeaf],
    scope: &Scope,
    check_filter: Option<&CliCheckFilter>,
    shard_plan: Option<&Plan>,
) -> ExitCode {
    let is_manifest = matches!(scope, Scope::Manifest { .. });
    // Dry-run is a plan inspection, not a recorded run, so it has
    // no three-state verdict in which to classify an unavailable
    // required input. Preserve its existing fail-fast diagnostic;
    // real runs continue below and record EnvironmentError.
    for (_name, path, definition, inputs, _secrets) in resolved {
        if let Some(missing) = definition
            .inputs
            .iter()
            .filter(|(_, decl)| !decl.inherit)
            .map(|(name, _)| name)
            .find(|name| !inputs.contains_key(*name))
        {
            eprintln!("{}: missing required input: `{missing}`", path.display());
            return ExitCode::FAILURE;
        }
    }
    let mut stdout = std::io::stdout().lock();
    let mut wrote = false;
    for (name, leaf_path, def, _inputs, _secrets) in resolved {
        let shard_criteria = shard_plan.map(|plan| {
            plan.criteria(
                &crate::shard_plan::Plan::key(leaf_path, scope),
                def.criteria.iter().map(|criterion| criterion.id.clone()),
            )
        });
        let leaf_filter = check_filter.and_then(|f| {
            if f.has_criterion_filters() {
                f.for_verification(name)
            } else {
                Some(CliCheckFilter::matches_nothing())
            }
        });
        // If a filter was passed and nothing scopes to this leaf,
        // skip — no spurious "no checks matched" line per-leaf.
        if check_filter.is_some() && leaf_filter.is_none() {
            continue;
        }
        for criterion in &def.criteria {
            for check in &criterion.checks {
                let matched = match &leaf_filter {
                    None => true,
                    Some(f) => {
                        use duhem_runtime::CheckFilter;
                        f.matches(&criterion.id, &check.id)
                    }
                };
                if matched
                    && shard_criteria
                        .as_ref()
                        .is_none_or(|owned| owned.contains(&criterion.id))
                {
                    let line = if is_manifest {
                        format!("WOULD RUN: {}::{}::{}", name, criterion.id, check.id)
                    } else {
                        format!("WOULD RUN: {}::{}", criterion.id, check.id)
                    };
                    if let Err(e) = writeln!(stdout, "{line}") {
                        eprintln!("dry-run: {e}");
                        return ExitCode::FAILURE;
                    }
                    wrote = true;
                }
            }
        }
    }
    if let Scope::Manifest { native, .. } = scope {
        let (mut selected, mut excluded) = native_selection(check_filter, native);
        if let Some(plan) = shard_plan {
            plan.native("$manifest", &mut selected, &mut excluded);
        }
        // Preserve ordinary dry-run output; root-native plan lines are new only
        // for the distributed surface, where the partition must be inspectable.
        if shard_plan.is_some() {
            for run in selected {
                if let Err(error) = writeln!(stdout, "WOULD RUN NATIVE: $manifest::{}", run.id) {
                    eprintln!("dry-run: {error}");
                    return ExitCode::FAILURE;
                }
                wrote = true;
            }
        }
    }
    for (name, leaf_path, definition, _inputs, _secrets) in resolved {
        let (mut selected, mut excluded) = native_selection(check_filter, &definition.native);
        if let Some(plan) = &shard_plan {
            plan.native(
                &crate::shard_plan::Plan::key(leaf_path, scope),
                &mut selected,
                &mut excluded,
            );
        }
        for run in selected {
            let line = if is_manifest {
                format!("WOULD RUN NATIVE: {name}::{}", run.id)
            } else {
                format!("WOULD RUN NATIVE: {}", run.id)
            };
            if let Err(e) = writeln!(stdout, "{line}") {
                eprintln!("dry-run: {e}");
                return ExitCode::FAILURE;
            }
            wrote = true;
        }
        if !excluded.is_empty() {
            let line = format!("EXCLUDED NATIVE: {}", excluded.join(", "));
            if let Err(e) = writeln!(stdout, "{line}") {
                eprintln!("dry-run: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    if !wrote && let Err(e) = writeln!(stdout, "WOULD RUN: (no checks matched filter)") {
        eprintln!("dry-run: {e}");
        return ExitCode::FAILURE;
    }
    // RESOLVED INPUTS (spec #155): the post-precedence input map
    // (`--inputs` last-wins > selected profile > VD `default:`),
    // one `name = value` per line so a black-box VD can assert the
    // winning value directly off stdout — the value-level assertion
    // that was only reachable indirectly before (via type levers).
    // Qualified by verification name on manifest runs, mirroring the
    // `WOULD RUN` lines. Values render deterministically (strings
    // bare, other types as compact JSON of the coerced value).
    for (name, _path, _def, inputs, secrets) in resolved {
        let leaf_filter = check_filter.and_then(|f| {
            if f.has_criterion_filters() {
                f.for_verification(name)
            } else {
                Some(CliCheckFilter::matches_nothing())
            }
        });
        if check_filter.is_some() && leaf_filter.is_none() {
            continue;
        }
        if inputs.is_empty() {
            let line = if is_manifest {
                format!("RESOLVED INPUT: {name}:: (none)")
            } else {
                "RESOLVED INPUT: (none)".to_string()
            };
            if let Err(e) = writeln!(stdout, "{line}") {
                eprintln!("dry-run: {e}");
                return ExitCode::FAILURE;
            }
            continue;
        }
        for (key, value) in inputs {
            let rendered = secrets.mask(&render_input_value(value)).text;
            let line = if is_manifest {
                format!("RESOLVED INPUT: {name}::{key} = {rendered}")
            } else {
                format!("RESOLVED INPUT: {key} = {rendered}")
            };
            if let Err(e) = writeln!(stdout, "{line}") {
                eprintln!("dry-run: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Err(e) = stdout.flush() {
        eprintln!("dry-run: {e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
