# VD Anti-Corrosion Report — `crawlab-spider-task-lifecycle`

> **RE-DERIVE NOW.** The VD as shipped no longer executes against the current
> Crawlab Pro target, and where its checks can be forced to run they pass a large
> fraction of runs that did no real work: **D̂ = 0.38, 95% CI [0.33, 0.44]**
> (θ = 0.60), from **N = 292** executable runs. A single target commit — 7 days
> after freeze — flipped the full VD from executable to broken; the drift curve is
> measured, not inferred. A sibling VD (`crawlab-create-project`) run through the
> same meter is **faithful (D̂ = 0)**, so the meter discriminates.
> — *measured 2026-07-10, target crawlab-pro@dfd1132 (v0.7.3), A = 50 commits since freeze.*

Measurement is strictly downstream of the verdict: `duhem.yml`, `criteria.md`, and
the judge were never touched, and the judge stayed deterministic and LLM-free. All
runs drove a live Crawlab **Pro** cluster (master + worker over gRPC, real MongoDB)
via `scripts/up.sh`. Every number traces to per-run records collected during
measurement (verdict + Mongo task state + output-line count per run) or to `git`;
the aggregates are embedded in `vd-corrosion-report.json` and the raw runs are
reproducible via the method below.

## Evidence base

| Dataset | Runs | What it establishes |
|---|---:|---|
| AC-2 (task-to-completion), pooled | 146 | P(pass), T̂, D̂, per-assertion ρ — the core drift number |
| AC-3 (spider→task link), pooled | 146 | worst criterion: passes errored/cancelled tasks |
| Full VD as shipped | 30 | non-executability against the current target |
| Time series (5 target commits) | 5 pts | the measured D(t) step |
| `crawlab-create-project` breadth | 24 | faithful contrast (D̂ = 0) |

---

## Finding 1 — the VD is already broken by target drift

Running the **full VD as shipped** 30 times (fixture varied over the spider
script) produced **0 pass / 0 fail / 30 inconclusive**. Every run aborts at the
same step with the same engine error:

```
engine (...duhem.yml): step `savefile`: unresolved reference
  `$runtime.format("{}/{}/files/save", $inputs.spiders_url, $steps.create.outputs.body.data._id)`
```

AC-1, AC-2, and AC-3 each create a spider named `$inputs.spider_name`. On
**2026-07-02** — 7 days *after* this VD froze (2026-06-25) — Crawlab commit
**`98e99a2`** ("feat(spider): reject duplicate spider name within a project") added
a per-project uniqueness check. The *second* create in a run now returns **409**,
`body.data._id` is absent, and the run dies before AC-2 reaches a verdict. AC-1
still executes and passes (the spider is really persisted); AC-2/AC-3 never
produce a verdict. **P(pass) for the shipped VD is undefined (0/0)** — the target
moved and the VD's assumption (create-the-same-name-twice) became invalid. This
alone is an unconditional RE-DERIVE trigger.

To measure the *check-layer* drift underneath the break, each criterion was run in
isolation with the documented `--filter` (the only way the frozen checks execute
today), fresh MongoDB per run, across a fixture spread.

---

## Finding 2 — the numbers (N = 292 executable runs)

Headline scalars are the **executable core** — AC-2 + AC-3 pooled — since the
shipped VD yields no conclusive verdict.

| metric | value | meaning |
|---|---:|---|
| **P(pass)** | **0.969** (283/292) | of runs that reached a verdict, 97% passed |
| **T̂** | **0.589** | P(pass) × in-spec fraction of passes |
| **D̂** | **0.380** | **38% of passing verdicts bless runs that did no real work** |
| **95% CI on D̂** | **[0.327, 0.436]** | Wilson on the in-spec proportion (172/283), propagated |
| **in-spec fraction of passes (p_is)** | **0.608** | the workload-independent core signal — ~39% of passes are hollow |

The **stable, workload-independent** quantity is **p_is ≈ 0.61**: across two very
different fixture mixes (balanced N=30 and passing-heavy N=116) p_is held at
0.63/0.63 for AC-2 and 0.53/0.62 for AC-3. D̂ = P(pass)·(1−p_is) *scales with the
workload's pass rate*, so it is reported per mix; p_is does not move.

The independent re-review that produces T̂ reads a signal the judge never looks at:
the task's **captured stdout output-line count** (Crawlab's own
`/var/log/crawlab/tasks/<id>/log.txt`, framework lines filtered).
`in_spec := status==finished AND output_line_count ≥ 1`. (The `error` field is
deliberately **not** used: under load, 8 finished, output-producing tasks carried a
transient `"Task reset … after node reconnection"` note — a genuinely failed task
has `status=="error"`, not `"finished"`.)

**Per-criterion** (this is where the drift concentrates):

