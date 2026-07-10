// DiffPage (#212): renders the run-to-run regression from the diff API.

import { render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import DiffPage from "../views/DiffPage";

afterEach(() => vi.unstubAllGlobals());

function renderDiff(runId = "R01") {
  return render(
    <MemoryRouter initialEntries={[`/run/${runId}/diff`]}>
      <Routes>
        <Route path="/run/:runId/diff" element={<DiffPage />} />
      </Routes>
    </MemoryRouter>,
  );
}

function stub(diff: unknown) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response(JSON.stringify(diff), { status: 200 })),
  );
}

describe("DiffPage (#212)", () => {
  it("renders the regression transitions and the flipped assertion", async () => {
    stub({
      current: { run_id: "R01", started_at: null, verdict: "fail" },
      baseline: { run_id: "R00", started_at: null, verdict: "pass" },
      criteria: [
        {
          id: "AC-1",
          baseline_verdict: "pass",
          current_verdict: "fail",
          changed: true,
          checks: [
            {
              id: "AC-1.1",
              baseline_verdict: "pass",
              current_verdict: "fail",
              changed: true,
              assertions: [
                {
                  assertion_index: 0,
                  baseline_state: "pass",
                  current_state: "fail",
                  current_detail: "actual false, expected true",
                  changed: true,
                },
              ],
              baseline_artifacts: [],
              current_artifacts: [],
            },
          ],
        },
      ],
    });
    renderDiff();
    expect(await screen.findByText("Regression diff")).toBeTruthy();
    expect(screen.getByText("AC-1.1")).toBeTruthy();
    expect(screen.getByText(/actual false, expected true/)).toBeTruthy();
    // The transition renders both recorded verdicts as badges.
    expect(screen.getAllByText("fail").length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText("pass").length).toBeGreaterThanOrEqual(1);
  });

  it("shows baseline → current when the assertion detail itself changed", async () => {
    stub({
      current: { run_id: "R01", started_at: null, verdict: "fail" },
      baseline: { run_id: "R00", started_at: null, verdict: "fail" },
      criteria: [
        {
          id: "AC-1",
          baseline_verdict: "fail",
          current_verdict: "fail",
          changed: false,
          checks: [
            {
              id: "AC-1.1",
              baseline_verdict: "fail",
              current_verdict: "fail",
              changed: true,
              assertions: [
                {
                  assertion_index: 0,
                  baseline_state: "fail",
                  current_state: "fail",
                  baseline_detail: "actual 200, expected 201",
                  current_detail: "actual 500, expected 201",
                  changed: true,
                },
              ],
              baseline_artifacts: [],
              current_artifacts: [],
            },
          ],
        },
      ],
    });
    renderDiff();
    expect(await screen.findByText(/actual 500, expected 201/)).toBeTruthy();
    // The prior detail is shown (struck through) alongside the new one.
    expect(screen.getByText("actual 200, expected 201")).toBeTruthy();
  });

  it("shows the honest empty state when there is no baseline", async () => {
    stub({
      current: { run_id: "R01", started_at: null, verdict: "fail" },
      baseline: null,
      criteria: [
        { id: "AC-1", baseline_verdict: null, current_verdict: "fail", changed: false, checks: [] },
      ],
    });
    renderDiff();
    expect(await screen.findByTestId("diff-no-baseline")).toBeTruthy();
  });
});
