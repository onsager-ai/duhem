// Pure fold from a stream of trace events (the #84 SSE payloads —
// raw `trace.jsonl` lines) to the same shape `GET /api/runs/:id`
// serves. The browser never computes a verdict: every verdict below
// is lifted verbatim from the judge's `*_finished` events.

import type { CriterionDetail, LifecycleBlock, RunDetail, TraceEvent } from "./api";

/** Fields `foldRun` cannot derive from the event stream. `useRun` carries
 *  them over from the authoritative `GET /api/runs/:id` so a live run shows
 *  what a finished one shows (#491). Typed as a `RunDetail` subset, so a new
 *  field added to `RunDetail` has to be classified rather than silently
 *  reverting to the fold's default on every event. */
export type FoldUnknowable = Pick<
  RunDetail,
  "verification" | "started_at" | "inputs" | "has_definition" | "viewport"
>;

/** Overlay the fetched detail's fold-unknowable fields onto a live fold. */
export function carryFetched(folded: RunDetail, fetched: RunDetail): RunDetail {
  const carried: FoldUnknowable = {
    verification: fetched.verification,
    started_at: fetched.started_at,
    inputs: fetched.inputs,
    has_definition: fetched.has_definition,
    viewport: fetched.viewport,
  };
  return { ...folded, ...carried };
}

export function foldRun(runId: string, events: TraceEvent[]): RunDetail {
  const detail: RunDetail = {
    run_id: runId,
    verification: runId,
    started_at: null,
    inputs: {},
    verdict: null,
    status: "running",
    setup_aborted: false,
    // Not knowable from the event stream: `run_started` carries the
    // snapshot, but the fold does not read it. The caller carries the
    // fetched value forward instead — see `carryFetched` (#491).
    has_definition: false,
    cleanup: [],
    lifecycle: [],
    criteria: [],
  };
  const criteria = new Map<string, CriterionDetail>();
  const criterionOf = new Map<string, string>();

  // Index the first step owner before folding. It is stronger than the
  // additive check_finished fallback even in a malformed stream (#490).
  for (const evt of events) {
    if (evt.kind === "step_started") {
      const checkId = String(evt.check_id);
      if (!criterionOf.has(checkId)) {
        criterionOf.set(checkId, String(evt.criterion_id));
      }
    }
  }

  const noteCheck = (criterionId: string, checkId: string) => {
    let crit = criteria.get(criterionId);
    if (!crit) {
      crit = { id: criterionId, verdict: null, checks: [] };
      criteria.set(criterionId, crit);
      detail.criteria.push(crit);
    }
    if (!crit.checks.some((c) => c.id === checkId)) {
      crit.checks.push({ id: checkId, verdict: null });
    }
  };

  for (const evt of events) {
    if (detail.started_at === null && evt.ts) {
      detail.started_at = evt.ts;
    }
    switch (evt.kind) {
      case "run_started":
        detail.verification = String(evt.verification_path ?? runId);
        detail.inputs = (evt.inputs as Record<string, unknown>) ?? {};
        break;
      case "setup_finished":
        if (evt.phase !== "teardown" && evt.criterion_id == null &&
            evt.check_id == null && evt.fixture_name == null) {
          detail.setup_aborted = Boolean(evt.aborted);
        }
        break;
      case "setup_step_started":
        if (evt.phase === "teardown") {
          detail.cleanup!.push({
            step_index: Number(evt.step_index),
            uses: String(evt.uses),
            outcome: "ok",
            fixture_name: typeof evt.fixture_name === "string" ? evt.fixture_name : undefined,
            check_id: typeof evt.check_id === "string" ? evt.check_id : undefined,
            criterion_id: typeof evt.criterion_id === "string" ? evt.criterion_id : undefined,
          });
        }
        break;
      case "setup_step_finished":
        if (evt.phase === "teardown") {
          const step = [...detail.cleanup!]
            .reverse()
            .find((candidate) => candidate.step_index === Number(evt.step_index) &&
              candidate.fixture_name === evt.fixture_name &&
              candidate.check_id === evt.check_id &&
              candidate.criterion_id === evt.criterion_id);
          if (step) {
            step.outcome = evt.outcome as typeof step.outcome;
          }
        }
        break;
      case "step_started":
        if (criterionOf.get(String(evt.check_id)) === String(evt.criterion_id)) {
          noteCheck(String(evt.criterion_id), String(evt.check_id));
        }
        break;
      case "check_finished": {
        const checkId = String(evt.check_id);
        const recordedCriterion =
          typeof evt.criterion_id === "string" ? evt.criterion_id : undefined;
        const critId = criterionOf.get(checkId) ?? recordedCriterion;
        if (critId) {
          if (!criterionOf.has(checkId)) {
            criterionOf.set(checkId, critId);
          }
          noteCheck(critId, checkId);
        }
        const crit = critId ? criteria.get(critId) : undefined;
        const check = crit?.checks.find((c) => c.id === checkId);
        if (check) {
          check.verdict = String(evt.verdict);
          if (typeof evt.gated_judging_steps === "number" && evt.gated_judging_steps > 0) {
            check.gated_judging_steps = evt.gated_judging_steps;
          } else {
            delete check.gated_judging_steps;
          }
        }
        break;
      }
      case "criterion_finished": {
        const id = String(evt.criterion_id);
        if (!criteria.has(id)) {
          const crit: CriterionDetail = { id, verdict: null, checks: [] };
          criteria.set(id, crit);
          detail.criteria.push(crit);
        }
        criteria.get(id)!.verdict = String(evt.verdict);
        break;
      }
      case "run_finished":
        detail.verdict =
          typeof evt.verdict === "string" ? String(evt.verdict) : null;
        detail.status = "finished";
        break;
      case "run_aborted":
        detail.status = "aborted";
        break;
    }
  }
  detail.lifecycle = foldLifecycle(events).filter((block) => block.scope.length <= 1);
  return detail;
}

