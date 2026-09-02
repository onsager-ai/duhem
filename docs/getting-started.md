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

### Environment files and operational knobs

`duhem run` loads the nearest `.env` starting at the root-manifest directory
(or the current directory when there is no manifest) and walking upward only
as far as the enclosing `.git` boundary. A variable already exported in the
real process environment always wins. Use `--env-file <path>` to select a file
explicitly or `--no-env-file` to disable loading.

The in-tree parser deliberately supports only `KEY=VALUE`, an optional
`export ` prefix, blank lines, full-line `#` comments, trailing ` #` comments
on unquoted values, single- and double-quoted values, and `\n` / `\t` escapes
inside double quotes. Whitespace around keys and unquoted values is trimmed.
It rejects variable interpolation (`${OTHER}`) and multi-line values with a
line-numbered error. `.env` inheritance chains are not followed. This explicit
subset keeps sourcing predictable; the parser can later be replaced behind
the same seam.

Ambient variables are reserved for **operational knobs**: settings that change
how Duhem executes without changing what a check claims. The closed browser
execution set is:

- `DUHEM_HEADED=1|true` — show the browser;
- `DUHEM_SLOW_MO=<milliseconds>` — delay Playwright operations for observation;
- `DUHEM_NODE=<command>` — select the Node executable;
- `DUHEM_SIDECAR_DIR=<path>` — select the Playwright sidecar;
- `DUHEM_NO_BROWSER_INSTALL=1|true` — suppress automatic browser installation;
- `DUHEM_BROWSER_EXECUTABLE=<path>`, `DUHEM_BROWSER_CHANNEL=<name>`, and
  `DUHEM_BROWSER_ARGS="..."` — select/configure the browser process.

Timeouts, base URLs, credentials, and other **semantic parameters** change
what is verified, so Duhem never reads ambient variables as global overrides
for them. Declare an `inputs:` entry with `env:` and reference
`$inputs.<name>` instead; this makes the dependency visible in the VD and uses
the existing `--inputs` → profile → `env:` → `default:` precedence ladder.
Mark credentials `secret: true`; values sourced from `.env` then enter the
same masking registry as every other secret input. Suite-wide `timeout:` is a
`defaults:` setting on the root manifest, while a leaf without a manifest uses
the engine default. There is intentionally no `DUHEM_DEFAULT_TIMEOUT`.

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

When a step itself cannot complete, the failure includes the recorded
engine/action cause rather than stopping at a bare `error`. For example:

```text
inconclusive:missing_observation
  AC-1::AC-1.1:
    inconclusive:missing_observation  implicit: step `click` satisfied == true
        (action `ui/click` failed: locator `#submit` never resolved)
