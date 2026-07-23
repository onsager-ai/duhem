//! `duhem init` — scaffold a Verification Definition skeleton.
//!
//! Deterministic, offline. AI-assisted criteria-to-check
//! translation is the Phase 1 generation service (`docs/duhem-
//! spec.md` §14), not this command. Spec on issue #48.
//!
//! Templates are baked at compile time from
//! `crates/duhem-cli/templates/`. Each template substitutes two
//! placeholders:
//!
//! - `{{NAME}}` — the slug from `--name` or the TTY prompt.
//! - `{{PATH}}` — the target directory as the author will type
//!   it in a `duhem run` command (best-effort relative to cwd;
//!   otherwise absolute).
//!
//! Exit codes follow the spec's three-tier shape:
//!
//! - `0` — success.
//! - `2` — refusal to overwrite a non-empty target (Conflict).
//! - `3` — Pattern B fell back to a TODO-stub manifest (Warning).
//!
//! Any other failure (IO, slug validation) is `1` via the dispatch
//! in `main.rs`.
//!
//! See `crates/duhem-cli/src/init.rs` tests at the bottom for the
//! per-rule unit coverage.
//
// Templates are `include_str!`'d, not loaded at runtime: the
// shipped `duhem` binary must work on a fresh checkout with no
// repo on disk.

use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Pattern A (single-file VD) vs. Pattern B (co-located + root
/// manifest). Pattern C (centralized) is out of scope per #48.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    A,
    B,
}

impl std::str::FromStr for Pattern {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "A" | "a" => Ok(Pattern::A),
            "B" | "b" => Ok(Pattern::B),
            "C" | "c" => Err(
                "pattern C (centralized verification directory) is out of scope for `duhem init` \
                 v1; see issue #48"
                    .into(),
            ),
            other => Err(format!("unknown --pattern `{other}`: expected `A` or `B`")),
        }
    }
}

/// Which action family the scaffolded first check uses. `Api` (the
/// default) is browser-free — a `duhem run` of the scaffold needs
/// only a network connection. `Ui` scaffolds a browser-driven
/// `ui/*` check, which additionally needs `duhem browser install`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Api,
    Ui,
}

impl std::str::FromStr for Kind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "api" | "API" => Ok(Kind::Api),
            "ui" | "UI" => Ok(Kind::Ui),
            other => Err(format!("unknown --kind `{other}`: expected `api` or `ui`")),
        }
    }
}

/// Outcome of a successful scaffold. Distinguishes the
/// Pattern-B-TODO-stub case so `main.rs` can map it to the
/// warning exit code (3) and a separate stderr line.
#[derive(Debug)]
pub struct InitOutcome {
    pub created: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

pub struct InitArgs {
    pub path: Option<PathBuf>,
    pub pattern: Pattern,
    pub kind: Kind,
    pub name: Option<String>,
    pub force: bool,
}

/// Top-level error from `init`. The `ExistingNonEmpty` variant
/// carries the conflicting paths so `main.rs` can surface them
/// verbatim — the spec's Test § requires the message names them.
#[derive(Debug)]
pub enum InitError {
    InvalidName(String),
    NameRequiredNonTty,
    ExistingNonEmpty { dir: PathBuf, entries: Vec<PathBuf> },
    Io(String),
}

impl std::fmt::Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InitError::InvalidName(n) => write!(
                f,
                "invalid --name `{n}`: expected a slug of lowercase letters, digits, and hyphens \
                 (regex `^[a-z0-9][a-z0-9-]*[a-z0-9]$` or a single `[a-z0-9]`)"
            ),
            InitError::NameRequiredNonTty => write!(
                f,
                "--name is required when stdin is not a terminal (cannot prompt)"
            ),
            InitError::ExistingNonEmpty { dir, entries } => {
                writeln!(
                    f,
                    "refusing to overwrite non-empty target {}: pass --force to overwrite",
                    dir.display()
                )?;
                writeln!(f, "conflicting paths:")?;
                for e in entries {
                    writeln!(f, "  - {}", e.display())?;
                }
                Ok(())
            }
            InitError::Io(msg) => write!(f, "{msg}"),
        }
    }
}

