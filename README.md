# Duhem

A holistic verification platform for AI-delivered software. Sits between
AI coding agents and production: captures human intent as acceptance
criteria, translates them into mechanically-judged checks that exercise
the real delivery web (code + prompts + tools + data + runtime), and
gates merge/deploy on the verdict.

> **Status:** Phase 0 — Foundation. The repo currently contains the
> product spec and brand docs; the CLI, runtime, and judge are being
> stood up. See `docs/duhem-spec.md` §14 for the roadmap.

## Why "Duhem"

Pierre Duhem argued that no scientific hypothesis can be tested in
isolation — only the whole web of theory, apparatus, and assumption
gets tested. AI delivery is the engineering instance of that thesis:
when an agent ships a feature, what gets delivered is code × prompt ×
tool config × data state × runtime × upstream contracts. Verification
must be holistic, mechanical, and structurally independent of the AI
making the claim. `docs/duhem-spec.md` Appendix C unpacks the
philosophy; `docs/duhem-brand.md` shows how the mark visualizes it.

## Reading order

1. **`docs/duhem-spec.md`** — canonical product specification. Start
   with §1 (Why), §4 (Solution Overview), §7 (Core Concepts).
2. **`docs/duhem-brand.md`** — the mark, design rationale, and
   relationship to Onsager.
3. **`.claude/skills/`** — the dev process for working on this repo
   under Claude Code. Start with `duhem-dev-process`.

## Onsager

Duhem's first dogfood customer is [`onsager-ai/onsager`](https://github.com/onsager-ai/onsager).
The relationship and the cross-repo seam are documented in the
`onsager-dogfood` skill and `docs/duhem-spec.md` Appendix D.

## Contributing

Non-trivial changes start as a GitHub spec issue on this repo. See
`.claude/skills/duhem-dev-process/SKILL.md` for the SDD loop and
`.claude/skills/issue-spec/SKILL.md` for spec-issue authoring.
