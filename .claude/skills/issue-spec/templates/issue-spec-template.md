<!-- Issue body template for lean-spec style spec issues on onsager-ai/duhem -->
<!-- Title: spec(<area>): <short description> -->
<!-- Labels: spec, <type>, area:<area>, priority:<level>, draft -->
<!-- Plus schema-impact if the change touches the Verification Definition format -->

## Overview

<!-- Problem statement and motivation. 2-4 sentences.
     Why does this matter? What's the impact of not doing it?
     Don't describe the solution here — that's Design's job.
     Tie back to commitments in docs/duhem-spec.md when relevant. -->

## Design

<!-- Technical approach at intent level.
     Data flow, schema shape, judge contract — not line-by-line code.
     Include what's explicitly OUT OF SCOPE.
     Respect the Duhem invariants: holistic verification (no mocking
     the web), mechanical judgment (no LLM-in-the-loop verdict),
     stable criteria / derivative checks. -->

## Plan

- [ ] <!-- Verb + concrete deliverable -->
- [ ] <!-- Each item independently verifiable -->
- [ ] <!-- Order reflects implementation sequence -->

## Test

- [ ] <!-- Test type: what to verify -->
- [ ] <!-- Maps to plan items above -->

## Schema impact

<!-- Required when this change touches the Verification Definition format,
     the action-type catalog, runtime expressions, judge semantics, or any
     externally observable contract. Drop the section ONLY if the change
     provably touches no schema surface (e.g. CI tweaks, repo hygiene). -->

- Fields added / removed / renamed:
- Semantics changed:
- Migration path for in-flight Verification Definitions:
- Breaking change?  yes / no — if yes, add `schema-impact` label

## Worked example

<!-- Required when this change introduces or modifies user-visible
     product surface (new action type, new schema field, new CLI
     command, new judge behavior). A minimal Verification Definition
     showing the surface end-to-end, or a link to one. See the
     `verification-authoring` skill. -->

```yaml
# minimal example exercising the new surface
```

## Alignment

### Human decides
- [ ] <!-- Decision requiring judgment, context, or authority -->

### AI implements
- [ ] <!-- Concrete task tied to plan items above -->

### Open questions
<!-- Remove this subsection if none. Questions block draft → planned. -->

> <!-- Question with enough context to answer -->
> Impact: <!-- Which plan items are affected -->

## Notes

<!-- Tradeoffs, related issues (#N), references. Omit section if empty. -->
