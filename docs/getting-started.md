# Getting started

This guide takes you from install to a green verdict on a check you
wrote yourself. It assumes no prior Duhem knowledge and no repo
checkout — just the published CLI.

For the *why* (the Holistic Verification Principle, criteria-vs-checks,
mechanical judgment), read [`docs/duhem-spec.md`](./duhem-spec.md). This
is the *how*.

## 1. Install

```bash
npm i -g duhem
duhem --version
```

That's everything for browser-free checks (`api/*`, `db/*`, `cli/*`).
Browser-driven `ui/*` checks additionally need Node ≥ 20 and a Chromium
sidecar. `duhem run` provisions it automatically the first time a `ui/*`
check needs it, so this step is **optional** — run it to pre-warm the
download (e.g. in a CI image), or to add the OS libraries with
`--with-deps`:

```bash
duhem browser install          # optional pre-warm; add --with-deps in CI images
```

To manage the browser yourself (air-gapped hosts, a pinned system
Chromium), set `DUHEM_NO_BROWSER_INSTALL=1` to turn auto-provision off and
`DUHEM_BROWSER_EXECUTABLE=/path/to/chrome` to point at your binary.

## 2. Scaffold your first Verification Definition

```bash
duhem init ./verifications/hello --name hello
```

This writes a runnable **Verification Definition (VD)** — three files:

| File | What it is |
|------|------------|
| `duhem.yml` | the runnable checks (what Duhem executes and judges) |
| `criteria.md` | the human intent prose (what "done" means) |
| `README.md` | a short pointer for whoever opens the directory |

The default scaffold is **browser-free**: a single `api/call` check that
requests `https://example.com` and asserts it answers `200`. It's a
known-good baseline you mutate toward your real workload. (Want the
visual, browser-driven variant instead? `duhem init … --kind ui`.)

## 3. Validate and run it

```bash
duhem validate ./verifications/hello      # structural check — fast, offline
duhem run      ./verifications/hello       # execute end-to-end, print a verdict
```

`duhem run` prints a per-check line and the run verdict:

```
checks: pass
pass
```

`pass` means every assertion held against the *real* system — no mocks.
A `fail` names the assertion that broke; an `inconclusive` means Duhem
couldn't observe cleanly (a timeout, an environment that wouldn't come
up) and deliberately refuses to call that a pass.

**Watch it in the terminal.** On a TTY, `duhem run` narrates progress
on stderr while the run executes. A live criterion/check/step board
uses aligned one-cell text indicators — `✓` pass, `✗` fail, `◐`
inconclusive, `○` pending — while the active action gets a spinner,
elapsed time, and any timeout budget. On a manifest run, a header
separates each verification (`── login (2/5) ──`) and the shared environment's
bring-up and teardown narrate too (`env: up…` / `env: ready (2.5s)` /
`env: down…`). Piped or CI output is untouched (stdout carries only
the reporter); `--live` / `--no-live` force it either way — a forced
capture keeps plain append lines, plus a `… still in cli/invoke (12s)`
heartbeat once a step is stuck past 10s, so logs stay grep-able.

**Watch it live.** If `duhem dashboard` is serving the same store (or
`DUHEM_DASHBOARD_URL` points at one), `duhem run` prints the run's
dashboard deep link on stderr *before* the run starts:

```
live: http://127.0.0.1:7878/#/run/01JC…
```

Open it while the run executes — the run page streams each step as it
happens, and the runs list refreshes itself, so an in-flight run shows
up (marked `● live`) without a reload. Or skip the copy-paste: `duhem
run --watch` opens that page in your browser automatically (once per
invocation; `DUHEM_OPENER` overrides the platform opener).

## 4. The anatomy of a VD

```yaml
verification: hello — Duhem init skeleton

inputs:                      # values you can override at run time
  base_url:
    type: string
    default: https://example.com

criteria:
  - id: AC-1
    description: |           # the human commitment: what "done" means,
      The endpoint responds successfully.   # in 1–3 stable sentences
    checks:
      - id: AC-1.1
        description: Request the landing page and confirm it answers 200.
        steps:              # ordered actions that exercise the system
          - id: home
            uses: api/call
            with: { method: GET, url: $inputs.base_url, within: 15s }
        assertions:         # structured, mechanically judged — no LLM
          - $steps.home.outputs.status == 200   # no outputs: block needed
```

- **`criteria`** are the *stable* human commitment. Keep them readable —
  a non-technical stakeholder should be able to decide yes/no.
