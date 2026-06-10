// The live fold must lift verdicts verbatim from the judge's events
// and preserve trace order — it presents, it never judges.

import { describe, expect, it } from "vitest";
import { foldRun } from "../fold";
import type { TraceEvent } from "../api";

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
  it("is live with no verdict until run_finished arrives", () => {
    const partial = foldRun("r1", trace.slice(0, 4));
    expect(partial.live).toBe(true);
    expect(partial.verdict).toBeNull();
    expect(partial.criteria).toHaveLength(1);
    expect(partial.criteria[0].checks[0]).toEqual({ id: "AC-1.1", verdict: null });
  });

  it("finalizes with the judge's verdicts, in trace order", () => {
    const done = foldRun("r1", trace);
    expect(done.live).toBe(false);
    expect(done.verdict).toBe("pass");
    expect(done.inputs).toEqual({ user: "u1" });
    expect(done.criteria[0].verdict).toBe("pass");
    expect(done.criteria[0].checks[0].verdict).toBe("pass");
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
});
