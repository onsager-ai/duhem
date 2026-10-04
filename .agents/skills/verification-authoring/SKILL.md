---
name: verification-authoring
description: Author Duhem criteria/check Verification Definitions or worked product-surface examples in an authorized owning checkout. Preserve stable human intent, real-environment verification, deterministic assertions and independent verdicts.
---

# Duhem verification authoring

## Scope

Owns Duhem criteria-to-check authoring method. docs/duhem-spec.md §7/§8/§10/§11.2
owns the product contract; [duhem-dev-process](../duhem-dev-process/SKILL.md) owns
this repo's spec, worked-example, schema/DX and merge gates. A product's VDs live
in that product's `.duhem/`, and its contribution process owns their changes.
Only Duhem self-verification lives here in verifications/.

The current session must be authorized for the owning checkout. This skill does
not expand a harness-specific scope restriction. For seam ownership use
[onsager-dogfood](../onsager-dogfood/SKILL.md); for current tool mapping use
[harness-operations](../harness-operations/SKILL.md).

## Prerequisites

Identify the criteria source, owning suite and pinned Duhem/schema version. Read
relevant spec sections and the [authoring reference](references/authoring-reference.md)
for criteria/check structure, actions/assertions, examples and refusal tests.
Check the actual CLI/action catalog; do not assume an example's action exists at
another pin. Product-surface changes here require a worked VD through the local
overlay, not through a hardcoded rule in a shared spec skill.

## Procedure

1. Lift existing human intent verbatim. Keep criteria stable across implementation
   changes; checks are derivative. One criterion is one coherent commitment.
2. Scaffold with the owning pinned tool or templates and establish a known-good
   real-environment baseline before authoring. Product suites use their explicit
   `.duhem/` path; do not place consumer VDs in this repo by historical inference.
3. Translate each criterion into named checks/steps and deterministic assertions
   using supported actions and outputs. Preserve the detailed reference's IDs,
   expression, assertion, fixture and environment requirements.
4. Exercise the real delivery web together: code, prompts, tools, data and runtime.
   Do not mock that web, including dogfood, or introduce LLM judgment of a verdict.
5. Prove meaningful failures/refusals as well as success. Checks that cannot fail
   on a violated criterion do not establish it. Keep evidence independently
   recorded by the verifier and avoid a product self-attesting past fail.
6. Validate every modified VD through the owning pinned CLI and run the applicable
   real suite/gates. Inspect structured verdict/evidence. Parser validation alone
   does not establish behavioral verification or independence.
7. Register the VD in its owning suite and update the affected contract/examples.
   Publish through the owning contribution process. Shared
   [issue-spec](../issue-spec/SKILL.md), [pre-push](../pre-push/SKILL.md) and
   [pr-lifecycle](../pr-lifecycle/SKILL.md) supply method, not extra authority.

## Completion

Report criteria source, suite/tool/schema pins, check-to-criterion mapping,
validation and real-run/negative evidence, actual verdict, modified registrations
and blocked prerequisites. Distinguish authored, schema-valid and behaviorally
verified definitions. A missing real environment is not a passed verification.
