# Adopt Duhem in a product repo (`.duhem/` template)

Drop-in skeleton for **co-locating Duhem Verification Definitions with
the product they verify**. Duhem is used here as a *tool*: your repo
owns its checks, next to the code they exercise. Copy the `.duhem/`
tree, the `.claude/skills/` skill, and the `CODEOWNERS` stanza into
your product repo.

```
your-product/
├── .duhem/
│   ├── duhem.yml                # root manifest — aggregates the suite
│   └── factory-cli/
│       └── duhem.yml            # a leaf Verification Definition
├── .claude/skills/
│   └── verification-authoring/  # teaches your coding agent to author VDs
├── CODEOWNERS                   # routes /.duhem/ edits to a verifier
└── … your product code …
```

`.duhem/` is hidden and tool-namespaced (like `.github/`), so it reads
as "this repo adopts the Duhem tool" and never collides with your own
`verifications/` or test folders. Inside it, a root manifest
(`duhem.yml`) aggregates one or more leaf Verification Definitions.

Commit the `.duhem/` VDs, but ignore the evidence DB `duhem run`
writes there — add this to your `.gitignore`:

```gitignore
# Duhem run evidence — ignore the DB, commit the VDs
.duhem/*.db
.duhem/*.db-*
```

## Run it locally

```bash
duhem validate .duhem/duhem.yml    # schema-check the suite
duhem run .duhem/duhem.yml         # run it; exit 0 == pass
```

## Author with an AI agent

`.claude/skills/verification-authoring/` is a Claude Code skill that
teaches a coding agent to write Verification Definitions for your
product — the criteria-vs-checks split, the terse authoring form, and
the retrieval loop (`duhem actions` / `describe` / `validate`, or
`duhem mcp`). Keep it in your repo so any agent working here authors
against the version-exact contract instead of guessing.

## Mode A — self-gate on your own PRs

Make the suite a required check on your repo. Two ways:

**Direct CLI (simplest; no GitHub Action).** Install or build `duhem`,
then run the suite. Best for page-free CLI/API suites that need no
browser.

```yaml
# .github/workflows/duhem.yml
name: duhem
on: pull_request
jobs:
  verify:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      # …bring your product's real environment up (build, serve, seed)…
      - name: Install duhem
        run: npm i -g @onsager/duhem   # or download a release binary
      - name: Verify
        run: duhem run .duhem/duhem.yml
```

**Via `duhem/run` (batteries included).** Use the composite action with
`verification-source: workspace` so it resolves your co-located VD from
your checkout. It sets up Node + Chromium, parses the verdict, and can
ship evidence to a hub — worth it for UI-heavy suites.

```yaml
# .github/workflows/duhem.yml
name: duhem
on: pull_request
jobs:
  verify:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      # …bring your product's real environment up…
      - name: Verify
        uses: onsager-ai/duhem/.github/actions/run@v0.1
        with:
          verification-source: workspace
          verification-path: .duhem/duhem.yml
```

Then add the `duhem` check as required in branch protection.

## Mode B — Duhem monitors compatibility (nothing to set up here)

The Duhem project can run your `.duhem/` suite against a freshly-built
`duhem` in its own CI, so a change to Duhem that would break your VD
surfaces on the *Duhem* side before it ships. You don't configure this;
it's arranged with the Duhem maintainers.

## Guard against silent self-weakening

Because you own your VD, a PR could in principle weaken a check to dodge
a failing verdict. Two lightweight guards (review and evidence
discipline):

1. **CODEOWNERS on `/.duhem/`** (the `CODEOWNERS` file here) routes VD
   edits to a verifier reviewer. Needs branch protection with "Require
   review from Code Owners".
2. **Hub-recorded verdicts** (`duhem ship`) record each verdict with
   `(verifier_repo/sha, target_repo/sha)` provenance the product PR
   can't rewrite.

What makes a `pass` meaningful is mechanical judgment (no LLM in the
judge) plus a self-consistent Duhem contract — not where the VD lives.
