# Chreode — the factory CLI builds and ships an app

Acceptance criterion for Chreode's headless entry point: `pnpm factory
"<description>"` runs the full factory pipeline with no server and no
dashboard, and ships a working app. This is the worked example for
Duhem's `cli/invoke` action (#102) — the criterion is the stable human
commitment, `duhem.yml` the derivative mechanism.

Target: `onsager-ai/arbor`. Verified against the real factory pipeline
in its default deterministic mode — FakeAgent (no live LLM) + dry-run
drivers, which stand up a real local preview server. No mocks at the
web boundary (`docs/duhem-spec.md` §8); `cli/invoke` runs the **real**
`pnpm factory` binary and judges its exit code and output.

## AC-1

Running the factory CLI with a plain-language description builds an app
end to end — understand, plan, generate, verify, deploy — and exits
successfully, reporting that it shipped a reachable app.
