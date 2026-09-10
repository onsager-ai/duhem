import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, expect, it, vi } from "vitest";
import type { TraceEvent } from "../api";
import { parseDefinition } from "../definition";
import CheckPage, { Timeline } from "../views/CheckPage";

const definition = `
criteria:
  - id: AC-1
    checks:
      - id: C
        steps:
          - id: before
            uses: cli/invoke
          - id: loop
          - id: after
            uses: cli/invoke
`;

function events(count?: number): TraceEvent[] {
  let seq = 0;
  const step = (index: number, iteration?: number): TraceEvent[] => {
    const common = { ts: "2026-09-10T00:00:00Z", step_index: index };
    return [
      { ...common, seq: seq++, kind: "setup_step_started", phase: "setup",
        criterion_id: "AC-1", check_id: "C", uses: "cli/invoke",
        ...(iteration === undefined ? {} : { flow: { name: "for_each", invocation: "loop", inner_index: 0, iteration } }),
      },
      { ...common, seq: seq++, kind: "setup_step_observation", output_name: "item", value: iteration ?? index },
      { ...common, seq: seq++, kind: "setup_step_finished", outcome: iteration === 37 ? "error" : "ok" },
    ];
  };
  return [step(0), ...(count === undefined ? [step(1)] : Array.from({ length: count }, (_, i) => step(1, i))), step(2)].flat();
}

function report(count?: number, step?: string, hasDefinition = true, omitOwners = false) {
  vi.stubGlobal("fetch", vi.fn(async (url: string) => {
    if (String(url).includes("/checks/")) return new Response(JSON.stringify({
      criterion_id: "AC-1", check_id: "C", verdict: "fail", spans: [], timeline: events(count).map((event) => {
        if (!omitOwners) return event;
        const { criterion_id: _criterion, check_id: _check, ...legacy } = event;
        return legacy;
      }), artifacts: [],
    }));
    if (String(url).endsWith("/definition")) return new Response(definition);
    return new Response(JSON.stringify({
      run_id: "r", verification: "loops", status: "finished", verdict: "fail", inputs: {},
      started_at: "2026-09-10T00:00:00Z", setup_aborted: false, has_definition: hasDefinition,
      criteria: [{ id: "AC-1", verdict: "fail", checks: [{ id: "C", verdict: "fail" }] }],
    }));
  }));
  return render(<MemoryRouter initialEntries={[`/run/r/check/AC-1::C${step ? `?step=${encodeURIComponent(step)}` : ""}`]}>
    <Routes><Route path="/run/:runId/check/:pair" element={<CheckPage />} /></Routes>
  </MemoryRouter>);
}

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

it("pins the no-loop timeline markup", () => {
  const view = render(<MemoryRouter><Timeline events={events()} /></MemoryRouter>);
  expect(view.container.innerHTML).toMatchSnapshot();
});

it("pins the no-loop sidebar links and selection", async () => {
  const view = report(undefined, "after");
  await screen.findByRole("link", { name: "after" });
  expect(screen.getByTestId("step-children").innerHTML).toMatchSnapshot();
  const selected = view.container.querySelectorAll(".step-selected");
  expect(selected).toHaveLength(1);
  expect(selected[0].getAttribute("data-step-index")).toBe("2");
});

it("a deep link to failing iteration 37 selects that iteration and its label agrees", async () => {
  const key = parseDefinition(definition).stepId("AC-1", "C", 1,
    { name: "for_each", invocation: "loop", inner_index: 0, iteration: 37 })!;
  const view = report(50, key);
  await screen.findByTestId("step-children");
  await waitFor(() => {
    const selected = view.container.querySelectorAll(".step-selected");
    expect(selected).toHaveLength(1);
    expect(selected[0].textContent).toContain('"value": 37');
    expect(selected[0].getAttribute("data-flow-iteration")).toBe("37");
    const link = view.container.querySelector('[aria-current="step"]');
    expect(link?.getAttribute("aria-label")).toContain("Iteration 37");
    expect(link?.getAttribute("href")).toContain(encodeURIComponent(key));
  });
});

it("groups 50 iterations in the timeline and sidebar, opening only failure 37", async () => {
  report(50);
  const loop = await screen.findByTestId("loop-group");
  expect(screen.getAllByTestId("loop-group")).toHaveLength(1);
  expect(loop.querySelector("summary")?.textContent).toContain("50 iterations");
  for (const testId of ["loop-iteration", "rail-loop-iteration"]) {
    const iterations = await screen.findAllByTestId(testId);
    expect(iterations).toHaveLength(50);
    expect(iterations.filter((item) => (item as HTMLDetailsElement).open)
      .map((item) => item.getAttribute("data-iteration"))).toEqual(["37"]);
  }
  fireEvent.click(screen.getByRole("link", { name: "loop › Iteration 37" }));
  expect(document.querySelectorAll(".step-selected")).toHaveLength(1);
});