function foldLifecycle(events: TraceEvent[]): LifecycleBlock[] {
  const blocks: (LifecycleBlock & {
    started_ms: number;
    finished: boolean;
    step_started: number[];
    step_finished: boolean[];
  })[] = [];
  const scope = (evt: TraceEvent) => [
    ["criterion", evt.criterion_id],
    ["check", evt.check_id],
    ["fixture", evt.fixture_name],
  ].filter((item): item is [string, string] => typeof item[1] === "string")
    .map(([kind, id]) => ({ kind, id }));
  const same = (a: LifecycleBlock["scope"], b: LifecycleBlock["scope"]) =>
    JSON.stringify(a) === JSON.stringify(b);
  const active = (evt: TraceEvent) => [...blocks].reverse().find((block) =>
    !block.finished && block.phase === (evt.phase === "teardown" ? "teardown" : "setup") &&
    same(block.scope, scope(evt)));

  for (const evt of events) {
    if (evt.kind === "setup_started") {
      blocks.push({
        phase: evt.phase === "teardown" ? "teardown" : "setup",
        scope: scope(evt), status: "aborted", started_at: evt.ts,
        duration_ms: 0, steps: [], timeline: [evt],
        started_ms: Date.parse(evt.ts), finished: false, step_started: [], step_finished: [],
      });
      continue;
    }
    if (!evt.kind.startsWith("setup_step_") && evt.kind !== "setup_finished") continue;
    const block = active(evt);
    if (!block) continue;
    block.timeline.push(evt);
    if (evt.kind === "setup_step_started") {
      block.steps.push({
        index: Number(evt.step_index), uses: String(evt.uses),
        flow: typeof evt.flow === "object" ? evt.flow as LifecycleBlock["steps"][number]["flow"] : undefined,
        outcome: "ok", duration_ms: 0,
      });
      block.step_started.push(Date.parse(evt.ts));
      block.step_finished.push(false);
    } else if (evt.kind === "setup_step_finished") {
      const position = [...block.steps].map((step, index) => ({ step, index })).reverse()
        .find(({ step, index }) => step.index === Number(evt.step_index) && !block.step_finished[index])?.index;
      if (position !== undefined) {
        const step = block.steps[position];
        step.outcome = evt.outcome as typeof step.outcome;
        step.detail = typeof evt.detail === "string" ? evt.detail : undefined;
        step.duration_ms = Math.max(0, Date.parse(evt.ts) - block.step_started[position]);
        block.step_finished[position] = true;
        if ((evt.outcome === "error" || evt.outcome === "timeout") && block.failing_step === undefined) {
          block.failing_step = position;
        }
      }
    } else if (evt.kind === "setup_finished") {
      block.duration_ms = Math.max(0, Date.parse(evt.ts) - block.started_ms);
      block.status = Boolean(evt.aborted) && block.phase === "setup" ? "aborted"
        : block.failing_step !== undefined ? "failed"
        : Boolean(evt.aborted) ? "aborted" : "passed";
      block.finished = true;
    }
  }
  return blocks.map(({
    started_ms: _started,
    finished: _finished,
    step_started: _steps,
    step_finished: _stepFinished,
    ...block
  }) => block);
}