/// Entry point. `prompt_name` indirection lets tests inject a
/// canned answer without owning a real TTY.
pub fn run(args: InitArgs) -> Result<InitOutcome, InitError> {
    run_with_prompt(args, prompt_name_from_tty)
}

pub(crate) fn run_with_prompt(
    args: InitArgs,
    prompt: fn() -> Result<String, InitError>,
) -> Result<InitOutcome, InitError> {
    let name = resolve_name(args.name.as_deref(), prompt)?;
    let target = args
        .path
        .unwrap_or_else(|| PathBuf::from("verifications").join(&name));

    // Conflict check up front. An empty target (or no target at
    // all) is fine; a target that contains anything other than
    // hidden dotfiles is the "already populated" case the spec
    // gates behind `--force`.
    if target.exists() {
        let entries = visible_entries(&target)
            .map_err(|e| InitError::Io(format!("read target {}: {e}", target.display())))?;
        if !entries.is_empty() && !args.force {
            return Err(InitError::ExistingNonEmpty {
                dir: target.clone(),
                entries,
            });
        }
    }

    fs::create_dir_all(&target)
        .map_err(|e| InitError::Io(format!("create {}: {e}", target.display())))?;

    let mut created = Vec::new();
    let mut warnings = Vec::new();
    let path_in_commands = display_path_for_commands(&target);

    match args.pattern {
        Pattern::A => {
            let duhem_yml = match args.kind {
                Kind::Api => include_str!("../templates/init-pattern-a/duhem.api.yml"),
                Kind::Ui => include_str!("../templates/init-pattern-a/duhem.ui.yml"),
            };
            write_template(
                &target.join("duhem.yml"),
                duhem_yml,
                &name,
                &path_in_commands,
                &mut created,
            )?;
            write_template(
                &target.join("criteria.md"),
                include_str!("../templates/init-pattern-a/criteria.md"),
                &name,
                &path_in_commands,
                &mut created,
            )?;
            write_template(
                &target.join("README.md"),
                include_str!("../templates/init-pattern-a/README.md"),
                &name,
                &path_in_commands,
                &mut created,
            )?;
        }
        Pattern::B => {
            let duhem_yml = match args.kind {
                Kind::Api => include_str!("../templates/init-pattern-b/duhem.api.yml"),
                Kind::Ui => include_str!("../templates/init-pattern-b/duhem.ui.yml"),
            };
            write_template(
                &target.join("duhem.yml"),
                duhem_yml,
                &name,
                &path_in_commands,
                &mut created,
            )?;
            write_template(
                &target.join("criteria.md"),
                include_str!("../templates/init-pattern-b/criteria.md"),
                &name,
                &path_in_commands,
                &mut created,
            )?;
            write_template(
                &target.join("README.md"),
                include_str!("../templates/init-pattern-b/README.md"),
                &name,
                &path_in_commands,
                &mut created,
            )?;

            // Sibling root manifest: if the parent already has one,
            // we leave it alone (the loader spec owns the merge
            // semantics; clobbering blindly would be worse than the
            // current TODO state). If it doesn't, drop the stub.
            //
            // Both branches describe the per-feature VD using
            // `<target-basename>/duhem.yml` — the *relative* path
            // from the manifest's parent. Using `--name` here would
            // be wrong whenever the caller's explicit `PATH` leaf
            // differs from `--name` (e.g. `duhem init feat-a/v1
            // --name feat-a`).
            let parent = target.parent().unwrap_or_else(|| Path::new("."));
            let manifest = parent.join("duhem.yml");
            let leaf = target
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| name.clone());
            if manifest.exists() {
                warnings.push(format!(
                    "left existing root manifest {} unchanged; add `{leaf}/duhem.yml` to its \
                     verifications list when the root-manifest loader spec lands",
                    manifest.display(),
                ));
            } else {
                let stub = include_str!("../templates/init-pattern-b/manifest-stub.yml");
                let rendered = stub.replace("{{LEAF}}", &leaf);
                fs::write(&manifest, rendered)
                    .map_err(|e| InitError::Io(format!("write {}: {e}", manifest.display())))?;
                created.push(manifest.clone());
                warnings.push(format!(
                    "wrote TODO-stub root manifest at {}; the root-manifest loader spec is \
                     pending — `duhem run` against the stub will fail until that lands. Run the \
                     per-feature `{}/duhem.yml` directly in the meantime.",
                    manifest.display(),
                    target.display(),
                ));

                // Per-developer override fragment (spec #67 `includes:`):
                // a gitignored `.duhem.local.yml` alongside the root
                // manifest, plus a `.gitignore` snippet so it stays out
                // of version control. Convention only — the loader does
                // not check git. Both are scaffolded only when we just
                // created the root manifest (Pattern B), and never
                // clobber a pre-existing file.
                let local = parent.join(".duhem.local.yml");
                if !local.exists() {
                    fs::write(
                        &local,
                        include_str!("../templates/init-pattern-b/duhem.local.yml"),
                    )
                    .map_err(|e| InitError::Io(format!("write {}: {e}", local.display())))?;
                    created.push(local.clone());
                }
                append_gitignore_snippet(parent, &mut created, &mut warnings)?;
            }
        }
    }

    Ok(InitOutcome { created, warnings })
}

