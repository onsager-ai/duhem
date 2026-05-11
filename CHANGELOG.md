# Changelog

All notable schema-impacting changes to Duhem are recorded here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning is described in `docs/duhem-spec.md` §11.3 (Phase 1 keeps
the schema closed and breaking; deprecation policy lands at v1.0).

## v0.1.0 — schema introduced

First on-the-wire shape for a Verification Definition (`Pattern A`,
single-file). No prior version exists; this is a non-breaking initial
release.

### Added

- `VerificationDefinition` — top-level document. Fields: `verification`,
  `spec_ref?`, `inputs?`, `setup?`, `criteria`. Unknown top-level keys
  rejected (`deny_unknown_fields`).
- `InputDecl` — `{ type: String, default?: any }`.
- `Criterion` — `{ id, description, checks }`. `id` is authored and
  required.
- `Check` — `{ id, description?, steps?, assertions }`. `id` is
  authored and required.
- `Step` — `{ id?, uses, with?, outputs? }`. `uses:` is any non-empty
  string at v0.1; the typed action catalog lands in
  `spec(actions): ui/* action types v1`.
- `Assertion` — closed enum of six forms: bare boolean expression,
  `type_check`, `matches`, `in`, `exists`, `equal`. The closed-enum
  shape is the structural enforcer for the *mechanical judgment, not
  LLM judgment* identity commitment (`CLAUDE.md`).
- `TypeCheckKind` — closed enum: `uuid`, `string`, `integer`, `float`,
  `boolean`, `object`, `array`, `null`.
- `Expr` AST + `chumsky`-based parser. Boolean expressions are parsed
  at schema-load time (decision in #8) so syntax errors surface before
  the runtime is invoked. Grammar covers literals, `$steps.*` /
  `$inputs.*` / `$runtime.*` paths, function calls, comparisons
  (`== != < <= > >=`), boolean logic (`&& || !`), and parens.
- `validate()` — structural validator. Rules: non-empty `criteria`;
  unique `Criterion.id`, `Check.id` per criterion, and `Step.id` per
  check; every `$steps.<id>.outputs.<output>` and `$inputs.<name>`
  resolves against the same definition.
- `SchemaError` — wraps `serde_yml::Error`; preserves line/column on
  parse failures.
- `duhem validate <path>` — CLI subcommand (preview); the full
  `init` / `validate` / `run` surface lives in
  `spec(cli): duhem init / validate / run skeletons`.

### Deferred (named for traceability)

- Action-type catalog (`uses:` is a string today) —
  `spec(actions): ui/* action types v1`.
- Root manifest format (`duhem.yml`) —
  `spec(schema): root manifest v0.1`.
- Expression *evaluation* (paths/calls resolve to live values) —
  `spec(runtime): expression evaluator v1`.
- Assertion *evaluation* and verdict aggregation —
  `spec(judge): three-state verdict aggregation rules`.
