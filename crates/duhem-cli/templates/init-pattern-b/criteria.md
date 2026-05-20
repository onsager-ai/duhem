# {{NAME}}

Acceptance criteria for {{NAME}}. Authored as a Duhem
Verification Definition skeleton by `duhem init --pattern B`.
Replace each criterion with one that describes a real commitment
of your feature.

Criteria are stable across implementation churn; their
mechanical translation lives next door in `duhem.yml`. The root
manifest one level up (`../duhem.yml`) aggregates this and any
sibling Verification Definitions.

## AC-1

The example.com landing page renders its canonical heading.

> Replace this criterion before flipping your spec issue to
> `planned`. See `docs/duhem-spec.md` §7.2 / §7.3 for the
> criteria-vs-checks discipline.

## Identity-commitment notes

- **Holistic.** Skeleton check exercises a real public URL via a
  real browser. No mocks at the web boundary
  (`docs/duhem-spec.md` §8).
- **Mechanical judgment.** Assertions are structural; no LLM
  interprets the verdict.
- **Two-document discipline.** `criteria.md` is the human
  commitment; `duhem.yml` is its mechanical translation. Keep
  them separate from the first commit.