fn write_template(
    path: &Path,
    template: &str,
    name: &str,
    path_in_commands: &str,
    created: &mut Vec<PathBuf>,
) -> Result<(), InitError> {
    let rendered = template
        .replace("{{NAME}}", name)
        .replace("{{PATH}}", path_in_commands)
        .replace("{{SCHEMA}}", &schema_ref_for(path));
    fs::write(path, rendered)
        .map_err(|e| InitError::Io(format!("write {}: {e}", path.display())))?;
    created.push(path.to_path_buf());
    Ok(())
}

/// Ensure `<dir>/.gitignore` carries the `.duhem.local.yml` ignore rule
/// (spec #67). The per-developer include is gitignored by convention,
/// not by the loader, so the scaffold writes the rule for the author.
/// Creates the file if absent, appends the rule if the file exists but
/// doesn't already mention `.duhem.local.yml`, and otherwise leaves it
/// untouched (idempotent re-scaffold).
fn append_gitignore_snippet(
    dir: &Path,
    created: &mut Vec<PathBuf>,
    warnings: &mut Vec<String>,
) -> Result<(), InitError> {
    const RULE: &str = ".duhem.local.yml";
    let snippet = include_str!("../templates/init-pattern-b/gitignore-snippet");
    let gitignore = dir.join(".gitignore");
    if gitignore.exists() {
        let existing = fs::read_to_string(&gitignore)
            .map_err(|e| InitError::Io(format!("read {}: {e}", gitignore.display())))?;
        if existing.lines().any(|l| l.trim() == RULE) {
            return Ok(());
        }
        let mut next = existing;
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        next.push('\n');
        next.push_str(snippet);
        fs::write(&gitignore, next)
            .map_err(|e| InitError::Io(format!("write {}: {e}", gitignore.display())))?;
        warnings.push(format!(
            "appended `.duhem.local.yml` ignore rule to {}",
            gitignore.display(),
        ));
    } else {
        fs::write(&gitignore, snippet)
            .map_err(|e| InitError::Io(format!("write {}: {e}", gitignore.display())))?;
        created.push(gitignore);
    }
    Ok(())
}

/// Reference to the `duhem.schema.json` artifact for a generated
/// `duhem.yml`'s `# yaml-language-server: $schema=...` header, so editors
/// load it for live key/enum autocomplete (#133).
///
/// Walks up from the file's directory looking for a committed
/// `schema/duhem.schema.json` and emits a `../`-prefixed relative path to
/// it (works offline, pins to the local schema). When the artifact can't
/// be located on disk — the common case for a scaffold created outside a
/// Duhem checkout, e.g. after `npm i -g duhem` — falls back to the
/// published raw URL rather than a repo-internal relative path the author
/// doesn't have. yaml-language-server fetches remote `$schema` URLs, so
/// the header still resolves (#260).
fn schema_ref_for(yaml_path: &Path) -> String {
    const FALLBACK: &str =
        "https://raw.githubusercontent.com/onsager-ai/duhem/main/schema/duhem.schema.json";
    let Some(dir) = yaml_path.parent() else {
        return FALLBACK.to_string();
    };
    let abs_dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let mut prefix = String::new();
    let mut cursor: &Path = &abs_dir;
    // Bounded walk: stop at the filesystem root.
    loop {
        if cursor.join("schema/duhem.schema.json").is_file() {
            return format!("{prefix}schema/duhem.schema.json");
        }
        match cursor.parent() {
            Some(p) => {
                cursor = p;
                prefix.push_str("../");
            }
            None => return FALLBACK.to_string(),
        }
    }
}

