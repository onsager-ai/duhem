# Decision table

Filled in from Phase 5a/5b/5c. **No recommendation** — that is the
architect's call.

| Evidence | Implication | Holds? |
|---|---|---|
| Recall high at human-visible severity **and** C-corpus shows B/D failures | Relational class is viable; the 466 FPs are a tunable engineering problem, not a design verdict. Prior conclusion reverses. | **Half.** Recall is high (100% for protrusion at ≥4 px, 100% for overlap at ≥16 px, 88% for viewport at ≥4 px) — but the generated corpus shows *fewer* B/D failures than production sites, not more: 2.5 findings per 1000 visible elements vs 20.3, with 15 of 26 pages entirely clean and zero viewport escapes. |
| Recall low even at gross severity | Relational class is dead on evidence. Prior conclusion stands and is now supported. | **No.** Recall is high. Two structural blind spots exist (all SVG content; overlaps below the 0.25 cover gate) but they are bugs with addresses, not evidence of an unworkable class. |
| Recall high but C-corpus failures concentrate in A | Ship a token/scale linter. DSL and relational predicates both over-engineered. | **Closest match, with a caveat.** Size-normalised, generated pages are 2.3× less regular in font-size, 1.9× in text colour, 1.4× in spacing. That is the only axis where the target distribution is measurably worse than production. The caveat: this is *elevated scatter*, not counted A-class violations — no frozen scale existed to check membership against, so nothing "failed". |
| Sweep density needed for recall is unaffordable **and** infidelity is not auto-detectable | D-class cannot be a reproducible CI gate in current form. Report as a blocking constraint. | **No — this row is now closed.** Both halves fail. Infidelity is page-specific (4 of 6 pages agree at every width, including far from capture width) and is cleanly detectable by a single scalar (`archive.sw / live.sw` = 1.000 for all agreeing pairs vs 2.35–3.52 for the disagreeing ones). Sparse capture plus a cheap live validation is viable, so the 164-minute / 3.1 GB dense-capture row is not required. |

## Interpretation

The two numbers that were missing now exist, and they point in different
directions than the prior phase assumed. Recall is not the problem — the
frozen detectors find injected faults reliably above roughly 4–16 px, localise
them exactly, and never fired on a clean baseline, so the Phase 2 verdict was
indeed drawn from evidence that could not support it. But the target
distribution does not rescue the relational class either: AI-generated pages
turn out to be *quieter* than mature production sites on relational geometry,
plausibly because they are smaller, flatter, and built from a few conservative
component libraries, and the one axis where they are measurably worse is
A-class scale regularity. Meanwhile the constraint that looked like it might
block class D has largely dissolved — the archive-fidelity problem was
overstated from a single page, both confirmed small-range defects reproduce
exactly in archives, and infidelity is detectable for the price of one page
load. What remains genuinely unresolved is signal-to-noise: an injected fault
is detected but arrives alongside a median of 17–22 pre-existing findings on
the same page, all of which Phase 2 adjudicated as false positives at 46/46 —
so the relational class is *detectable* but not yet *reportable*, and those
are different problems with different fixes.
