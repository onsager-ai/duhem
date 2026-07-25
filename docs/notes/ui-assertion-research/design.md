# Deterministic UI assertion layer — design

**Verdict: build something smaller.** Build a class-C/D *responsive and
observability* checker with roughly six predicates, as a typed host-language
API over a versioned `Facts` snapshot, integrated as a Duhem action pack.
Do **not** build a Galen-style relational predicate DSL: the B-class
relational checks that would justify a metalanguage produced 0 confirmed
true positives out of 46 adjudicated findings on 23 production pages
(`phase2-failure-distribution.md`), while the metamorphic cross-viewport
sweep produced 3 real defects with no oracle. A parser, an LSP, error
messages and a docs surface are a permanent cost; six predicates do not
amortise it.

## 1. Prior art teardown

**Galen Framework** — last release 2.4.4, 2019-03-15; unmaintained.
Its language design is still the best statement of the problem:
`@objects` (separating selector from name), `@rule` + `@forEach` (rule
reuse instead of per-element repetition), `@on <tag>` viewport groups,
built-in tolerances (`~0px`, `5 to 10 px`), relative expressions
(`width 100% of screen/width`), a spec generator from page dumps, and
**mutation testing of the specs themselves**.

*Inherit:* the spec generator and the mutation-testing idea — they are
the answer to "how do we know an assertion isn't vacuous". Also the
tolerance-as-first-class-syntax instinct: sub-pixel equality is never
what you mean. *Reject:* the DSL itself, on the evidence above, and its
element-box relational vocabulary, which is the primitive my
adjudication showed to be wrong.

