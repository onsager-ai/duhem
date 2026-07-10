# VD Anti-Corrosion Report — `crawlab-spider-task-lifecycle`

> **RE-DERIVE NOW.** The VD as shipped no longer executes against the current
> Crawlab Pro target (30/30 full runs abort with an engine error), and where its
> checks *can* be forced to run they pass **38%** of runs that did no real work
> (D̂ = 0.38, 95% CI [0.27, 0.50]; θ = 0.60). One target commit 7 days after
> freeze did this. — *measured 2026-07-10, N=30, target crawlab-pro@dfd1132 (v0.7.3), A=50 commits since freeze.*

Measurement is strictly downstream of the verdict: `duhem.yml`, `criteria.md`,
and the judge were never touched. All runs drove a live Crawlab **Pro** cluster
(master + worker over gRPC, real MongoDB) via `scripts/up.sh`; the judge stayed
deterministic and LLM-free. Every number below traces to recorded run data
(`ac2_runs.jsonl`, `ac3_runs.jsonl`, `fullvd_runs.jsonl`) or to `git`.

---

## The headline finding first: the VD is already broken by target drift

Running the **full VD as shipped** 30 times (fixture varied over the spider
script) produced **0 pass / 0 fail / 30 inconclusive**. Every run aborts at the
same place with the same engine error:

```
engine (...duhem.yml): step `savefile`: unresolved reference
  `$runtime.format("{}/{}/files/save", $inputs.spiders_url, $steps.create.outputs.body.data._id)`
```

