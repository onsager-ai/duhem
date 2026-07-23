// Live runs-list polling (#298): the list refetches on an interval
// while the tab is visible — an in-flight run appears and its verdict
// resolves without a manual reload — and a hidden tab stops polling.

import { act, cleanup, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import RunsList from "../views/RunsList";
import { POLL_INTERVAL_MS } from "../hooks/use-polled-runs";
import type { RunsListEntry } from "../api";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

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

/** fetch stub whose response is re-read from `pages` on every call. */
function stubRunsSequence(pages: RunsListEntry[][]): () => number {
  let calls = 0;
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => {
      const page = pages[Math.min(calls, pages.length - 1)];
      calls += 1;
      return new Response(JSON.stringify(page), { status: 200 });
    }),
  );
  return () => calls;
}

function setVisibility(state: DocumentVisibilityState) {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    get: () => state,
  });
}

describe("RunsList polling (#298)", () => {
  it("an in-flight run appears and resolves without a reload", async () => {
    vi.useFakeTimers();
    setVisibility("visible");
    const calls = stubRunsSequence([
      [leaf("old-run", "pass")],
      [leaf("new-run", null, true), leaf("old-run", "pass")],
      [leaf("new-run", "pass"), leaf("old-run", "pass")],
    ]);

    render(
      <MemoryRouter>
        <RunsList />
      </MemoryRouter>,
    );

    // Initial fetch: only the finished run.
    await act(() => vi.advanceTimersByTimeAsync(0));
    expect(screen.getByText("old-run")).toBeTruthy();
    expect(screen.queryByText("new-run")).toBeNull();

    // One interval later the in-flight run has appeared, live.
    await act(() => vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS));
    expect(screen.getByText("new-run")).toBeTruthy();
    expect(document.querySelector(".badge.live")?.textContent ?? "live").toContain("live");

    // Another interval and its verdict resolved — still no reload.
    await act(() => vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS));
    expect(calls()).toBe(3);
    expect(document.querySelectorAll(".badge.verdict-pass").length).toBe(2);
  });

  it("a hidden tab stops polling; returning refreshes immediately", async () => {
    vi.useFakeTimers();
    setVisibility("visible");
    const calls = stubRunsSequence([[leaf("only-run", "pass")]]);

    render(
      <MemoryRouter>
        <RunsList />
      </MemoryRouter>,
    );
    await act(() => vi.advanceTimersByTimeAsync(0));
    expect(calls()).toBe(1);

    // Hidden: intervals elapse, no fetches happen.
    setVisibility("hidden");
    await act(() => vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS * 3));
    expect(calls()).toBe(1);

    // Back to visible: an immediate refresh, not a stale interval wait.
    setVisibility("visible");
    await act(async () => {
      document.dispatchEvent(new Event("visibilitychange"));
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(calls()).toBe(2);
  });

  it("a failed background refresh keeps the last good list", async () => {
    vi.useFakeTimers();
    setVisibility("visible");
    let calls = 0;
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => {
        calls += 1;
        if (calls > 1) throw new Error("dashboard went away");
        return new Response(JSON.stringify([leaf("kept-run", "pass")]), { status: 200 });
      }),
    );

    render(
      <MemoryRouter>
        <RunsList />
      </MemoryRouter>,
    );
    await act(() => vi.advanceTimersByTimeAsync(0));
    expect(screen.getByText("kept-run")).toBeTruthy();

    await act(() => vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS));
    expect(calls).toBe(2);
    // Still the data, not an error flash.
    expect(screen.getByText("kept-run")).toBeTruthy();
    expect(document.querySelector(".error")).toBeNull();
  });
});
