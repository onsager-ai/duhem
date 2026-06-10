# `onsager-dashboard-create-spec-plan`

Duhem's dogfood Verification Definition. Targets the Onsager
dashboard's guided **Create Plan** flow end-to-end — dev-login +
form-driven plan authoring + live `compile_dry_run` gate + HITL
`submit_spec_plan` commit + persistence read-back — against a real
Onsager dev stack, real portal MCP server, real plan compiler, real
Postgres, real browser. No mocks at the web boundary, per
`docs/duhem-spec.md` §8.

Replaces the retired `onsager-dashboard-create-project/` VD, whose
target feature was removed from `onsager-ai/onsager`.

- Criteria prose: [`criteria.md`](criteria.md)
- Verification Definition: [`duhem.yml`](duhem.yml)
- Spec issue: [`onsager-ai/duhem#79`](https://github.com/onsager-ai/duhem/issues/79)

## What this verifies

The Create Plan flow (Onsager spec #542) is a golden path through
four layers — auth, MCP-routed compile, HITL-gated write, and
persistence read-back — and breaks loudly when any one of them
regresses. The three criteria (AC-1, AC-2, AC-3) correspond to
landing on the form, compiling+submitting a valid plan, and seeing
it in the Spec Plans list.

The submit control only enables when the live `compile_dry_run`
actually passes, so a green AC-2 is direct evidence the real
compiler accepted the plan — there is nothing to mock.

## Running the verification

### Local Onsager dev stack

The VD's `environment:` block boots and tears down the stack via
[`scripts/up.sh`](scripts/up.sh) / [`scripts/down.sh`](scripts/down.sh).
Set `DUHEM_ONSAGER_REPO_DIR` to an `onsager-ai/onsager` checkout so
`up.sh` knows where to run `just dev`:

```sh
export DUHEM_ONSAGER_REPO_DIR=/path/to/onsager
duhem run verifications/onsager-dashboard-create-spec-plan/duhem.yml \
  --inputs plan_id="duhem-fixture-$(uuidgen)"
```

`up.sh` boots `just dev` (Postgres + portal `:3002` + stiglab
`:3000` + synodic `:3001` + scheduler + dashboard `:5173`), waits
for the portal to be healthy, and seeds a workflow (see below).
Duhem then waits for `http://localhost:5173/api/health` (the
dashboard, which proxies `/api/*` to the portal) before running the
checks, and tears the stack down afterward. To iterate against a
stack you brought up yourself:

```sh
duhem run verifications/onsager-dashboard-create-spec-plan/duhem.yml \
  --no-env-up --keep-env \
  --inputs plan_id="duhem-fixture-$(uuidgen)"
```

The URL inputs (`health_url`, `login_url`, `new_plan_url`,
`plans_url`) default to the local `just dev` stack and the
dev-login-seeded workspace whose slug is `dev`. Staging passes them
explicitly.

### Filtering during authoring

```sh
duhem run verifications/onsager-dashboard-create-spec-plan/duhem.yml \
  --inputs plan_id="duhem-fixture-$(uuidgen)" \
  --filter AC-1
```

## Auth: dev-login, not a password

Onsager auth is GitHub OAuth or a **Dev Login** button, never an
email/password form. Dev-login is on by default in Onsager's debug
builds and on Railway preview environments; in release builds it is
gated behind `DEV_LOGIN_ENABLED=true`. Portal boot auto-seeds the
dev user **and a `dev` workspace**, so `/workspaces/dev/...` resolves
without any user provisioning. Each check repeats the one-click
dev-login prelude because the runtime gives each check its own
browser context (no cookie sharing); when session-passing `setup:`
lands, the prelude collapses.

## Seeding the workflow

The Create Plan compile gate only accepts a spec whose **kind** is
registered as a workflow in the active workspace. A fresh `dev`
workspace has none, so `up.sh` registers a minimal no-op workflow of
kind `Issue` via the portal `submit_workflow` MCP tool after the
portal is healthy.

The seed payload was confirmed end-to-end against a live portal:
`submit_workflow` registers the kind, and `compile_dry_run` then
returns `ok: true` for a one-spec plan of that kind — i.e. the
dashboard's submit gate opens. The one non-obvious requirement is that
the workflow node `id` is a UUID (`NodeId`), not a free string.

The seed stays **non-fatal**: if `up.sh` logs `WARNING — could not
seed a workflow` (e.g. the portal MCP surface changed), register a
workflow of kind `Issue` in the `dev` workspace manually — via the
dashboard's workflow builder or chat — before re-running. Confirm a
kind is present with the `list_workflows_v2` MCP tool; the kind name
must match the VD's `spec_kind` input (default `Issue`).

> First-run checklist (the spec's Test §, #79): on the first live
> run, confirm three selectors that were mined from Onsager's smoke
> test rather than a running browser — the kind-combobox **option**
> role (`role: option`), the **Spec Plans list** item markup
> (`role: listitem` + plan-id text), and the **dialog**-scoped confirm
> button. Adjust the locators in `duhem.yml` if the live DOM differs;
> the criteria in `criteria.md` do not change.

## Cross-repo seam

Auth, the dev workspace, workflow seeding, and any Create-Plan UI
changes on the Onsager side are *Onsager* concerns. Per
`docs/duhem-spec.md` §11.2 and `.claude/skills/onsager-dogfood`:

- Duhem authors the checks in this directory. Onsager never edits
  them.
- If Onsager's Create Plan UI is restructured (heading text, button
  label, combobox markup), the **check** here changes — the
  **criteria** in `criteria.md` do not. Open a Duhem-side PR.
- A first-class, headless dogfood-fixture seeding entrypoint on the
  Onsager side (so `up.sh` doesn't hand-roll `submit_workflow`) is a
  *parallel* Onsager-side spec, not an edit here.