| criterion | N | P(pass) | T̂ | D̂ | 95% CI | note |
|---|---:|---:|---:|---:|---|---|
| **AC-1** (create + persist) | 30 | 1.00 | 1.00 | **0.00** | — | faithful; spider genuinely persisted, no execution to be hollow |
| **AC-2** (run to completion) | 146 | 0.938 | 0.589 | **0.349** | [0.278, 0.428] | 51 of 137 passes hollow (finished, zero output) |
| **AC-3** (spider→task link) | 146 | 1.00 | 0.589 | **0.411** | [0.334, 0.492] | **146/146 pass — including errored & cancelled tasks** |

**AC-3 is the worst.** Its assertions check referential integrity
(`task.spider_id == spider._id`, `task._id == run's id`). Those links are written
at *schedule* time, so they hold for **any** created task — errored, cancelled by
timeout, or hollow. AC-3 passed **146/146**, certifying "the link is real" while
the task behind it failed or idled in 60 of those runs.

A structural fact that makes the blind spot intrinsic: Crawlab's own Mongo
data-yield metric `task_stats.result_count` was **0 for every one of the 283
executable passes**, and Crawlab's default *file* log driver keeps output off Mongo
entirely. Neither Mongo signal the judge *could* have reached distinguishes a real
run from a hollow one — the only discriminator (output line count) lives where the
judge never looks.

---

## Finding 3 — ranked structural blind spots (per-assertion ρ, N = 146)

ρ = (# times the assertion was judged **true** while the artifact was truly
**out-of-spec**) ÷ (# times judged true). Higher = fires green on more work that
didn't meet intent. Estimates are now on 137–146 judged-true observations each
(was 24–30 at N=30); the ordering is unchanged and the values tightened.

| rank | criterion | assertion | ρ | true-while-OOS / judged-true | exposed by |
|---|---|---|---:|---:|---|
| 1 | AC-3 | `rows[0].spider_id == create…data._id` | **0.411** | 60/146 | nonzero, timeout, noop, noop2 |
| 1 | AC-3 | `rows[0]._id == run…data[0]` | **0.411** | 60/146 | nonzero, timeout, noop, noop2 |
| 1 | AC-2 | `executed…rows[0].cmd == $inputs.spider_cmd` | **0.411** | 60/146 | nonzero, timeout (cmd is set at schedule time), noop, noop2 |
| 1 | AC-2 | `executed…row_count >= 1` | **0.411** | 60/146 | nonzero, timeout (a *row* exists ≠ the task succeeded), noop, noop2 |
| 1 | AC-2 | `savefile…status == 200` | **0.411** | 60/146 | nonzero, timeout, noop, noop2 |
| 1 | AC-2 | `run…status == 200` | **0.411** | 60/146 | nonzero, timeout, noop, noop2 |
| 7 | AC-2 | `awaited…satisfied == true` | 0.372 | 51/137 | noop, noop2 only |
| 7 | AC-2 | `executed…rows[0].status == "finished"` | 0.372 | 51/137 | noop, noop2 only |

The two lowest-ρ assertions are the only ones doing real gating work — they reject
`nonzero` and `timeout`. Everything above is decorative: a 200 here, a row exists,
a string matches, an id links — all true on a task that errored, timed out, or
produced nothing. **The single most-blind check is AC-3's referential-integrity
pair (it passed on every errored and cancelled task).** The most economical repair
is one assertion that reads a real-work signal (e.g. an output-line / non-empty-log
assertion) so "finished" must be backed by yield.

---

## Finding 4 — the drift curve, measured across target history

The report previously *inferred* the step from the breaking commit + code diff.
It is now **measured**: crawlab-pro was rebuilt at 5 historical commits (a
non-destructive `git worktree`; the main checkout stayed on HEAD) and the meter run
at each. All 5 commits built.

| commit | date | phase | **full VD executable?** | full-VD verdicts |
|---|---|---|:--:|---|
| `aa75559` | 2026-07-01 | pre-break | **✅ yes** | 5 pass / 1 fail |
| `dd84593` | 2026-07-01 | pre-break | **✅ yes** | 5 pass / 1 fail |
| `98e99a2` | 2026-07-02 | **the break** | **❌ no** | 6/6 engine-abort |
| `532cd59` | 2026-07-02 | post-break | **❌ no** | 6/6 engine-abort |
| `dfd1132` | 2026-07-09 | HEAD | **❌ no** | 6/6 engine-abort |

The full VD's executability steps from **True → False exactly at the breaking
commit** and stays broken through HEAD. This *is* the drift curve: a discontinuity,
not a slope. `D(t) = D0 + ρ_dynamic·A` is the wrong model — **ρ_dynamic is
undefined** (the drift is a step, not gradual), and `D0` (the design-level hollow
blind spot) is the reliable N=146 number above.

The AC-2 criterion still *runs* at every commit (P(pass) ≈ 0.92), so only the full
VD's create-collision is target-broken. (Per-commit AC-2 D̂ is **not** read as a
trend: the first few runs after each fresh cluster under-report output — a
log-capture warm-up artifact — so those points are noisy; the reliable D̂ is the
long-lived-cluster N=146 measurement.)

