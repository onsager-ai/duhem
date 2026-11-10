---
name: onsager-dogfood
description: Resolve Duhem/product ownership and verify an authorized dogfood or consumer-compatibility change using co-located product VDs, independent mechanical verdicts and explicit cross-repo contracts. Use for ownership questions, consumer drift, verdict evidence or in-scope dogfood wiring.
---

# Duhem product dogfood

## Scope

This repo skill owns Duhem's side of the verification seam. Product code and
product Verification Definitions live with the product under `.duhem/`; only
Duhem self-verification VDs live here under verifications/. The current session's
actual scope and authority must cover the work. A skill trigger never authorizes
another repo's review, gatekeeping or VD authoring. Consult the native harness
adapter for its existing restriction; do not expand Claude's repo-only scope.

Read docs/duhem-spec.md §10.1 Pattern D, §11.2 and relevant Appendix D/§14 context.
Use [duhem-dev-process](../duhem-dev-process/SKILL.md) for Duhem's policy overlay,
[verification-authoring](../verification-authoring/SKILL.md) for VD method and
[harness-operations](../harness-operations/SKILL.md) only for tool mapping.

## Prerequisites

Identify the question's knowledge owner, artifact owner, acceptance criteria and
exact Duhem/product refs. If answering needs product knowledge outside the session
scope, hand it to that product's authorized session. A Duhem defect reported from
consumer usage remains a Duhem issue. Record handoff for work already in flight.

Historical Onsager work is paused. Product VDs were moved out of Duhem by epic
#225. The former Chreode drift lane is retired (#380; last on main at be5a8b2),
pending its deploy-contract replacement. Read current contracts before enabling
new wiring; do not reactivate a retired lane from an old skill example.

## Procedure

1. Resolve ownership from the artifact's current home and the knowledge needed.
   Schema/CLI/runtime/judge changes belong to Duhem. Product behavior and its
   co-located VDs belong to that product. Cross-repo work is separate specs/PRs
   linked by the contract each side observes/exposes; follow the actual dependency
   ordering without creating an impossible requirement for both PRs to merge first.
2. For an authorized product-side VD task, work in the product's checkout and
   contribution process. Lift its criteria; scaffold the co-located suite through
   its pinned Duhem tool/templates; exercise the real environment without mocking
   the delivery web. Missing Duhem surface requires its own Duhem spec and worked VD.
3. Distinguish Mode A from Mode B. Mode A is a product self-gate using its pinned
   Duhem. Mode B is an explicitly declared Duhem consumer-drift run at pinned
   product refs using the candidate tool. A Mode B result does not establish that
   the product has adopted that tool version. Do not claim Mode B is active merely
   because a retired workflow once implemented it.
4. For pin currency use .github/drift-consumers.yml and the owning consumer-pins
   workflow/xtask. That scheduled lane is warn-only and may be skipped until armed;
   skipped/warn-only currency evidence is different from a passing product gate.
5. Inspect verdict evidence and target/tool provenance. Mechanical judgment remains
   independent of the author; no LLM judges the verdict. Optional product CODEOWNERS
   and hub-recorded verdicts retain their declared ownership/enforcement posture.
   Never weaken a product criterion/check silently or self-attest past fail.
6. For fail or inconclusive, localize from evidence while preserving holistic limits.
   Use the recorded product's verdict policy; do not assume one default applies to
   every consumer. Handle environment failure, missing observations and actual
   contract defects separately. Route product repairs to its owner and Duhem
   defects to Duhem; do not use this skill as unsolicited external gatekeeping.
7. Publish only under task/repo authority using [issue-spec](../issue-spec/SKILL.md),
   [pre-push](../pre-push/SKILL.md) and [pr-lifecycle](../pr-lifecycle/SKILL.md).
   Each spec/PR references its own delivery slice and the cross-repo contract.

## Completion

Report artifact/knowledge owners, session scope, tool/product pins, actual suite
and verdict evidence, pin-currency result, linked specs/PRs and handoffs. Distinguish
an authoring draft, product gate pass, Duhem drift pass and consumer adoption.
