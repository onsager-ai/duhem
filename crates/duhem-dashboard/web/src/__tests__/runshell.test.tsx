// Run-scoped report shell (#280): tab routing, the Suites status
// filter, Categories cause-grouping, and the per-check Timeline spans.

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import RunPage, { activeTab } from "../views/RunPage";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

// pegasus-register-shaped run: 4 checks — 2 pass, 1 fail, 1 inconclusive.
const RUN = {
  run_id: "R1",
  verification: "pegasus-register",
  started_at: "2026-07-22T14:03:00.000Z",
  inputs: {},
  verdict: "fail",
  live: false,
  setup_aborted: false,
  criteria: [
    {
      id: "AC-5",
      verdict: "fail",
      checks: [
        { id: "AC-5.1", verdict: "fail" },
        { id: "AC-5.2", verdict: "pass" },
      ],
    },
    {
      id: "AC-7",
      verdict: "inconclusive:environment_error",
      checks: [{ id: "AC-7.1", verdict: "inconclusive:environment_error" }],
    },
    { id: "AC-6", verdict: "pass", checks: [{ id: "AC-6.1", verdict: "pass" }] },
  ],
};

function stub() {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string) => {
      if (String(url).includes("/checks/")) {
        return new Response(
          JSON.stringify({
            criterion_id: "AC-5",
            check_id: "AC-5.1",
            verdict: "fail",
            spans: [{ seq: 1, layer: "ui", ok: false, detail: "text present" }],
            timeline: [
              {
                seq: 1,
                ts: "t",
                kind: "assertion_evaluated",
                state: "fail",
                detail: 'expected text "Manager" to be absent within 5s, but 1 still matched',
                step_index: 0,
              },
            ],
            artifacts: [],
          }),
          { status: 200 },
        );
      }
      return new Response(JSON.stringify(RUN), { status: 200 });
    }),
  );
}

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path="/run/:runId" element={<RunPage />} />
        <Route path="/run/:runId/suites" element={<RunPage />} />
        <Route path="/run/:runId/categories" element={<RunPage />} />
        <Route path="/run/:runId/timeline" element={<RunPage />} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("activeTab", () => {
  it("reads the tab from the trailing path segment, else Overview", () => {
    expect(activeTab("/run/R1")).toBe("overview");
    expect(activeTab("/run/R1/suites")).toBe("suites");
    expect(activeTab("/run/R1/categories")).toBe("categories");
    expect(activeTab("/run/R1/timeline")).toBe("timeline");
    // Non-tab sub-routes (diff, check) are Overview as far as the tabs go.
    expect(activeTab("/run/R1/diff")).toBe("overview");
  });
});

describe("run report shell", () => {
  it("Overview shows the donut, and the active tab is marked", async () => {
    stub();
    renderAt("/run/R1/suites");
    await waitFor(() => expect(screen.getByTestId("run-tabs")).toBeTruthy());
    const on = screen.getByTestId("run-tabs").querySelector(".run-tab.on");
    expect(on?.textContent).toBe("Suites");
  });

  it("Overview donut totals the run's checks", async () => {
    stub();
    renderAt("/run/R1");
    await waitFor(() => expect(screen.getByTestId("status-summary")).toBeTruthy());
    expect(screen.getByTestId("status-summary").querySelector(".donut-center")?.textContent).toBe(
      "4",
    );
    expect(screen.getByTestId("count-pass").textContent).toContain("2");
    expect(screen.getByTestId("count-fail").textContent).toContain("1");
  });

  it("Suites filter chips narrow the tree to a verdict family", async () => {
    stub();
    renderAt("/run/R1/suites");
    await waitFor(() => expect(screen.getByTestId("suite-filter")).toBeTruthy());
    // All four checks visible initially — a passing one is present.
    expect(screen.getByText("AC-5.2")).toBeTruthy();
    // Click "✗ fail" → only failing checks remain.
    fireEvent.click(screen.getByTestId("chip-fail"));
    expect(screen.queryByText("AC-5.2")).toBeNull();
    expect(screen.queryByText("AC-6.1")).toBeNull();
    expect(screen.getByText("AC-5.1")).toBeTruthy();
  });

  it("Categories groups non-passing checks by mechanical cause, with lazy detail", async () => {
    stub();
    renderAt("/run/R1/categories");
    await waitFor(() => expect(screen.getByTestId("cat-fail")).toBeTruthy());
    const failGroup = screen.getByTestId("cat-fail");
    expect(failGroup.textContent).toContain("the artifact is wrong");
    expect(failGroup.textContent).toContain("(1)");
    // The inconclusive cause is its own bucket, not lumped with fail.
    expect(screen.getByTestId("cat-inconclusive:environment_error")).toBeTruthy();
    // The lazily-fetched semantic reason lands on the fail item.
    await waitFor(() => expect(failGroup.textContent).toContain('text "Manager"'));
  });

  it("Timeline lists a delivery-web chain per check", async () => {
    stub();
    renderAt("/run/R1/timeline");
    await waitFor(() => expect(screen.getAllByTestId("span-row").length).toBe(4));
    await waitFor(() =>
      expect(screen.getAllByTestId("spanchain").length).toBeGreaterThan(0),
    );
  });
});
