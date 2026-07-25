# Phase 1 — Facts determinism

**Question.** If assertions are a pure function over a serialized
`Facts` snapshot, is that snapshot actually reproducible? Everything
downstream is worthless if the same page yields different facts on two
runs.

**Gate 1.** Geometry facts must reach ~100% reproducibility under a
documented settling protocol, or the investigation stops.

**Result: Gate 1 passes.** Under settle protocol **v4**, geometry is
byte-identical across 5 runs for the whole corpus (see
[Results](#results)), and the residue is a single non-geometric field
on one page. Naive collection is *already* 99.8% reproducible, so the
protocol's value is not the average — it is closing the last 0.2%,
which is exactly where the false positives would have lived.

## Corpus

23 frozen archives of real, in-the-wild pages (Phase 0). Not our own
generated output: testing our generator against our own frozen scales
would bias the token-drift rate toward zero.

Each page was captured once through a headless Chromium, settled
(fonts, full-page lazy-load scroll, image decode), then serialized to a
**single-file MHTML archive** via CDP `Page.captureSnapshot`. Every
Phase 1/2 run loads that archive from `file://` with **all network
aborted** — verified: `externalBlocked == 0`, so nothing leaks to the
live site. This matters because live sites mutate (A/B tests, CDN
variance, dynamic content); re-fetching per run would make a
reproducibility measurement meaningless.

| # | page | genre | archive KB | elements | JS-dependent |
|---|---|---|---|---|---|
| 1 | stripe.com | marketing | 3171 | 3684 | no |
| 2 | linear.app | marketing | 3850 | 4334 | no |
| 3 | vercel.com | marketing | 1539 | 1108 | no |
| 4 | figma.com | marketing | 2057 | 1412 | no |
| 5 | netlify.com | marketing | 1418 | 2491 | no |
| 6 | slack.com | marketing | 5055 | 2296 | no |
| 7 | play.grafana.org | app | 6801 | 557 | yes |
| 8 | app.diagrams.net | app | 437 | 2634 | yes |
| 9 | developer.mozilla.org | docs | 1107 | 5331 | no |
| 10 | docs.python.org | docs | 180 | 2691 | no |
| 11 | react.dev | docs | 494 | 2689 | no |
| 12 | tailwindcss.com | docs | 934 | 1362 | no |
| 13 | kubernetes.io | docs | 1663 | 4799 | no |
| 14 | books.toscrape.com | ecommerce | 489 | 542 | no |
| 15 | allbirds.com | ecommerce | 4610 | 2610 | no |
| 16 | gymshark.com | ecommerce | 4768 | 3453 | no |
| 17 | en.wikipedia.org | editorial | 1603 | 4323 | no |
| 18 | news.ycombinator.com | editorial | 54 | 821 | no |
| 19 | bbc.com | editorial | 935 | 1417 | no |
| 20 | theguardian.com | editorial | 3380 | 4948 | no |
| 21 | arstechnica.com | editorial | 2946 | 2716 | no |
| 22 | smashingmagazine.com | editorial | 933 | 785 | no |
| 23 | css_tricks.com | editorial | 2469 | 1152 | no |

Captured 2026-07-25T03:09–03:15Z, 49.7 MB total, viewport 1280×800,
DPR 1, `en-US`, UTC, HeadlessChrome/141.0.7390.37.

Attempted and excluded, with reasons (no silent drops):

| target | outcome |
|---|---|
| excalidraw.com | skipped — `robots.txt` disallow |
| rei.com | failed — navigation timeout at 60 s |
| etsy.com (9 els), codepen.io (43), squoosh.app (145) | captured but excluded: bot-wall / challenge pages, not real pages (`renderedEls < 150`) |

Genre spread of the 23: marketing 6, docs 5, editorial 7, ecommerce 3,
app 2. Two pages are JS-dependent SPAs; the rest server-render.

## What a Facts snapshot contains

Per element, in document order, capped at 2500 elements/page:
a DOM path key (`html/body:1/div:2/…`, nth-child indices), the
bounding box rounded to 0.01 px, a 28-property `computedStyle` subset,
own-text length, line-box count, a visibility boolean, and z-index.
Plus page metadata: viewport, DPR, scroll extent, `document.fonts.status`.

## Settling protocols compared

| | what it does |
|---|---|
| **naive** | load, collect immediately |
| **v1** | post-load CSS `animation-duration:0.0001s` + `animation-play-state:paused` **and** JS `getAnimations().pause(); currentTime=0` |
| **v3** | pre-paint CSS `animation:none;transition:none` via `addInitScript`, SVG SMIL `pauseAnimations()`, `fonts.ready`, capped `decode()`, 2×rAF |
| **v4** | v3 **+** WAAPI `getAnimations().pause()` (twice, to catch late-created), pinned `Date.now`/`performance.now`, `requestAnimationFrame` callbacks pinned to t=0 |

## Results

5 runs per page per protocol, 23 pages, 40 211 elements compared.

| protocol | mean identical | pages 100% all-field | pages 100% geometry | element-weighted geometry |
|---|---|---|---|---|
| naive | 99.81% | 17 / 23 | 19 / 23 | 99.821% |
| v1 | 99.70% | 17 / 23 | 20 / 23 | 99.821% |
| v3 | 99.74% | 17 / 23 | 21 / 23 | 99.823% |
| **v4** | **99.9953%** | **22 / 23** | **23 / 23** | **100.0000%** |

Under v4 the only surviving drift in the entire corpus is `lineBoxes`
on `en.wikipedia.org` (2 element-instances, 67 vs 65 line boxes).
Every geometry field — `box.x/y/w/h` — is byte-identical for all
40 211 elements across all 5 runs on all 23 pages.

Element set stability was 100% at every protocol: the same DOM paths
appear in all 5 runs on all 23 pages. No element appears or disappears
between runs. That is a stronger result than the field-level rate and
it is the one that matters for assertion stability — a check that
references an element will find it.

**v1 was worse than naive.** It mixed two contradictory freeze
strategies — CSS saying "jump to the end instantly", JS saying "rewind
to 0 and pause" — and applied both *after* first paint, so the outcome
depended on which won the race. The lesson generalizes: a settle
protocol must apply **one** strategy, and apply it **before** the first
byte is parsed.

## Drift causes, ranked

Every residual drift under v3 was traced to a specific mechanism.
None of them is browser nondeterminism; all are *animation or time*.

| cause | evidence | fix |
|---|---|---|
| **WAAPI animations** (`element.animate()`) | gymshark.com `transform: matrix(0.3088…)` → `matrix(0.4066…)` across runs | CSS `animation:none` does **not** touch WAAPI. Needs `getAnimations().pause()`. |
| **SVG SMIL** (`<animate>`) | linear.app `<circle>` opacity flipping 0.3 ↔ 1 on 12 elements | CSS `animation:none` does not touch SMIL either. Needs `svg.pauseAnimations()`. |
| **rAF-driven JS animation** | bbc.com opacity 0.3379 / 0.3092 / 0.3207 — a continuum, not a toggle | Pin `performance.now()` and `Date.now()`; a frame computed from a constant clock is constant. |
| **Timer-driven DOM mutation** | linear.app element `visibility: visible → hidden` after run 1 | Not fixed by freezing paint; the DOM itself changes on a `setTimeout`. Pinning `Date.now` helps only if the site derives state from the clock rather than the timer firing. |
| **Text line-box count** | en.wikipedia.org `lineBoxes` 67 vs 65 | The only *unfixed* residue. Suspected late font swap after `fonts.ready` resolved. |

### The trap worth naming

`img.decode()` **never settles** for an image whose fetch failed. In an
offline archive replay that is common, and an un-capped `await` on it
hangs forever — it silently killed the first corpus capture run for 12
minutes before it was diagnosed. Every settle primitive needs a
timeout; "wait for quiescence" is not safely expressible as an
unbounded await.

This is the same class of bug as axe-core #1730 (cited in the brief):
a settling helper that no-ops or never completes in an edge case, and
so produces different verdicts on identical input.

## Load sensitivity

Under protocol v3, `linear.app` scored 100% when run alone and 98.95%
when run with 4 workers competing for 4 cores. A protocol whose
correctness depends on wall-clock waits is load-sensitive, which in CI
means flaky. v4 removes the dependence on elapsed time for the
animation classes; the full-corpus v4 run was executed **under 4-way
CPU contention on purpose**, so its numbers are the contended case.

## Verdict

**Gate 1: PASS.** Geometry facts are reproducible to 100% per-page
under v4 with a documented, mechanical settling protocol, and the
element set is stable across every page and protocol. The residual
non-geometric drift is one field on one page with an identified
suspected cause.

The important qualification: this measures **replay determinism** of a
frozen archive in one browser build at one viewport. It does *not*
establish cross-browser or cross-version stability, which is a
different (and for a gate, probably harder) question. See
`unverified.md`.
