# Deterministic UI Assertion Layer — Phase 0 Ground Truth & Gate 0 Stop

**Mode:** investigation. **Status:** stopped at Gate 0 (corpus unavailable).
**Date:** 2026-07-24. **Author:** Claude Code session on `claude/ui-assertion-research-qpkgdt`.

> **One-line verdict:** *Phase 0 clears; **Gate 0 fails** — this ephemeral cloud
> session cannot obtain ≥20 real Chreode-generated pages, so per §4 I stop before
> Phase 1 rather than measure determinism on synthetic or wrong-corpus pages.*

This is the sanctioned "report and stop" outcome, not a dead end: the blocker is
infrastructural and has a precise, cheap unblock (§4 below). Everything that
*could* be verified without the corpus was verified; the reproducibility and
failure-distribution numbers that would decide *build / don't build* cannot be
honestly produced here, and I have not faked them.

---

## 1. KB note — read and treated as source of record for §1

Note *"UI 审美的可测性分层与静态断言设计"* (Notes DB, Status: Inbox, Domain
`foundry/duhem` + `self/learning`, sourced "Claude 对话 + 文献检索 2026-07-24").
It confirms the brief's §1 model and adds the academic lineage the brief
compressed. Load-bearing points, verbatim-faithful:

- **L1/L2/L3 ladder** as in the brief. Its sharpest framing: *L2 is the most
  dangerous layer — it looks assertable but its threshold is human preference in
  disguise.* This is exactly the brief's "L2 is the trap."
