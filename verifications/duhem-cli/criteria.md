# Duhem — the `duhem` CLI contract (self-regression)

Acceptance criteria for the `duhem` command-line interface, verified
black-box by driving the **real** binary through the `cli/invoke`
action (`duhem.yml` is the derivative mechanism; these criteria are the
stable human commitment). Part of epic #148 (Duhem-on-Duhem regression
coverage).

**Self-reference caveat.** This is regression coverage, not an
independent trust anchor. Duhem verifying Duhem is correlated failure
(a judge defect could wrongly pass its own self-test), so a green run
here means "no CLI regression detected", never "independently
attested". The asymmetric Onsager seam holds the trust role
(`docs/duhem-spec.md` §11.2). It is also doubly self-referential —
AC-5 runs `duhem run` inside a `duhem run`.

Target: this repo's own `duhem` binary. No mocks at the boundary
(`docs/duhem-spec.md` §8) — `cli/invoke` runs the real process and
judges its exit code, stdout, and stderr.

## AC-1

Asking the CLI for its version reports both the product version and the
schema version it speaks, so an operator can tell at a glance which
`duhem` and which Verification-Definition schema they are on.

## AC-2

Validating a well-formed Verification Definition succeeds — the
structural validator accepts a definition that is in fact valid, so
authors get a clean signal and CI does not false-fail.

## AC-3

Validating a malformed Verification Definition fails loudly — the
validator rejects it with a non-zero exit and an error that names the
offending part, so a broken definition cannot slip through as if it
were valid.

## AC-4

Scaffolding a new Verification Definition produces one that is itself
valid — `duhem init` emits a definition that `duhem validate` accepts,
so an author's very first run is green before they have written
anything.

## AC-5

Running an offline example end-to-end reaches a pass verdict — the full
`duhem run` pipeline (load → validate → execute → judge → report) works
on a known-green definition without any external dependency.
