// The stream fold must lift verdicts verbatim from the judge's events
// and preserve trace order — it presents, it never judges.

import { describe, expect, it } from "vitest";
import { carryFetched, foldRun } from "../fold";
import type { RunDetail, TraceEvent } from "../api";

const trace: TraceEvent[] = [
  {
    seq: 0,
    ts: "2026-06-10T10:00:00.000Z",
    kind: "run_started",
    verification_path: "verifications/login.yml",
    inputs: { user: "u1" },
  },
  { seq: 1, ts: "2026-06-10T10:00:01.000Z", kind: "step_started", criterion_id: "AC-1", check_id: "AC-1.1", step_index: 0, uses: "ui/navigate" },
  { seq: 2, ts: "2026-06-10T10:00:02.000Z", kind: "step_finished", step_index: 0, outcome: "ok" },
  { seq: 3, ts: "2026-06-10T10:00:03.000Z", kind: "assertion_evaluated", check_id: "AC-1.1", assertion_index: 0, state: "pass" },
  { seq: 4, ts: "2026-06-10T10:00:04.000Z", kind: "check_finished", check_id: "AC-1.1", verdict: "pass" },
  { seq: 5, ts: "2026-06-10T10:00:05.000Z", kind: "criterion_finished", criterion_id: "AC-1", verdict: "pass" },
  { seq: 6, ts: "2026-06-10T10:00:06.000Z", kind: "run_finished", verdict: "pass" },
];

describe("foldRun", () => {
  it("is running with no verdict until run_finished arrives", () => {
    const partial = foldRun("r1", trace.slice(0, 4));
    expect(partial.status).toBe("running");
    expect(partial.verdict).toBeNull();
    expect(partial.criteria).toHaveLength(1);
    expect(partial.criteria[0].checks[0]).toEqual({ id: "AC-1.1", verdict: null });
  });

  it("finalizes with the judge's verdicts, in trace order", () => {
    const done = foldRun("r1", trace);
    expect(done.status).toBe("finished");
    expect(done.verdict).toBe("pass");
    expect(done.inputs).toEqual({ user: "u1" });
    expect(done.criteria[0].verdict).toBe("pass");
    expect(done.criteria[0].checks[0].verdict).toBe("pass");
  });

  it("finishes without inventing a verdict", () => {
    const done = foldRun("suite", [
      { seq: 0, ts: "2026-06-10T10:00:00.000Z", kind: "run_finished" },
    ]);
    expect(done.status).toBe("finished");
    expect(done.verdict).toBeNull();
  });

  it("flags an aborted setup", () => {
    const aborted = foldRun("r1", [
      trace[0],
      { seq: 1, ts: "2026-06-10T10:00:01.000Z", kind: "setup_finished", aborted: true },
      { seq: 2, ts: "2026-06-10T10:00:02.000Z", kind: "run_finished", verdict: "inconclusive:environment_error" },
    ]);
    expect(aborted.setup_aborted).toBe(true);
    expect(aborted.verdict).toBe("inconclusive:environment_error");
  });

  it("folds teardown steps without treating teardown as setup", () => {
    const done = foldRun("r1", [
      trace[0],
      { seq: 1, ts: "2026-06-10T10:00:01.000Z", kind: "setup_finished", aborted: false },
      { seq: 2, ts: "2026-06-10T10:00:02.000Z", kind: "setup_step_started", phase: "teardown", step_index: 0, uses: "api/call" },
      { seq: 3, ts: "2026-06-10T10:00:03.000Z", kind: "setup_step_finished", phase: "teardown", step_index: 0, outcome: "error" },
      { seq: 4, ts: "2026-06-10T10:00:04.000Z", kind: "setup_finished", phase: "teardown", aborted: true },
      { seq: 5, ts: "2026-06-10T10:00:05.000Z", kind: "run_finished", verdict: "pass" },
    ]);
    expect(done.setup_aborted).toBe(false);
    expect(done.cleanup).toEqual([
      { step_index: 0, uses: "api/call", outcome: "error" },
    ]);
    expect(done.verdict).toBe("pass");
  });

  it("keeps the first step owner over a disagreeing finished-event owner (#490)", () => {
    const done = foldRun("r1", [
      {
        seq: 0,
        ts: "2026-06-10T10:00:00.000Z",
        kind: "step_started",
        criterion_id: "AC-1",
        check_id: "AC-1.1",
      },
      {
        seq: 1,
        ts: "2026-06-10T10:00:01.000Z",
        kind: "check_finished",
        criterion_id: "AC-2",
        check_id: "AC-1.1",
        verdict: "fail",
      },
    ]);

    expect(done.criteria).toEqual([
      { id: "AC-1", verdict: null, checks: [{ id: "AC-1.1", verdict: "fail" }] },
    ]);
  });
});

describe("carryFetched", () => {
  // `has_definition` gates the whole description overlay. A live fold
  // cannot derive it, so resetting it to the fold default on every SSE
  // event is what made a running run show bare ids until it finished
  // (#491).
  const fetched: RunDetail = {
    run_id: "r1",
    verification: "verifications/login.yml",
    started_at: "2026-06-10T10:00:00.000Z",
    inputs: { user: "u1" },
    verdict: null,
    status: "running",
    setup_aborted: false,
    has_definition: true,
    viewport: { width: 1280, height: 720 },
    cleanup: [],
    criteria: [],
  };

  it("keeps the fetched definition flag across a live fold", () => {
    const live = foldRun("r1", trace.slice(0, 2));
    expect(live.has_definition).toBe(false);
    expect(carryFetched(live, fetched).has_definition).toBe(true);
  });

  it("keeps the fetched identity rather than the run-id fallback", () => {
    const merged = carryFetched(foldRun("r1", []), fetched);
    expect(merged.verification).toBe("verifications/login.yml");
    expect(merged.started_at).toBe("2026-06-10T10:00:00.000Z");
    expect(merged.inputs).toEqual({ user: "u1" });
    expect(merged.viewport).toEqual({ width: 1280, height: 720 });
  });

  it("leaves execution progress to the fold", () => {
    const merged = carryFetched(foldRun("r1", trace), fetched);
    // The fold owns these even though `fetched` still says "running".
    expect(merged.status).toBe("finished");
    expect(merged.verdict).toBe("pass");
    expect(merged.criteria[0].checks[0].verdict).toBe("pass");
  });

  it("never enables the overlay for a run without a snapshot", () => {
    const merged = carryFetched(foldRun("r1", trace.slice(0, 2)), {
      ...fetched,
      has_definition: false,
    });
    expect(merged.has_definition).toBe(false);
  });
});