/// Slug rule for `--name`: lowercase ASCII, digits, hyphens.
/// Must begin and end with `[a-z0-9]` (no leading or trailing
/// hyphen, no double-hyphen sandwich at the boundaries). A single
/// character is allowed.
///
/// Tighter than the schema's `verification:` field, which is just
/// a String — the constraint is here to keep on-disk directory
/// names portable and the generated YAML title readable.
fn validate_slug(s: &str) -> Result<(), InitError> {
    if s.is_empty() {
        return Err(InitError::InvalidName(s.into()));
    }
    let bytes = s.as_bytes();
    let valid = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-';
    let alnum = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    if !bytes.iter().all(|&b| valid(b)) {
        return Err(InitError::InvalidName(s.into()));
    }
    if !alnum(*bytes.first().unwrap()) || !alnum(*bytes.last().unwrap()) {
        return Err(InitError::InvalidName(s.into()));
    }
    Ok(())
}

fn resolve_name(
    flag: Option<&str>,
    prompt: fn() -> Result<String, InitError>,
) -> Result<String, InitError> {
    match flag {
        Some(s) => {
            validate_slug(s)?;
            Ok(s.to_string())
        }
        None => {
            if !io::stdin().is_terminal() {
                return Err(InitError::NameRequiredNonTty);
            }
            let s = prompt()?;
            let s = s.trim();
            validate_slug(s)?;
            Ok(s.to_string())
        }
    }
}

fn prompt_name_from_tty() -> Result<String, InitError> {
    let mut out = io::stderr();
    write!(out, "verification name (slug): ").map_err(|e| InitError::Io(e.to_string()))?;
    out.flush().map_err(|e| InitError::Io(e.to_string()))?;
    let mut buf = String::new();
    io::stdin()
        .read_line(&mut buf)
        .map_err(|e| InitError::Io(e.to_string()))?;
    Ok(buf)
}

/// Entries in `dir` excluding hidden dotfiles. The spec's "empty"
/// criterion is "no author-visible content"; a stray `.DS_Store`
/// or `.gitkeep` shouldn't trigger the conflict path.
fn visible_entries(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        out.push(entry.path());
    }
    out.sort();
    Ok(out)
}

/// Render the target as the author would type it in a `duhem run`
/// command. Prefer a relative-to-cwd path so the README's example
/// commands are copy-pasteable from a checkout root; fall back to
/// the absolute path when the target is outside cwd or cwd can't
/// be queried.
fn display_path_for_commands(target: &Path) -> String {
    let abs = match target.canonicalize() {
        Ok(p) => p,
        Err(_) => return target.display().to_string(),
    };
    let cwd = match std::env::current_dir().and_then(|p| p.canonicalize()) {
        Ok(p) => p,
        Err(_) => return abs.display().to_string(),
    };
    match abs.strip_prefix(&cwd) {
        Ok(rel) => rel.display().to_string(),
        Err(_) => abs.display().to_string(),
    }
}

