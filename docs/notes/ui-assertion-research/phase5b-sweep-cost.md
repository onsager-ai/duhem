# Phase 5b — Pricing the responsive sweep

**Question.** Phase 2 found a frozen archive is faithful only at its capture
viewport, which would force one capture per asserted width and block the only
predicate class that produced confirmed defects. Price that before designing
around it.

**Headline: the constraint is much weaker than Phase 2 claimed, and the
expensive workaround is unnecessary.** Infidelity is page-specific, not
universal, and it is mechanically detectable with a single scalar.

## B1 — capture cost per width

12 live captures (4 pages × 3 widths), full settle + lazy-load scroll + MHTML
serialisation.

| | median | range |
|---|---|---|
| wall-time per capture | **6.3 s** | 1.8 – 9.5 s |
| archive size per capture | **2.03 MB** | 54 KB – 5.1 MB |

Size is dominated by page weight, not viewport: news.ycombinator.com is 54 KB
at every width, theguardian.com 2.7–5.1 MB.

## B2 — extrapolated cost for the 23-page corpus

| sampling density | captures | wall-time | storage |
|---|---|---|---|
| 8 fixed widths (what Phase 2 used) | 184 | ~19 min | 0.36 GB |
| ReDeCheck density (60 px over 320–1400) | 414 | ~43 min | 0.82 GB |
| 16 px steps (see B3) | 1 564 | ~164 min | 3.10 GB |

Serial, single browser. The Phase-1 harness parallelised ~4× on 4 cores, so
divide wall-time accordingly; storage does not divide.

## B3 — the observable width window of each known defect

This is the recall-risk number, not a convenience number. Each confirmed
defect was probed live across a coarse grid, then the boundaries bisected in
4 px steps.

| defect | observable window | width of window |
|---|---|---|
| news.ycombinator.com — table overflows | **752 – 800 px** | **52 px** |
| allbirds.com — 6 px overflow | **≤316 – 328 px** | **~16 px** |
| app.diagrams.net — 4 px overflow | 1152 – 1920 px (and above) | ≥768 px |

Fine grid for Hacker News: `740· 744· 748· 752✓ … 800✓ 804· 808·` — the defect
exists only between 752 and 800, and vanishes on both sides.

**Implication for uniform sampling.** To *guarantee* hitting a window of width
W, the step must be ≤ W. ReDeCheck's 60 px stepping would not guarantee
catching the Hacker News defect (52 px window) and would very likely miss
allbirds (16 px window). Guaranteeing the allbirds class needs ~16 px steps —
the 164-minute, 3.1 GB row above.

**But uniform sampling is the wrong strategy.** Both narrow windows contain a
*conventional device width* — 320 px and 768 px. That is not a coincidence:
these defects occur where designers put breakpoints, so a handful of
conventional widths plus the page's own CSS-declared breakpoints beats a
uniform grid at a fraction of the cost. Phase 2's 8-width sampling caught both
narrow defects for exactly this reason.

## B4 — is archive infidelity mechanically detectable?

Yes — and the Phase 2 finding it was testing turns out to be **overstated**.

6 pages × 3 widths, archive vs live, judged on the *observable* predicate
(attempt the scroll, read back `scrollX`) rather than the `scrollWidth`
heuristic.

| page | 320 | 768 | 1280 (capture width) |
|---|---|---|---|
| news.ycombinator.com | agree | agree (both scroll 36 px) | agree |
| react.dev | agree | agree | agree |
| allbirds.com | agree (both scroll 6 px) | agree | agree |
| docs.python.org | agree | agree | agree |
| **stripe.com** | **disagree** | **disagree** | agree |
| **bbc.com** | **disagree** | — | — |

**4 of 6 pages agree at every width, including 320 and 768 — far from the
1280 capture width.** Phase 2 generalised from a single page (stripe.com) to
"an archive is faithful only at its capture viewport". That is wrong as
stated: infidelity is a property of *some pages*, not of the archive format.

Crucially, **both confirmed small-range defects reproduce exactly in the
archive** — Hacker News 36 px at 768, allbirds 6 px at 320 — so archive replay
did not cost us the findings that matter.

**The infidelity signal is a clean scalar.** `archive.scrollWidth /
live.scrollWidth` separates the two populations with no overlap:

| | swRatio |
|---|---|
| all 16 agreeing page/width pairs | **1.000** |
| stripe @320 / @768, bbc @320 | **2.35 / 3.25 / 3.52** |

So a sparse-capture-plus-validation architecture is viable: capture at a few
conventional widths, and validate a candidate width with one cheap live load
comparing a single number. Dense capture is not required, and the
cost is **not** irreducible.

## Live sweeping, priced honestly

The alternative to per-width capture is sweeping live, which avoids the
fidelity question entirely but forfeits reproducibility. Phase 2 observed
figma.com and en.wikipedia.org flipping verdicts within minutes. This phase
saw the same instability from the other direction: stripe.com reported
`scrollWidth` 2497 at 768 in the archive against 768 live, while
news.ycombinator.com reproduced to the pixel across both. Live sweeps are
reproducible for some pages and not others, and which is which is not
knowable in advance without measuring — which is what the swRatio check does.

I did not quantify live-sweep flip *rate* across repeated runs; that needs N
repeats per width and was not run. See `unverified.md`.
