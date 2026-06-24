# Crawlab — sign in and reach the dashboard

Acceptance criteria for Crawlab Pro's web dashboard (the Vue UI), the
companion to the REST dogfood (`../crawlab-create-project/`). A user can
sign in through the dashboard and reach the authenticated app, and the
project-management surface renders for them.

These criteria are the stable human commitment; `duhem.yml` is the
mechanism. Each check drives the **real** Vue dashboard against the
**real** Crawlab REST backend over a real MongoDB — no mocks at the web
boundary (`docs/duhem-spec.md` §8). Duhem authors these checks against
Crawlab; Crawlab never authors its own (the asymmetric-trust seam).

## AC-1

Signing in with valid credentials from the dashboard authenticates the
user and takes them into the app — the sign-in form is left behind and
the authenticated home view is shown. No errors are surfaced.

## AC-2

Once signed in, the project-management page renders the authenticated
projects surface — the list view with its create affordance — served
from the real backend.
