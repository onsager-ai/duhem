# Duhem — the dashboard web surface (self-regression)

Acceptance criteria for the `duhem dashboard` product surface, verified
black-box by driving the **real** dashboard binary — its embedded
React/Vite SPA, its JSON API, and its live SSE stream — over a **real**
run's evidence (`duhem.yml` is the derivative mechanism; these criteria
are the stable human commitment). Part of epic #148 (Duhem-on-Duhem
regression coverage).

**Self-reference caveat.** This is regression coverage, not an
independent trust anchor. Duhem verifying Duhem is correlated failure
(a judge defect could wrongly pass its own self-test), so a green run
here means "no dashboard regression detected", never "independently
attested". The asymmetric Onsager seam holds the trust role
(`docs/duhem-spec.md` §11.2).

Target: this repo's own `duhem dashboard`. No mocks at the boundary
(`docs/duhem-spec.md` §8) — `environment.up` produces a genuine run
through the real `duhem run` pipeline, then serves it with the real
dashboard binary. The SPA only renders a verdict because the API
actually served one; the API only serves one because the judge actually
recorded it.

## AC-1

The dashboard's JSON API serves the run's recorded verdict and its
criteria/check breakdown in the documented shape, so any client (the
SPA, a static export, an operator's curl) reads the judge's verdict
straight from the evidence — never a re-judged one.

## AC-2

An operator opening a run in the dashboard sees the run rendered: the
app shell loads, the run's verdict is shown, and the run's criteria and
checks are listed. The browser actually executes the shipped SPA bundle
against the live API — not a placeholder page.

## AC-3

The dashboard's live stream replays a run's evidence to a connecting
client over Server-Sent Events, ending the stream once the run has
finished — so an operator watching a run receives the trace events and
the final recorded verdict, in the documented SSE shape.