Why: AC-1, AC-2, and AC-3 each create a spider named `$inputs.spider_name`.
On **2026-07-02** — 7 days *after* this VD was frozen (2026-06-25) — Crawlab
commit **`98e99a2`** ("feat(spider): reject duplicate spider name within a
project") added a per-project uniqueness check. So the *second* create in a run
now returns **409**, `body.data._id` is absent, and the run dies before AC-2 ever
reaches a verdict. AC-1 still executes and passes (the spider is really
persisted) in all 30 runs; AC-2 and AC-3 never produce a verdict.

This is corrosion in its bluntest form: **P(pass) for the shipped VD is
undefined (0/0)** — not because the target regressed, but because the target
*moved* and the VD's checks encoded an assumption (create-the-same-name-twice)
that the target invalidated. This alone is an unconditional RE-DERIVE trigger.

To measure the *check-layer* drift underneath that break, each criterion was run
in isolation with the documented `--filter` (the only way the frozen checks
execute today), fresh MongoDB per run, across the same fixture spread. Those
runs are the basis for every number below.

---

## The numbers, and what each means for this VD

Headline scalars are the **executable core** — AC-2 + AC-3 pooled (60 runs),
since the shipped VD yields no conclusive verdict at all.

| metric | value | meaning for this VD |
|---|---:|---|
| **P(pass)** | **0.917** (55/60) | Of runs that reached a verdict, 91.7% passed. Inconclusive excluded (there were none in the executable core). |
| **T̂** | **0.533** | P(pass) × in-spec fraction of passes. Only ~53% of what the VD certifies actually corresponds to real work. |
| **D̂** | **0.383** | P(pass) − T̂. **38% of the VD's passing verdicts bless runs that did not meet real intent.** D>0 = false-pass drift (Goodhart). |
| **95% CI on D̂** | **[0.273, 0.504]** | Wilson interval on the in-spec proportion (32/55), propagated. Wide but entirely above zero — the drift is real, its size is uncertain. |
| **A** | **50** commits (1 release, 0.7.3) | Target commits on crawlab-pro since freeze. The breaking commit sits mid-window. |
| **θ** | **0.60** | Re-derive threshold. |

**P(pass) = 0.917.** Nearly everything passes once it runs — which is exactly
what makes the drift dangerous: a high pass rate is read as "healthy," but…

**T̂ = 0.533 vs P(pass) = 0.917.** …only 53% of the pass mass is real. The gap is
the corrosion. The independent re-review that produced T̂ reads a signal the
judge never looks at: the task's **captured stdout output-line count** (from
Crawlab's own `/var/log/crawlab/tasks/<id>/log.txt`, framework lines filtered),
cross-checked against Mongo `tasks.status`/`error` and
`task_stats.result_count`/`runtime_duration`. `in_spec := status==finished AND
error=="" AND output_line_count ≥ 1`.

**D̂ = 0.383.** This is the whole point of the meter. The VD equates
"task row exists with `status==finished` and matching `cmd`" with "the spider
ran and did its job." A no-op (`true`) finishes identically to `echo hello` —
same `finished` status, same `cmd`, same (zero) `result_count`, same ~10 ms
runtime. The VD cannot tell them apart, so it passes both.

**Per-criterion breakdown** (this is where the drift concentrates):

| criterion | P(pass) | T̂ | D̂ | 95% CI | note |
|---|---:|---:|---:|---|---|
| **AC-1** (create + persist) | 1.00 | 1.00 | **0.00** | [0.00, 0.00] | Faithful. The spider document genuinely lands in Mongo; no spider is executed, so the hollow-work blind spot doesn't apply. |
| **AC-2** (run to completion) | 0.833 | 0.533 | **0.30** | [0.169, 0.462] | 25 pass / 5 fail. 9 of 25 passes were hollow (finished, zero output). |
| **AC-3** (spider→task link) | 1.00 | 0.533 | **0.467** | [0.302, 0.639] | **30 pass / 0 fail — including tasks that ERRORED and were CANCELLED.** CI upper bound crosses θ. |

**AC-3 is the worst.** Its two assertions check referential integrity
(`task.spider_id == spider._id`, `task._id == run's reported id`). Those links
are written at *schedule* time, so they hold for **any** created task — one that
errored, one that was cancelled by timeout, one that did nothing. AC-3 therefore
passed 30/30, certifying "the link is real" while the task behind the link
failed or idled 14 times.

**A structural fact that makes the blind spot intrinsic:** Crawlab's own Mongo
data-yield metric, `task_stats.result_count`, was **0 for all 55 executable
passes**. Crawlab's default *file* log driver keeps spider output on disk, not in
Mongo. So neither Mongo signal the judge *could* have reached distinguishes a
real run from a hollow one — the only discriminator (output line count) lives
where the judge never looks.

---

## Ranked structural blind spots (per-assertion ρ)

ρ = (# times the assertion was judged **true** while the artifact was truly
**out-of-spec**) ÷ (# times judged true). Higher = the assertion fires green on
more work that didn't meet intent. All six below are tied at the top because
they all certify *structure* (a 200, a row exists, a string matches, an id
links) rather than *execution outcome*.

| rank | criterion | assertion | ρ | true-while-OOS / judged-true | fixture that exposed it |
|---|---|---|---:|---:|---|
| 1 | AC-3 | `rows[0].spider_id == create…data._id` | **0.467** | 14/30 | `nonzero` (errored task still links), `timeout` (cancelled task still links), `noop`/`noop2` (hollow) |
| 1 | AC-3 | `rows[0]._id == run…data[0]` | **0.467** | 14/30 | `nonzero`, `timeout`, `noop`, `noop2` |
| 1 | AC-2 | `executed…rows[0].cmd == $inputs.spider_cmd` | **0.467** | 14/30 | `nonzero`, `timeout` (cmd is set at schedule time — true even for tasks that never ran cleanly), `noop`/`noop2` |
| 1 | AC-2 | `executed…row_count >= 1` | **0.467** | 14/30 | `nonzero`, `timeout` (a task *row* exists ≠ the task *succeeded*), `noop`/`noop2` |
| 1 | AC-2 | `savefile…status == 200` | **0.467** | 14/30 | `nonzero`, `timeout`, `noop`, `noop2` (saving a file says nothing about execution) |
| 1 | AC-2 | `run…status == 200` | **0.467** | 14/30 | `nonzero`, `timeout`, `noop`, `noop2` (accepting a run says nothing about its outcome) |
| 7 | AC-2 | `awaited…satisfied == true` | 0.360 | 9/25 | `noop`/`noop2` only — this one *does* catch `nonzero`/`timeout`, but is blind to hollow finishes |
| 7 | AC-2 | `executed…rows[0].status == "finished"` | 0.360 | 9/25 | `noop`/`noop2` only — catches error/timeout, blind to hollow |

The two lowest-ρ assertions (`awaited.satisfied`, `status=="finished"`) are the
only ones doing real gating work: they reject the `nonzero` and `timeout`
fixtures. Everything above them is decorative — it turns green on a task that
errored, timed out, or produced nothing. **The single most-blind check is the
AC-3 referential-integrity pair: it passed on every errored and cancelled task
in the sample.** The most economical repair is to add one assertion that reads a
real-work signal (e.g. `task_stats.result_count >= 1`, or an output-line
assertion) so "finished" must be backed by yield.

---

## Drift over target time

Model `D(t) = D0 + ρ_dynamic · A`:

- **D0 = 0.383** — the executable-core drift measured now. The hollow-pass blind
  spot is *design-level*: it existed at freeze (the checks never inspected
  yield), so D0 is the right reading of the frozen VD's intrinsic gap.
- **ρ_dynamic = null — insufficient time series (one measurement point).** And a
  linear slope is the wrong model here anyway. The dominant drift is **a step
  function**: at crawlab commit `98e99a2` (2026-07-02) the full VD went from
  *executable* to *non-executable*. Not a gentle slope — a cliff, already fallen
  off. A real ρ_dynamic needs repeated runs across future target releases.
- **A = 50** commits since freeze (1 tagged release, 0.7.3). The breaking commit
  landed at A ≈ mid-window, 7 days after freeze.

---

## Trigger evaluation (θ = 0.60)

- Is D̂ > θ now? **Executable-core D̂ = 0.383 < 0.60 → in isolation, MONITOR.**
  But AC-3 alone has D̂ = 0.467 with **CI upper bound 0.639 > θ**.
- Releases until D crosses θ? **Zero — the trigger has already fired.** The step
  to non-executability landed within the observed A=50 commits; the VD is broken
  *today*.

### Recommendation: **RE-DERIVE NOW**

1. **Unbreak execution (mandatory).** The three criteria must stop re-creating
   the same `$inputs.spider_name`. Re-derive so each criterion uses a distinct
   name (or a shared create), matching Crawlab's now-enforced per-project name
   uniqueness (commit `98e99a2`).
2. **Close the yield blind spot (mandatory).** Add at least one assertion that a
   passing task did real work — `task_stats.result_count >= 1`, or an
   output-line-count / non-empty-log assertion. Today "finished + cmd matches"
   is satisfied by a no-op; that is the 38% false-pass mass.
3. **Fix AC-3's structural tautology.** `spider_id`/`_id` links are true for any
   scheduled task; pair them with a terminal-outcome assertion so AC-3 cannot
   pass an errored or cancelled task.

Re-run this meter after re-derivation to confirm D̂ collapses and to begin a real
time series for ρ_dynamic.

---

## Uncertainty & sample-size honesty

- **AC-2 D̂ = 0.30** rests on **n = 25 passes**; 95% CI **[0.169, 0.462]** is
  wide. Tightening the in-spec-proportion CI half-width to ±0.05 needs ~**384
  passes** (≈ **460 runs** at this pass rate). Treat the *sign and rough size* of
  D̂ as solid; the second decimal is not.
- **ρ values** rest on 24–30 judged-true observations each; practical
  uncertainty band ≈ **±0.1**. The *ordering* (structural checks ≫ outcome
  checks) is robust; the exact ρ is not.
- **ρ_dynamic** has no estimate: one time point. A slope requires repeated runs
  across future crawlab-pro releases.
- **Inconclusive accounting:** the executable core had **0** inconclusive; the
  full shipped VD had **30/30** inconclusive (engine error). These are reported
  separately and never folded into pass or fail.

## Reproduction

- Runs: `--filter AC-2|AC-3 --no-env-up --keep-env --db <store> --reporter json`
  against the `scripts/up.sh` cluster; fixture varied via `--inputs @<file>`
  (`spider_file_data` only). Full-VD runs identical minus `--filter`.
- Fixtures: `echo` (1 line), 3-line echo, real HTTP fetch (`curl` the master
  health endpoint), `true`/`cd /` (zero output), `exit 1` (non-zero), `sleep 105`
  (exceeds the 90 s poll budget).
- Ground truth per run: Mongo `tasks`/`task_stats` by task `_id` +
  `/var/log/crawlab/tasks/<id>/log.txt` output-line count. Raw records in
  `ac2_runs.jsonl`, `ac3_runs.jsonl`, `fullvd_runs.jsonl`.
