use std::path::Path;

// Catches only these literal spellings, not semantically equivalent predicates.
#[test]
fn no_inline_expression_predicates() {
    const ALLOW: &[(&str, &str, &str)] = &[
        (
            "crates/duhem-schema/src/expr/mod.rs",
            "raw.trim_start().starts_with('$')",
            "shared helper",
        ),
        (
            "crates/duhem-schema/src/flows_validate.rs",
            "if raw.starts_with('$') {",
            "leading spaces are literals in value_type",
        ),
        (
            "crates/duhem-schema/src/validate_sessions.rs",
            "if name.is_empty() || name.trim() != name || name.starts_with('$') {",
            "session name rule",
        ),
    ];
    fn scan(root: &Path, dir: &Path, violations: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                scan(root, &path, violations);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let relative = path.strip_prefix(root).unwrap().to_str().unwrap();
                for (index, line) in std::fs::read_to_string(&path).unwrap().lines().enumerate() {
                    let matches = line.contains(concat!("starts_with(", "'$')"))
                        || line.contains(concat!("starts_with(", "\"$\")"));
                    if matches
                        && !ALLOW
                            .iter()
                            .any(|(file, allowed, _)| *file == relative && line.trim() == *allowed)
                    {
                        violations.push(format!("{relative}:{}: {line}", index + 1));
                    }
                }
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let crates = root.join("crates");
    let mut violations = Vec::new();
    for entry in std::fs::read_dir(crates).unwrap() {
        let src = entry.unwrap().path().join("src");
        if src.is_dir() {
            scan(&root, &src, &mut violations);
        }
    }
    scan(&root, &root.join("xtask/src"), &mut violations);
    assert!(
        violations.is_empty(),
        "Use duhem_schema::expr::is_expression or is_valid_expression instead of inline predicates:\n{}",
        violations.join("\n")
    );
}
