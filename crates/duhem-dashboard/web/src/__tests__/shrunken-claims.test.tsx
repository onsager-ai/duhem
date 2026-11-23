import { cleanup, render } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, expect, it } from "vitest";
import type { CheckDetail, RunDetail, TraceEvent } from "../api";
import { foldRun } from "../fold";
import { CheckSummary, Timeline } from "../views/CheckPage";
import { RunSummary } from "../views/RunPage";

afterEach(cleanup);

const events: TraceEvent[] = [
  { seq: 0, ts: "2026-09-10T00:00:00Z", kind: "step_started", criterion_id: "AC-1", check_id: "C", step_index: 0, uses: "db/observe" },
  { seq: 1, ts: "2026-09-10T00:00:01Z", kind: "step_finished", step_index: 0, outcome: { skipped: {
    reason: "condition evaluated false", condition: "$steps.count_rows.outputs.count > 0",
    operands: { "$steps.count_rows.outputs.count": 0 },
  } } },
  { seq: 2, ts: "2026-09-10T00:00:01Z", kind: "check_finished", check_id: "C", criterion_id: "AC-1", verdict: "pass", gated_judging_steps: 1 },
  { seq: 3, ts: "2026-09-10T00:00:01Z", kind: "criterion_finished", criterion_id: "AC-1", verdict: "pass" },
  { seq: 4, ts: "2026-09-10T00:00:01Z", kind: "run_finished", verdict: "pass" },
];
const check: CheckDetail = { criterion_id: "AC-1", check_id: "C", verdict: "pass", spans: [], timeline: [], artifacts: [] };

it("the run report and check summary render shrunken and whole checks differently", () => {
  const gated = foldRun("r", events);
  const whole: RunDetail = { ...gated, criteria: [{ id: "AC-1", verdict: "pass", checks: [{ id: "C", verdict: "pass" }] }] };
  const report = render(<MemoryRouter><RunSummary run={whole} /></MemoryRouter>);
  const allRan = report.container.innerHTML;
  report.rerender(<MemoryRouter><RunSummary run={gated} /></MemoryRouter>);
  expect(report.container.innerHTML).not.toBe(allRan);
  expect(report.container.textContent).toContain("AC-1::C: 1 judging step gated");
  report.unmount();
  const summary = render(<CheckSummary detail={check} />);
  const wholeSummary = summary.container.innerHTML;
  summary.rerender(<CheckSummary detail={{ ...check, gated_judging_steps: 1 }} />);
  expect(summary.container.innerHTML).not.toBe(wholeSummary);
  expect(summary.container.textContent).toContain("1 judging step gated");
});

it("zero and absent counts add no rendered output on either surface", () => {
  const run = foldRun("r", events.map((event) => {
    const { gated_judging_steps: _count, ...old } = event;
    return old;
  }));
  expect(run.criteria[0].checks[0]).not.toHaveProperty("gated_judging_steps");
  const report = render(<MemoryRouter><RunSummary run={run} /></MemoryRouter>);
  const before = report.container.innerHTML;
  run.criteria[0].checks[0].gated_judging_steps = 0;
  report.rerender(<MemoryRouter><RunSummary run={run} /></MemoryRouter>);
  expect(report.container.innerHTML).toBe(before);
  expect(report.container.textContent).not.toContain("gated");
  report.unmount();
  const summary = render(<CheckSummary detail={check} />);
  const previous = summary.container.innerHTML;
  summary.rerender(<CheckSummary detail={{ ...check, gated_judging_steps: 0 }} />);
  expect(summary.container.innerHTML).toBe(previous);
  expect(summary.container.textContent).not.toContain("gated");
});

it("renders the evaluated condition and operands in the step timeline", () => {
  const timeline = render(<MemoryRouter><Timeline events={events} artifacts={[]} /></MemoryRouter>);
  expect(timeline.container.textContent).toContain("$steps.count_rows.outputs.count > 0");
  expect(timeline.container.textContent).toContain("$steps.count_rows.outputs.count = 0");
});

it("shows present and missing scores separately from the check verdicts", () => {
  const run = foldRun("r", events);
  const report = render(<MemoryRouter><RunSummary run={run} /></MemoryRouter>);
  expect(report.queryByRole("heading", { name: "Scores (non-verdict)" })).toBeNull();
  const verdictSummary = report.getByRole("region", { name: "Checks" }).textContent;
  const scored: RunDetail = { ...run, scores: {
    "AC-1": { C: { quality: { value: 0 }, clarity: { missing_cause: "timed out" } } },
  } };
  report.rerender(<MemoryRouter><RunSummary run={scored} /></MemoryRouter>);
  const scores = report.getByRole("region", { name: "Scores (non-verdict)" });
  expect(scores.textContent).toContain("quality0");
  expect(scores.textContent).toContain("clarityMissing (timed out)");
  expect(report.getByRole("region", { name: "Checks" }).textContent).toBe(verdictSummary);
});
