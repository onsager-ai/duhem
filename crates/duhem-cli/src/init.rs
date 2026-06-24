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
            write_template(
                &target.join("duhem.yml"),
                include_str!("../templates/init-pattern-a/duhem.yml"),
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
            write_template(
                &target.join("duhem.yml"),
                include_str!("../templates/init-pattern-b/duhem.yml"),
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
        .replace("{{PATH}}", path_in_commands);
    fs::write(path, rendered)
        .map_err(|e| InitError::Io(format!("write {}: {e}", path.display())))?;
    created.push(path.to_path_buf());
    Ok(())
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
    let args = InitArgs {
        path,
        pattern,
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
                "\nAuthoring guide: .claude/skills/verification-authoring/SKILL.md"
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
                name: Some("feature".into()),
                force: false,
            },
            ok_prompt,
        )
        .expect("init ok");
        // duhem.yml + criteria.md + README.md + ../duhem.yml
        assert_eq!(outcome.created.len(), 4, "created: {:?}", outcome.created);
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

    /// Schema-validity guard: the generated `duhem.yml` must parse
    /// and validate against the v0.1 schema (#8). Otherwise the
    /// scaffold ships a file `duhem validate` rejects, defeating
    /// the "first invocation works" premise of #48.
    #[test]
    fn generated_pattern_a_yaml_passes_schema_validate() {
        use duhem_schema::{VerificationDefinition, validate};
        let tmp = tempfile::tempdir().unwrap();
        run_with_prompt(
            args(tmp.path(), Some("smoke"), Pattern::A, false),
            ok_prompt,
        )
        .expect("init ok");
        let src = std::fs::read_to_string(tmp.path().join("duhem.yml")).unwrap();
        let def = VerificationDefinition::from_yaml_str(&src).expect("parse");
        validate(&def).expect("validate");
    }

    #[test]
    fn generated_pattern_b_yaml_passes_schema_validate() {
        use duhem_schema::{VerificationDefinition, validate};
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("feature");
        run_with_prompt(
            InitArgs {
                path: Some(target.clone()),
                pattern: Pattern::B,
                name: Some("feature".into()),
                force: false,
            },
            ok_prompt,
        )
        .expect("init ok");
        let src = std::fs::read_to_string(target.join("duhem.yml")).unwrap();
        let def = VerificationDefinition::from_yaml_str(&src).expect("parse");
        validate(&def).expect("validate");
    }
}
