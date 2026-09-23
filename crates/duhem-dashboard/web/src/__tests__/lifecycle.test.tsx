import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { LifecycleBlock, TraceEvent } from "../api";
import { LifecycleSections, Timeline } from "../views/CheckPage";
import { LifecycleList } from "../views/RunPage";

afterEach(cleanup);

function block(
  phase: LifecycleBlock["phase"],
  scope: LifecycleBlock["scope"],
  seq: number,
): LifecycleBlock {
  const timeline: TraceEvent[] = [
    { seq, ts: "2026-01-01T00:00:00.000Z", kind: "setup_started", phase },
    { seq: seq + 1, ts: "2026-01-01T00:00:00.001Z", kind: "setup_step_started", phase, step_index: 0, uses: "cli/invoke" },
    { seq: seq + 2, ts: "2026-01-01T00:00:00.002Z", kind: "setup_step_finished", phase, step_index: 0, outcome: "ok" },
    { seq: seq + 3, ts: "2026-01-01T00:00:00.003Z", kind: "setup_finished", phase, aborted: false },
  ];
  return {
    phase,
    scope,
    status: "passed",
    started_at: timeline[0].ts,
    duration_ms: 3,
    steps: [{ index: 0, uses: "cli/invoke", outcome: "ok", duration_ms: 1 }],
    timeline,
  };
}

describe("lifecycle presentation", () => {
  it("renders an unknown three-segment scope without naming known kinds", () => {
    const future = block("setup", [
      { kind: "alpha", id: "1" },
      { kind: "beta", id: "2" },
      { kind: "gamma", id: "3" },
    ], 1);
    render(<LifecycleList blocks={[future]} />);
    expect(screen.getByText("alpha:1 / beta:2 / gamma:3")).toBeTruthy();
  });

  it("shows leaf and criterion blocks on the run page", () => {
    render(<LifecycleList blocks={[
      block("setup", [], 1),
      block("teardown", [{ kind: "criterion", id: "AC-1" }], 10),
    ]} />);
    expect(screen.getByText("leaf")).toBeTruthy();
    expect(screen.getByText("criterion:AC-1")).toBeTruthy();
  });

  it("keeps check setup before navigable steps and teardown after", () => {
    const steps: TraceEvent[] = [
      { seq: 20, ts: "2026-01-01T00:00:01.000Z", kind: "step_started", step_index: 0, uses: "db/query" },
      { seq: 21, ts: "2026-01-01T00:00:01.001Z", kind: "step_finished", step_index: 0, outcome: "ok" },
    ];
    const { container } = render(<>
      <LifecycleSections blocks={[block("setup", [{ kind: "check", id: "AC-1.1" }], 1)]} />
      <div data-testid="check-steps"><Timeline events={steps} /></div>
      <LifecycleSections blocks={[block("teardown", [{ kind: "check", id: "AC-1.1" }], 30)]} />
    </>);
    const setup = screen.getByTestId("check-lifecycle-setup");
    const check = screen.getByTestId("check-steps");
    const teardown = screen.getByTestId("check-lifecycle-teardown");
    expect(setup.compareDocumentPosition(check) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(check.compareDocumentPosition(teardown) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(container.textContent).toContain("cli/invoke");
  });
});
