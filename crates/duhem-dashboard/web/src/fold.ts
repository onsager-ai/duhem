// Pure fold from a stream of trace events (the #84 SSE payloads —
// raw `trace.jsonl` lines) to the same shape `GET /api/runs/:id`
// serves. The browser never computes a verdict: every verdict below
// is lifted verbatim from the judge's `*_finished` events.

import type { CriterionDetail, RunDetail, TraceEvent } from "./api";

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
        if (evt.phase !== "teardown") {
          detail.setup_aborted = Boolean(evt.aborted);
        }
        break;
      case "setup_step_started":
        if (evt.phase === "teardown") {
          detail.cleanup!.push({
            step_index: Number(evt.step_index),
            uses: String(evt.uses),
            outcome: "ok",
          });
        }
        break;
      case "setup_step_finished":
        if (evt.phase === "teardown") {
          const step = [...detail.cleanup!]
            .reverse()
            .find((candidate) => candidate.step_index === Number(evt.step_index));
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
  return detail;
}
