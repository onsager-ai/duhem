# Phase 5a — Recall under fault injection

**Question.** The prior phase measured precision on a corpus with almost no
defects and concluded the relational class should be dropped. That does not
follow. Do the frozen detectors *find* faults when faults exist?

**Detectors were frozen.** `checkPage()` (phase2a) and `overlapCheck()`
(phase2e) were imported unmodified. The only edit to either file was adding
the `export` keyword so they could be imported, plus a `main()` guard so
importing did not re-run the phase-2 sweep. No thresholds were touched.

## Method

Faults are injected by mutating one known element in a frozen archive at load
time, so ground truth is exact and adjudication is objective — this also
removes the self-adjudication weakness of Phase 2.

Two methodological corrections were necessary before any number meant
anything, and both would have produced fake results if missed:

1. **Severity is reported as the *achieved* geometric delta, not the
   requested one.** An uncalibrated "shift down by 1px" lands nowhere near
   1px of overlap once the element's natural gap and `box-sizing` are
   involved — one uncalibrated 1px request produced a *632px* protrusion.
   Each injection is now calibrated so achieved ≈ requested.
2. **Ground truth must be measured in the currency the predicate uses.** The
   overlap predicate compares *text-run* rects, so measuring injected overlap
   as *border-box* intersection manufactures misses: a box can overlap by
   16px of padding with no glyph collision at all.

Targets are additionally required to (a) not already exhibit the fault before
injection, and (b) be free of clipping ancestors — otherwise the observability
gate correctly suppresses the injected fault and the "miss" is the gate
working, not the detector failing.

One element is pinned per (page, fault) by probing at maximum severity and
keeping the strongest responder, so the whole ladder is measured on a constant
element.

23 pages × 3 fault classes × 5 severities = 257 records; 235 usable, 22
skipped (20 no eligible target, 2 injection had no effect).

## Recall × severity

Recall is computed only over runs where a fault was actually created
(achieved > 0.5px). Severity below is achieved pixels.

| achieved | overlap | protrude | viewport |
|---|---|---|---|
| 0 (baseline) | — 0 detections / 16 | — 0 / 15 | — 0 / 16 |
| 1 px | **0%** (0/15) | **0%** (0/15) | **0%** (0/16) |
| 4 px | **20%** (3/15) | **100%** (15/15) | **88%** (14/16) |
| 16 px | **100%** (15/15) | **100%** (15/15) | **88%** (14/16) |
| 48 px | **88%** (14/16) | **100%** (15/15) | **88%** (14/16) |

**The 90% crossing:** protrusion and viewport escape cross between 1 and 4 px;
overlap crosses between 4 and 16 px. All three are *below* the threshold at
which a human would call it a defect — a 1px overlap is not a bug, and the
detectors correctly ignore it. **The gate is working, not broken.**

The baseline row is the control that makes this readable: with no fault
injected, the detectors implicated the target element **zero times** in 47
runs.

**Localisation is perfect.** In every one of the detections above, the
finding named the exact element that was mutated. There were no cases of
detecting the fault but pointing somewhere else.

**The observability gate never suppressed an injected fault** (0 gated across
all classes and severities), because targets were pre-screened to be
unclipped. This confirms the gate discriminates rather than blankets.

## Two structural blind spots

`viewport` plateaus at 88% — 2 of 16 pages are never detected at *any*
severity, including 48px. That is not a threshold effect, it is a hole.

**Both misses are `<svg>` elements** (kubernetes.io, arstechnica.com). Cause,
confirmed directly: the collector's visibility test is

```js
vis: !!(el.offsetParent || cs.position === 'fixed') && …
```

and `offsetParent` is an **`HTMLElement`-only property — it is `undefined` on
every SVG element**. Measured on kubernetes.io: 0 of 2 SVG elements have an
`offsetParent` (type `undefined`), against 23 of 23 sampled `div`s.

**Consequence: every SVG element is classified invisible and excluded from
every detector.** On a corpus of modern sites — where icons, logos, charts and
frequently entire hero illustrations are inline SVG — this is a large silent
exclusion. It survived Phase 2 undetected precisely because a
precision-only measurement cannot see a detector that never fires.

The two persistent `overlap` misses are less severe: linear.app achieved only
3.5px (below the detector's effective threshold, arguably correct) and
css_tricks.com 13.1px on a `<code>` element inside a `<p>`, where the cover
ratio falls under the 0.25 gate.

## FP stability — the number that matters most

Recall is academic if the true positive lands in a pile of noise.

| | overlap | protrude | viewport |
|---|---|---|---|
| baseline findings per page (median) | 20 | 17 | 17 |
| findings added by the injected fault (median) | 2 | 3 | 3 |
| total findings on the page when the fault *is* detected (median) | 16 | 21 | 22 |

So the detectors do find the injected defect, and they report it alongside
**roughly 17–22 pre-existing findings on the same page** — which Phase 2
adjudicated as false positives at a rate of 46/46. The signal is real and it
is buried at about 1:20.

This is the precise sense in which the Phase 2 conclusion was wrong and also
not wrong: the relational class is *detectable* (this phase) but not
*reportable* (Phase 2). Those are different defects with different fixes.

## Not run: the contrast fault class

The brief's fault table includes "mutate a text colour toward its background"
against a `contrast ≥ 4.5` predicate. **No contrast detector exists** in the
frozen set — the detectors are `B1.overlap`, `B2.protrude`, `C1.viewport`,
plus the text-run overlap variant and the sweep predicates. Nothing computes
luminance or contrast ratio.

Per the brief's constraint, this is reported rather than substituted. Building
one now would produce a recall number with no matching precision measurement,
which is exactly the asymmetry the freeze rule exists to prevent. Measuring it
properly requires building the detector *and* running it against the wild
corpus for precision first.

## Known limit

Synthetic faults are structurally simpler than natural ones. A CSS-injected
overlap is a clean geometric displacement; a real generator defect may have
different shape, and may co-occur with other faults. **These numbers are an
upper bound on recall.** Phase 5c is the check on that bound.
