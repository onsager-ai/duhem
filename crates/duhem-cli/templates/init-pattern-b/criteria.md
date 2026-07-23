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

The example.com landing page is reachable and serves its content.

> Replace this criterion before flipping your spec issue to
> `planned`. A good criterion reads as intent, not procedure —
> no mechanism (browser, endpoint, table), just the commitment.

## Identity-commitment notes

- **Holistic.** Skeleton check exercises a real, deployed system
  end-to-end. No mocks at the web boundary.
- **Mechanical judgment.** Assertions are structural; no LLM
  interprets the verdict.
- **Two-document discipline.** `criteria.md` is the human
  commitment; `duhem.yml` is its mechanical translation. Keep
  them separate from the first commit.