```

The same secret-masked cause appears under the `error` or `timeout` step
in the dashboard trace.

**Watch it in the terminal.** On a TTY, `duhem run` narrates progress
on stderr while the run executes. A bounded rolling board uses
cell-level updates instead of repainting completed history: the active
criterion/check/step stays expanded, completed passes collapse to one
row, failures and inconclusive details stay visible, and older passing
rows roll into an aggregate count. Aligned one-cell indicators show
`✓` pass, `✗` fail, `◐` inconclusive, and `○` pending, while the active
action gets a spinner, elapsed time, and any timeout budget. On a
manifest run, a header separates each verification (`── login (2/5) ──`)
and the shared environment's bring-up and teardown narrate too
(`env: up…` / `env: ready (2.5s)` / `env: down…`). Piped or CI output
is untouched (stdout carries only
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
    env: APP_BASE_URL        # optional process-environment fallback
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
            with: { method: GET, url: $inputs.base_url, timeout: 15s }
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
- **Step gating** defaults to `if: success`: after an earlier step errors,
  times out, or produces a failed implicit judgment, later default steps
  in that check are recorded as skipped. Binding `satisfied` for manual
  composition opts out of both implicit judgment and gating. Use
  `if: always` for teardown and `if: failure` for failure-only
  diagnostics. Both forms still dispatch after a step-level engine error;
  the original error aborts the run after cleanup drains. Gate state is per
  check and never affects sibling checks. Retried checks run cleanup once
  per attempt, so cleanup steps must be idempotent.
- **`teardown:`** is leaf-scoped action cleanup after all criteria and
  before `provision.down:`. It runs only after setup dispatched at least
  one action; failures are evidence only. `--keep-env` skips both cleanup
  layers and leaves the world as the run left it.
- **`assertions`** are evaluated deterministically — no model in the
  judging loop. They're also **optional**: a `ui/assert-*` (or `api/poll`)
  step *is* the judgment, so an all-assert check needs no `assertions:`
  block at all. Bind `satisfied` and assert it yourself only for manual
  control (e.g. a disjunction across steps).
- **Sensitive inputs** use `env: VARIABLE_NAME` with `secret: true`.
  Values resolve from `--inputs` → selected profile → process
  `env:` → `default:` and registered secret values are masked from
  recorded text and terminal output. A secret input cannot have a
  committed `default:`. Screenshots/video and transformations beyond
  base64, percent encoding, and JSON escaping remain outside the
  substring-masking guarantee. For a credential shared by a suite,
  declare it once under the root manifest's `inputs:` and list its name
  under each consuming leaf's `inputs:` with `inherit: true`:

  ```yaml
  # .duhem/duhem.yml
  inputs:
    password:
      type: string
      env: APP_PASSWORD
      secret: true
  profiles:
    staging:
      username: admin

  # a leaf Verification Definition
  inputs:
    password: { inherit: true, secret: true }
  ```

  A declared inherited value uses the manifest declaration's precedence
  and type checks. The leaf declaration remains required, so a typoed
  `$inputs.*` reference cannot silently bind a same-named manifest input.
- **Shared UI locators** live under a two-level `pages:` catalog in the
  root manifest, an ordinary include, or a leaf. Reference an entry as a
  whole locator value:

  ```yaml
  # pages.yml, listed under the root manifest's includes:
  pages:
    login:
      username: { role: textbox, name: Username }
      submit: { role: button, name: Sign In }

  # a leaf step
  - uses: ui/type
    with: { locator: $pages.login.username, text: $inputs.user }
  ```

  Root entries win over includes; leaf-local entries win over the
  composed manifest. A catalog locator may contain `$inputs.*`, but
  every leaf receiving it must declare those names. `duhem validate`
  catches dangling names offline (with a nearby-name suggestion), and
  `duhem resolve --provenance` shows each winning entry's source file.
- **Credentials returned by a step** use `secret_outputs:` on the producing
  step. Name the sensitive scalar output path, not its containing
  object or array:

  ```yaml
  - id: login
    uses: api/call
    secret_outputs: [body.data]
    with:
      method: POST
      url: $inputs.login_url
      body: { username: $inputs.username, password: $inputs.password }
  - id: projects
    uses: api/call
    with:
      method: GET
      url: $inputs.projects_url
      headers: { Authorization: $steps.login.outputs.body.data }
  ```

  `body.data` is registered before the login step records its own
  response, so both that response and the later `Authorization` value
  become `[redacted:login.body.data]`. A path such as `body` (object) or
  `body.items` (array) fails; name one scalar leaf such as
  `body.items[0].key`. Some actions may mark credential outputs secret
  in their contract, in which case no authored `secret_outputs:` entry is
  needed.

### Reuse a real browser login

For UI behind authentication, log in once during `setup:`, capture the
resulting browser state, and seed each authenticated check explicitly:

```yaml
setup:
  - uses: ui/navigate
    with: { url: $inputs.login_url }
  - uses: ui/type
    with: { locator: { label: Password }, text: $inputs.password }
  - uses: ui/click
    with: { role: button, name: Sign in }
  - id: session
    uses: ui/capture-session

criteria:
  - id: AC-1
    description: An administrator can see the workspace list.
    checks:
      - id: AC-1.1
        session: $setup.session.outputs.state
        steps:
          - uses: ui/navigate
            with: { url: $inputs.workspaces_url }
          - uses: ui/assert-element
            with:
              locator: { role: heading, name: Workspaces }
              expected: visible
```

`session:` must be one whole `$` reference; inline cookie/state literals
are rejected. Each check still gets a fresh isolated context, so mutations
do not leak to siblings. Omit `session:` for the signed-out path. The
captured `state` is contract-secret automatically—do not add a `secret_outputs:`
entry—and evidence carries only the expression plus a SHA-256 digest.

### Reuse a step sequence with `flows:`

Session capture (above) is the right tool when every check should carry
forward the *same* signed-in identity. Sometimes there's no single
session to share — an admin check and a member check need to sign in as
*different* users. Put the repeated step sequence in a `flows:` catalog
once, parameterize what varies, and invoke it with `call:` wherever you
need it:

```yaml
pages:
  login:
    username: { role: textbox, name: Username }
    password: { role: textbox, name: Password }
    submit: { role: button, name: Sign In }

