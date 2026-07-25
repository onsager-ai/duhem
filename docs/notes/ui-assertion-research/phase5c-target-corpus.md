# Phase 5c — Target-distribution corpus

**Question.** Everything so far was measured on mature production sites. The
intended target is AI-generated UI. What does the failure distribution look
like on the actual target distribution?

## Corpus

**26 usable pages from 3 generators**, captured with the same protocol and run
through the same frozen detectors as the wild corpus.

| generator | usable | provenance |
|---|---|---|
| Lovable (`*.lovable.app`) | 13 | strong — the domain hosts generator output exclusively |
| Bolt (`*.bolt.host`) | 10 | strong — same |
| Replit (`*.replit.app`) | 3 | **weak — mixed**; the domain hosts both Agent-generated apps and hand-written Repls, and nothing in the page distinguishes them |

44 URLs attempted, discovered by domain-scoped web search. 17 dropped as
too-small (<150 elements: placeholder, error, or login pages), 1 navigation
error. Chreode was not included — it has no publicly reachable output and the
local stack has no Docker daemon, as established in Phase 0.

The three-generator requirement is met, but the mix is unbalanced (13/10/3)
and the third leg is the weak-provenance one. Treat this as two solid
generators plus a partial third.

## Failure distribution vs the wild corpus

Generated pages are about half the size of production pages (median 450 vs
855 visible elements), so raw totals mislead; the rate is normalised.

| metric | generated (n=26) | wild (n=23) |
|---|---|---|
| **findings per 1000 visible elements** | **2.5** | **20.3** |
| pages with zero findings | **15 / 26** | 6 / 23 |
| findings by class | B1.overlap 30, B2.protrude 4, C1.viewport **0** | B1.overlap 401, B2.protrude 27, C1.viewport 38 |
| text-run overlap findings | 17 (3 exactly-stacked) | 729 (582 stacked) |
| inconclusive | 3 | 22 |
| median visible elements | 450 | 855 |

**Generated pages produce roughly 8× fewer relational findings per element
than production sites, and well over half of them produce none at all.**

That is the opposite of the expected direction, and the most likely
explanation is not that generators are better designers: generated pages are
smaller, flatter, and overwhelmingly built on one of a few component
libraries with conservative default spacing. They have less opportunity to
collide. Notably **zero** viewport escapes at 1280 across the whole generated
corpus.

## A-class scale regularity

On a corpus with no frozen scale to check membership against, A-class is
measured as internal regularity: how many distinct values, and how many cover
90% of usages (`n90`). Since distinct-value counts grow with page size, the
size-normalised figure is the comparable one.

| | generated | wild |
|---|---|---|
| median distinct font-sizes | 10 | 8 |
| median font-size `n90` | 5 | 4 |
| median distinct text colours | 11 | 8 |
| median distinct spacings | 15 | 18 |
| median spacing `n90` | 8 | 7 |
| **distinct font-sizes per 100 visible elements** | **2.54** | **1.10** |
| **distinct colours per 100 visible elements** | **2.24** | **1.15** |
| **distinct spacings per 100 visible elements** | **3.66** | **2.56** |

**Size-normalised, generated pages are markedly less regular on every axis:**
2.3× more distinct font sizes per element, 1.9× more text colours, 1.4× more
spacing values. A production site of 855 elements gets by on 8 font sizes; a
generated page of 450 elements uses 10.

This is the one dimension where the target distribution is measurably worse
than the wild baseline, and it is the dimension that needs no relational
geometry — only set membership against a frozen scale.

Per generator (small samples, directional only):

| | n | median distinct font-sizes | median distinct colours | total findings |
|---|---|---|---|---|
| lovable | 13 | 11 | 9 | 19 |
| bolt | 10 | 10 | 13 | 14 |
| replit | 3 | 9 | 8 | 1 |

## Limits

- **26 pages, unbalanced across generators**, 3 of them weak-provenance.
- **Published pages are a survivorship-biased sample.** These are projects
  someone chose to publish and share; a generator's raw first-draft output is
  plausibly worse and is not represented here.
- **Ratios are not per-page-normalised.** The per-1000-element rate treats the
  corpus as one pool; a few large pages carry disproportionate weight.
- **A-class here is regularity, not violation.** Nothing was checked against a
  frozen scale, because these pages have no declared scale. "Less regular"
  is not the same as "would fail a token check", and the threshold separating
  the two remains a human decision — the L2 trap the original brief warned
  about.
