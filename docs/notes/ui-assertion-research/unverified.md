# Unverified

Everything load-bearing I could **not** confirm this session, and what
would confirm it. Claims resolved during the session are listed at the
bottom so the record shows what changed.

| # | Claim (as used) | Why unverified | What would confirm it |
|---|---|---|---|
| U1 | **Recall.** B-class overlap detection catches real defects when they exist. | The corpus is 23 professionally-built production sites; they mostly have no visible desktop-width defects. 0/46 true positives measures **precision, not recall**. | A corpus with *known* defects: AI-generated pages, or the same pages with injected CSS faults (mutation of the page, not the facts). |
| U2 | **The adjudication is correct.** | All 46 inspected findings were adjudicated **by me, from rendered crops** — not by an independent human reviewer, which is what the brief asked for. I am both the author of the checks and the judge of their output. | A second reviewer labelling the same 71 crops blind; report agreement. |
| U3 | **The 8-width sweep has adequate resolution.** | ReDeCheck samples 320–1400 px at 60 px steps plus CSS-extracted breakpoints and binary-searches change points. I used 8 fixed widths. Two of my three confirmed defects were single-width ("Small-Range"), so coarse sampling is a live recall risk. | Re-run at 60 px steps with breakpoint refinement; count defects found only at unsampled widths. |
| U4 | **Cross-browser / cross-version determinism.** | Phase 1 measured *replay* determinism in one Chromium build (141.0.7390.37) at one viewport. Says nothing about Firefox/WebKit, or Chromium N+1. | Repeat the 5-run protocol across engines and across a version bump; a CI gate depends on this far more than on same-build repeatability. |
| U5 | **The `lineBoxes` residue is a late font swap.** | On `en.wikipedia.org`, 67 vs 65 line boxes survives protocol v4. "Suspected late font swap after `fonts.ready` resolved" is a hypothesis I did not isolate. | Instrument `document.fonts` events across the 5 runs and diff the loaded-face set at collect time. |
| U6 | **Per-width capture actually restores fidelity.** | I proved a 1280-captured archive is unfaithful away from 1280. I did **not** verify that capturing *at* 375 makes the 375 assertions match the live page. The design depends on this. | Capture the same page at 320/768/1280, compare each against the live page at its own width. |
| U7 | **The economics thesis** (spec cost < value at our volume). | Depends on defect *rate* in AI-generated UI and on pages/day, neither measured. This corpus cannot speak to it — it is not AI-generated output. | Combine a defect-rate measurement on generated pages (U1) with real throughput from a generator's telemetry. |
| U8 | **A semantic `InconclusiveCause` is the right addition.** | The judge's four existing causes are infrastructural; I argue a semantic one is needed. Whether that is better than reusing `MissingObservation` is a design call touching a spec-ratified, doctrinally-closed surface. | Human decision against `docs/duhem-spec.md` §7.6, through the schema-impact gate. |
| U9 | **Domain pack vs core, and the Phase-0 timing.** | Recommendation only; the brief reserves it for a human. | Human confirmation against the KB and roadmap §14. |
| U10 | **ReDeCheck's 5 RLF types are the exhaustive L1 relational set.** | The KB lists this as its own open question (「是否可穷举」). I implemented 3 of 5 and did not test exhaustiveness. | Read ReDeCheck's source; check whether observed real defects fall outside the five types. |
| U11 | **Galen's evaluator internals.** | I verified Galen's release currency (2.4.4, 2019-03-15) and reasoned about its *language* from documented syntax. I did **not** read its grammar or evaluator source this session, which the brief asked for. | Read `galenframework/galen` parser + evaluator; revisit the DSL-vs-API argument against what the evaluator actually does. |
| U12 | **`figma.com` / `en.wikipedia.org` flipping between runs is live mutation.** | Both reported horizontal scroll in the sweep and none minutes later in verification. Most likely consent banners / A-B variants, but I did not isolate the cause. | Repeat the live sweep N times per width and report verdict variance; that number is the real cost of live sweeping. |

## Resolved this session

| Was unverified | Now |
|---|---|
| **≥20 real generated pages obtainable (Gate 0)** | **Superseded.** The brief changed the corpus rule to *crawled* real sites. 23 frozen archives assembled; Gate 0 cleared. |
| Facts reproducibility (Gate 1) | **Measured.** 100% geometry, 40 211 elements × 5 runs × 23 pages, protocol v4. |
| Failure distribution (Gate 2) | **Measured.** See `phase2-failure-distribution.md`. |
| `inconclusive` / NOI rate | **Measured.** 4.5% once the observability gate is computed rather than inferred (89% when inferred). |
| axe-core still active | **Confirmed.** 4.12.1 published 2026-06-10; canary builds 2026-07-24. |
| Galen unmaintained | **Confirmed.** Last release 2.4.4, 2019-03-15. |
| **"Duhem's judge is two-state; tri-state is the gap"** | **False — corrected.** `duhem-judge` is already tri-state with causes, folding rules, and an `InconclusivePolicy`. My earlier Phase 0 note asserted this from memory rather than from the code. The real gap is narrower (U8). |
