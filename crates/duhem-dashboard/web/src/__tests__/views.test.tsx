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
  it("renders events in the order given (trace order, no re-sort)", () => {
    const events: TraceEvent[] = [
      { seq: 4, ts: "t4", kind: "step_started" },
      { seq: 5, ts: "t5", kind: "step_finished" },
      { seq: 6, ts: "t6", kind: "check_finished" },
    ];
    const { container } = render(<Timeline events={events} />);
    const kinds = [...container.querySelectorAll(".kind")].map((el) => el.textContent);
    expect(kinds).toEqual(["step_started", "step_finished", "check_finished"]);
    const seqs = [...container.querySelectorAll(".seq")].map((el) => el.textContent);
    expect(seqs).toEqual(["#4", "#5", "#6"]);
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
});
