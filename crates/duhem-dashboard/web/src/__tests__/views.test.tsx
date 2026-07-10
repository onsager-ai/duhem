// Component tests (#86): runs list (empty / one / many), timeline
// ordering, artifact rendering.

import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import RunsList, { matchesFilters } from "../views/RunsList";
import { Artifacts, Timeline } from "../views/CheckPage";
import type { RunsListEntry, TraceEvent } from "../api";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function stubRuns(entries: RunsListEntry[]) {
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response(JSON.stringify(entries), { status: 200 })),
  );
}

function leaf(id: string, verdict: string | null, live = false): RunsListEntry {
  return {
    run_id: id,
    verification: "login",
    started_at: "2026-06-10T10:00:00.000Z",
    duration_ms: 1234,
    verdict,
    kind: "leaf",
    live,
  };
}

describe("RunsList", () => {
  it("renders the empty state", async () => {
    stubRuns([]);
    render(
      <MemoryRouter>
        <RunsList />
      </MemoryRouter>,
    );
    await waitFor(() => expect(screen.getByText(/No runs/)).toBeTruthy());
  });

  it("renders a single leaf row", async () => {
    stubRuns([leaf("01JRUNA", "pass")]);
    const { container } = render(
      <MemoryRouter>
        <RunsList />
      </MemoryRouter>,
    );
    await waitFor(() => expect(screen.getByText("01JRUNA")).toBeTruthy());
    expect(container.querySelector(".badge.verdict-pass")?.textContent).toBe("pass");
  });

  it("renders many rows including nested run-set leaves and a live badge", async () => {
    stubRuns([
      {
        ...leaf("login", "fail"),
        kind: "run-set",
        children: [leaf("01JRUNA", "pass"), leaf("01JRUNB", "fail")],
      },
      leaf("01JRUNC", null, true),
    ]);
    render(
      <MemoryRouter>
        <RunsList />
      </MemoryRouter>,
    );
    await waitFor(() => expect(screen.getByText("01JRUNA")).toBeTruthy());
    expect(screen.getByText("01JRUNB")).toBeTruthy();
    expect(screen.getByText("01JRUNC")).toBeTruthy();
    expect(screen.getByText("● live")).toBeTruthy();
  });
});

describe("matchesFilters", () => {
  it("filters by verdict family and live state", () => {
    expect(matchesFilters(leaf("a", "pass"), "", ["pass"], "", "")).toBe(true);
    expect(matchesFilters(leaf("a", "fail"), "", ["pass"], "", "")).toBe(false);
    expect(
      matchesFilters(leaf("a", "inconclusive:timeout"), "", ["inconclusive"], "", ""),
    ).toBe(true);
    expect(matchesFilters(leaf("a", null, true), "", ["live"], "", "")).toBe(true);
  });

  it("filters by verification and date range", () => {
    expect(matchesFilters(leaf("a", "pass"), "login", [], "", "")).toBe(true);
    expect(matchesFilters(leaf("a", "pass"), "other", [], "", "")).toBe(false);
    expect(matchesFilters(leaf("a", "pass"), "", [], "2026-06-01", "2026-06-30")).toBe(true);
    expect(matchesFilters(leaf("a", "pass"), "", [], "2026-06-11", "")).toBe(false);
  });
});

