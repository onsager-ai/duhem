// The run header's verdict band (#563): the verdict as a word at display
// size, a summary line, the first failure on fail, and a collapsed
// single-line state once the reader scrolls. Every case here was checked
// by breaking the component first.

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { CheckDetail, RunDetail, RunsListEntry } from "../api";
import { RunsProvider } from "../runs-context";
import { RunScaffold, VerdictBand } from "../views/RunScaffold";
import {
  criteriaTally,
  findRunEntry,
  finishedAt,
  firstFailedAssertion,
  firstFailedCheck,
  utcStamp,
  verdictWord,
} from "../verdict-band";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function run(overrides: Partial<RunDetail> = {}): RunDetail {
  return {
    run_id: "R1",
    verification: "pegasus-register",
    started_at: "2026-07-22T14:03:00.000Z",
    inputs: {},
    verdict: "pass",
    status: "finished",
    setup_aborted: false,
    has_definition: false,
    criteria: [
      { id: "AC-1", verdict: "pass", checks: [{ id: "AC-1.1", verdict: "pass" }] },
      { id: "AC-2", verdict: "pass", checks: [{ id: "AC-2.1", verdict: "pass" }] },
    ],
    ...overrides,
  };
}

// AC-7 is inconclusive and listed first, so a derivation that picks the
// first non-pass check (rather than the first *fail*) lands on it.
const FAILING = run({
  verdict: "fail",
  criteria: [
    { id: "AC-7", verdict: "inconclusive:environment_error", checks: [{ id: "AC-7.1", verdict: "inconclusive:environment_error" }] },
    {
      id: "AC-5",
      verdict: "fail",
      checks: [
        { id: "AC-5.1", verdict: "pass" },
        { id: "AC-5.2", verdict: "fail" },
      ],
    },
    { id: "AC-6", verdict: "fail", checks: [{ id: "AC-6.1", verdict: "fail" }] },
    { id: "AC-8", verdict: "pass", checks: [{ id: "AC-8.1", verdict: "pass" }] },
  ],
});

function check(timeline: CheckDetail["timeline"]): CheckDetail {
  return {
    criterion_id: "AC-5",
    check_id: "AC-5.2",
    verdict: "fail",
    spans: [],
    timeline,
    artifacts: [],
  };
}

const assertion = (state: string, detail: string, expr?: string) => ({
  seq: 1,
  ts: "2026-07-22T14:03:00.020Z",
  kind: "assertion_evaluated",
  state,
  detail,
  ...(expr ? { expr } : {}),
});

const FAILED_CHECK = check([
  assertion("pass", "actual 200, expected 200", "$steps.a.outputs.status == 200"),
  assertion("fail", "actual 500, expected 201", "$steps.b.outputs.status == 201"),
  assertion("fail", "actual 404, expected 200"),
]);

