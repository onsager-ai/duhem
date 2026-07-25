# Phase 2 — Failure distribution

**Question.** Over a corpus of real pages, where do the failures
actually land — A (token/unary), B (relational), C (global), D
(cross-state)? And how many of them would a human agree are real?

**Gate 2 verdict, in one sentence:** *B-class static overlap detection
produced 0 confirmed true positives out of 46 adjudicated findings on
23 production pages, while the class-D cross-viewport sweep produced 3
confirmed, observable defects — so the relational predicate DSL is not
justified, but a narrow class-C/D responsive checker is.*

That is a **build something smaller** result, not a "proceed to DSL"
result.

## Method

Four hardcoded checks, no DSL, no abstraction, run as a **pure function
over the serialized Facts** — no browser, no network. That the static
checks run with zero browser involvement is itself the first
confirmation of the architectural pivot: rendering is fact collection;
assertion is a pure function of the snapshot.

Adjudication is visual: each sampled finding is re-rendered with the
implicated geometry outlined, cropped, and inspected against the actual
pixels. **This was adjudicated by me, not by an independent human
reviewer** — see `unverified.md`.

## Result 1 — the observability gate is what makes the numbers usable

The first version of the gate inferred clipping from a *property*
("does an ancestor have `overflow != visible`?"). That marked **89%**
of all candidates `inconclusive` — a gate that swallows everything is
not a gate.

Recomputing the gate **geometrically** — intersect the clip rects of
all non-`visible` ancestors, then ask whether the specific collision
region survives — changes the distribution completely:

| | count |
|---|---|
| raw candidates | 3 075 |
| **suppressed** (determinable non-violation) | 2 587 (84%) |
| **fail** | 466 |
| **inconclusive** (genuinely undeterminable) | 22 (4.5%) |

Suppression causes: collision region clipped 960, element clipped 1 572,
protrusion clipped 59, occluded by opaque top 22. Remaining
`inconclusive` causes: occluder background is a gradient/image so no
single colour resolves (13), absolutely-positioned off-viewport (9).

**The design lesson.** A violation that is *provably* not rendered is a
`pass`, not an `inconclusive`. `inconclusive` must be reserved for
"we cannot determine", which here is almost entirely one cause: the
occluding element's background is an image or gradient, so occlusion
cannot be resolved from computed style alone. That is exactly ReDeCheck's
NOI category, and exactly what VISER resolves with a pixel probe.

## Result 2 — B-class overlap is dominated by false positives

**Border-box overlap: 466 fails, 28 sampled, 28 false positives (100%).**

Every one traced to the border box being the wrong primitive:

| artifact | example |
|---|---|
| a wrapped inline's rect is the union of its line boxes, so it covers text owned by other elements | arstechnica.com footer links |
| inline siblings sharing one line box have overlapping rects | stripe.com `<h1>` + following `<p>` |
| a container's rect covers visually-nested but not DOM-nested peers | play.grafana.org search field + `ctrl+k` chip |
| intentional carousel peek | figma.com template row |

So I changed the primitive and re-measured, rather than concluding from
one bad implementation. **Text-run rects** (one rect per rendered line,
via `Range.getClientRects` on text nodes) are the right unit: text does
not collide with boxes, text collides with text.

| primitive | pages with 0 fails | total fails |
|---|---|---|
| element border box | 0 / 23 | 466 |
| **text-run rect** | **10 / 23** | 729 → **147** after dropping exact stacks |

79.8% of text-run fails have `cover = 1.00` — one text run exactly
covering another. Inspection shows these are *duplicated text nodes*
(a11y duplicates, animation clones, icon-font pairs), not collisions.
Dropping them leaves 147 findings across 10 pages. A further 18-case
visual sample of those found **no true positives** either.

Net: **0 / 46 adjudicated B-class findings were real** — 28 border-box
plus 18 text-run. (71 crops were *generated*; 46 were actually
inspected. An earlier revision of this note reported 71 adjudicated,
conflating crops rendered with crops read. Corrected here.)

## Result 3 — class D is where the real defects are

The cross-viewport sweep (8 widths, 320→1920) is metamorphic: it needs
no definition of "good", only consistency. Run **live**, because of the
validity problem below.