- **`checks`** are *derivative*: how you observe that a criterion holds.
  They change as the implementation does; the criterion doesn't.
- **`steps`** run in order; give a step an `id` and reference any output
  its action produces as `$steps.<id>.outputs.<name>` — **no `outputs:`
  block required**, the action's declared outputs (including nested ones,
  `…outputs.body.data._id`) resolve directly. Add `outputs:` only for the
  two things a native name can't do: *rename* a field
  (`outputs: { code: status }`) or bind a *deep extraction* to a short
  alias (`outputs: { project_id: body.data._id }`) so you write the path
  once instead of repeating it across assertions.
- **`assertions`** are evaluated deterministically — no model in the
  judging loop. They're also **optional**: a `ui/assert-*` (or `api/poll`)
  step *is* the judgment, so an all-assert check needs no `assertions:`
  block at all. Bind `satisfied` and assert it yourself only for manual
  control (e.g. a disjunction across steps).

## 5. Author a real check

Point it at *your* system by changing three things in `duhem.yml`:

1. **`inputs`** — your real URLs / connection strings / commands.
2. **`steps`** — the actions that drive your system. Pick from the
   action catalog:

   | Family | Use for | Needs a browser? |
   |--------|---------|:---:|
   | `api/*` | HTTP requests + observing responses | no |
   | `db/*`  | seeding / querying / observing a database | no |
   | `cli/*` | invoking a local command | no |
   | `ui/*`  | driving a real browser (navigate, click, type, assert) | yes |

   To learn a chosen action's `with:` fields, outputs, and a
   worked example, ask the CLI — it prints the version-exact
   contract:

   ```bash
   duhem actions                 # list the catalog
   duhem describe api/call       # one action's with:/outputs/example
   ```

   The same catalog, browsable, is
   [`docs/action-reference.md`](./action-reference.md).

3. **`assertions`** — what must be true. Assert over a step's **scalar**
   outputs (e.g. `status`, `body_text`), each assertion one verifiable
   claim:

   ```yaml
   assertions:
     - $steps.home.outputs.status == 200
     - $runtime.contains($steps.home.outputs.body_text, "Example Domain")
   ```

   `$runtime.contains(...)` is membership: on a **string** it's a literal
   substring test (the line above — the usual way to check response
   *content*); on an **array** it's element membership
   (`$runtime.contains($steps.api.outputs.body.tags, "prod")`). For a
   regular expression instead of a literal, use
   `$runtime.matches(body_text, "Example ?Domain")`.

Then `duhem validate` (catch shape errors early) and `duhem run`. A
type error in an assertion — say `contains` against a number — is a
`fail` that names the mismatch, not a silent pass.

## 6. Verifying a real workload that needs a running system

Most real checks need the system *up* first. A VD (or a suite manifest)
can declare environment hooks that provision it once and tear it down
after:

```yaml
environment:
  up: ./scripts/up.sh          # stand up the real thing (no mocks)
  down: ./scripts/down.sh
  ready:
    http: { url: http://127.0.0.1:8080/health, timeout: 120s }
```

Duhem runs `up`, waits for `ready`, runs every check against the real
environment, then runs `down`. If the environment doesn't come up, the
run is `inconclusive` — never a false `pass`.

## Authoring with an AI assistant (MCP)

Duhem is a v0.x framework no model has a reliable pretraining corpus for, so
don't let an assistant *guess* the syntax — give it the version-exact contract
as tools. `duhem mcp` runs a Model Context Protocol server over stdio exposing:

- `duhem_actions` — list the catalog;
- `duhem_describe` — one action's `with:` fields / `outputs` / example;
- `duhem_validate` — validate a VD (structural + field-check), so the assistant
  can author → validate → fix with no repo checkout.

Register it once with your MCP client (Claude Desktop / Cursor / …):

```json
{
  "mcpServers": {
    "duhem": { "command": "duhem", "args": ["mcp"] }
  }
}
```

There is deliberately **no `run` tool** — running a VD needs your real system,
which stays on your side (local or CI), never in the server.

## Next steps

- **Grow a suite.** A root `duhem.yml` manifest can run many co-located
  VDs against one shared environment. See
  [`docs/duhem-spec.md`](./duhem-spec.md) §10.
- **Gate CI on it.** Run `duhem run` in CI and block merge on the
  verdict; the `duhem/run` composite action wraps this.
- **The full model.** [`docs/duhem-spec.md`](./duhem-spec.md) —
  especially §7 (Core Concepts) and §8 (Holistic Verification).
