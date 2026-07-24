// Run report shell (#284 tree redesign): the run header verdict, the
// criteria → check tree rail (each check a link named by its check id),
// the summary roll-up tiles, and the active-check highlight when a check
// is open. Replaces the former per-run tab-bar tests.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import RunPage from "../views/RunPage";
import CheckPage from "../views/CheckPage";
import CriterionPage from "../views/CriterionPage";
import DefinitionPage from "../views/DefinitionPage";
import ResultsPage from "../views/ResultsPage";

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
  has_definition: true,
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

function stub(run = RUN) {
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
      return new Response(JSON.stringify(run), { status: 200 });
    }),
  );
}

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <Routes>
        <Route path="/run/:runId" element={<RunPage />} />
        <Route path="/run/:runId/results" element={<ResultsPage />} />
        <Route path="/run/:runId/criterion/:criterionId" element={<CriterionPage />} />
        <Route path="/run/:runId/check/:pair" element={<CheckPage />} />
        <Route path="/run/:runId/definition" element={<DefinitionPage />} />
      </Routes>
    </MemoryRouter>,
  );
}

describe("run report tree", () => {
  it("renders the run verdict inside a heading", async () => {
    stub();
    renderAt("/run/R1");
    // The run header is a heading carrying the verification, run id, and
    // the verdict badge text — the self-verification VD asserts the
    // verdict is rendered inside a heading.
    await waitFor(() =>
      expect(
        screen.getByRole("heading", { name: /pegasus-register.*fail/s }),
      ).toBeTruthy(),
    );
  });

  it("keeps the criteria tree in Results and names each check link by its id", async () => {
    stub();
    renderAt("/run/R1/results");
    const tree = await screen.findByTestId("run-tree");
    // Every check link is present without interaction, and its accessible
    // name is exactly the check id (no verdict text bleeding in).
    expect(within(tree).getByRole("link", { name: "AC-5.1" })).toBeTruthy();
    expect(within(tree).getByRole("link", { name: "AC-5.2" })).toBeTruthy();
    expect(within(tree).getByRole("link", { name: "AC-7.1" })).toBeTruthy();
    expect(within(tree).getByRole("link", { name: "AC-6.1" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Results" }).getAttribute("aria-selected")).toBe(
      "true",
    );
    expect(
      screen.getAllByTestId("check-children").every((group) =>
        group.className.includes("ml-8"),
      ),
    ).toBe(true);
  });

  it("gives Summary and Definition full width without the Results tree", async () => {
    stub();
    const summary = renderAt("/run/R1");
    await screen.findByTestId("run-summary");
    expect(screen.queryByTestId("run-tree")).toBeNull();
    expect(screen.getByRole("tab", { name: "Summary" }).getAttribute("aria-selected")).toBe(
      "true",
    );
    summary.unmount();

    renderAt("/run/R1/definition");
    await screen.findByTestId("vd-yaml");
    expect(screen.queryByTestId("run-tree")).toBeNull();
    expect(
      screen.getByRole("tab", { name: "Definition" }).getAttribute("aria-selected"),
    ).toBe("true");
  });

  it("makes an inconclusive criterion with no checks navigable", async () => {
    stub({
      ...RUN,
      criteria: [
        ...RUN.criteria,
        {
          id: "AC-8",
          verdict: "inconclusive:empty_aggregation",
          checks: [],
        },
      ],
    });
    renderAt("/run/R1/results");
    const tree = await screen.findByTestId("run-tree");
    const criterion = within(tree).getByRole("link", { name: /AC-8/ });
    fireEvent.click(criterion);
    const detail = await screen.findByTestId("criterion-detail");
    expect(detail.textContent).toContain("No checks were recorded");
    expect(detail.textContent).toContain("empty aggregation");
  });

  it("totals the run's checks by verdict family in the summary tiles", async () => {
    stub();
    renderAt("/run/R1");
    await waitFor(() => expect(screen.getByTestId("run-summary")).toBeTruthy());
    expect(screen.getByTestId("checks-pass").textContent).toContain("2");
    expect(screen.getByTestId("checks-fail").textContent).toContain("1");
    expect(screen.getByTestId("checks-inconclusive").textContent).toContain("1");
  });

  it("labels criterion counts separately when a criterion has no checks", async () => {
    stub({
      ...RUN,
      criteria: [
        ...RUN.criteria,
        {
          id: "AC-8",
          verdict: "inconclusive:empty_aggregation",
          checks: [],
        },
      ],
    });
    renderAt("/run/R1");
    await waitFor(() => expect(screen.getByTestId("run-summary")).toBeTruthy());

    expect(screen.getByTestId("checks-inconclusive").textContent).toContain("1");
    expect(screen.getByTestId("criteria-inconclusive").textContent).toContain(
      "2",
    );
    const criteria = screen.getByRole("region", { name: "Criteria" });
    expect(within(criteria).getByText("4 total")).toBeTruthy();
  });

  it("keeps long run inputs in a closed, wrapping disclosure", async () => {
    const longSelector =
      "body > div.min-w-0 > div.transition-all:nth-of-type(2)";
    stub({
      ...RUN,
      inputs: { assistant_message_css: longSelector },
    });
    renderAt("/run/R1");
    const summary = await screen.findByTestId("run-summary");
    const details = within(summary)
      .getByText("Run configuration")
      .closest("details");

    expect(details?.open).toBe(false);
    const value = within(details as HTMLElement).getByText(
      JSON.stringify(longSelector),
    );
    expect(value.className).toContain("break-all");
  });

  it("marks the open check active in the rail on the check page", async () => {
    stub();
    renderAt("/run/R1/check/AC-5%3A%3AAC-5.1");
    const tree = await screen.findByTestId("run-tree");
    expect(tree.className).toContain("max-w-full");
    expect(tree.className).toContain("overflow-x-hidden");
    const active = within(tree).getByRole("link", { name: "AC-5.1" });
    expect(active.getAttribute("aria-current")).toBe("page");
    // A sibling check is not marked active.
    expect(
      within(tree).getByRole("link", { name: "AC-5.2" }).getAttribute("aria-current"),
    ).toBeNull();
  });
});