function stubFetch(checkDetail: CheckDetail = FAILED_CHECK, runs: RunsListEntry[] = []) {
  const fetchMock = vi.fn(async (url: string) => {
    const path = String(url);
    if (path.includes("/checks/")) return new Response(JSON.stringify(checkDetail));
    if (path === "api/runs.json") return new Response(JSON.stringify(runs));
    return new Response(JSON.stringify(FAILING));
  });
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

function renderBand(detail: RunDetail, props: { collapsed?: boolean; durationMs?: number | null } = {}) {
  return render(
    <MemoryRouter>
      <VerdictBand run={detail} connection="connected" {...props} />
    </MemoryRouter>,
  );
}

// Mirrors the self-verification VD's `{role: heading, text: pass}`:
// Playwright's `role=heading >> internal:has-text="pass"i` — a heading
// whose text contains the needle, case-insensitively.
function headingWithText(needle: RegExp): HTMLElement | undefined {
  return screen
    .queryAllByRole("heading")
    .find((heading) => needle.test(heading.textContent ?? ""));
}

describe("verdict band derivations", () => {
  it("names the verdict as a word with its tone", () => {
    expect(verdictWord("pass", "finished")).toEqual({ word: "Passed", tone: "pass" });
    expect(verdictWord("fail", "finished")).toEqual({ word: "Failed", tone: "fail" });
    expect(verdictWord("inconclusive:timeout", "finished")).toEqual({ word: "Inconclusive", tone: "inconclusive" });
    expect(verdictWord(null, "running")).toEqual({ word: "Running", tone: "live" });
    // A running run never shows a stale or partial verdict as final.
    expect(verdictWord("fail", "running")).toEqual({ word: "Running", tone: "live" });
    expect(verdictWord(null, "aborted")).toEqual({ word: "Aborted", tone: "none" });
    expect(verdictWord(null, "finished")).toEqual({ word: "No verdict", tone: "none" });
  });

  it("tallies passing criteria", () => {
    expect(criteriaTally(FAILING)).toBe("1 of 4 criteria pass");
    expect(criteriaTally(run())).toBe("2 of 2 criteria pass");
    expect(criteriaTally(run({ criteria: [{ id: "AC-1", verdict: "fail", checks: [] }] }))).toBe(
      "0 of 1 criterion passes",
    );
    expect(criteriaTally(run({ criteria: [] }))).toBe("No criteria recorded");
  });

  it("finds the first failed check in criteria order, skipping non-fail verdicts", () => {
    expect(firstFailedCheck(FAILING)).toEqual({ criterionId: "AC-5", checkId: "AC-5.2" });
  });

  it("finds no failed check on a run without one", () => {
    expect(firstFailedCheck(run())).toBeNull();
    expect(
      firstFailedCheck(run({ verdict: "inconclusive:timeout", criteria: FAILING.criteria.slice(0, 1) })),
    ).toBeNull();
  });

  it("splits the first failed assertion into expected and observed", () => {
    expect(firstFailedAssertion(FAILED_CHECK)).toEqual({
      expr: "$steps.b.outputs.status == 201",
      expected: "201",
      observed: "500",
      full: "actual 500, expected 201",
    });
    // The semantic shape the check page also splits.
    expect(
      firstFailedAssertion(check([assertion("fail", 'expected text "Manager" to be absent within 5s, but 1 still matched')])),
    ).toMatchObject({ expected: 'text "Manager" to be absent within 5s', observed: "1 still matched" });
  });

  it("truncates each side at about 80 characters and keeps the full detail", () => {
    const long = `"${"x".repeat(200)}"`;
    const failure = firstFailedAssertion(check([assertion("fail", `actual ${long}, expected "short"`)]));
    expect(failure?.expected).toBe('"short"');
    expect(failure?.observed).toBe(`${long.slice(0, 80)}…`);
    expect(failure?.full).toBe(`actual ${long}, expected "short"`);
    const reason = firstFailedAssertion(check([assertion("fail", "y".repeat(120))]));
    expect(reason?.expected).toBeUndefined();
    expect(reason?.reason).toBe(`${"y".repeat(80)}…`);
  });

  it("returns null for a check detail without a timeline", () => {
    expect(firstFailedAssertion({ ...check([]), timeline: undefined } as unknown as CheckDetail)).toBeNull();
  });

  it("returns null when no assertion failed", () => {
    expect(firstFailedAssertion(check([assertion("pass", "actual 1, expected 1")]))).toBeNull();
  });

  it("reads the duration from the runs list, including run-set children", () => {
    const entry = (run_id: string, children?: RunsListEntry[]): RunsListEntry => ({
      run_id,
      verification: "v",
      started_at: null,
      duration_ms: run_id === "child" ? 42 : 1,
      verdict: null,
      kind: children ? "run-set" : "leaf",
      status: "finished",
      children,
    });
    expect(findRunEntry([entry("a"), entry("set", [entry("child")])], "child")?.duration_ms).toBe(42);
    expect(findRunEntry(null, "a")).toBeNull();
    expect(finishedAt("2026-07-22T14:03:00.000Z", 1500)?.toISOString()).toBe("2026-07-22T14:03:01.500Z");
    expect(finishedAt("2026-07-22T14:03:00.000Z", null)).toBeNull();
  });
});

describe("verdict band timestamps", () => {
  it("stamps a fixed UTC minute whatever the input offset", () => {
    expect(utcStamp("2026-09-27T14:24:59.900Z")).toEqual({
      text: "2026-09-27 14:24 UTC",
      iso: "2026-09-27T14:24:59.900Z",
    });
    expect(utcStamp("2026-09-27T23:24:00+09:00")?.text).toBe("2026-09-27 14:24 UTC");
    expect(utcStamp(null)).toBeNull();
    expect(utcStamp("not a date")).toBeNull();
  });

  it("shows the finished time in UTC with the full ISO timestamp in the title", () => {
    stubFetch();
    renderBand(run(), { durationMs: 1234 });
    const time = screen.getByTestId("verdict-time");
    expect(time.textContent).toBe("finished 2026-07-22 14:03 UTC");
    const stamp = time.querySelector("time");
    expect(stamp?.getAttribute("title")).toBe("2026-07-22T14:03:01.234Z");
    expect(stamp?.getAttribute("datetime")).toBe("2026-07-22T14:03:01.234Z");
  });

  it("falls back to the started time in the same format", () => {
    stubFetch();
    renderBand(run({ verdict: null, status: "running" }), { durationMs: null });
    const time = screen.getByTestId("verdict-time");
    expect(time.textContent).toBe("started 2026-07-22 14:03 UTC");
    expect(time.querySelector("time")?.getAttribute("title")).toBe("2026-07-22T14:03:00.000Z");
  });
});

describe("VerdictBand", () => {
  it("renders a pass at display size inside the heading", () => {
    stubFetch();
    renderBand(run(), { durationMs: 1234 });
    const word = screen.getByTestId("verdict-word");
    expect(word.textContent).toBe("Passed");
    expect(word.className).toContain("text-pass");
    expect(word.className).toContain("text-3xl");
    // The VD's `{role: heading, text: pass}` still resolves.
    expect(headingWithText(/pass/i)).toBeTruthy();
    expect(headingWithText(/pass/i)?.contains(word)).toBe(true);
    const mark = screen.getByTestId("verdict-band").querySelector("svg");
    expect(mark?.getAttribute("data-verdict")).toBe("pass");
    expect(mark?.getAttribute("class")).toContain("size-10");
    const summary = screen.getByTestId("verdict-summary");
    expect(summary.textContent).toContain("2 of 2 criteria pass");
    expect(summary.textContent).toContain("took 1.2s");
    expect(summary.textContent).toContain("finished");
    expect(screen.queryByTestId("first-failure")).toBeNull();
  });

  it("renders a fail with the first failure linked to its check", async () => {
    const fetchMock = stubFetch();
    renderBand(FAILING);
    const word = screen.getByTestId("verdict-word");
    expect(word.textContent).toBe("Failed");
    expect(word.className).toContain("text-fail");
    const line = screen.getByTestId("first-failure");
    expect(line.getAttribute("href")).toBe(`/run/R1/check/${encodeURIComponent("AC-5::AC-5.2")}`);
    expect(line.textContent).toContain("AC-5.2");
    await waitFor(() => expect(screen.getByTestId("first-failure-expected").textContent).toBe("201"));
    expect(screen.getByTestId("first-failure-observed").textContent).toBe("500");
    expect(line.getAttribute("title")).toBe("actual 500, expected 201");
    // The detail is fetched for the first failed check only.
    const checkCalls = fetchMock.mock.calls.filter(([url]) => String(url).includes("/checks/"));
    expect(checkCalls.map(([url]) => String(url))).toEqual([
      `api/runs/R1/checks/${encodeURIComponent("AC-5::AC-5.2")}.json`,
    ]);
    // The first-failure line sits outside the heading, so the heading's
    // accessible name stays the verdict and the run.
    expect(within(screen.getByRole("heading")).queryByTestId("first-failure")).toBeNull();
  });

  it("still names the failed check when its detail cannot be read", async () => {
    // A payload without a timeline, as a stub that answers every URL
    // with the run detail returns: no pair, no unhandled error.
    stubFetch(FAILING as unknown as CheckDetail);
    renderBand(FAILING);
    const line = screen.getByTestId("first-failure");
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(line.textContent).toBe("First failureAC-5.2");
    expect(screen.queryByTestId("first-failure-expected")).toBeNull();
  });

  it("renders an inconclusive run with its cause and no failure line", () => {
    stubFetch();
    renderBand(
      run({
        verdict: "inconclusive:environment_error",
        criteria: [{ id: "AC-7", verdict: "inconclusive:environment_error", checks: [{ id: "AC-7.1", verdict: "inconclusive:environment_error" }] }],
      }),
    );
    const word = screen.getByTestId("verdict-word");
    expect(word.textContent).toBe("Inconclusive");
    expect(word.className).toContain("text-inconclusive");
    expect(screen.getByRole("heading").textContent).toContain("environment_error");
    expect(screen.queryByTestId("first-failure")).toBeNull();
  });

  it("renders a running run in the live tone, without a duration", () => {
    stubFetch();
    renderBand(run({ verdict: null, status: "running" }), { durationMs: 900 });
    const word = screen.getByTestId("verdict-word");
    expect(word.textContent).toBe("Running");
    expect(word.className).toContain("text-live");
    expect(screen.getByRole("heading").textContent).toContain("connected");
    expect(screen.getByTestId("verdict-band").querySelector("svg")?.getAttribute("data-verdict")).toBe("running");
    const summary = screen.getByTestId("verdict-summary").textContent ?? "";
    expect(summary).not.toContain("took");
    expect(summary).toContain("started");
  });

  it("collapses to a single line that keeps the verdict in the heading", () => {
    stubFetch();
    renderBand(FAILING, { collapsed: true, durationMs: 1234 });
    expect(screen.getByTestId("verdict-band").getAttribute("data-collapsed")).toBe("true");
    expect(screen.queryByTestId("verdict-summary")).toBeNull();
    expect(screen.queryByTestId("first-failure")).toBeNull();
    expect(screen.getByRole("heading", { name: /^Failed\s*pegasus-register\s*R1$/ })).toBeTruthy();
    expect(screen.getByTestId("verdict-word").className).toContain("text-base");
    expect(screen.getByTestId("verdict-band").querySelector("svg")?.getAttribute("class")).toContain("size-6");
  });
});

describe("RunScaffold verdict band", () => {
  function scroll(el: Element, top: number, runway = 400) {
    Object.defineProperty(el, "scrollTop", { configurable: true, value: top });
    Object.defineProperty(el, "clientHeight", { configurable: true, value: 300 });
    Object.defineProperty(el, "scrollHeight", { configurable: true, value: 300 + runway });
    act(() => {
      fireEvent.scroll(el);
    });
  }

  function renderScaffold() {
    return render(
      <MemoryRouter>
        <RunsProvider>
          <RunScaffold runId="R1" activeResults>
            {() => <div data-testid="pane-body">body</div>}
          </RunScaffold>
        </RunsProvider>
      </MemoryRouter>,
    );
  }

  it("collapses when a Results pane scrolls and expands back at the top", async () => {
    stubFetch();
    const { container } = renderScaffold();
    const band = await screen.findByTestId("verdict-band");
    expect(band.getAttribute("data-collapsed")).toBe("false");
    const pane = container.querySelector(".run-results-detail")!;
    scroll(pane, 10);
    expect(screen.getByTestId("verdict-band").getAttribute("data-collapsed")).toBe("false");
    scroll(pane, 120);
    expect(screen.getByTestId("verdict-band").getAttribute("data-collapsed")).toBe("true");
    // Hysteresis: partway back up stays collapsed; only the top expands.
    scroll(pane, 10);
    expect(screen.getByTestId("verdict-band").getAttribute("data-collapsed")).toBe("true");
    scroll(pane, 0);
    expect(screen.getByTestId("verdict-band").getAttribute("data-collapsed")).toBe("false");
  });

  it("does not collapse a pane too short to stay scrolled once it grows", async () => {
    stubFetch();
    const { container } = renderScaffold();
    await screen.findByTestId("verdict-band");
    scroll(container.querySelector(".run-results-detail")!, 60, 80);
    expect(screen.getByTestId("verdict-band").getAttribute("data-collapsed")).toBe("false");
  });

  it("leaves the failed check's id naming only its Results-tree link", async () => {
    // Duhem compiles `{role: link, name: X}` to Playwright's public
    // `role=link[name="X"]`, a whole-name, case-insensitive match. This
    // asserts the stricter case-insensitive *substring* match (what
    // `getByRole({ name })` does by default), which covers both.
    stubFetch();
    renderScaffold();
    const band = await screen.findByTestId("first-failure");
    await waitFor(() => expect(screen.getByTestId("first-failure-expected")).toBeTruthy());
    const named = screen.getAllByRole("link", {
      name: (accessibleName) => accessibleName.toLowerCase().includes("ac-5.2"),
    });
    expect(named).toHaveLength(1);
    expect(screen.getByTestId("run-tree").contains(named[0])).toBe(true);
    // The band link keeps the id visible and the detail in its title.
    expect(screen.getByRole("link", { name: "Open first failed check" })).toBe(band);
    expect(band.textContent).toContain("AC-5.2");
    expect(band.getAttribute("title")).toBe("actual 500, expected 201");
  });

  it("shows the duration recorded on the runs list", async () => {
    stubFetch(FAILED_CHECK, [
      {
        run_id: "R1",
        verification: "pegasus-register",
        started_at: "2026-07-22T14:03:00.000Z",
        duration_ms: 65_000,
        verdict: "fail",
        kind: "leaf",
        status: "finished",
      },
    ]);
    renderScaffold();
    await waitFor(() =>
      expect(screen.getByTestId("verdict-summary").textContent).toContain("took 1m 5s"),
    );
  });
});