**axe-core** — 4.12.1 (2026-06-10), canary builds within the last day;
unambiguously active and industrial. Two things to take verbatim.
First, four result buckets, not two: *passes*, *violations*,
*inapplicable* ("no matching content was found"), and *incomplete*
("also known as 'needs review'"). Second, composition: rules combine
checks via `any` ("if none pass, generate a violation"), `all` ("if any
fails, generate a violation"), and `none` ("if any pass, generate a
violation").

*Inherit:* the four-bucket surface. My Phase 2 result reproduced its
necessity independently — 84% of raw candidates were *determinable
non-violations* and needed a bucket that is neither fail nor "needs
review". *Reject:* nothing material.

**ReDeCheck (ISSTA 2017)** — Responsive Layout Graph; sampling 320–1400 px
at 60 px steps plus CSS-extracted breakpoints, binary-searching the
change point; five RLF types (Element Collision, Element Protrusion,
Viewport Protrusion, Small-Range, Wrapping); reports split TP / FP /
**NOI** (Non-Observable Issue). *Inherit:* the metamorphic framing
(compare the page against itself across viewports — no golden), the
breakpoint-aware sampling, and NOI as the model for `inconclusive`.
My sweep implements 3 of the 5 RLF types and used 8 fixed widths, which
is coarser than ReDeCheck's sampling; two of my three confirmed defects
were Small-Range, so the coarseness is a real recall risk.

**DSL vs typed host API.** Arguing both sides honestly: a DSL wins when
non-programmers author specs at volume, when the spec must be a
reviewable artifact distinct from code, and when a generator can emit
it. All three arguments apply to Duhem's *criteria* — and Duhem already
has a YAML surface for exactly that. They do not apply to a six-predicate
geometry library. A typed TS/Rust API gets type-checking, editor support,
and composition for free, and it keeps the predicate set honest: adding a
predicate costs a function, so nobody adds one speculatively. **Host API
wins.** If the predicate set ever grows past ~20 and non-engineers start
authoring, revisit.

## 2. Facts schema

Versioned, serializable, diffable; the primary stored artifact. Assertions
are a pure function of it — Phase 2's static checks ran with no browser at
all, which is the proof the split is real.

```
FactsV1 {
  schema: "duhem.ui.facts/1",
  capture: { url, capturedAt, viewport{w,h}, dpr, locale, tz, ua,
             settleProtocol: "v4", archiveDigest },
  elements: [ { path, tag, box[x,y,w,h], style{...28 props},
                textRuns: [ [x,y,w,h], ... ],   // one rect per line box
                clipRect[x1,y1,x2,y2],           // resolved, not inferred
                scrollable{x,y}, visible, zIndex } ],
  page: { scrollW, scrollH, canScrollX, canScrollY, declaredBreakpoints[] }
}
```

Three fields carry the investigation's findings and are not optional:
**`textRuns`** (border boxes are the wrong collision primitive),
**`clipRect`** *resolved at collection time* (the observability gate must
be computed, not inferred), and **`canScrollX`** (measured by attempting
a scroll, because `scrollWidth > clientWidth` was only 43% precise).

`archiveDigest` binds facts to the exact frozen input, so a verdict is
reproducible or explicitly not.

## 3. Settling protocol (v4)

The preconditions that make collection a pure function. Measured:
100% geometry reproducibility over 40 211 elements × 5 runs × 23 pages,
under 4-way CPU contention.

1. Before first paint (`addInitScript`): inject `animation:none;
   transition:none`; pin `Date.now`, `performance.now`, and rAF
   callback timestamps.
2. After load: `svg.pauseAnimations()` + `setCurrentTime(0)` (SMIL is
   *not* covered by CSS `animation:none`); `getAnimations().pause()`
   twice, before and after a short delay (WAAPI is not covered either,
   and animations are created late by IntersectionObserver).
3. `await document.fonts.ready`; `img.decode()` **with a timeout**.
4. Two rAFs, then collect.

Two rules generalise out of this. **One freeze strategy, applied before
first paint** — protocol v1 mixed "jump to end" (CSS) with "rewind and
pause" (JS) after paint and scored *worse than no settling at all*.
And **every settle primitive needs a timeout**: `img.decode()` never
resolves for an image whose fetch failed, which silently hung the first
corpus run for 12 minutes. "Wait for quiescence" is not safely
expressible as an unbounded await.

## 4. Predicate algebra

What survived. Six predicates, all class C/D except the A-class pair
that only applies to our own output.

| # | predicate | class | status |
|---|---|---|---|
| 1 | `noHorizontalScroll(width)` — attempt scroll, assert `scrollX == 0` | C | 3 confirmed defects |
| 2 | `withinViewport(el, width)` — right edge ≤ vw, gated on clip rect | C | exact once gated |
| 3 | `layoutChangePointsSubsetOf(declaredBreakpoints)` | D | metamorphic, no oracle |
| 4 | `stableAcross(widths, invariant)` — generic metamorphic harness | D | subsumes long-text/zoom/RTL/theme |
| 5 | `fontSizeIn(scale)` / `spacingIn(scale)` / `colorIn(tokens)` | A | only against a frozen scale |
| 6 | `textRunsDoNotCollide(a,b)` | B | **kept behind a flag, off by default** |

Composition uses axe-core's `any` / `all` / `none` rather than a
bespoke algebra.

Predicate 6 is retained but disabled: I could not make it precise on
production pages, and shipping it on by default would train users to
ignore output. It stays because my corpus measured *precision, not
recall* — 23 well-built sites are the wrong instrument for proving a
defect class doesn't occur. Re-enable it only when a known-defective
corpus shows it catching something.

Explicitly **not** in the algebra: anything requiring a magic threshold
derived from taste. Scale-regularity is measurable (median 4 font sizes
cover 90% of text) but "how regular is regular enough" is a human
decision and belongs in the freeze layer.

## 5. The observability gate, generalised

This is the main claimed contribution, and it does generalise — as a
*discipline*, not a rule list:

> For every predicate, the geometric violation and the observable
> violation are different propositions. Compute the second; never infer
> it from a property.

Three independent instances measured:

| predicate | inferred (wrong) | computed (right) | effect |
|---|---|---|---|
| overlap | "an ancestor has `overflow != visible`" | intersect ancestor clip rects; does the *collision region* survive? | inconclusive 89% → 4.5% |
| viewport escape | `right > vw` | plus: is the element inside a resolved clip rect? | 84% of candidates suppressed |
| horizontal scroll | `scrollWidth > clientWidth` | `scrollTo(9999,0)`; did `scrollX` move? | precision 43% → exact |

The gate has three outcomes, and the two-way split is the bug most
implementations make:

- **provably not rendered → `pass`** (a determinable non-violation)
- **provably rendered → `fail`**
- **cannot determine → `inconclusive`**

Folding the first into `inconclusive` is what produced an 89%
inconclusive rate — a gate nobody would read. After the split, the
*only* residual inconclusive cause of substance is "the occluding
element's background is a gradient or image, so occlusion cannot be
resolved from computed style". That is ReDeCheck's NOI exactly, and
VISER's opacity pixel-probe is the known escalation path — a
deterministic image operation, still LLM-free, if the rate ever
justifies it. At 4.5% it does not.

## 6. Verdict folding

**Reuse Duhem's, do not invent one.** Verified in-tree this session:
`duhem-judge` already exposes `VerdictState { Pass, Fail,
Inconclusive(InconclusiveCause) }`, folds with "any `Fail` → `Fail`;
any `Inconclusive` and no `Fail` → `Inconclusive`; all `Pass` → `Pass`"
(first cause wins) identically at assertion → check → criterion → run,
and offers a criterion-level `InconclusivePolicy { Block, Warn, Pass }`.

*A correction to my own earlier Phase 0 note, which claimed the judge
was two-state and called that the central gap: that was wrong.* The
gap is narrower and additive. `InconclusiveCause` currently carries
four causes — `Timeout`, `MissingObservation`, `EnvironmentError`,
`EmptyAggregation` — all *infrastructural*: "we never got to look".
This domain needs a *semantic* cause: "we looked, the geometry is
known, and observability is undeterminable." The enum is
`#[non_exhaustive]`, so adding one is source-compatible, but a new wire
value is a schema-impact event and needs the spec gate.

## 7. Spec lifecycle and vacuous assertions

AI-drafted → human-reviewed → **frozen**. The freeze is the whole
mechanism by which an L3 judgment becomes an L1 membership test, and it
happens once, not per run.

Vacuity detection generalises Galen's mutation testing, and the Facts
snapshot makes it cheap: mutate the *facts*, not the page. Perturb a
box by 20 px, swap a colour off-scale, shrink the viewport past a
declared breakpoint — any assertion that still passes under a mutation
that should break it is vacuous and fails CI. This needs no browser and
no re-render, because assertions are a pure function of the snapshot.
Run it in the same job that authored the spec.

## 8. Freeze layer

DTCG Design Tokens Format Module (first stable 2025.10) for
`TypeScale` / `SpacingScale` / `TokenSet`. No deviation justified: the
A-class predicates are set-membership tests and DTCG already expresses
the sets. Interop matters more than fit here — the tokens should be the
same artifact design tooling emits.

## 9. Boundary — what this cannot decide

All of L3 and most of L2, by construction. It cannot say a page is
attractive, professional, or shippable to a client; those admit only
ordinal comparison (UI-Bench's methodological retreat to pairwise
Thurstone comparison is the current state of that art, over 4 000+
expert judgments). It cannot say a layout is *balanced* — Bauerly & Liu
found the classic "balance" metric uncorrelated with human judgment.
It cannot detect a defect that is invisible in geometry and computed
style: wrong copy, a wrong image, a bad colour *choice* inside the
frozen palette.

Two limits specific to what I measured:

- **It cannot certify a page it has not captured at that width.** A
  frozen MHTML archive is faithful only at its capture viewport;
  responsive assertions need one capture per asserted width.
- **Recall is unmeasured.** Everything above is a precision result on
  correct pages.

## 10. Integration — recommend, but confirm with a human

**Recommendation: a Duhem action pack, not a standalone tool.** Duhem
already supplies, verified in-tree: a Playwright Node sidecar over
stdio JSON-RPC (`crates/duhem-actions/src/browser.rs`, #71), a
`ui/assert-*` action family whose contracts emit a boolean `satisfied`
output, the spec-#253 path letting a `Check` carry no `assertions` and
rely on judging steps, and the entire tri-state judge and folding
machinery. Standalone would mean rebuilding all of it to gain nothing.

The shape that fits without touching the closed `Assertion` enum:

- `ui/collect-facts` — an action producing a `FactsV1` artifact.
- `ui/assert-layout` — a judging step emitting `satisfied`, carrying
  the six predicates. Per #253 the check needs no `assertions` entry.

This is the seam that matters: the closed `Assertion` enum is the
structural enforcement of "mechanical judgment, not LLM judgment", and
this design **does not extend it**. The predicates ride the action
contract instead, so the identity commitment is untouched and no
schema-breaking change is required — except the one additive
`InconclusiveCause`, which does need the schema-impact gate.

**Flagged for human confirmation:** whether a UI-geometry domain pack
belongs in Duhem's core at Phase 0, or waits for the roadmap slot in
`docs/duhem-spec.md` §14. That is a product-scope call, not a technical
one, and the brief reserves it for a human.

## 11. What I would build first

One week, in this order, stopping if any step disappoints:

1. `ui/collect-facts` + the v4 settle protocol + `FactsV1`. Independently
   useful: a diffable snapshot of a page's rendered geometry.
2. Predicates 1–3 (no-horizontal-scroll, viewport containment,
   change-points-subset-of-breakpoints) with per-width capture.
3. Facts-mutation vacuity checking.
4. Only then consider 4–6.

The corpus, the harness, and the adjudication method are the reusable
part of this investigation; they are what would tell you quickly
whether step 4 is worth it.
