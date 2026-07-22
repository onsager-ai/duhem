# Verification corpus

This directory holds **Duhem's own** Verification Definitions (VDs) —
where you come to *watch Duhem author real checks*. Every file here runs
end-to-end against a real target; none of it is mocked (that's the
[Holistic Verification Principle](../docs/duhem-spec.md#8-the-holistic-verification-principle)).

Two kinds of directory live here, for two kinds of reader:

- **Feature examples** (`*-example/`) — each isolates ONE manifest or
  authoring feature and works it end-to-end, with a spec number in its
  header comment. Walk them in the ladder below to learn Duhem's shape
  one concept at a time.
- **Self-verification suites** — Duhem's dogfood: full-scale VDs that
  drive the *shipped* `duhem` binary, its dashboard, and the DB actions,
  and gate this repo's CI. Read these for what a real, multi-step,
  real-system check looks like.

> New to Duhem? Start with the
> [getting-started guide](../docs/getting-started.md), then walk the
> ladder below in order. From the repo root, `duhem run
> verifications/<dir>` runs any of them.

## Feature examples — the learning ladder

Read top to bottom: each leans on the mental model the previous one
built. Examples 1–3 are single-file authoring; 4–6 are suite
composition.

| # | Directory | Teaches | Runs against | Spec |
|--:|-----------|---------|--------------|:----:|
| 1 | [`defaults-example`](defaults-example/) | Root-manifest `defaults:` — set the suite-wide timeout + inconclusive policy **once**, not per leaf | nothing — fully offline (`noop/*`) | #66 |
| 2 | [`implicit-outputs-example`](implicit-outputs-example/) | Reference `$steps.<id>.outputs.<field>` with **no `outputs:` ceremony** — the validator resolves it against the action contract | network (`api/call` → example.com) | #267 |
| 3 | [`implicit-judgment-example`](implicit-judgment-example/) | Judging steps (`ui/assert-*`, `api/poll`) **self-assert** — an all-assert check needs no `assertions:` block; contrasts terse vs. explicit authoring | browser + a local login app on `:8080` | #253 |
| 4 | [`includes-example`](includes-example/) | `includes:` composition — a shared committed fragment plus a per-developer override, under the root-wins merge rule | network | #67 |
| 5 | [`environments-example`](environments-example/) | Named `environments:` — declare configs, select one (`--environment`), feed the leaf input chain and the `$env.*` whitelist | network | #68 |
| 6 | [`inherits-example`](inherits-example/) | Inherited inputs — a leaf lists input *names* under `inherits:` and reads suite-wide values the manifest provides (a DI container) | network | #135 |

## Self-verification suites — Duhem's dogfood

Real VDs that gate this repo's CI. Bigger, multi-step, and pointed at a
real system stood up by `environment:` hooks. This is what production
Duhem usage looks like — and, being Duhem-on-Duhem, it catches
*regressions* in the shipped tool, which is why it self-gates here.

| Directory | Drives | Real target | Spec |
|-----------|--------|-------------|:----:|
| [`duhem-cli`](duhem-cli/) | The shipped `duhem` binary via `cli/invoke` — its black-box command-line contract | the `duhem` build under test | #148 |
| [`duhem-dashboard`](duhem-dashboard/) | The `duhem dashboard` web surface — embedded SPA + JSON API + live SSE — over a real run's evidence | a dashboard server on `:7878` (`up:`/`down:` hooks) | #148 |
| [`db-task-state`](db-task-state/) | `db/seed` + `db/query` — asserts a task-lifecycle fact that lives in **database state** | a seeded DB (`up:` hook) | #101 |

## Where product VDs live

These are *Duhem's own* VDs. A VD that verifies a **product** lives
co-located in that product's repo under a `.duhem/` suite (epic #225) —
e.g. `onsager-ai/chreode/.duhem/`, self-gated in its own CI and
drift-monitored here. The drop-in skeleton for a new product repo is
[`templates/product-repo/`](../templates/product-repo/); the shape is
[`docs/duhem-spec.md`](../docs/duhem-spec.md) §10.1 Pattern D.