flows:
  sign_in:
    description: Sign in with supplied credentials
    params:
      user: { type: string }
      password: { type: string, secret: true }
    steps:
      - uses: ui/type
        with: { locator: $pages.login.username, text: $params.user }
      - uses: ui/type
        with: { locator: $pages.login.password, text: $params.password }
      - uses: ui/click
        with: { locator: $pages.login.submit }

criteria:
  - id: AC-1
    description: An administrator can open the settings page.
    checks:
      - id: AC-1.1
        steps:
          - uses: ui/navigate
            with: { url: $inputs.login_url }
          - call: sign_in
            with: { user: $inputs.admin_user, password: $inputs.admin_password }
          - uses: ui/navigate
            with: { url: $inputs.settings_url }
          - uses: ui/assert-element
            with:
              locator: { role: heading, name: Settings }
              expected: visible

  - id: AC-2
    description: A regular member can also open the settings page.
    checks:
      - id: AC-2.1
        steps:
          - uses: ui/navigate
            with: { url: $inputs.login_url }
          - call: sign_in
            with: { user: $inputs.member_user, password: $inputs.member_password }
          - uses: ui/navigate
            with: { url: $inputs.settings_url }
          - uses: ui/assert-element
            with:
              locator: { role: heading, name: Settings }
              expected: visible
```

A step picks exactly one of `uses:` or `call:`. `flows:` composes the
same way as `pages:` — declared on the root manifest, an include, or a
leaf, root-wins on name collision. A flow body is **hygienic**: its
steps see only `$params.*` and `$pages.*`, never the caller's
`$inputs.*` or `$steps.*` — pass every dependency through `with:`
explicitly, so the flow stays portable across suites. A flow may
declare its own `outputs:` map (`$steps.<inner-id>.outputs.<name>`) to
expose one of its inner step's results to the caller as
`$steps.<call-id>.outputs.<name>`; inner step ids are otherwise
private. `duhem resolve --provenance` prints the expanded steps and
which flow each one came from — useful when a call's behavior is a
surprise.

**Running one leaf by path still resolves the catalog.** `duhem run
path/to/leaf.yml` (or `-f`) doesn't require going through the suite
manifest: it walks up from the leaf's own directory for a root
manifest — the same ancestor search a path-less `duhem run` uses — and,
if one is found, merges its `pages:`/`flows:` catalog into the leaf
before resolving `$pages.*`/`call:`. Sibling leaves the manifest lists
are not loaded or run; the run stays scoped to the one you asked for.
If no manifest is found and the leaf still references an undeclared
`$pages.*` name or flow, it fails naming exactly what's missing.

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

   Substitution in a `with:` value is whole-string only. An embedded
   reference such as `"#row-$inputs.id"` stays literal — it is not an
   error, and the action receives it verbatim. Compose a value with
   `$runtime.format(fmt, args...)` or `$runtime.concat(args...)` instead:

   ```yaml
   - uses: ui/navigate
     with:
       url: $runtime.format("{}/{}/{}", $inputs.base_url, "projects", $steps.create.outputs.body.id)
   ```

Then `duhem validate` (catch shape errors early) and `duhem run`. When
composition or input precedence is surprising, inspect the exact
pre-execution document first:

```bash
duhem resolve . --profile staging --provenance
duhem resolve . --profile staging --format json > resolved.json
```

`resolve` merges includes, expands leaves, resolves inherited and
profile/process/default inputs, substitutes static step values, and
fills default timeouts. Secret values stay `••••••`; validation
failures are reported inside the output. It never launches a browser,
runs `provision:` hooks, writes evidence, or performs network I/O.
JSON is the stable machine-readable form; YAML is optimized for
humans.

A type error in an assertion — say `contains` against a number — is a
`fail` that names the mismatch, not a silent pass.

## 6. Verifying a real workload that needs a running system

Most real checks need the system *up* first. A VD (or a suite manifest)
can declare environment hooks that provision it once and tear it down
after:

```yaml
provision:
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
