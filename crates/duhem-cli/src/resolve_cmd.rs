//! Read-only composition view for `duhem resolve` (spec #370).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::ValueEnum;
use duhem_runtime::engine::context::json_to_value;
use duhem_runtime::engine::template::substitute_with;
use duhem_runtime::engine::translate::apply_default_timeout;
use duhem_runtime::{RunContext, RunState};
use duhem_schema::{
    InputDecl, Loaded, LoadedLeaf, ManifestDefaults, VerificationDefinition,
    validate_with_contract_outputs,
};
use serde::Serialize;

use crate::inputs::{self, InputValue};
use crate::profile;
use crate::resolve::resolve_leaf_inputs;
use crate::resolve_provenance::{
    ManifestOrigins, Origin, ValueProvenance, collect_leaf_page_origins, collect_manifest_origins,
    input_override_origins, origin,
};

const MASK: &str = "••••••";

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum ResolveFormat {
    Yaml,
    Json,
}

pub(crate) struct ResolveArgs {
    pub path: Option<PathBuf>,
    pub profile: Option<String>,
    pub inputs: Vec<String>,
    pub format: ResolveFormat,
    pub provenance: bool,
}

#[derive(Serialize)]
struct ResolveOutput {
    schema_version: &'static str,
    source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    errors: Vec<ResolveError>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    verifications: Vec<ResolvedVerification>,
}

#[derive(Serialize)]
struct ResolveError {
    stage: &'static str,
    message: String,
}

#[derive(Serialize)]
struct ResolvedVerification {
    source: String,
    document: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    errors: Vec<ResolveError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    provenance: Option<BTreeMap<String, ValueProvenance>>,
}

