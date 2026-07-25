# Phase 0 — Ground truth

Supersedes the earlier `gate0-stop-report.md`, which stopped at Gate 0
because no corpus of ≥20 real *generated* pages was reachable. The
brief was then revised: **crawl real in-the-wild sites instead of
generating them**, precisely because testing our own generator against
our own frozen scales biases the token-drift rate toward zero. Under
that rule Gate 0 clears.

## 1. The KB note is the source of record

`UI 审美的可测性分层与静态断言设计` (Notes DB, Status: Inbox; captured
2026-07-24). Re-read this session. It supplies the measurability
ladder used throughout:

- **L1** constraint violations (contrast, overflow, overlap, out-of-bounds,
  token escape, grid alignment) — boolean, no model needed.
- **L2** perceptual proxies (visual complexity, colourfulness, whitespace) —
  continuous + a threshold that is human preference in disguise.
- **L3** overall appeal — ordinal only, needs multiple raters.

Its key judgment: *L1 is fully assertable, L3 has never been done, and
**L2 is the dangerous layer** — it looks assertable but the threshold is
taste wearing a lab coat.* The academic lineage it records (Birkhoff
1933 → Ngo 2003 → Bauerly & Liu 2006, where "balance" turned out
uncorrelated with human judgment → Reinecke CHI 2013's ~50% variance
ceiling) is the evidence for that.

It also records the software-engineering line that actually reached L1
by restating the problem as **defect detection rather than aesthetic
scoring**: ReDeCheck's Responsive Layout Graph, its five RLF types, and
its TP/FP/**NOI** split — where NOI ("DOM coordinates collide but the
eye cannot see it") is named as the natural instance of `inconclusive`.
UI-Bench (2025) is the counterweight: 4 000+ expert judgments, and a
methodological retreat to *pairwise* comparison because absolute
aesthetic scores misalign with human preference. Aesthetics has order,
not magnitude.

The note's three open questions are, verbatim in intent, Gate 2's
fork: is the L1 rule set exhaustive; is the L1 fail rate on AI-generated
UI enough to gate on; and how large is the NOI/inconclusive share.

## 2. Duhem's actual shape (read from code, not memory)

| surface | file | what it is |
|---|---|---|
| `Assertion` | `crates/duhem-schema/src/assertion.rs` | **Closed** enum: `Expr`, `TypeCheck`, `Matches`, `In`, `Exists`, `Equal`. Its doc comment states the closed shape *is* the structural enforcement of "mechanical judgment, not LLM judgment" — no free-text variant exists, so a judge implementing it cannot call a model. |
| `Criterion` / `Check` | `crates/duhem-schema/src/criterion.rs` | `Criterion{id, description, checks[]}`; `Check{id, description?, steps[], assertions[]}`. Since spec #253 a check may carry **no** assertions and rely on *judging steps* — actions whose contract emits a boolean `satisfied`. |
| Verdict | `crates/duhem-judge/src/verdict.rs` | `VerdictState { Pass, Fail, Inconclusive(InconclusiveCause) }`. Causes: `Timeout`, `MissingObservation`, `EnvironmentError`, `EmptyAggregation`; `#[non_exhaustive]`. |
| Folding | `crates/duhem-judge/src/aggregate.rs` | Any `Fail` → `Fail`; any `Inconclusive` and no `Fail` → `Inconclusive`; all `Pass` → `Pass`. First cause wins. Identical at every level. Plus `InconclusivePolicy { Block, Warn, Pass }` at criterion level. |
| Browser | `crates/duhem-actions/src/browser.rs` | Playwright via a Node sidecar over stdio JSON-RPC (#71); one sidecar per `duhem run`. |
| UI actions | `crates/duhem-actions/src/ui/` | `navigate`, `click`, `type_`, `select`, `assert_element`, `assert_state`, `assert_url`. `assert_element` declares outputs `["satisfied", "count"]`. |

**Correction to the earlier note.** It claimed the judge was two-state
and that making it tri-state was the central design decision. That is
false: the judge has been tri-state with causes, folding rules and a
policy layer all along. The earlier claim came from memory rather than
from the code — exactly the failure mode the brief's authority rule
exists to prevent. The real gap is much narrower and is additive: every
existing `InconclusiveCause` is *infrastructural* ("we never got to
look"), and this domain needs a *semantic* one ("we looked; observability
is undeterminable"). See `design.md` §6 and `unverified.md` U8.

**Where a domain pack plugs in.** Without touching the closed
`Assertion` enum: a `ui/collect-facts` action producing a versioned
snapshot, and a `ui/assert-layout` judging step emitting `satisfied`
(the #253 path). The identity commitment stays intact because the
predicates ride the action contract, not the assertion grammar.

## 3. Corpus (Gate 0)

23 frozen single-file MHTML archives of real sites — marketing 6,
docs 5, editorial 7, ecommerce 3, app 2 — captured 2026-07-25
03:09–03:15Z at 1280×800, DPR 1, `en-US`, UTC. 49.7 MB total. Each
reloads **fully offline** (verified: zero external requests permitted,
`externalBlocked == 0`), which is what makes Phase 1's reproducibility
measurement meaningful — live sites mutate under A/B tests and CDN
variance.

Full table, provenance, and the excluded targets (robots.txt disallow,
timeout, bot-wall pages) are in `phase1-facts-determinism.md`.

**Gate 0: cleared** — 23 ≥ 20, spanning five genres and design systems
our freeze layer has never seen.

One capture-mechanics note that became a Phase 2 finding: Chromium's
own proxy CONNECT was refused in this environment, so the crawler
routes the browser's network through Node's (working) proxy path via
request interception, then serializes with CDP `Page.captureSnapshot`.
Full JS execution, no TLS verification disabled.