describe("Timeline", () => {
  it("renders each event as a legible row in trace order, raw one click away", () => {
    const events: TraceEvent[] = [
      {
        seq: 4,
        ts: "2026-01-01T00:00:00.000Z",
        kind: "step_started",
        uses: "ui/navigate",
        layer: "ui",
        with: { url: "http://x/" },
      },
      {
        seq: 5,
        ts: "2026-01-01T00:00:01.000Z",
        kind: "assertion_evaluated",
        check_id: "AC-1.1",
        assertion_index: 0,
        state: "fail",
        detail: "actual false, expected true",
      },
      {
        seq: 6,
        ts: "2026-01-01T00:00:01.100Z",
        kind: "check_finished",
        check_id: "AC-1.1",
        verdict: "fail",
      },
    ];
    const { container } = render(<Timeline events={events} />);
    // Legible labels, in trace order — no raw JSON, no re-sort.
    const labels = [...container.querySelectorAll(".ev-label")].map((el) => el.textContent);
    expect(labels).toEqual(["navigate", "assertion failed", "verdict: fail"]);
    // The failing assertion row carries its recorded detail and fail tone.
    expect(container.querySelector(".ev.tone-fail .ev-detail")?.textContent).toContain(
      "actual false",
    );
    // Raw JSON is preserved, tucked behind a per-row <details> toggle.
    const raws = container.querySelectorAll(".ev-raw pre");
    expect(raws).toHaveLength(3);
    expect(raws[0].textContent).toContain("ui/navigate");
  });
});

describe("Artifacts", () => {
  it("renders screenshots as images and other blobs as links", () => {
    const sha = "a".repeat(64);
    const { container } = render(
      <Artifacts
        artifacts={[
          { id: sha, kind: "screenshot", url: `run/r/artifact/${sha}.png` },
          { id: "b".repeat(64), kind: "body", url: `run/r/artifact/${"b".repeat(64)}.json` },
        ]}
      />,
    );
    const imgs = container.querySelectorAll("img");
    expect(imgs).toHaveLength(1);
    expect(imgs[0].getAttribute("src")).toBe(`run/r/artifact/${sha}.png`);
    const links = container.querySelectorAll("a");
    expect(links).toHaveLength(2);
  });

  it("renders the runner's capture/* kinds inline (spec #202)", () => {
    // The runtime's failure-evidence capture emits exactly these
    // output names; the extensionless store URL is what the reader
    // builds. The screenshot must render as an image on kind alone.
    const shot = "c".repeat(64);
    const dom = "d".repeat(64);
    const { container } = render(
      <Artifacts
        artifacts={[
          { id: shot, kind: "capture/screenshot", url: `run/r/artifact/${shot}` },
          { id: dom, kind: "capture/dom", url: `run/r/artifact/${dom}` },
        ]}
      />,
    );
    const imgs = container.querySelectorAll("img");
    expect(imgs).toHaveLength(1);
    expect(imgs[0].getAttribute("src")).toBe(`run/r/artifact/${shot}`);
  });
});

// ---- #193: ④ delivery-web span chain --------------------------------

import { SpanChain } from "../views/CheckPage";
import { Sparkline } from "../views/VerificationPage";
import type { SpanModel } from "../api";

describe("SpanChain (④)", () => {
  it("renders the ordered layer chain with per-layer outcome", () => {
    const spans: SpanModel[] = [
      { seq: 1, layer: "ui", ok: true },
      { seq: 3, layer: "api", ok: true },
      { seq: 5, layer: "data", ok: false, detail: "timeout" },
    ];
    render(<SpanChain spans={spans} />);
    const chain = screen.getByTestId("spanchain");
    expect(chain.textContent).toContain("ui");
    expect(chain.textContent).toContain("api");
    // The broken layer carries its detail inline.
    expect(chain.textContent).toContain("data ✕ timeout");
  });

  it("says layer unknown for a pre-tag run instead of guessing", () => {
    render(<SpanChain spans={[]} />);
    expect(screen.getByTestId("spanchain-unknown").textContent).toContain(
      "layer unknown",
    );
  });
});

// ---- #193: ② criterion sparkline ------------------------------------

describe("Sparkline (②)", () => {
  it("renders one dot per run, absent runs dashed", () => {
    const { container } = render(
      <Sparkline verdicts={["pass", "fail", null, "inconclusive:timeout"]} />,
    );
    const dots = container.querySelectorAll(".dot");
    expect(dots).toHaveLength(4);
    expect(dots[0].className).toContain("verdict-pass");
    expect(dots[1].className).toContain("verdict-fail");
    expect(dots[2].className).toContain("dot-absent");
    expect(dots[3].className).toContain("verdict-inconclusive");
  });
});
