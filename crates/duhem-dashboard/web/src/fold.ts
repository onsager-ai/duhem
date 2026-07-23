// Pure fold from a stream of trace events (the #84 SSE payloads —
// raw `trace.jsonl` lines) to the same shape `GET /api/runs/:id`
// serves. The browser never computes a verdict: every verdict below
// is lifted verbatim from the judge's `*_finished` events.

import type { CriterionDetail, RunDetail, TraceEvent } from "./api";

export function foldRun(runId: string, events: TraceEvent[]): RunDetail {
  const detail: RunDetail = {
    run_id: runId,
    verification: runId,
    started_at: null,
    inputs: {},
    verdict: null,
    live: true,
    setup_aborted: false,
    // A live fold doesn't surface the definition; the authoritative
    // re-fetch on `run_finished` fills this in (#302).
    has_definition: false,
    criteria: [],
  };
  const criteria = new Map<string, CriterionDetail>();
  const criterionOf = new Map<string, string>();

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
    criterionOf.set(checkId, criterionId);
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
        detail.setup_aborted = Boolean(evt.aborted);
        break;
      case "step_started":
        noteCheck(String(evt.criterion_id), String(evt.check_id));
        break;
      case "check_finished": {
        const critId = criterionOf.get(String(evt.check_id));
        const crit = critId ? criteria.get(critId) : undefined;
        const check = crit?.checks.find((c) => c.id === evt.check_id);
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
        detail.verdict = String(evt.verdict);
        detail.live = false;
        break;
    }
  }
  return detail;
}