7 pages reported `scrollWidth > clientWidth` at ≥1 width. But that
predicate is *not* the observable property — `overflow-x: hidden`
clips without shrinking `scrollWidth`. Testing actual scrollability
(`scrollTo(9999,0)`, read back `scrollX`) separates them:

| page @ width | `scrollWidth > clientWidth` | actually scrollable |
|---|---|---|
| news.ycombinator.com @768 | yes (804) | **yes, 36 px** ✔ |
| allbirds.com @320 | yes (326) | **yes, 6 px** ✔ |
| app.diagrams.net @1280 | yes (1284) | **yes, 4 px** ✔ |
| gymshark.com @320 | yes (340) | no |
| figma.com @375 | yes (2433) | no |
| figma.com @1280 | yes (2746) | no |
| en.wikipedia.org @768 | yes (777) | no |

**3 confirmed observable defects; 4 false positives (43% precision).**
Replacing the heuristic with the scrollability test makes the predicate
exact — the third instance in this investigation of the same rule:
*compute observability, never infer it*.

Two of the three are "small-range" defects in ReDeCheck's sense —
present at exactly one sampled width. A checker that only tested 1280
would have found none of them.

## Result 4 — the archive fidelity limit (methodological)

The class-D sweep was originally run over the frozen MHTML archives.
Validating one page against its live original showed the archive
reports horizontal scroll the live page does not have:

| width | archive `sw`/`vw` | live `sw`/`vw` | agree |
|---|---|---|---|
| 320 | 752 / 320 → scroll | 320 / 320 → none | **no** |
| 768 | 2497 / 768 → scroll | 768 / 768 → none | **no** |
| 1280 | 1280 / 1280 → none | 1280 / 1280 → none | yes |
| 1440 | 1440 / 1440 → none | 1440 / 1440 → none | yes |

The same element (`UL.logo-carousel__marquee`) overflows in both, but
live an ancestor clips it. Agreement holds at and near the **capture
viewport (1280)** and breaks away from it.

**A frozen MHTML archive is only faithful at the viewport it was
captured at.** Re-rendering it at another width does not reproduce the
live page, because capture-time JS-computed styles/attributes are baked
in and the JS that would recompute them on resize does not re-run
equivalently. Consequence for any design: *responsive assertions
require a capture per asserted width*, not one capture resized.

Running the sweep live avoids that but forfeits reproducibility —
figma.com and en.wikipedia.org both flipped verdicts between the sweep
and the verification run minutes later.

## A-class on a wild corpus

On crawled sites there is no frozen scale to violate, so A-class was
measured as internal regularity: how many distinct values, and how many
cover 90% of usages (`n90`).

| | median distinct | median n90 |
|---|---|---|
| font-size | 8 | 4 |
| spacing | 18 | 7 |
| text colour | 7 | — |

Professionally-built pages cluster tightly: 4 font sizes typically
cover 90% of text. The spread is wide though — `n90` for font-size
ranges 1 (app.diagrams.net) to 9 (slack.com), and spacing `n90` ranges
3 to 19. So scale-regularity *is* a discriminating signal, but the
threshold separating "regular" from "scattered" is a human judgment —
which is precisely the L2 trap the brief warns about, and which belongs
in the freeze layer, not the evaluator.

## Gate 2 fork

- **Mostly A?** No — A-class is not even expressible on a wild corpus
  without a frozen scale.
- **Meaningful B?** **No.** 0/46 adjudicated true positives across two
  different geometric primitives.
- **Meaningful D?** **Yes** — 3 real, observable, oracle-free defects,
  two of them only visible at a single viewport width.
- **High inconclusive?** No — 4.5% after the gate is computed properly.

**Proceed, but narrowed to C/D.** The relational (B) predicate algebra
that would justify a Galen-style DSL did not survive contact with the
corpus.

## The honest limitation

This corpus is 23 *professionally built production sites*. They mostly
do not have visible layout defects at desktop width, so measuring 0
true positives for B-class measures **precision, not recall**. I cannot
conclude "B-class defects don't happen"; I can only conclude "B-class
detectors as implemented fire constantly on correct pages, which makes
them unusable regardless of recall". Establishing recall needs a corpus
with known defects — see `unverified.md`.