/// CLI glue for `duhem init`: parse the pattern, run [`run`], print the
/// created files + next-step pointers, and map the outcome to an exit
/// code (0 ok, 3 warnings, 2 conflict-refusal, 1 other error).
pub fn run_init(
    path: Option<PathBuf>,
    pattern: &str,
    kind: &str,
    name: Option<String>,
    force: bool,
) -> ExitCode {
    let pattern: Pattern = match pattern.parse() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let kind: Kind = match kind.parse() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let args = InitArgs {
        path,
        pattern,
        kind,
        name,
        force,
    };
    match run(args) {
        Ok(outcome) => {
            let mut stdout = io::stdout().lock();
            let _ = writeln!(stdout, "Created:");
            for p in &outcome.created {
                let _ = writeln!(stdout, "  {}", p.display());
            }
            // The per-feature `duhem.yml` is always the first entry
            // `run` pushes (Pattern A: only one; Pattern B: pushed
            // before the parent root manifest). Use that as the
            // next-command pointer.
            let vd_path = outcome
                .created
                .first()
                .cloned()
                .unwrap_or_else(|| PathBuf::from("<verification>/duhem.yml"));
            let _ = writeln!(stdout, "\nNext:");
            let _ = writeln!(stdout, "  duhem validate {}", vd_path.display());
            let _ = writeln!(stdout, "  duhem run      {}", vd_path.display());
            let _ = writeln!(
                stdout,
                "\nAuthoring guide: https://github.com/onsager-ai/duhem/blob/main/docs/getting-started.md"
            );
            let _ = stdout.flush();

            if outcome.warnings.is_empty() {
                ExitCode::SUCCESS
            } else {
                for w in &outcome.warnings {
                    eprintln!("warning: {w}");
                }
                // Spec § Plan: "exit with a non-zero warning code
                // (distinct from the error code)." Reserve 2 for the
                // conflict-refusal error path; use 3 for warnings.
                ExitCode::from(3)
            }
        }
        Err(e @ InitError::ExistingNonEmpty { .. }) => {
            eprintln!("{e}");
            // Spec § Design: existing non-empty target without --force
            // exits 2.
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_prompt() -> Result<String, InitError> {
        Ok("from-prompt\n".into())
    }

    fn args(tmp: &Path, name: Option<&str>, pattern: Pattern, force: bool) -> InitArgs {
        InitArgs {
            path: Some(tmp.to_path_buf()),
            pattern,
            kind: Kind::Api,
            name: name.map(|s| s.into()),
            force,
        }
    }

    #[test]
    fn slug_validator_accepts_simple_names() {
        for s in ["a", "foo", "foo-bar", "foo-bar-baz", "v0", "0a"] {
            validate_slug(s).unwrap_or_else(|_| panic!("`{s}` should validate"));
        }
    }

    #[test]
    fn slug_validator_rejects_bad_inputs() {
        for s in [
            "", "-foo", "foo-", "Foo", "foo_bar", "foo bar", "foo!", "üfoo",
        ] {
            assert!(validate_slug(s).is_err(), "`{s}` should reject");
        }
    }

    #[test]
    fn pattern_from_str() {
        assert_eq!("A".parse::<Pattern>().unwrap(), Pattern::A);
        assert_eq!("a".parse::<Pattern>().unwrap(), Pattern::A);
        assert_eq!("B".parse::<Pattern>().unwrap(), Pattern::B);
        assert!("C".parse::<Pattern>().is_err());
        assert!("foo".parse::<Pattern>().is_err());
    }

    #[test]
    fn pattern_a_creates_three_files_in_empty_target() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("verifications").join("example");
        let outcome = run_with_prompt(
            InitArgs {
                path: Some(target.clone()),
                pattern: Pattern::A,
                kind: Kind::Api,
                name: Some("example".into()),
                force: false,
            },
            ok_prompt,
        )
        .expect("init ok");
        assert!(target.join("duhem.yml").is_file());
        assert!(target.join("criteria.md").is_file());
        assert!(target.join("README.md").is_file());
        assert_eq!(outcome.created.len(), 3);
        assert!(outcome.warnings.is_empty());
    }

    #[test]
    fn pattern_a_yaml_substitutes_name() {
        let tmp = tempfile::tempdir().unwrap();
        let outcome = run_with_prompt(
            args(tmp.path(), Some("foo-bar"), Pattern::A, false),
            ok_prompt,
        )
        .expect("init ok");
        let yml = std::fs::read_to_string(&outcome.created[0]).unwrap();
        assert!(
            yml.contains("verification: foo-bar — Duhem init skeleton"),
            "yml: {yml}"
        );
        assert!(!yml.contains("{{NAME}}"), "leftover placeholder: {yml}");
    }

    #[test]
    fn existing_non_empty_target_without_force_refuses() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("preexisting.txt"), "x").unwrap();
        let err = run_with_prompt(args(tmp.path(), Some("ex"), Pattern::A, false), ok_prompt)
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("refusing to overwrite"), "msg: {msg}");
        assert!(msg.contains("preexisting.txt"), "msg names conflict: {msg}");
        assert!(msg.contains("--force"), "msg points at remedy: {msg}");
    }

    #[test]
    fn hidden_dotfiles_do_not_count_as_non_empty() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".DS_Store"), "x").unwrap();
        run_with_prompt(args(tmp.path(), Some("ex"), Pattern::A, false), ok_prompt)
            .expect("dotfiles should not block");
        assert!(tmp.path().join("duhem.yml").is_file());
    }

    #[test]
    fn existing_empty_target_fills_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        // tmp.path() exists and is empty.
        run_with_prompt(args(tmp.path(), Some("ex"), Pattern::A, false), ok_prompt)
            .expect("init ok");
        assert!(tmp.path().join("duhem.yml").is_file());
    }

    #[test]
    fn force_overwrites_non_empty_target() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("preexisting.txt"), "x").unwrap();
        run_with_prompt(args(tmp.path(), Some("ex"), Pattern::A, true), ok_prompt)
            .expect("--force overrides");
        assert!(tmp.path().join("duhem.yml").is_file());
        // The conflicting file is left alone; --force only allows
        // the *init* writes to land, it does not sweep the dir.
        assert!(tmp.path().join("preexisting.txt").is_file());
    }

    #[test]
    fn pattern_b_without_sibling_manifest_writes_stub_and_warns() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("feature");
        let outcome = run_with_prompt(
            InitArgs {
                path: Some(target.clone()),
                pattern: Pattern::B,
                kind: Kind::Api,
                name: Some("feature".into()),
                force: false,
            },
            ok_prompt,
        )
        .expect("init ok");
        // duhem.yml + criteria.md + README.md + ../duhem.yml +
        // ../.duhem.local.yml + ../.gitignore (spec #67 includes:
        // scaffolding).
        assert_eq!(outcome.created.len(), 6, "created: {:?}", outcome.created);
        assert_eq!(outcome.warnings.len(), 1);
        let warn = &outcome.warnings[0];
        assert!(warn.contains("TODO-stub"), "warning text: {warn}");
        let manifest = tmp.path().join("duhem.yml");
        assert!(manifest.is_file());
        let body = std::fs::read_to_string(&manifest).unwrap();
        assert!(
            body.contains("feature/duhem.yml"),
            "stub mentions per-feature path: {body}"
        );
    }

    #[test]
    fn pattern_b_scaffolds_local_override_and_gitignore() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("feature");
        run_with_prompt(
            InitArgs {
                path: Some(target.clone()),
                pattern: Pattern::B,
                kind: Kind::Api,
                name: Some("feature".into()),
                force: false,
            },
            ok_prompt,
        )
        .expect("init ok");
        // The gitignored per-developer fragment lands next to the
        // root manifest, and a `.gitignore` ignores it.
        let local = tmp.path().join(".duhem.local.yml");
        assert!(local.is_file(), "local override scaffolded");
        let gitignore = tmp.path().join(".gitignore");
        assert!(gitignore.is_file(), ".gitignore scaffolded");
        let ignore_body = std::fs::read_to_string(&gitignore).unwrap();
        assert!(
            ignore_body.lines().any(|l| l.trim() == ".duhem.local.yml"),
            ".gitignore carries the rule: {ignore_body}"
        );
        // The fragment is a valid partial manifest (no manifest_version).
        let local_body = std::fs::read_to_string(&local).unwrap();
        duhem_schema::PartialRootManifest::from_yaml_str(&local_body)
            .expect("local override parses as a partial manifest");
    }

    #[test]
    fn pattern_b_appends_to_existing_gitignore_without_duplicating() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".gitignore"), "target/\n").unwrap();
        let target = tmp.path().join("feature");
        run_with_prompt(
            InitArgs {
                path: Some(target),
                pattern: Pattern::B,
                kind: Kind::Api,
                name: Some("feature".into()),
                force: false,
            },
            ok_prompt,
        )
        .expect("init ok");
        let body = std::fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(body.contains("target/"), "preserved existing rule: {body}");
        assert_eq!(
            body.lines()
                .filter(|l| l.trim() == ".duhem.local.yml")
                .count(),
            1,
            "rule added exactly once: {body}"
        );
    }

    /// Regression for PR #58 review: when the caller's explicit
    /// `PATH` leaf differs from `--name`, the stub manifest and
    /// the warning text must point at the *real* per-feature
    /// directory (the path basename), not the slug. Otherwise a
    /// future root-manifest loader would resolve the stub against
    /// a non-existent leaf.
    #[test]
    fn pattern_b_stub_uses_path_basename_when_different_from_name() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("feat-a").join("v1");
        let outcome = run_with_prompt(
            InitArgs {
                path: Some(target.clone()),
                pattern: Pattern::B,
                kind: Kind::Api,
                name: Some("feat-a".into()),
                force: false,
            },
            ok_prompt,
        )
        .expect("init ok");
        let manifest = tmp.path().join("feat-a").join("duhem.yml");
        assert!(manifest.is_file());
        let body = std::fs::read_to_string(&manifest).unwrap();
        assert!(
            body.contains("v1/duhem.yml"),
            "stub references path basename, not slug: {body}"
        );
        assert!(
            !body.contains("feat-a/duhem.yml"),
            "stub must not reference the slug when it differs from the leaf: {body}"
        );
        let warn = &outcome.warnings[0];
        assert!(
            warn.contains("v1/duhem.yml") || warn.contains(target.to_string_lossy().as_ref()),
            "warning references the real per-feature path: {warn}"
        );
    }

    #[test]
    fn pattern_b_with_existing_sibling_manifest_leaves_it_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("feature");
        let manifest = tmp.path().join("duhem.yml");
        std::fs::write(&manifest, "verifications: []\n").unwrap();

        let outcome = run_with_prompt(
            InitArgs {
                path: Some(target.clone()),
                pattern: Pattern::B,
                kind: Kind::Api,
                name: Some("feature".into()),
                force: false,
            },
            ok_prompt,
        )
        .expect("init ok");
        assert_eq!(outcome.created.len(), 3, "manifest stays untouched");
        assert_eq!(outcome.warnings.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&manifest).unwrap(),
            "verifications: []\n",
            "existing manifest preserved verbatim"
        );
    }

    #[test]
    fn invalid_name_flag_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let err = run_with_prompt(
            args(tmp.path(), Some("Foo Bar"), Pattern::A, false),
            ok_prompt,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("invalid --name"), "msg: {msg}");
    }

    /// Schema-validity guard: the generated `duhem.yml` must parse and
    /// pass `duhem validate` (#8). Since #267 the scaffold is terse — it
    /// references `status` with no `outputs:` binding — so this asserts
    /// the *contract-aware* path the CLI actually runs
    /// (`validate_with_contract_outputs`), not the pure-schema `validate`
    /// which stays strict by design. Otherwise the scaffold would ship a
    /// file `duhem validate` accepts but this guard rejects.
    #[test]
    fn generated_pattern_a_yaml_passes_schema_validate() {
        use duhem_schema::{VerificationDefinition, validate_with_contract_outputs};
        let tmp = tempfile::tempdir().unwrap();
        run_with_prompt(
            args(tmp.path(), Some("smoke"), Pattern::A, false),
            ok_prompt,
        )
        .expect("init ok");
        let src = std::fs::read_to_string(tmp.path().join("duhem.yml")).unwrap();
        let def = VerificationDefinition::from_yaml_str(&src).expect("parse");
        validate_with_contract_outputs(&def, &|u| crate::contract_check::contract_outputs(u))
            .expect("validate");
    }

    #[test]
    fn generated_pattern_b_yaml_passes_schema_validate() {
        use duhem_schema::{VerificationDefinition, validate_with_contract_outputs};
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("feature");
        run_with_prompt(
            InitArgs {
                path: Some(target.clone()),
                pattern: Pattern::B,
                kind: Kind::Api,
                name: Some("feature".into()),
                force: false,
            },
            ok_prompt,
        )
        .expect("init ok");
        let src = std::fs::read_to_string(target.join("duhem.yml")).unwrap();
        let def = VerificationDefinition::from_yaml_str(&src).expect("parse");
        validate_with_contract_outputs(&def, &|u| crate::contract_check::contract_outputs(u))
            .expect("validate");
    }

    /// A scaffold created outside a Duhem checkout (the `npm i -g duhem`
    /// case, here a tempdir) can't resolve a local `schema/
    /// duhem.schema.json`, so its `$schema` header must point at the
    /// published URL — never a repo-internal `../../schema/...` path the
    /// author doesn't have (#260).
    #[test]
    fn out_of_repo_scaffold_uses_published_schema_url() {
        let tmp = tempfile::tempdir().unwrap();
        run_with_prompt(
            args(tmp.path(), Some("smoke"), Pattern::A, false),
            ok_prompt,
        )
        .expect("init ok");
        let src = std::fs::read_to_string(tmp.path().join("duhem.yml")).unwrap();
        let header = src.lines().next().unwrap_or_default();
        assert!(
            header.contains("$schema=https://"),
            "expected a URL $schema header, got {header:?}"
        );
        assert!(
            !header.contains("../"),
            "scaffold must not leak a repo-internal relative schema path: {header:?}"
        );
    }

    #[test]
    fn kind_from_str() {
        assert_eq!("api".parse::<Kind>().unwrap(), Kind::Api);
        assert_eq!("ui".parse::<Kind>().unwrap(), Kind::Ui);
        assert!("browser".parse::<Kind>().is_err());
    }

    /// The default (`api`) scaffold must be browser-free — no `ui/*`
    /// step — so a stranger's first `duhem run` needs no Chromium.
    #[test]
    fn default_api_scaffold_is_browser_free() {
        let tmp = tempfile::tempdir().unwrap();
        run_with_prompt(
            args(tmp.path(), Some("smoke"), Pattern::A, false),
            ok_prompt,
        )
        .expect("init ok");
        let src = std::fs::read_to_string(tmp.path().join("duhem.yml")).unwrap();
        assert!(
            src.contains("api/call"),
            "api scaffold uses api/call: {src}"
        );
        assert!(
            !src.contains("ui/"),
            "default scaffold must be browser-free: {src}"
        );
    }

    /// The generated `criteria.md` is implementation-neutral: a criterion
    /// states intent, not mechanism. It must not carry browser/locator
    /// prose — that both violates the criteria-vs-checks discipline and
    /// is false for the default browser-free `api` scaffold (#293).
    #[test]
    fn criteria_md_is_implementation_neutral() {
        let tmp = tempfile::tempdir().unwrap();
        run_with_prompt(
            args(tmp.path(), Some("smoke"), Pattern::A, false),
            ok_prompt,
        )
        .expect("init ok");
        let md = std::fs::read_to_string(tmp.path().join("criteria.md")).unwrap();
        let low = md.to_lowercase();
        for term in ["browser", "locator", "renders"] {
            assert!(
                !low.contains(term),
                "criteria.md must be implementation-neutral; found `{term}`: {md}"
            );
        }
    }

    /// The `--kind ui` scaffold is opt-in but still schema-valid and
    /// actually browser-driven.
    #[test]
    fn ui_kind_scaffold_validates_and_is_browser_driven() {
        use duhem_schema::{VerificationDefinition, validate};
        let tmp = tempfile::tempdir().unwrap();
        run_with_prompt(
            InitArgs {
                path: Some(tmp.path().to_path_buf()),
                pattern: Pattern::A,
                kind: Kind::Ui,
                name: Some("smoke-ui".into()),
                force: false,
            },
            ok_prompt,
        )
        .expect("init ok");
        let src = std::fs::read_to_string(tmp.path().join("duhem.yml")).unwrap();
        let def = VerificationDefinition::from_yaml_str(&src).expect("parse");
        validate(&def).expect("validate");
        assert!(src.contains("ui/navigate"), "ui scaffold uses ui/*: {src}");
    }
}