- **Computational-aesthetics line (all L2/L3, all with an oracle problem):**
  Birkhoff 1933 (M = O/C); Ngo 2003 (14 equally-weighted metrics — "most cited,
  most doubted"); **Bauerly & Liu 2006** — the load-bearing negative result:
  *"balance" correlates with nothing*, i.e. a metric can compute cleanly and
  still not measure what it claims. Reinecke CHI 2013 (450 sites / 548 subjects,
  colorfulness + complexity explain ~**50%** of first-impression variance — *the
  L2 ceiling*). Deep-learning line (Webthetics/NIMA) needs >16 000 labelled
  screenshots.
- **Software-engineering line (the one that actually shipped L1)** by reframing
  aesthetics-scoring as **defect detection**: **ReDeCheck** (ISSTA 2017) builds a
  *Responsive Layout Graph* (nodes = elements, edges = above/below/contained +
  per-width visibility), samples 320–1400px at 60px steps + CSS-declared
  breakpoints, binary-searches change points, and emits **5 RLF types**
  (Element Collision / Element Protrusion / Viewport Protrusion / Small-Range /
  Wrapping). Crucially it needs **no oracle** — it compares a page against
  *itself* across viewports. This is the brief's class-D (metamorphic) thesis
  with a citation.
- **Tri-state is not novel — it is ReDeCheck's TP / FP / NOI.** NOI
  (Non-Observable Issue) = DOM-level collision/overflow that is invisible to the
  eye (transparent/occluded edges). *NOI is the natural instance of
  `inconclusive`.* Follow-up tool **VISER** filters NOI by nudging element
  opacity and doing pixel-diff. → the brief's "observability gate" already exists
  as prior art; the open question is only whether it *generalizes* (Phase-4 item 4).
- **UI-Bench 2025** (10 tools / 300 sites / 4000+ expert judgements) *abandoned
  absolute aesthetic scoring for pairwise Thurstone comparison* because automated
  aesthetic metrics are "systematically misaligned with human preference." →
  independent confirmation of the brief's L3 claim: **aesthetics is ordinal, not
  cardinal.**

The KB note's own open questions match the brief's falsification targets exactly:
*"For AI-generated UI, is L1's fail-rate enough for a quality gate, or do most
defects land in L2/L3? What is the NOI/inconclusive ratio?"* — these are Gate 2,
and they are unanswerable without the corpus.

## 2. Duhem's assertion path — mapped from code (not memory)

**Where verdicts come from today (`crates/duhem-schema`, `duhem-judge`,
`duhem-actions`):**

- `Assertion` (`duhem-schema/src/assertion.rs`) is a **closed enum**:
  `Expr | TypeCheck | Matches | In | Exists | Equal`. The file's own doc comment
  states the closed shape *is* the structural enforcement of the "mechanical
  judgment, not LLM judgment" identity commitment — "there is no free-text
  let-the-LLM-decide variant." Any new predicate is either a new enum variant
  (**schema-breaking, goes through the schema-impact gate**) or lives outside the
  enum as an action.
- `Criterion { id, description (opaque prose), checks[] }` → `Check { id, steps[],
  assertions[] }` (`criterion.rs`). Criteria are stable; checks derivative — the
  brief's §1 freeze discipline is already the repo's identity commitment.
- **Two judging paths, and the second is the natural plug-in point.** Per
  `criterion.rs` (spec #253): a check may carry explicit `assertions[]`, **or**
  carry none and rely on **judging steps** — actions whose contract emits a
  boolean `satisfied` output. The existing `ui/assert-element` /
  `ui/assert-state` / `ui/assert-url` actions are exactly this shape:
  async-trait `Action`s with an `ActionContract` whose `outputs` include
  `satisfied`.
- **Rendering is already fact-collection, not assertion.** `duhem-actions`
  drives a **Playwright Node sidecar over newline-delimited JSON-RPC** (`browser.rs`,
  #71): one sidecar per `duhem run`, one browser context + page per check. The
  judge itself is Rust, LLM-free, and evaluates step outputs. **Duhem's
  architecture already embodies the brief's §1.2 pivot** ("rendering belongs to
  fact collection; assertions stay pure").

**Answer to "where would a domain-specific assertion pack plug in?"** Two seams,
and they compose:

1. **A Facts-collection action** — e.g. `ui/collect-facts` — implemented like the
   existing browser actions against the sidecar, emitting a serialized `Facts`
   snapshot (per-element box + computedStyle subset + text metrics + visibility +
   stacking) as a normal step output. This is additive, **non-schema-breaking**.
2. **Predicate actions over that snapshot** — `ui/assert-no-overlap`,
   `ui/assert-within-viewport`, `ui/assert-token-conformant`,
   `ui/assert-breakpoints`, each emitting `satisfied` (+ an `inconclusive`
   signal — see gap below). Also the **non-breaking #253 path**; no new
   `Assertion` enum variant required.

**One real gap the code surfaces:** the judge is currently **two-state**. Actions
report `satisfied: true|false` with `Outcome::{Ok, Timeout, ActionError}`; there
is no first-class `inconclusive` verdict that folds through aggregation the way
`pass`/`fail` do. The brief's observability gate (Phase-4 item 4) is therefore not
just a new action — it is a **schema/judge change to tri-state verdicts**, which
is the load-bearing design question and a schema-impact item. *Flag for human:
whether `inconclusive` becomes a first-class verdict in `duhem-judge` is the
single biggest architectural decision here, and it touches the identity-committed
judge.*

## 3. Corpus — Gate 0 assessment (the blocker)

**Requirement:** ≥20 *real generated* pages; "synthetic fixtures do not answer
the question." **Accessible count: 0.** Evidence:

| Source checked | Finding | Usable? |
|---|---|---|
| Chreode repo in-tree | It is the **factory**, not an output store. Only `index.html` files are Chreode's *own* surfaces (`apps/app/web`, `apps/landing`) + one `packages/factory/templates/app` template. | No — no generated output stored |
| Live generation here | Chreode `CLAUDE.md`: **the dev container has no docker daemon**; `CHREODE_DRIVERS=docker` is "host-run only"; real agent runs "spend money/quota, keep them rare and intentional." Investigation brief forbids product runs (§5). | No — no daemon, not sanctioned |
| Gauntlet slate | `GAUNTLET_SLATE` = **7 app definitions** (2 blocked on #171), each with a 2–3 step evolution script. Fully run ≈ 7 apps × ~3 states ≈ ~20 page-states — **but only against the live agent on a docker host.** | Path exists, not runnable here |
| FakeAgent (zero-spend) | Copies **one** checked-in template app (`packages/agent-runner/src/fake.ts`, ADR 0002). Single page, deterministic, synthetic. | No — explicitly excluded |
| Vercel deployments | User's projects = v0.dev exports (`v0-*`), Crawlab marketing/docs sites, personal apps. **Not Chreode output**; heterogeneous production sites (auth-gated / evolved), wrong source. | No — wrong corpus |

Scraping 20 arbitrary production Vercel sites 5× each to fake a corpus is exactly
the "expensive validation on the wrong corpus" the brief's guardrails forbid, and
it would not answer the question the brief asks (determinism of *Chreode's*
generated-UI facts).

## 4. What clears Gate 0 (precise, cheap)

The gauntlet is the intended generator; it just needs infrastructure this session
lacks. To clear Gate 0 and unblock Phases 1–4, run **one** of:

1. **Preferred — run the gauntlet on a docker host with a real agent.**
   On a machine with a docker daemon: `CHREODE_AGENT=claude`, `CHREODE_DRIVERS=docker`,
   run `GAUNTLET_SLATE` tiers A + D through their evolution scripts. That yields
   ~20 real generated page-states with provenance, in the intended source. Cost:
   one bounded real-agent gauntlet run (money/quota — an explicit, human decision).
   Land the generated pages (or their built `dist/`) as a frozen corpus fixture so
   the determinism probe is reproducible.
2. **Point me at an existing corpus** — a directory of ≥20 previously-generated
   Chreode app builds, or ≥20 stable preview URLs from a running instance. Then
   the Phase-1 Playwright harness runs here (Chromium *is* available in this
   environment) with no daemon needed.
3. **Human waiver** to substitute a labelled non-Chreode corpus (e.g. the v0.dev
   exports) — but this changes what the result generalizes to and must be recorded
   as a deviation, not silently absorbed. *This is a decision for the human, not me.*

## 5. Provisional design skeleton — NOT a recommendation

To save re-derivation *if* Gate 0 later clears. This is **un-falsified**: none of
the numbers that would justify it exist yet. Do not treat as a conclusion.

- **Facts schema** — versioned, serializable, diffable; the stored artifact. Maps
  cleanly onto the sidecar's existing per-element access.
- **Settling protocol** — `document.fonts.ready`, forced `animation/transition:
  none`, image `decode()`, fixed DPR/locale/scroll; watch specifically for the
  axe-core #1730 class (`scrollIntoView` no-op giving different verdicts at
  adjacent widths). *Unproven until Phase 1 measures reproducibility.*
- **Predicate algebra** — A (unary/token), B (binary/relational), C (global), D
  (cross-state metamorphic). D is highest-value (no oracle) and is *exactly*
  ReDeCheck's RLG — **inherit, don't reinvent**.
- **Observability gate** — generalize ReDeCheck's NOI / VISER opacity-pixel-diff
  into one mechanism, expressed as a **first-class `inconclusive` verdict** in
  `duhem-judge` (see §2 gap). *This is the main claimed contribution and the main
  open risk — Phase-4 item 4 says "if it can't be generalized, say so"; that can
  only be decided after Gate 2.*
- **DSL vs typed host API** — Galen (`.gspec`) is **last released 2.4.4,
  2019-03-15, unmaintained** (verified). Its *language ideas* (`@objects`,
  `@rule`+`@forEach`, `@on <tag>` viewport groups, tolerances, spec-mutation
  testing) are worth inheriting; a *custom DSL* almost certainly is not — Duhem
  already has a typed assertion/action model, and the #253 action path gives
  composable predicates **for free**. Provisional lean: **no new DSL; a typed
  action pack + Facts snapshot.** Argue both sides properly in Phase 3.
- **Freeze layer** — DTCG **reached first stable (2025.10) on 2025-10-28**
  (verified). Interop with DTCG for `TokenSet`/`SpacingScale`/`TypeScale` rather
  than a bespoke format.
- **Integration (Phase-4 item 9)** — provisional: a **Duhem domain pack**
  (Facts-collection action + predicate actions), not a standalone tool, because
  Duhem already owns the sidecar, the LLM-free judge, and the criteria/checks
  freeze model. **Requires human confirmation against the KB** — flagged, not
  decided.

## 6. Boundary (unchanged by the stop)

This layer can decide **L1** only. It cannot decide any of **L3** (taste,
"ship-to-client") — UI-Bench proves that has no cardinal oracle — and it must not
touch **L2** thresholds inside the evaluator; any L2-shaped number belongs in the
freeze layer as a human decision, per §5 non-goals.

---

*Sources for currency checks:*
[Galen releases](https://github.com/galenframework/galen/releases) ·
[DTCG first stable 2025.10](https://www.w3.org/community/design-tokens/2025/10/28/design-tokens-specification-reaches-first-stable-version/)
