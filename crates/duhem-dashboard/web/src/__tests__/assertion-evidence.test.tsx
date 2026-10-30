import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
import { afterEach, expect, it, vi } from "vitest";
import type { CheckDetail, RunDetail, TraceEvent } from "../api";
import CheckPage, { Timeline } from "../views/CheckPage";

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
const event = (seq: number, kind: string, fields: Record<string, unknown>): TraceEvent => ({ seq, ts: "2026-10-30T00:00:00Z", kind, ...fields });
const events = [
  event(1, "step_started", { step_index: 0, uses: "api/call", criterion_id: "AC", check_id: "C" }),
  event(2, "step_finished", { step_index: 0, outcome: "ok" }),
  event(3, "assertion_evaluated", { assertion_index: 0, state: "inconclusive:type_mismatch", expr: "equal($inputs.before, $inputs.after)", operands: { "$inputs.before": null, "$inputs.after": "System prompt 4.2k" } }),
  event(4, "assertion_evaluated", { assertion_index: 1, step_index: 0, state: "fail", expr: "$steps.request.outputs.status == 200", operands: { "$steps.request.outputs.status": 500 } }),
  event(5, "assertion_evaluated", { assertion_index: 2, state: "inconclusive:missing_observation", expr: "$inputs.missing == 1", operands: {} }),
  event(6, "assertion_evaluated", { assertion_index: 3, step_index: 0, state: "pass", expr: "step `request` satisfied == true" }),
];

it("renders JSON null, strings, and absence separately, including assertions owned by a step", () => {
  render(<Timeline events={events} />);
  expect(screen.getAllByTestId("assertion-operands").map((row) => row.textContent).join("\n")).toContain('$inputs.before= null');
  expect(screen.getAllByTestId("assertion-operands").map((row) => row.textContent).join("\n")).toContain('= "System prompt 4.2k"');
  expect(screen.getByText(/No resolved operand values/)).toBeTruthy();
  expect(screen.getAllByTestId("assertion-operands").some((row) => row.textContent?.includes("status= 500"))).toBe(true);
});

it("keeps legacy explicit assertions readable without inventing values", () => {
  render(<Timeline events={[event(1, "assertion_evaluated", { assertion_index: 0, expr: "1 == 2", state: "fail" })]} />);
  expect(screen.getByText(/Operand values were not recorded/)).toBeTruthy();
  expect(screen.queryByTestId("assertion-operands")).toBeNull();
});

function Location() { const location = useLocation(); return <output data-testid="location">{location.search}</output>; }

it("lists every explicit assertion after steps and navigates to standalone and step-owned details", async () => {
  const run: RunDetail = { run_id: "R", verification: "case.yml", started_at: null, inputs: {}, verdict: "fail", status: "finished", setup_aborted: false, has_definition: false,
    criteria: [{ id: "AC", verdict: "fail", checks: [{ id: "C", verdict: "fail" }] }] };
  const check: CheckDetail = { criterion_id: "AC", check_id: "C", verdict: "fail", spans: [], timeline: events, artifacts: [] };
  vi.stubGlobal("fetch", vi.fn(async (url: string) => new Response(JSON.stringify(url.includes("/checks/") ? check : run), { status: 200 })));
  const scroll = vi.fn();
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", { value: scroll, configurable: true });
  render(<MemoryRouter initialEntries={["/run/R/check/AC%3A%3AC"]}>
    <Routes><Route path="/run/:runId/check/:pair" element={<CheckPage />} /></Routes><Location />
  </MemoryRouter>);
  await waitFor(() => expect(screen.getAllByTestId("assertion-rail-row")).toHaveLength(3));
  const rows = screen.getAllByTestId("assertion-rail-row");
  expect(rows[0].textContent).toContain("inconclusive");
  expect(rows[1].textContent).toContain("fail");
  fireEvent.click(rows[0]);
  await waitFor(() => expect(screen.getByTestId("location").textContent).toBe("?assertion=a3"));
  await waitFor(() => expect(scroll).toHaveBeenCalledWith({ block: "start" }));
  scroll.mockClear();
  fireEvent.click(rows[1]);
  await waitFor(() => expect(screen.getByTestId("location").textContent).toBe("?assertion=a4"));
  await waitFor(() => expect(scroll).toHaveBeenCalled());
  expect(document.querySelector('[data-assertion-key="a4"]')?.closest("details")?.open).toBe(true);
});
