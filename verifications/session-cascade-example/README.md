# Session cascade

This browser example is **validated offline** by `scripts/validate-suites.sh`;
`just self-verify` does not execute it. Running it requires a real application
URL and operator-acquired Playwright storage state supplied through inputs.
It uses the runnable application's `Admin`/`Login` headings from
[`../named-sessions-example/`](../named-sessions-example/): set `base_url` to
its `/admin` page and acquire the operator seed through its real login flow.
Do not commit credentials or fabricated login state.

The leaf supplies a seed, AC-1.1 inherits it for setup/body/cleanup, AC-1.2
explicitly opts out, and AC-1.3 selects a named context at a flow call site.
Every block starts with fresh contexts; cleanup navigates again. Adapt the
flow's actions to the application's cleanup operation; its heading assertion
checks that cleanup really retained the administrator identity.

The runtime browser smoke suite exercises seeded lifecycle blocks, fixture
up/down, overrides, signed-out opt-out, and named cookie isolation against a
local application: `just test browser-actions`.
