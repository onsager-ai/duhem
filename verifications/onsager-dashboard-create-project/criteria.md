# Onsager dashboard — create project

Acceptance criteria for the Onsager dashboard's "create a new
project" flow. Authored as Duhem's first dogfood Verification
Definition (`onsager-ai/duhem#35`). Criteria are stable across
implementation churn: they describe what *done* means for the
feature, not how the UI happens to be wired today.

The translation of these criteria into mechanical checks lives
next door in `duhem.yml`.

## AC-1

An authenticated user lands on the create-project page and sees
the create-project form, including a heading that identifies the
page and a project-name input ready for entry.

## AC-2

Submitting the form with a valid project name navigates the user
to the new project's settings page.

## AC-3

Within five seconds of creation, the new project appears in the
user's project list at the projects-index page.

## Identity-commitment notes

These criteria are intent-bearing and free of implementation
language:

- No endpoint paths, function names, or DB tables.
- No step-by-step procedure ("click the button labelled …").
- A non-technical stakeholder can read each one and decide
  yes/no.
- The criteria survive plausible implementation changes — the
  same user-visible commitment holds whether the form posts to
  `/projects`, `/api/v2/projects`, or a GraphQL mutation.

Implementation-shape decisions (locator names, URL regex, login
flow) live in `duhem.yml` as checks. Per
`docs/duhem-spec.md` §7.3, checks may change as the implementation
changes; the criteria above must not.
