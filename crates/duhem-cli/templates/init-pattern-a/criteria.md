# {{NAME}}

Acceptance criteria for {{NAME}}. Authored as a Duhem
Verification Definition skeleton by `duhem init`. Replace each
criterion with one that describes a real commitment of your
feature.

Criteria are stable across implementation churn: they describe
what *done* means, not how the feature happens to be wired today.
The mechanical translation into checks lives next door in
`duhem.yml`.

## AC-1

The example.com landing page is reachable and serves its content.

> Replace this criterion before flipping your spec issue to
> `planned`. A good criterion:
>
> - Names a single user-visible commitment.
> - Survives plausible implementation changes (rewording a
>   button, restructuring a URL, swapping an API endpoint).
> - Reads as intent, not procedure: no "click the button labeled
>   X" prose, no endpoint paths, no DB tables.
> - Can be decided yes/no by a non-technical stakeholder.

## Identity-commitment notes

- **Holistic.** The skeleton's check exercises a real, deployed
  system end-to-end — no mocks at the web boundary. Your
  replacement should preserve this posture.
- **Mechanical judgment.** Assertions are structural — equality
  and predicates over observed outputs. No LLM in the loop
  interprets the verdict.
- **Two-document discipline.** This file is the human commitment
  (`criteria.md`); `duhem.yml` is its mechanical translation.
  Keep them separate from the first commit — conflating the two
  is a defect in the authoring discipline.
