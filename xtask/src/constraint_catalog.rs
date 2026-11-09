//! Closed spec catalogs compared with their implementation (#541).
//! CI uses fail mode; `--mode=warn` exists so a broken extractor can be
//! downgraded in a follow-up PR while it is repaired.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;

fn between<'a>(text: &'a str, start: &str, end: &str) -> Result<&'a str> {
    let (_, rest) = text
        .split_once(start)
        .ok_or_else(|| anyhow!("missing anchor: {start}"))?;
    let (body, _) = rest
        .split_once(end)
        .ok_or_else(|| anyhow!("missing anchor: {end}"))?;
    Ok(body)
}

fn quoted(text: &str) -> Result<BTreeSet<String>> {
    let re = Regex::new(r"`([^`]+)`")?;
    Ok(re.captures_iter(text).map(|c| c[1].to_owned()).collect())
}

fn compare(name: &str, docs: BTreeSet<String>, code: BTreeSet<String>) -> Result<()> {
    let missing: Vec<_> = code.difference(&docs).collect();
    let extra: Vec<_> = docs.difference(&code).collect();
    if !missing.is_empty() || !extra.is_empty() {
        bail!(
            "{name}: missing from spec: {missing:?}; extra in spec (missing from code): {extra:?}"
        );
    }
    eprintln!(
        "constraint-catalog: {name} covers {} spec/code tokens",
        code.len()
    );
    Ok(())
}

fn helpers(spec: &str) -> Result<()> {
    let text = between(spec, "authored helpers\nare exactly ", ". The evaluator")?;
    compare(
        "authored runtime helpers",
        quoted(text)?,
        duhem_schema::authored_runtime_helper_names()
            .map(str::to_owned)
            .collect(),
    )
}

fn causes(spec: &str, source: &str) -> Result<()> {
    let text = between(
        spec,
        "(snake-case wire tokens): ",
        ". The runtime evaluator",
    )?;
    let file = syn::parse_file(source).context("parse verdict source")?;
    let method = file.items.iter().find_map(|item| match item {
        syn::Item::Impl(i) if matches!(&*i.self_ty, syn::Type::Path(p) if p.path.is_ident("InconclusiveCause")) => i.items.iter().find_map(|item| match item {
            syn::ImplItem::Fn(f) if f.sig.ident == "as_wire" => Some(f), _ => None,
        }), _ => None,
    }).ok_or_else(|| anyhow!("missing InconclusiveCause::as_wire"))?;
    let expr = match method.block.stmts.as_slice() {
        [syn::Stmt::Expr(syn::Expr::Match(m), None)] => m,
        _ => bail!("unexpected InconclusiveCause::as_wire body"),
    };
    let mut code = BTreeSet::new();
    for arm in &expr.arms {
        match &*arm.body {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(s),
                ..
            }) => {
                code.insert(s.value());
            }
            _ => bail!("unexpected wire token expression in InconclusiveCause::as_wire"),
        }
    }
    compare("inconclusive cause wire tokens", quoted(text)?, code)
}

fn kinds(spec: &str, source: &str) -> Result<()> {
    let text = between(
        spec,
        "the `is:` catalog is closed at v1: `",
        "`. (Extending it",
    )?;
    let docs = text.split('|').map(|s| s.trim().to_owned()).collect();
    let file = syn::parse_file(source).context("parse assertion source")?;
    let en = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Enum(e) if e.ident == "TypeCheckKind" => Some(e),
            _ => None,
        })
        .ok_or_else(|| anyhow!("missing TypeCheckKind enum"))?;
    // Serde uses snake_case on this enum. Reject attributes that alter the
    // convention rather than silently guessing a wire token.
    if !en.attrs.iter().any(|a| {
        a.path().is_ident("serde")
            && a.meta
                .require_list()
                .is_ok_and(|l| l.tokens.to_string().contains("rename_all = \"snake_case\""))
    }) {
        bail!("TypeCheckKind no longer declares serde snake_case");
    }
    let mut code = BTreeSet::new();
    for variant in &en.variants {
        if !variant.attrs.is_empty() {
            bail!("TypeCheckKind variant attributes require extractor update");
        }
        let name = variant.ident.to_string();
        let mut wire = String::new();
        for (i, c) in name.chars().enumerate() {
            if i > 0 && c.is_uppercase() {
                wire.push('_');
            }
            wire.extend(c.to_lowercase());
        }
        code.insert(wire);
    }
    compare("type-check kinds", docs, code)
}

fn actions(spec: &str) -> Result<()> {
    let text = between(spec, "#### Implemented (v1)", "#### Planned")?;
    let re = Regex::new(r"(?m)^- `([^`]+)` —")?;
    let docs = re.captures_iter(text).map(|c| c[1].to_owned()).collect();
    let code = duhem_actions::catalog()
        .into_iter()
        .map(|c| c.uses.to_owned())
        .collect();
    compare("implemented actions", docs, code)
}

pub fn run(args: Vec<String>) -> Result<()> {
    let warn = match args.as_slice() {
        [] => false,
        [mode] if mode == "--mode=fail" => false,
        [mode] if mode == "--mode=warn" => true,
        _ => bail!("usage: constraint-catalog [--mode=fail|warn]"),
    };
    eprintln!(
        "constraint-catalog does NOT cover: verdict mapping semantics (#537); behavioural acceptance (#533); prose contradictions (#539); reporting semantics (#531/#535/#519)"
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or_else(|| anyhow!("workspace root missing"))?;
    let check = || -> Result<()> {
        let spec = std::fs::read_to_string(root.join("docs/duhem-spec.md"))?;
        let verdict = std::fs::read_to_string(root.join("crates/duhem-judge/src/verdict.rs"))?;
        let assertion = std::fs::read_to_string(root.join("crates/duhem-schema/src/assertion.rs"))?;
        let results = [
            helpers(&spec),
            causes(&spec, &verdict),
            kinds(&spec, &assertion),
            actions(&spec),
        ];
        let errors: Vec<String> = results
            .into_iter()
            .filter_map(|r| r.err().map(|e| format!("{e:#}")))
            .collect();
        if !errors.is_empty() {
            bail!("{}", errors.join("\n"));
        }
        Ok(())
    };
    match check() {
        Err(e) if warn => {
            eprintln!("constraint-catalog warning: {e:#}");
            Ok(())
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SPEC: &str = include_str!("../../docs/duhem-spec.md");
    const VERDICT: &str = include_str!("../../crates/duhem-judge/src/verdict.rs");
    const ASSERTION: &str = include_str!("../../crates/duhem-schema/src/assertion.rs");

    #[test]
    fn missing_runtime_helper_fails() {
        assert!(helpers(SPEC).is_ok());
        assert!(helpers(&SPEC.replace("`contains`, `matches`,", "`contains`,")).is_err());
    }
    #[test]
    fn missing_inconclusive_cause_fails() {
        assert!(causes(SPEC, VERDICT).is_ok());
        assert!(
            causes(
                &SPEC.replace(
                    "`environment_error`, `empty_aggregation`",
                    "`environment_error`"
                ),
                VERDICT
            )
            .is_err()
        );
    }
    #[test]
    fn missing_type_check_kind_fails() {
        assert!(kinds(SPEC, ASSERTION).is_ok());
        assert!(
            kinds(
                &SPEC.replace("array | null`. (Extending it", "array`. (Extending it"),
                ASSERTION
            )
            .is_err()
        );
    }
    #[test]
    fn missing_action_fails() {
        assert!(actions(SPEC).is_ok());
        assert!(actions(&SPEC.replace("- `db/observe` —", "- `db/observe` removed —")).is_err());
    }
}
