import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CheckDetail, LifecycleBlock, RunDetail } from "../api";
import { lifecycleEnclosesCheck, lifecycleKey } from "../lifecycle";
import { RunTree } from "../views/RunScaffold";
import { LifecycleEvidence } from "../views/LifecyclePage";
import CheckPage from "../views/CheckPage";
import { LifecycleList } from "../views/RunPage";

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

function block(seq: number, phase: LifecycleBlock["phase"], scope: LifecycleBlock["scope"], failed = false): LifecycleBlock {
  return {
    phase, scope, status: failed ? "failed" : "passed",
    started_at: "2026-01-01T00:00:00.000Z", duration_ms: 13,
    steps: [{ index: 0, uses: "db/query", outcome: failed ? "error" : "ok", detail: failed ? "this is deliberately invalid SQL" : undefined, duration_ms: 12 }],
    failing_step: failed ? 0 : undefined,
    timeline: [
      { seq, ts: "2026-01-01T00:00:00.000Z", kind: "setup_started", phase },
      { seq: seq + 1, ts: "2026-01-01T00:00:00.001Z", kind: "setup_step_started", phase, step_index: 0, uses: "db/query" },
      { seq: seq + 2, ts: "2026-01-01T00:00:00.013Z", kind: "setup_step_finished", phase, step_index: 0, outcome: failed ? "error" : "ok", detail: failed ? "this is deliberately invalid SQL" : undefined },
    ],
  };
}

const blocks = [
  block(1, "setup", []),
  block(10, "setup", [{ kind: "criterion", id: "AC-1" }]),
  block(20, "setup", [{ kind: "criterion", id: "AC-1" }, { kind: "check", id: "AC-1.1" }]),
  block(30, "setup", [{ kind: "check", id: "AC-1.1" }, { kind: "fixture", id: "db" }]),
  block(40, "teardown", [{ kind: "check", id: "AC-1.1" }, { kind: "fixture", id: "db" }]),
  block(50, "teardown", [{ kind: "criterion", id: "AC-1" }]),
  block(60, "teardown", [], true),
];

const run: RunDetail = {
  run_id: "run-1", verification: "example", started_at: null, inputs: {}, verdict: "pass",
  status: "finished", setup_aborted: false, has_definition: false,
  lifecycle: blocks,
  criteria: [{ id: "AC-1", verdict: "pass", checks: [{ id: "AC-1.1", verdict: "pass" }] }],
};

function Location() {
  const location = useLocation();
  return <output data-testid="location">{location.pathname}{location.search}</output>;
}

describe("lifecycle navigation", () => {
  it("lists every scope at its level with status and opens check blocks by URL", () => {
    render(<MemoryRouter><RunTree run={run} /><Location /></MemoryRouter>);
    const rows = screen.getAllByTestId("lifecycle-rail-row");
    expect(rows).toHaveLength(7);
    expect(rows[0].getAttribute("aria-label")).toBe("leaf setup passed");
    expect(rows[1].getAttribute("aria-label")).toBe("criterion:AC-1 setup passed");
    expect(rows[2].getAttribute("aria-label")).toBe("criterion:AC-1 / check:AC-1.1 setup passed");
    expect(rows[3].getAttribute("aria-label")).toContain("fixture:db setup passed");
    expect(rows[6].getAttribute("data-status")).toBe("failed");
    fireEvent.click(rows[2]);
    expect(screen.getByTestId("location").textContent).toBe(`/run/run-1/check/AC-1%3A%3AAC-1.1?lifecycle=${lifecycleKey(blocks[2])}`);
    fireEvent.click(rows[6]);
    expect(screen.getByTestId("location").textContent).toBe("/run/run-1/lifecycle/60");
  });

  it("shows the failing step and recorded error in the lifecycle view", () => {
    render(<LifecycleEvidence block={blocks[6]} />);
    expect(screen.getByTestId("lifecycle-failure").textContent).toContain("this is deliberately invalid SQL");
    expect(screen.getByTestId("lifecycle-detail").textContent).toContain("13ms");
  });

  it("links summary rows to lifecycle detail and shows failure detail inline", () => {
    render(<MemoryRouter><LifecycleList runId="run-1" blocks={[blocks[6]]} /><Location /></MemoryRouter>);
    expect(screen.getByTestId("run-lifecycle").textContent).toContain("this is deliberately invalid SQL");
    fireEvent.click(screen.getByRole("link", { name: "leaf teardown" }));
    expect(screen.getByTestId("location").textContent).toBe("/run/run-1/lifecycle/60");
  });

  it("matches kind and id throughout the enclosing chain", () => {
    expect(blocks.filter((item) => lifecycleEnclosesCheck(item.scope, "AC-1", "AC-1.1"))).toHaveLength(7);
    expect(lifecycleEnclosesCheck([{ kind: "check", id: "other" }, { kind: "fixture", id: "AC-1.1" }], "AC-1", "AC-1.1")).toBe(false);
    expect(lifecycleEnclosesCheck([{ kind: "criterion", id: "AC-1.1" }], "AC-1", "AC-1.1")).toBe(false);
  });

  it("renders the enclosing setup chain before check steps and scrolls to a selected block", async () => {
    const scroll = vi.fn();
    vi.stubGlobal("fetch", vi.fn(async (url: string) => new Response(JSON.stringify(
      url.includes("/checks/")
        ? { criterion_id: "AC-1", check_id: "AC-1.1", verdict: "pass", spans: [], timeline: [], artifacts: [], lifecycle: blocks } satisfies CheckDetail
        : run,
    ), { status: 200 })));
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", { value: scroll, configurable: true });
    const { container } = render(
      <MemoryRouter initialEntries={[`/run/run-1/check/AC-1%3A%3AAC-1.1?lifecycle=${lifecycleKey(blocks[2])}`]}>
        <Routes><Route path="/run/:runId/check/:pair" element={<CheckPage />} /></Routes>
      </MemoryRouter>,
    );
    await waitFor(() => expect(screen.getAllByTestId("check-lifecycle-setup")).toHaveLength(4));
    const setups = screen.getAllByTestId("check-lifecycle-setup");
    expect(setups.map((item) => item.querySelector("h3")?.textContent)).toEqual([
      "leaf setuppassed",
      "criterion:AC-1 setuppassed",
      "criterion:AC-1 / check:AC-1.1 setuppassed",
      "check:AC-1.1 / fixture:db setuppassed",
    ]);
    expect(setups[3].compareDocumentPosition(container.querySelector(".run-detail-surface")!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    await waitFor(() => expect(scroll).toHaveBeenCalledWith({ block: "start" }));
  });
});
