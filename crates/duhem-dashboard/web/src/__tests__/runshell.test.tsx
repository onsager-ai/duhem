// Run report shell (#284 tree redesign): the run header verdict, the
// criteria → check tree rail (each check a link named by its check id),
// the summary roll-up tiles, and the active-check highlight when a check
// is open. Replaces the former per-run tab-bar tests.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes, useLocation } from "react-router-dom";
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

const CHECK = {
  criterion_id: "AC-5",
  check_id: "AC-5.1",
  verdict: "fail",
  spans: [{ seq: 1, layer: "ui", ok: false, detail: "text present" }],
  timeline: [
    {
      seq: 1,
      ts: "2026-07-22T14:03:00.000Z",
      kind: "step_started",
      criterion_id: "AC-5",
      check_id: "AC-5.1",
      step_index: 0,
      uses: "ui/navigate",
    },
    {
      seq: 2,
      ts: "2026-07-22T14:03:00.010Z",
      kind: "step_finished",
      step_index: 0,
      outcome: "ok",
    },
    {
      seq: 3,
      ts: "2026-07-22T14:03:00.020Z",
      kind: "assertion_evaluated",
      state: "fail",
      detail: 'expected text "Manager" to be absent within 5s, but 1 still matched',
      step_index: 0,
    },
  ],
  artifacts: [],
};

function stub(run = RUN, check: Record<string, unknown> = CHECK) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string) => {
      if (String(url).includes("/checks/")) {
        return new Response(JSON.stringify(check), { status: 200 });
      }
      if (String(url).endsWith("/definition")) {
        return new Response(`
criteria:
  - id: AC-5
    checks:
      - id: AC-5.1
        steps:
          - id: open-page
            uses: ui/navigate
          - id: submit-form
            uses: ui/click
`);
      }
      return new Response(JSON.stringify(run), { status: 200 });
    }),
  );
}

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <LocationProbe />
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

