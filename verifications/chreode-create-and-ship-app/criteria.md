# Chreode — create an app from a description, and ship it

Acceptance criteria for Chreode's core promise: a person describes an
app in plain language and Chreode builds and ships a working app for
them. These criteria are the stable human commitment; the checks in
`duhem.yml` are the derivative mechanism (`docs/duhem-spec.md`
§7.2 / §7.3).

Target: `onsager-ai/arbor` (Duhem dogfood customer #2). Verified
against the real factory pipeline running locally in its default
deterministic mode — FakeAgent (no live LLM) plus the dry-run deploy
drivers, which provision a **real** local preview server and return a
clickable live URL. No mocks at the web boundary
(`docs/duhem-spec.md` §8).

## AC-1

A person can describe an app in plain language on the dashboard and
start building it. Submitting the description begins a build run and
takes the person to that run's page so they can watch it progress. No
errors are shown.

## AC-2

A build run started from a description runs the full factory pipeline
— understand, plan, generate, verify, deploy — and finishes by
shipping a live, reachable app. The run's page surfaces the shipped
app's live link once it is ready.

## Identity-commitment notes

- **Holistic.** Every check drives the real Chreode dashboard against
  the real orchestrator, the real factory pipeline, and a real
  (dry-run) deploy that stands up an actual preview server. The
  default FakeAgent/dry-run mode keeps the run deterministic and
  zero-cost without mocking any seam (`docs/duhem-spec.md` §8).
- **Mechanical judgment.** Assertions are structural — equality on an
  observed HTTP status, role-based locators reaching a DOM state. No
  LLM interprets the verdict (`docs/duhem-spec.md` §11.2).
- **Asymmetric trust.** Duhem authors these checks against Chreode;
  Chreode never authors its own Duhem checks. The live link appearing
  is a fact about the real run, not a claim Chreode makes about itself.
- **Two-document discipline.** This file is the human commitment;
  `duhem.yml` is its mechanical translation. Keep them separate.