it("a single iteration has no grouping chrome and leaves following indices intact", async () => {
  const view = report(1);
  await screen.findByRole("link", { name: "after" });
  expect(screen.queryByTestId("loop-group")).toBeNull();
  expect(screen.queryByTestId("rail-loop-group")).toBeNull();
  expect(screen.queryByTestId("flow-group")).toBeNull();
  expect(screen.getAllByTestId("step-group").map((item) => item.getAttribute("data-step-index")))
    .toEqual(["0", "1", "2"]);
  fireEvent.click(screen.getByRole("link", { name: "after" }));
  expect(view.container.querySelector(".step-selected")?.getAttribute("data-step-index")).toBe("2");
});

it("keeps loop deep links stable without a definition, across reloads and keyboard navigation", async () => {
  const key = parseDefinition(definition).stepId("AC-1", "C", 1,
    { name: "for_each", invocation: "loop", inner_index: 0, iteration: 37 })!;
  const view = report(50, key, false);
  const link = await screen.findByRole("link", { name: /Iteration 37/ });
  expect(link.getAttribute("aria-current")).toBe("step");
  fireEvent.keyDown(link, { key: "ArrowDown" });
  await waitFor(() => expect(view.container.querySelector(".step-selected")?.getAttribute("data-flow-iteration")).toBe("38"));
  const next = screen.getByRole("link", { name: /Iteration 38/ });
  const href = next.getAttribute("href")!;
  expect((next.closest("details") as HTMLDetailsElement).open).toBe(true);
  view.unmount();
  const reloaded = report(50, new URL(href, "http://localhost").searchParams.get("step")!, false);
  await waitFor(() => expect(reloaded.container.querySelector(".step-selected")?.getAttribute("data-flow-iteration")).toBe("38"));
});

it("does not renumber the step after 50 iterations or share raw disclosure state", async () => {
  const view = report(50);
  const after = await screen.findByRole("link", { name: "after" });
  fireEvent.click(after);
  expect(view.container.querySelector(".step-selected")?.getAttribute("data-step-index")).toBe("2");
  const failed = view.container.querySelector('[data-flow-iteration="37"]')!;
  const raw = failed.querySelector<HTMLDetailsElement>('[data-testid="step-raw"]')!;
  raw.open = true;
  fireEvent(raw, new Event("toggle"));
  await waitFor(() => expect(screen.getAllByTestId("step-raw").filter((item) =>
    (item as HTMLDetailsElement).open)).toHaveLength(1));
  expect(view.container.querySelectorAll('[data-step-index="1"]')).toHaveLength(50);
  expect(new Set(screen.getAllByTestId("step-group").map((item) => item.id)).size).toBe(52);
});

it("preserves iteration selection across view switches without attributing ambiguous replay frames", async () => {
  const key = parseDefinition(definition).stepId("AC-1", "C", 1,
    { name: "for_each", invocation: "loop", inner_index: 0, iteration: 37 })!;
  const view = report(50, key);
  await screen.findByTestId("loop-group");
  fireEvent.click(screen.getByRole("tab", { name: "Replay" }));
  expect(screen.getByTestId("replay-ambiguous").textContent).toContain("Iteration 37");
  fireEvent.click(screen.getByRole("tab", { name: "Steps" }));
  expect(view.container.querySelector(".step-selected")?.getAttribute("data-flow-iteration")).toBe("37");
});

it("scroll selection ignores collapsed iterations sharing the same index", async () => {
  const view = report(50);
  await screen.findByTestId("loop-group");
  for (const step of screen.getAllByTestId("step-group")) {
    const index = step.getAttribute("data-step-index");
    const iteration = step.getAttribute("data-flow-iteration");
    vi.spyOn(step, "getBoundingClientRect").mockReturnValue({
      top: index === "2" ? 500 : iteration === "37" ? -1 : -10,
    } as DOMRect);
  }
  fireEvent.scroll(view.container.querySelector(".run-results-detail")!);
  await waitFor(() => expect(view.container.querySelector('[aria-current="step"]')?.getAttribute("aria-label"))
    .toBe("loop › Iteration 37"));
  expect(view.container.querySelectorAll(".step-selected")).toHaveLength(1);
});

it("preserves authored no-loop links when legacy step records omit owner IDs", async () => {
  const view = report(undefined, "after", true, true);
  await screen.findByRole("link", { name: "after" });
  expect(screen.getByRole("link", { name: "before" }).getAttribute("href")).toContain("step=before");
  expect(screen.getByRole("link", { name: "loop" }).getAttribute("href")).toContain("step=loop");
  expect(view.container.querySelectorAll(".step-selected")).toHaveLength(1);
  expect(view.container.querySelector(".step-selected")?.getAttribute("data-step-index")).toBe("2");
});
