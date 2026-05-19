# `onsager-dashboard-create-project`

Duhem's first dogfood Verification Definition. Targets the
Onsager dashboard's "create a new project" flow end-to-end —
auth + form validation + database write + project-list read —
against a real Onsager environment, real Postgres, real
browser. No mocks at the web boundary, per
`docs/duhem-spec.md` §8.

- Criteria prose: [`criteria.md`](criteria.md)
- Verification Definition: [`duhem.yml`](duhem.yml)
- Spec issue: [`onsager-ai/duhem#35`](https://github.com/onsager-ai/duhem/issues/35)

## What this verifies

The Onsager dashboard's create-project flow is the golden path
through three layers — auth, form-validated write, persistence
read-back — and therefore breaks loudly when any one of them
regresses. The three criteria (AC-1, AC-2, AC-3) correspond to
landing on the form, submitting it, and seeing the new project
in the list.

## Running the verification

### Local Onsager dev server

```sh
duhem run verifications/onsager-dashboard-create-project/duhem.yml \
  --inputs login_url=http://localhost:3000/login \
  --inputs new_project_url=http://localhost:3000/projects/new \
  --inputs projects_url=http://localhost:3000/projects \
  --inputs test_email="$DUHEM_FIXTURE_EMAIL" \
  --inputs test_password="$DUHEM_FIXTURE_PASSWORD" \
  --inputs project_name="duhem-fixture-$(uuidgen)"
```

Defaults for the three URL inputs point at the conventional
local Onsager port (`http://localhost:3000`), so the local
form collapses to:

```sh
duhem run verifications/onsager-dashboard-create-project/duhem.yml \
  --inputs test_email="$DUHEM_FIXTURE_EMAIL" \
  --inputs test_password="$DUHEM_FIXTURE_PASSWORD" \
  --inputs project_name="duhem-fixture-$(uuidgen)"
```

### Staging

Pass staging URLs explicitly. Credentials come from CI secrets;
never commit them.

```sh
duhem run verifications/onsager-dashboard-create-project/duhem.yml \
  --inputs login_url=https://staging.onsager.ai/login \
  --inputs new_project_url=https://staging.onsager.ai/projects/new \
  --inputs projects_url=https://staging.onsager.ai/projects \
  --inputs test_email="$ONSAGER_STAGING_FIXTURE_EMAIL" \
  --inputs test_password="$ONSAGER_STAGING_FIXTURE_PASSWORD" \
  --inputs project_name="duhem-fixture-$(uuidgen)"
```

### Filtering during authoring

To iterate on one criterion at a time (e.g. while diagnosing a
locator change after the dashboard's form heading is renamed):

```sh
duhem run … --filter AC-1
```

## Fixture user provisioning

The VD authenticates as a single fixture user. Provisioning is
manual today; an Onsager-side spec eventually expresses this
as an environment-provisioning step (see "Cross-repo seam"
below).

### Local

1. Start the Onsager dev server (`pnpm dev` or the Onsager
   repo's equivalent — check Onsager's `README.md`).
2. From the dashboard's sign-up flow, create a user with a
   throwaway email (e.g. `duhem-fixture+local@onsager.ai`).
3. Export the credentials in your shell so the
   `duhem run` command above picks them up:
   ```sh
   export DUHEM_FIXTURE_EMAIL="duhem-fixture+local@onsager.ai"
   export DUHEM_FIXTURE_PASSWORD="…"
   ```

The fixture user's projects can pile up across runs. Either
clear them periodically from the Onsager UI, or run with a
fresh database (`pnpm db:reset` on the Onsager side).

### Staging

The staging fixture user is provisioned out-of-band by the
human operator. Credentials live in the GitHub Actions secret
store on `onsager-ai/duhem`:

- `ONSAGER_STAGING_FIXTURE_EMAIL`
- `ONSAGER_STAGING_FIXTURE_PASSWORD`

The PR-check wiring spec (a separate, follow-on issue per
`#35`'s out-of-scope list) injects these into the workflow
that runs this VD on Onsager PRs.

## Why each check repeats the login prelude

The runtime today gives `setup:` its own browser context — only
named outputs cross into per-check browsers, not cookies or
session state (see
`crates/duhem-runtime/src/engine/setup.rs` and the comment in
`crates/duhem-actions/tests/fixtures/static-page.yml`). That
means each check authenticates from scratch. When session-
sharing setup lands as a follow-on spec, the four-step login
prelude inside each check collapses into a single setup block
and the checks shrink to just their criterion-specific slice.

## Cross-repo seam

The fixture user, the staging URL, and any auth-flow changes
on the Onsager side are *Onsager* concerns. Per
`docs/duhem-spec.md` §11.2 and `.claude/skills/onsager-dogfood`:

- Duhem authors the checks in this directory. Onsager never
  edits them.
- If Onsager's create-project UI is restructured (heading
  text changes, button label changes), the **check** here
  changes — the **criteria** in `criteria.md` do not. Open a
  Duhem-side PR.
- If Onsager needs to expose new observability for Duhem to
  verify a new criterion, that's a *parallel* Onsager-side
  spec on `onsager-ai/onsager`, not an edit here.

## Open items

These are explicit in the spec issue
[`#35`](https://github.com/onsager-ai/duhem/issues/35) and are
not blockers for the artifact itself; they are operator-side
work that lives next to running the VD:

- [ ] Confirm a working `onsager-ai/onsager` staging URL
      exists (or stand one up — Onsager-side spec).
- [ ] Capture a baseline `trace.jsonl` from a local green run
      and commit it as `fixtures/baseline-green.jsonl`
      (replay-determinism evidence).
- [ ] Inject a local regression (e.g. rename the submit
      button) and confirm the verdict flips to `Fail` with
      evidence pointing at the failing `assert-element`.
