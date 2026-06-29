# `chreode-factory-cli`

Worked example for Duhem's [`cli/invoke`](../../docs/duhem-spec.md)
action (#102): it drives Chreode's headless entry point —
`pnpm chreode "<description>"` — and judges the real process.

- Criterion prose: [`criteria.md`](criteria.md)
- Verification Definition: [`duhem.yml`](duhem.yml)

## What it verifies

| Criterion | Commitment |
| --------- | ---------- |
| **AC-1**  | `pnpm chreode "<description>"` runs the full factory pipeline with no server, exits 0, and prints `shipped → <url>`. |

`cli/invoke` runs the **real** binary — no shimmed shell, no fake exit
code (`docs/duhem-spec.md` §8). Determinism without mocking comes from
Chreode's default FakeAgent + dry-run mode, which still stands up a real
local preview server.

## Operator setup

1. An `onsager-ai/chreode` checkout with deps installed (`pnpm install`)
   and `pnpm` + Node on `PATH`.
2. A Playwright Chromium for Duhem's browser. The check has **no UI
   step**, but `cli/invoke` (like `api/call`) still reports
   `requires_page = true` today, so the runtime opens a browser anyway
   — stripping it for non-UI checks is a deferred optimization (#105
   tracks browser provisioning). On a host where the bundled Playwright
   can't install Chromium, point Duhem at an existing binary:
   ```sh
   export DUHEM_BROWSER_EXECUTABLE=/path/to/chrome
   ```

## Running

Run **from the duhem repo root** — `cli/invoke` resolves a relative
`cwd` against the `duhem` process working directory, and the default
`chreode_repo_dir` is `../chreode`:

```sh
duhem run verifications/chreode-factory-cli/duhem.yml
```

Point at a Chreode checkout elsewhere:

```sh
duhem run verifications/chreode-factory-cli/duhem.yml \
  --inputs chreode_repo_dir=/abs/path/to/chreode
```

## Status

Proven green end-to-end against a real Chreode checkout: `verdict: pass`,
`exit_code == 0`, stdout matched `shipped`. The factory pipeline ships
in a few seconds in FakeAgent + dry-run mode.