function LocationProbe() {
  const location = useLocation();
  return <output data-testid="location-search">{location.search}</output>;
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
    expect(
      screen.getAllByTestId("criterion-parent").every((parent) =>
        parent.className.includes("md:sticky"),
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

  it("keeps an unavailable Definition tab keyboard discoverable", async () => {
    stub({ ...RUN, has_definition: false });
    renderAt("/run/R1");
    await screen.findByTestId("run-summary");

    const definition = screen.getByRole("tab", { name: "Definition" });
    expect(definition.getAttribute("aria-disabled")).toBe("true");
    expect(definition.getAttribute("tabindex")).toBe("0");
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
    const { container } = renderAt("/run/R1/check/AC-5%3A%3AAC-5.1");
    const tree = await screen.findByTestId("run-tree");
    expect(tree.className).toContain("max-w-full");
    expect(tree.className).toContain("overflow-x-hidden");
    expect(tree.className).toContain("overflow-x-clip");
    const grid = container.querySelector(".run-results-grid") as HTMLElement;
    const rail = container.querySelector(".run-results-rail") as HTMLElement;
    const detail = container.querySelector(".run-results-detail") as HTMLElement;
    // Mobile-first: one-column source order. Desktop adds two columns and
    // independently scrollable panes without introducing fixed-width overflow.
    expect(grid.className).toContain("min-w-0");
    expect(grid.className).toContain("md:grid-cols-[17rem_minmax(0,1fr)]");
    expect(rail.className).toContain("md:overflow-y-auto");
    expect(detail.className).toContain("md:overflow-y-auto");
    const active = within(tree).getByRole("link", { name: "AC-5.1" });
    expect(active.getAttribute("aria-current")).toBe("page");
    // A sibling check is not marked active.
    expect(
      within(tree).getByRole("link", { name: "AC-5.2" }).getAttribute("aria-current"),
    ).toBeNull();
  });

  it("expands only the active check into status-propagated step links", async () => {
    stub();
    renderAt("/run/R1/check/AC-5%3A%3AAC-5.1?step=open-page");
    const tree = await screen.findByTestId("run-tree");
    await waitFor(() => expect(within(tree).getByRole("link", { name: "open-page" })).toBeTruthy());
    expect(screen.getAllByTestId("step-children")).toHaveLength(1);
    const step = within(tree).getByRole("link", { name: "open-page" });
    expect(step.getAttribute("aria-current")).toBe("step");
    expect(step.textContent).toContain("fail");
  });

  it("renders a video-relative step timeline that seeks and selects markers", async () => {
    const shot = "a".repeat(64);
    const video = "b".repeat(64);
    stub(RUN, {
      ...CHECK,
      timeline: [
        ...CHECK.timeline,
        {
          seq: 4,
          ts: "2026-07-22T14:03:00.021Z",
          kind: "step_started",
          criterion_id: "AC-5",
          check_id: "AC-5.1",
          step_index: 1,
          uses: "ui/click",
        },
        {
          seq: 5,
          ts: "2026-07-22T14:03:00.030Z",
          kind: "step_finished",
          step_index: 1,
          outcome: "ok",
        },
      ],
      replay: {
        version: 1,
        clock: "monotonic",
        duration_ms: 4_000,
        steps: [
          {
            step_index: 1,
            started_ms: 3_000,
            finished_ms: 4_000,
          },
          {
            step_index: 0,
            started_ms: 1_200,
            finished_ms: 2_000,
            screenshot: { id: shot, kind: "capture/step-screenshot", url: "shot.png" },
          },
        ],
        network: [
          { method: "GET", url: "/inside", status: 200, started_ms: 1_300, duration_ms: 200, wait_ms: 100, receive_ms: 100 },
          { method: "GET", url: "/outside", status: 200, started_ms: 4_500, duration_ms: 100, wait_ms: 100, receive_ms: 0 },
        ],
        performance: [
          { kind: "navigation", name: "document", started_ms: 1_300, duration_ms: 300 },
          { kind: "resource", name: "late", started_ms: 4_500, duration_ms: 100 },
        ],
        video: {
          started_ms: 1_000,
          finished_ms: 5_000,
          artifact: { id: video, kind: "capture/video", url: "replay.webm" },
        },
      },
    });
    renderAt(
      "/run/R1/check/AC-5%3A%3AAC-5.1?step=open-page&view=replay&inspector=performance",
    );
    const replay = await screen.findByTestId("replay");
    expect(within(replay).getByRole("img", { name: "Step open-page screenshot" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Replay" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("tab", { name: "Performance" }).getAttribute("aria-selected")).toBe(
      "true",
    );
    expect(within(screen.getByTestId("replay-performance")).getByText("document")).toBeTruthy();
    expect(within(replay).queryByText("late")).toBeNull();

    fireEvent.click(within(replay).getByRole("button", { name: "Video" }));
    const replayVideo = replay.querySelector("video.replay-video") as HTMLVideoElement;
    expect(replayVideo).toBeTruthy();
    expect(replayVideo.getAttribute("src")).toBe("replay.webm");
    // The selected step starts 200ms after the video, not at absolute
    // monotonic clock 1.2s.
    expect(replayVideo.currentTime).toBeCloseTo(0.2);

    const timeline = within(replay).getByTestId("replay-step-timeline");
    const markers = within(timeline).getAllByRole("button");
    expect(markers.map((marker) => marker.getAttribute("aria-label"))).toEqual([
      "Seek to step open-page",
      "Seek to step submit-form",
    ]);
    expect(parseFloat(markers[0].style.left)).toBeCloseTo(5);
    expect(parseFloat(markers[0].style.width)).toBeCloseTo(20);
    expect(markers[0].classList.contains("on")).toBe(true);
    expect(markers[0].getAttribute("aria-pressed")).toBe("true");

    // Loaded media duration is the more accurate pixel scale: 200ms into
    // a real 5s video maps to 4%, rather than the model span's 5%.
    Object.defineProperty(replayVideo, "duration", { configurable: true, value: 5 });
    fireEvent.loadedMetadata(replayVideo);
    await waitFor(() => expect(parseFloat(markers[0].style.left)).toBeCloseTo(4));
    expect(parseFloat(markers[0].style.width)).toBeCloseTo(16);

    fireEvent.click(markers[1]);
    expect(replayVideo.currentTime).toBeCloseTo(2);
    await waitFor(() =>
      expect(screen.getByTestId("location-search").textContent).toContain("step=submit-form"),
    );
    expect(markers[1].classList.contains("on")).toBe(true);
    expect(markers[0].classList.contains("on")).toBe(false);

    // Playback follows the clock window locally without rewriting the
    // addressable selection or adding navigation churn.
    replayVideo.currentTime = 0.5;
    fireEvent.timeUpdate(replayVideo);
    await waitFor(() => expect(markers[0].classList.contains("on")).toBe(true));
    expect(within(replay).getByText("Step open-page")).toBeTruthy();
    expect(screen.getByTestId("location-search").textContent).toContain("step=submit-form");

    // In the gap after open-page and before submit-form, playback holds
    // the last step that started instead of reverting to the selection.
    replayVideo.currentTime = 1.5;
    fireEvent.timeUpdate(replayVideo);
    await waitFor(() => expect(markers[0].getAttribute("aria-pressed")).toBe("true"));
    expect(markers[0].classList.contains("on")).toBe(true);
    expect(markers[1].classList.contains("on")).toBe(false);

    fireEvent.click(markers[0]);
    // After the final step finishes, playback keeps the highest-index
    // marker active through the video's trailing dead-time.
    replayVideo.currentTime = 4.5;
    fireEvent.timeUpdate(replayVideo);
    await waitFor(() => expect(markers[1].getAttribute("aria-pressed")).toBe("true"));
    expect(markers[1].classList.contains("on")).toBe(true);
    expect(markers[0].classList.contains("on")).toBe(false);

    fireEvent.click(screen.getByRole("tab", { name: "Network" }));
    const network = await screen.findByTestId("replay-network");
    expect(within(network).getByText("/inside")).toBeTruthy();
    expect(within(network).queryByText("/outside")).toBeNull();
  });
});