pub(crate) fn run(args: ResolveArgs) -> ExitCode {
    match resolve(args) {
        Ok((format, output)) => match render(format, &output) {
            Ok(text) => {
                print!("{text}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("resolve: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn resolve(args: ResolveArgs) -> Result<(ResolveFormat, ResolveOutput), String> {
    let cwd =
        std::env::current_dir().map_err(|e| format!("cannot determine current directory: {e}"))?;
    let target = duhem_schema::discover(args.path.as_deref(), &cwd)
        .map_err(|e| format!("[schema v{}] {e}", duhem_schema::SCHEMA_VERSION))?;
    let merged = inputs::merge_inputs(&args.inputs)?;
    let cli_origins = input_override_origins(&args.inputs);

    let loaded = match duhem_schema::load(&target) {
        Ok(loaded) => loaded,
        Err(error) => {
            return Ok((
                args.format,
                ResolveOutput {
                    schema_version: duhem_schema::SCHEMA_VERSION,
                    source: target.display().to_string(),
                    profile: args.profile,
                    warnings: Vec::new(),
                    errors: vec![ResolveError {
                        stage: "load",
                        message: error.to_string(),
                    }],
                    verifications: Vec::new(),
                },
            ));
        }
    };

    let (
        leaves,
        manifest_path,
        manifest_inputs,
        defaults,
        profile_values,
        profile_env,
        selected_name,
        warnings,
        manifest_origins,
    ) = match loaded {
        Loaded::Leaf { path, definition } => {
            let warning = args.profile.as_ref().map(|_| {
                "--profile has no effect for a single Verification Definition".to_string()
            });
            (
                vec![LoadedLeaf { path, definition }],
                None,
                BTreeMap::new(),
                None,
                BTreeMap::new(),
                BTreeMap::new(),
                None,
                warning.into_iter().collect(),
                ManifestOrigins::default(),
            )
        }
        Loaded::Manifest {
            manifest_path,
            manifest,
            leaves,
            warnings,
        } => {
            let selected = profile::select_profile(&manifest.profiles, args.profile.as_deref())?;
            let values = selected
                .as_ref()
                .map(|selected| selected.inputs.clone())
                .unwrap_or_default();
            let env = selected
                .as_ref()
                .map(|selected| selected.env.clone())
                .unwrap_or_default();
            let name = selected.as_ref().map(|selected| selected.name.clone());
            let origins = collect_manifest_origins(&manifest_path);
            (
                leaves,
                Some(manifest_path),
                manifest.inputs,
                manifest.defaults,
                values,
                env,
                name,
                warnings,
                origins,
            )
        }
    };

    let mut verifications = Vec::with_capacity(leaves.len());
    for leaf in leaves {
        verifications.push(resolve_leaf(
            leaf,
            &merged,
            &cli_origins,
            &profile_values,
            &profile_env,
            selected_name.as_deref(),
            &manifest_inputs,
            defaults.as_ref(),
            manifest_path.as_deref(),
            &manifest_origins,
            args.provenance,
        ));
    }

    Ok((
        args.format,
        ResolveOutput {
            schema_version: duhem_schema::SCHEMA_VERSION,
            source: target.display().to_string(),
            profile: selected_name,
            warnings,
            errors: Vec::new(),
            verifications,
        },
    ))
}

#[allow(clippy::too_many_arguments)]
fn resolve_leaf(
    leaf: LoadedLeaf,
    merged: &BTreeMap<String, InputValue>,
    cli_origins: &BTreeMap<String, ValueProvenance>,
    profile_values: &BTreeMap<String, serde_json::Value>,
    profile_env: &BTreeMap<String, String>,
    profile_name: Option<&str>,
    manifest_inputs: &BTreeMap<String, InputDecl>,
    defaults: Option<&ManifestDefaults>,
    manifest_path: Option<&Path>,
    manifest_origins: &ManifestOrigins,
    include_provenance: bool,
) -> ResolvedVerification {
    let mut errors = Vec::new();
    let candidate_secrets =
        candidate_secret_registry(&leaf.definition, manifest_inputs, merged, profile_values);
    if let Err(validation) = validate_with_contract_outputs(&leaf.definition, &|uses| {
        crate::contract_check::contract_outputs(uses)
    }) {
        errors.extend(validation.into_iter().map(|error| ResolveError {
            stage: "validation",
            message: error.to_string(),
        }));
    }

    let (resolved_inputs, mut secrets) =
        match resolve_leaf_inputs(merged, profile_values, &leaf.definition, manifest_inputs) {
            Ok(resolved) => resolved,
            Err(error) => {
                errors.push(ResolveError {
                    stage: "input_resolution",
                    message: redact_error(&candidate_secrets, &error),
                });
                (BTreeMap::new(), duhem_evidence::SecretRegistry::new())
            }
        };
    crate::resolve::register_flow_secrets(&mut secrets, &leaf.definition, Some(&resolved_inputs));

    let mut provenance = BTreeMap::new();
    for name in resolved_inputs.keys() {
        let value = input_provenance(
            name,
            &leaf.path,
            &leaf.definition.inputs,
            merged,
            cli_origins,
            profile_values,
            profile_name,
            manifest_inputs,
            manifest_origins,
        );
        provenance.insert(format!("inputs.{name}"), value);
    }
    let leaf_page_origins = collect_leaf_page_origins(&leaf.path);
    for (page, elements) in &leaf.definition.pages {
        for element in elements.keys() {
            let key = (page.clone(), element.clone());
            let mut value = leaf_page_origins
                .get(&key)
                .cloned()
                .or_else(|| manifest_origins.pages.get(&key).cloned())
                .unwrap_or_else(|| ValueProvenance {
                    rung: "page catalog".to_string(),
                    origin: origin(&leaf.path, &["pages", page, element]),
                    overridden: Vec::new(),
                });
            if leaf_page_origins.contains_key(&key)
                && let Some(inherited) = manifest_origins.pages.get(&key)
            {
                value.overridden.push(inherited.origin.clone());
                value.overridden.extend(inherited.overridden.clone());
            }
            provenance.insert(format!("pages.{page}.{element}"), value);
        }
    }

    let mut definition = leaf.definition;
    resolve_steps(
        &mut definition,
        &resolved_inputs,
        profile_env,
        defaults,
        manifest_path,
        &mut provenance,
        &mut errors,
    );

    let mut document = serde_json::to_value(&definition).unwrap_or_else(|error| {
        errors.push(ResolveError {
            stage: "serialization",
            message: error.to_string(),
        });
        serde_json::Value::Null
    });
    if let Some(object) = document.as_object_mut() {
        object.insert(
            "inputs".to_string(),
            serde_json::Value::Object(
                resolved_inputs
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone()))
                    .collect(),
            ),
        );
    }
    mask_document(&secrets, &mut document);

    ResolvedVerification {
        source: leaf.path.display().to_string(),
        document,
        errors,
        provenance: include_provenance.then_some(provenance),
    }
}

fn resolve_steps(
    definition: &mut VerificationDefinition,
    inputs: &BTreeMap<String, serde_json::Value>,
    env: &BTreeMap<String, String>,
    defaults: Option<&ManifestDefaults>,
    manifest_path: Option<&Path>,
    provenance: &mut BTreeMap<String, ValueProvenance>,
    errors: &mut Vec<ResolveError>,
) {
    let values = inputs
        .iter()
        .filter_map(|(name, value)| json_to_value(value).map(|value| (name.clone(), value)))
        .collect();
    let state = RunState::new_with_seed(values, 0)
        .with_env(env.clone())
        .with_pages(&definition.pages);
    let context = RunContext::new(&state);
    for (index, step) in definition.setup.iter_mut().enumerate() {
        resolve_with(
            &mut step.with,
            &context,
            &format!("setup.{index}.with"),
            provenance,
            errors,
        );
        apply_timeout(
            &mut step.with,
            defaults,
            manifest_path,
            &format!("setup.{index}.with.timeout"),
            provenance,
        );
    }
    for (criterion_index, criterion) in definition.criteria.iter_mut().enumerate() {
        for (check_index, check) in criterion.checks.iter_mut().enumerate() {
            for (step_index, step) in check.steps.iter_mut().enumerate() {
                let path = format!(
                    "criteria.{criterion_index}.checks.{check_index}.steps.{step_index}.with"
                );
                if let Some(flow) = &step.flow {
                    provenance.insert(
                        format!(
                            "criteria.{criterion_index}.checks.{check_index}.steps.{step_index}.flow"
                        ),
                        ValueProvenance {
                            rung: "flow expansion".to_string(),
                            origin: Origin {
                                source: format!(
                                    "flow `{}` invocation `{}` inner step {}",
                                    flow.name, flow.invocation, flow.inner_index
                                ),
                                line: None,
                            },
                            overridden: Vec::new(),
                        },
                    );
                }
                resolve_with(&mut step.with, &context, &path, provenance, errors);
                apply_timeout(
                    &mut step.with,
                    defaults,
                    manifest_path,
                    &format!("{path}.timeout"),
                    provenance,
                );
            }
        }
    }
}

fn resolve_with(
    value: &mut serde_yml::Value,
    context: &RunContext<'_>,
    path: &str,
    provenance: &mut BTreeMap<String, ValueProvenance>,
    errors: &mut Vec<ResolveError>,
) {
    match value {
        serde_yml::Value::Mapping(map) => {
            for (key, child) in map {
                let key = key.as_str().unwrap_or("?");
                resolve_with(child, context, &format!("{path}.{key}"), provenance, errors);
            }
        }
        serde_yml::Value::Sequence(sequence) => {
            for (index, child) in sequence.iter_mut().enumerate() {
                resolve_with(
                    child,
                    context,
                    &format!("{path}.{index}"),
                    provenance,
                    errors,
                );
            }
        }
        serde_yml::Value::String(raw) => {
            let expression = raw.trim().to_string();
            if !expression.starts_with('$') {
                return;
            }
            let original = serde_yml::Value::String(raw.clone());
            let mut resolved = original.clone();
            match substitute_with(&mut resolved, context) {
                Ok(()) if resolved != original => {
                    *value = resolved;
                    let source = expression
                        .strip_prefix("$inputs.")
                        .and_then(crate::resolve::reference_head)
                        .and_then(|name| provenance.get(&format!("inputs.{name}")).cloned())
                        .or_else(|| {
                            expression.strip_prefix("$pages.").and_then(|reference| {
                                provenance.get(&format!("pages.{reference}")).cloned()
                            })
                        });
                    provenance.insert(
                        path.to_string(),
                        source.unwrap_or(ValueProvenance {
                            rung: expression,
                            origin: Origin {
                                source: "resolved input context".to_string(),
                                line: None,
                            },
                            overridden: Vec::new(),
                        }),
                    );
                }
                Ok(()) => {}
                Err(error) => errors.push(ResolveError {
                    stage: "reference_resolution",
                    message: match error.context {
                        Some(context) => format!(
                            "{path}: unresolved `{}` (evaluating `{context}`)",
                            error.reference
                        ),
                        None => format!("{path}: unresolved `{}`", error.reference),
                    },
                }),
            }
        }
        _ => {}
    }
}

fn apply_timeout(
    with: &mut serde_yml::Value,
    defaults: Option<&ManifestDefaults>,
    manifest_path: Option<&Path>,
    path: &str,
    provenance: &mut BTreeMap<String, ValueProvenance>,
) {
    let Some(timeout) = defaults.and_then(|defaults| defaults.timeout.as_ref()) else {
        return;
    };
    let had_timeout = with
        .as_mapping()
        .is_some_and(|map| map.contains_key(serde_yml::Value::String("timeout".to_string())));
    apply_default_timeout(with, timeout.0);
    if !had_timeout
        && with
            .as_mapping()
            .is_some_and(|map| map.contains_key(serde_yml::Value::String("timeout".to_string())))
    {
        provenance.insert(
            path.to_string(),
            ValueProvenance {
                rung: "defaults.timeout".to_string(),
                origin: manifest_path.map_or(
                    Origin {
                        source: "manifest".to_string(),
                        line: None,
                    },
                    |path| origin(path, &["defaults", "timeout"]),
                ),
                overridden: Vec::new(),
            },
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn input_provenance(
    name: &str,
    leaf_path: &Path,
    leaf_inputs: &BTreeMap<String, InputDecl>,
    merged: &BTreeMap<String, InputValue>,
    cli_origins: &BTreeMap<String, ValueProvenance>,
    profile_values: &BTreeMap<String, serde_json::Value>,
    profile_name: Option<&str>,
    manifest_inputs: &BTreeMap<String, InputDecl>,
    manifest_origins: &ManifestOrigins,
) -> ValueProvenance {
    let leaf_decl = &leaf_inputs[name];
    let effective_decl = if leaf_decl.inherit {
        manifest_inputs.get(name).unwrap_or(leaf_decl)
    } else {
        leaf_decl
    };
    let declaration = if leaf_decl.inherit {
        manifest_origins
            .inputs
            .get(name)
            .cloned()
            .unwrap_or_else(|| ValueProvenance {
                rung: "manifest declaration".to_string(),
                origin: origin(leaf_path, &["inputs", name]),
                overridden: Vec::new(),
            })
    } else {
        ValueProvenance {
            rung: "leaf declaration".to_string(),
            origin: origin(leaf_path, &["inputs", name]),
            overridden: Vec::new(),
        }
    };

    let profile = profile_name.and_then(|profile| {
        profile_values.get(name)?;
        Some(
            manifest_origins
                .profiles
                .get(&(profile.to_string(), name.to_string()))
                .cloned()
                .unwrap_or(ValueProvenance {
                    rung: format!("profile {profile}"),
                    origin: Origin {
                        source: "manifest".to_string(),
                        line: None,
                    },
                    overridden: Vec::new(),
                }),
        )
    });
    let env = effective_decl.env.as_ref().and_then(|variable| {
        std::env::var(variable).ok()?;
        Some(ValueProvenance {
            rung: format!("env {variable}"),
            origin: declaration.origin.clone(),
            overridden: declaration.overridden.clone(),
        })
    });
    let default = effective_decl.default.as_ref().map(|_| ValueProvenance {
        rung: "default".to_string(),
        origin: declaration.origin.clone(),
        overridden: declaration.overridden.clone(),
    });

    let mut candidates = Vec::new();
    if let Some(cli) = cli_origins.get(name).cloned() {
        candidates.push(cli);
    } else if merged.contains_key(name) {
        candidates.push(ValueProvenance {
            rung: "--inputs".to_string(),
            origin: Origin {
                source: "command line".to_string(),
                line: None,
            },
            overridden: Vec::new(),
        });
    }
    if let Some(profile) = profile {
        candidates.push(profile);
    }
    if let Some(env) = env {
        candidates.push(env);
    }
    if let Some(default) = default {
        candidates.push(default);
    }
    let mut winner = candidates.into_iter().next().unwrap_or(declaration);
    for loser in candidates_for_losers(
        name,
        merged,
        cli_origins,
        profile_name,
        profile_values,
        effective_decl,
        manifest_origins,
        leaf_path,
    ) {
        if loser.source != winner.origin.source || loser.line != winner.origin.line {
            winner.overridden.push(loser);
        }
    }
    winner
}

#[allow(clippy::too_many_arguments)]
fn candidates_for_losers(
    name: &str,
    merged: &BTreeMap<String, InputValue>,
    cli_origins: &BTreeMap<String, ValueProvenance>,
    profile_name: Option<&str>,
    profile_values: &BTreeMap<String, serde_json::Value>,
    declaration: &InputDecl,
    manifest_origins: &ManifestOrigins,
    leaf_path: &Path,
) -> Vec<Origin> {
    let mut out = Vec::new();
    if merged.contains_key(name)
        && let Some(cli) = cli_origins.get(name)
    {
        out.extend(cli.overridden.clone());
    }
    if let Some(profile) = profile_name
        && profile_values.contains_key(name)
        && let Some(source) = manifest_origins
            .profiles
            .get(&(profile.to_string(), name.to_string()))
    {
        out.push(source.origin.clone());
        out.extend(source.overridden.clone());
    }
    if declaration
        .env
        .as_ref()
        .is_some_and(|name| std::env::var(name).is_ok())
        || declaration.default.is_some()
    {
        out.push(origin(leaf_path, &["inputs", name]));
    }
    out
}

fn mask_document(registry: &duhem_evidence::SecretRegistry, document: &mut serde_json::Value) {
    registry.mask_json(document);
    replace_redaction_markers(document);
}

fn candidate_secret_registry(
    definition: &VerificationDefinition,
    manifest_inputs: &BTreeMap<String, InputDecl>,
    merged: &BTreeMap<String, InputValue>,
    profile: &BTreeMap<String, serde_json::Value>,
) -> duhem_evidence::SecretRegistry {
    let mut registry = duhem_evidence::SecretRegistry::new();
    for (name, leaf) in &definition.inputs {
        let manifest = leaf.inherit.then(|| manifest_inputs.get(name)).flatten();
        if !(leaf.secret || manifest.is_some_and(|decl| decl.secret)) {
            continue;
        }
        if let Some(value) = merged.get(name) {
            match value {
                InputValue::Raw(value) => registry.register(name.clone(), value),
                InputValue::Typed(value) => registry.register_json(name.clone(), value),
            }
        }
        if let Some(value) = profile.get(name) {
            registry.register_json(name.clone(), value);
        }
        let declaration = manifest.unwrap_or(leaf);
        if let Some(variable) = &declaration.env
            && let Ok(value) = std::env::var(variable)
        {
            registry.register(name.clone(), &value);
        }
        if let Some(value) = &declaration.default
            && let Ok(value) = inputs::yml_to_json(value)
        {
            registry.register_json(name.clone(), &value);
        }
    }
    crate::resolve::register_flow_secrets(&mut registry, definition, None);
    registry
}

fn redact_error(registry: &duhem_evidence::SecretRegistry, error: &str) -> String {
    let masked = registry.mask(error);
    if masked.counts.is_empty() {
        error.to_string()
    } else {
        "input resolution failed for a secret value".to_string()
    }
}

fn replace_redaction_markers(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) if text.contains("[redacted:") => {
            *text = MASK.to_string();
        }
        serde_json::Value::Array(values) => {
            for value in values {
                replace_redaction_markers(value);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values_mut() {
                replace_redaction_markers(value);
            }
        }
        _ => {}
    }
}

fn render(format: ResolveFormat, output: &ResolveOutput) -> Result<String, String> {
    match format {
        ResolveFormat::Yaml => serde_yml::to_string(output).map_err(|error| error.to_string()),
        ResolveFormat::Json => {
            let mut text =
                serde_json::to_string_pretty(output).map_err(|error| error.to_string())?;
            text.push('\n');
            Ok(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_lists_composed_pages_with_entry_provenance() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("leaf")).unwrap();
        std::fs::write(
            tmp.path().join("pages.yml"),
            "pages:\n  login:\n    submit: { role: button, name: Included }\n    username: { role: textbox, name: Username }\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("leaf/duhem.yml"),
            r#"
verification: catalog leaf
pages:
  login:
    submit: { role: button, name: Sign In }
criteria:
  - id: AC-1
    description: shared locator
    checks:
      - id: AC-1.1
        steps:
          - uses: ui/assert-element
            with: { locator: $pages.login.submit, expected: visible }
"#,
        )
        .unwrap();
        let manifest = tmp.path().join("duhem.yml");
        std::fs::write(
            &manifest,
            r#"
manifest_version: 1
includes: [./pages.yml]
verifications:
  - path: ./leaf/duhem.yml
"#,
        )
        .unwrap();

        let (_, output) = resolve(ResolveArgs {
            path: Some(manifest),
            profile: None,
            inputs: Vec::new(),
            format: ResolveFormat::Json,
            provenance: true,
        })
        .unwrap();
        let verification = &output.verifications[0];
        assert_eq!(
            verification.document["pages"]["login"]["submit"]["name"],
            "Sign In"
        );
        assert_eq!(
            verification.document["criteria"][0]["checks"][0]["steps"][0]["with"]["locator"]["name"],
            "Sign In",
            "the execution view carries the spliced locator"
        );
        let provenance = verification.provenance.as_ref().unwrap();
        let submit = provenance
            .get("pages.login.submit")
            .expect("leaf-local entry provenance");
        assert!(submit.origin.source.ends_with("leaf/duhem.yml"));
        assert!(
            submit
                .overridden
                .iter()
                .any(|origin| origin.source.ends_with("pages.yml")),
            "{:?}",
            submit.overridden
        );
        let source = provenance
            .get("pages.login.username")
            .unwrap()
            .origin
            .source
            .clone();
        assert!(source.ends_with("pages.yml"), "{source}");
    }

    #[test]
    fn resolve_renders_flow_expansion_with_provenance() {
        let tmp = tempfile::tempdir().unwrap();
        let leaf = tmp.path().join("duhem.yml");
        std::fs::write(
            &leaf,
            r#"
verification: resolved flow
flows:
  echo:
    params:
      message: { type: string }
    steps:
      - id: say
        uses: cli/invoke
        with: { command: [echo, $params.message] }
    outputs:
      code: $steps.say.outputs.exit_code
criteria:
  - id: AC-1
    description: flow is expanded
    checks:
      - id: AC-1.1
        steps:
          - id: greeting
            call: echo
            with: { message: hello }
        assertions:
          - $steps.greeting.outputs.code == 0
"#,
        )
        .unwrap();

        let (_, output) = resolve(ResolveArgs {
            path: Some(leaf),
            profile: None,
            inputs: Vec::new(),
            format: ResolveFormat::Json,
            provenance: true,
        })
        .expect("resolve");
        let verification = &output.verifications[0];
        let step = &verification.document["criteria"][0]["checks"][0]["steps"][0];
        assert_eq!(step["id"], "greeting__say");
        assert_eq!(step["uses"], "cli/invoke");
        assert!(step.get("call").is_none());
        assert_eq!(step["with"]["command"][1], "hello");
        assert_eq!(
            verification.document["criteria"][0]["checks"][0]["assertions"][0],
            "$steps.greeting__say.outputs.exit_code == 0"
        );
        let flow = verification
            .provenance
            .as_ref()
            .unwrap()
            .get("criteria.0.checks.0.steps.0.flow")
            .expect("flow provenance");
        assert_eq!(flow.rung, "flow expansion");
        assert!(
            flow.origin
                .source
                .contains("flow `echo` invocation `greeting`")
        );
    }

    /// Regression coverage for the flow-secret masking leak (spec #376):
    /// a secret flow param bound to a *structured* value (mapping or
    /// sequence) must mask the value the `$inputs.*` reference resolves
    /// to, not the unresolved placeholder text. Each fixture below binds
    /// `token`'s resolved default through a differently-shaped `auth`
    /// flow param and asserts the value surfaces in `document.inputs`
    /// (a field unrelated to the flow secret itself) fully masked.
    #[test]
    fn register_flow_secrets_masks_mapping_bound_secret_param() {
        let tmp = tempfile::tempdir().unwrap();
        let leaf = tmp.path().join("duhem.yml");
        let secret = "mapping-secret-9f31ab";
        std::fs::write(
            &leaf,
            format!(
                r#"
verification: flow secret mapping leak
inputs:
  token: {{ type: string, default: {secret} }}
flows:
  call_api:
    params:
      auth: {{ type: object, secret: true }}
    steps:
      - id: invoke
        uses: cli/invoke
        with: {{ command: [echo, ok] }}
criteria:
  - id: AC-1
    description: mapping-bound secret flow param is masked
    checks:
      - id: AC-1.1
        steps:
          - id: step1
            call: call_api
            with: {{ auth: {{ header: $inputs.token }} }}
"#
            ),
        )
        .unwrap();

        let (_, output) = resolve(ResolveArgs {
            path: Some(leaf),
            profile: None,
            inputs: Vec::new(),
            format: ResolveFormat::Json,
            provenance: false,
        })
        .expect("resolve");
        let verification = &output.verifications[0];
        assert!(
            verification.errors.is_empty(),
            "{}",
            verification
                .errors
                .iter()
                .map(|error| format!("{}: {}", error.stage, error.message))
                .collect::<Vec<_>>()
                .join("; ")
        );
        assert_eq!(
            verification.document["inputs"]["token"], MASK,
            "a secret bound to a mapping flow param must mask the resolved value, not the `$inputs.token` placeholder"
        );
    }

    #[test]
    fn register_flow_secrets_masks_sequence_bound_secret_param() {
        let tmp = tempfile::tempdir().unwrap();
        let leaf = tmp.path().join("duhem.yml");
        let secret = "sequence-secret-4d2acb";
        std::fs::write(
            &leaf,
            format!(
                r#"
verification: flow secret sequence leak
inputs:
  token: {{ type: string, default: {secret} }}
flows:
  call_api:
    params:
      auth: {{ type: array, secret: true }}
    steps:
      - id: invoke
        uses: cli/invoke
        with: {{ command: [echo, ok] }}
criteria:
  - id: AC-1
    description: sequence-bound secret flow param is masked
    checks:
      - id: AC-1.1
        steps:
          - id: step1
            call: call_api
            with: {{ auth: [$inputs.token] }}
"#
            ),
        )
        .unwrap();

        let (_, output) = resolve(ResolveArgs {
            path: Some(leaf),
            profile: None,
            inputs: Vec::new(),
            format: ResolveFormat::Json,
            provenance: false,
        })
        .expect("resolve");
        let verification = &output.verifications[0];
        assert!(
            verification.errors.is_empty(),
            "{}",
            verification
                .errors
                .iter()
                .map(|error| format!("{}: {}", error.stage, error.message))
                .collect::<Vec<_>>()
                .join("; ")
        );
        assert_eq!(
            verification.document["inputs"]["token"], MASK,
            "a secret bound to a sequence flow param must mask the resolved value, not the `$inputs.token` placeholder"
        );
    }

    #[test]
    fn register_flow_secrets_still_masks_plain_string_bound_secret_param() {
        let tmp = tempfile::tempdir().unwrap();
        let leaf = tmp.path().join("duhem.yml");
        let secret = "plain-secret-7a10ee";
        std::fs::write(
            &leaf,
            format!(
                r#"
verification: flow secret plain string leak (regression)
inputs:
  token: {{ type: string, default: {secret} }}
flows:
  call_api:
    params:
      auth: {{ type: string, secret: true }}
    steps:
      - id: invoke
        uses: cli/invoke
        with: {{ command: [echo, ok] }}
criteria:
  - id: AC-1
    description: plain-string-bound secret flow param is masked
    checks:
      - id: AC-1.1
        steps:
          - id: step1
            call: call_api
            with: {{ auth: $inputs.token }}
"#
            ),
        )
        .unwrap();

        let (_, output) = resolve(ResolveArgs {
            path: Some(leaf),
            profile: None,
            inputs: Vec::new(),
            format: ResolveFormat::Json,
            provenance: false,
        })
        .expect("resolve");
        let verification = &output.verifications[0];
        assert!(
            verification.errors.is_empty(),
            "{}",
            verification
                .errors
                .iter()
                .map(|error| format!("{}: {}", error.stage, error.message))
                .collect::<Vec<_>>()
                .join("; ")
        );
        assert_eq!(
            verification.document["inputs"]["token"], MASK,
            "the historical plain-string secret flow param binding must keep masking the resolved value"
        );
    }
}