---

## Finding 5 — cross-VD breadth: the meter discriminates

The sibling `crawlab-create-project` VD, run through the same meter (24 runs,
`project_name` varied over normal / empty / whitespace / 400-char / injection-ish /
unicode), is **faithful**:

- **P(pass) = 1.00, D̂ = 0.00.** Every run — including degenerate empty and
  whitespace names — produces a project genuinely persisted in Mongo with the
  supplied name and a well-formed non-zero id (verified independently, exactly
  AC-5's deep slice). All five criteria pass 24/24.
- **Why it's faithful and the lifecycle VD isn't:** create-project verifies a
  *static resource* that cannot be hollow, and its deep checks (AC-4/AC-5) tie to
  the specific `_id`. The lifecycle VD certifies an *execution outcome* (`finished`)
  without checking that real work happened.
- One latent weakness noted but not exercised into a false pass: AC-2/AC-3 are
  existence-only (`total >= 1`), satisfied by any project rather than the created
  one; fresh-Mongo isolation + the deep AC-4/AC-5 checks compensate here.

This contrast is the credibility check: the meter does **not** always cry drift.
D̂ = 0.38 on the lifecycle VD and D̂ = 0.00 on create-project come from the same
apparatus.

---

## Trigger evaluation (θ = 0.60) → **RE-DERIVE NOW**

- Is D̂ > θ now? Executable-core **D̂ = 0.38 < 0.60** → in isolation, MONITOR. But
  AC-3 alone has D̂ = 0.41 with **CI upper bound 0.49**, and the **step to
  non-executability has already fired** — an unconditional re-derive trigger
  independent of θ.
- Releases until D crosses θ? **Zero — the trigger already fired** (the breaking
  commit landed within the observed A = 50 commits).

### Recommendation

1. **Unbreak execution (mandatory).** Stop re-creating the same
   `$inputs.spider_name` across criteria; re-derive so each uses a distinct name
   (or a shared create), matching Crawlab's now-enforced per-project uniqueness.
2. **Close the yield blind spot (mandatory).** Add one assertion that a passing
   task did real work (output-line / non-empty-log). Today "finished + cmd matches"
   is satisfied by a no-op — that is the 38% false-pass mass.
3. **Fix AC-3's structural tautology.** Pair the `spider_id`/`_id` link assertions
   with a terminal-outcome assertion so AC-3 cannot pass an errored/cancelled task.

Re-run this meter after re-derivation to confirm D̂ collapses.

---

## Uncertainty & sample-size honesty

- **AC-2 D̂ = 0.349** on **n = 137 passes**; 95% CI **[0.278, 0.428]**, half-width
  **±0.075** — down from ±0.147 at N=30. Sign and size are solid.
- **p_is (in-spec fraction of passes) ≈ 0.61** is the stable, workload-independent
  quantity. **D̂ itself is workload-dependent** (scales with the fixture mix's pass
  rate) and is reported per mix — do not read it as a single universal constant.
- **Time-series** per-commit D̂ is confounded by a post-restart log-capture warm-up;
  only the **executability step** (unaffected by log timing) is read as signal.
  Per-point N is small (full-VD 6, AC-2 12).
- **ρ_dynamic** has no linear estimate: the drift is a step, not a slope. A slope
  would require a target that drifts gradually.
- **Inconclusive accounting:** the executable core had **0** inconclusive; the full
  shipped VD had **30/30** inconclusive (engine error). Reported separately, never
  folded into pass or fail.

## Reproduction

- Executable-core runs: one criterion per invocation —
  `duhem run <vd> --filter AC-2 --no-env-up --keep-env --db <store> --reporter json --inputs @<file>`
  (and separately `--filter AC-3`) against the `scripts/up.sh` cluster; fixture
  varied via `spider_file_data` only. Full-VD runs are identical minus `--filter`.
  (AC-2 and AC-3 cannot share one invocation — each re-creates `$inputs.spider_name`,
  which now 409s.)
- Fixtures: `echo` (1 line), 3-line echo, real HTTP fetch, `true`/`cd /` (zero
  output), `exit 1` (non-zero), `sleep 105` (exceeds the 90 s poll budget).
- Ground truth per run: Mongo `tasks`/`task_stats` by task `_id` +
  `/var/log/crawlab/tasks/<id>/log.txt` output-line count.
- Time series: `git worktree` at each commit; `up.sh` rebuilds the Pro binary per
  commit; main checkout untouched. Aggregated results are in
  `vd-corrosion-report.json` (`time_series.points`, `cross_vd_breadth`,
  `per_criterion`); per-run raw records are reproducible via the steps above.
