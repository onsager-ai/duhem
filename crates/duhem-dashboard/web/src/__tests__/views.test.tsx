// Component tests (#86): runs list (empty / one / many), timeline
// ordering, artifact rendering.

import type { ReactElement } from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import RunsList, { matchesFilters } from "../views/RunsList";
import { RunsProvider } from "../runs-context";
import { Artifacts, DomViewer, HarTable, ScreenshotArtifact, Timeline } from "../views/CheckPage";
import type { RunsListEntry, TraceEvent } from "../api";

// RunsList now reads the shared runs context; the provider fetches via the
// stubbed global fetch, so wrapping it feeds the same entries.
function renderRuns(ui: ReactElement) {
  return render(
    <MemoryRouter>
      <RunsProvider>{ui}</RunsProvider>
    </MemoryRouter>,
  );
}

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
    renderRuns(<RunsList />);
    await waitFor(() => expect(screen.getByText(/No runs/)).toBeTruthy());
  });

  it("renders a single leaf row", async () => {
    stubRuns([leaf("01JRUNA", "pass")]);
    const { container } = renderRuns(<RunsList />);
    await waitFor(() => expect(screen.getByText("01JRUNA")).toBeTruthy());
    // The verdict renders as a shadcn Badge carrying the verdict text.
    expect(container.querySelector('[data-slot="badge"]')?.textContent).toBe(
      "pass",
    );
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
    const { container } = renderRuns(<RunsList />);
    // Run-set children are expanded by default (#49).
    await waitFor(() => expect(screen.getByText("01JRUNA")).toBeTruthy());
    expect(screen.getByText("01JRUNB")).toBeTruthy();
    expect(screen.getByText("01JRUNC")).toBeTruthy();
    // The in-progress run shows a "live" verdict badge — distinct from the
    // "live" filter chip in the toolbar, so scope to badge elements.
    const badges = [...container.querySelectorAll('[data-slot="badge"]')];
    expect(badges.some((b) => b.textContent?.trim() === "live")).toBe(true);
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
        step_index: 0,
        uses: "ui/navigate",
        layer: "ui",
        with: { url: "http://x/" },
      },
      {
        seq: 45,
        ts: "2026-01-01T00:00:00.500Z",
        kind: "step_finished",
        step_index: 0,
        outcome: "ok",
      },
      {
        seq: 5,
        ts: "2026-01-01T00:00:01.000Z",
        kind: "assertion_evaluated",
        check_id: "AC-1.1",
        assertion_index: 0,
        state: "fail",
        detail: "actual false, expected true",
        expr: "$steps.load.outputs.ok == true",
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
    // The step folds into a group; check-level events stay standalone,
    // legible labels in trace order. The step's full detail (started +
    // finished, each with its raw toggle) lives inside the group, so
    // we scope the top-level assertion to rows outside `.step-inner`.
    expect(container.querySelectorAll('[data-testid="step-group"]')).toHaveLength(1);
    const labels = [...container.querySelectorAll(".ev-label")]
      .filter((el) => !el.closest(".step-inner"))
      .map((el) => el.textContent);
    expect(labels).toEqual(["navigate", "assertion failed", "verdict: fail"]);
    // The failing assertion row surfaces its recorded operands as an
    // expected/actual pair (not a raw sentence) and carries the fail tone.
    const cmp = container.querySelector(".ev.tone-fail [data-testid='assert-cmp']");
    expect(cmp?.textContent).toContain("false"); // actual
    expect(cmp?.textContent).toContain("true"); // expected
    // And it surfaces *what was asserted* — the recorded rule (#284).
    expect(
      container.querySelector(".ev.tone-fail [data-testid='assert-expr']")?.textContent,
    ).toContain("$steps.load.outputs.ok == true");
    // Raw JSON is preserved behind a per-row <details> toggle on the
    // standalone events.
    const raws = [...container.querySelectorAll(".ev-raw pre")];
    expect(raws.length).toBeGreaterThanOrEqual(2);
    expect(raws.some((p) => p.textContent?.includes("actual false"))).toBe(true);
  });

  it("propagates a failed implicit judgment onto its step — red, reason inline, auto-expanded (#280)", () => {
    const events: TraceEvent[] = [
      {
        seq: 1,
        ts: "2026-01-01T00:00:00.000Z",
        kind: "step_started",
        step_index: 0,
        uses: "ui/assert-element",
        layer: "ui",
        with: { locator: { text: "Manager" }, expected: "not_exists", within: "5s" },
      },
      { seq: 2, ts: "2026-01-01T00:00:00.100Z", kind: "step_observation", step_index: 0, output_name: "satisfied", value: false },
      { seq: 3, ts: "2026-01-01T00:00:00.150Z", kind: "step_observation", step_index: 0, output_name: "count", value: 1 },
      { seq: 4, ts: "2026-01-01T00:00:00.200Z", kind: "step_finished", step_index: 0, outcome: "ok" },
      {
        seq: 5,
        ts: "2026-01-01T00:00:00.300Z",
        kind: "assertion_evaluated",
        check_id: "AC-5.1",
        assertion_index: 0,
        state: "fail",
        detail: 'expected text "Manager" to be absent within 5s, but 1 still matched',
        step_index: 0,
      },
      { seq: 6, ts: "2026-01-01T00:00:00.400Z", kind: "check_finished", verdict: "fail" },
    ];
    const { container, getByTestId } = render(<Timeline events={events} />);
    const group = getByTestId("step-group");
    // The judging step reads *failed*, not a green "step ok".
    expect(group.className).toContain("tone-fail");
    expect(getByTestId("step-outcome").textContent).toContain("step failed");
    expect(getByTestId("step-outcome").textContent).not.toContain("step ok");
    // The semantic reason is surfaced inline on the step.
    expect(getByTestId("step-reason").textContent).toContain(
      'expected text "Manager" to be absent',
    );
    // Failed steps auto-expand (Allure-style).
    expect(group.querySelector("details")?.hasAttribute("open")).toBe(true);
    // The assertion is no longer a standalone orphan row — only the step
    // and the verdict remain at the top level.
    const topLabels = [...container.querySelectorAll(".ev-label")]
      .filter((e) => !e.closest(".step-inner"))
      .map((e) => e.textContent);
    expect(topLabels).toEqual(["assert-element", "verdict: fail"]);
  });

  it("nests a request/response inspector under an api step, response open on 5xx (#280 follow-up)", () => {
    const events: TraceEvent[] = [
      {
        seq: 1,
        ts: "2026-01-01T00:00:00.000Z",
        kind: "step_started",
        step_index: 0,
        uses: "api/call",
        layer: "api",
        with: { method: "PUT", url: "http://x/api/roles/1", body: { name: "r" } },
      },
      { seq: 2, ts: "2026-01-01T00:00:00.100Z", kind: "step_observation", step_index: 0, output_name: "status", value: 500 },
      {
        seq: 3,
        ts: "2026-01-01T00:00:00.150Z",
        kind: "step_observation",
        step_index: 0,
        output_name: "body",
        value: { error: "write exception: immutable field" },
      },
      { seq: 4, ts: "2026-01-01T00:00:00.200Z", kind: "step_finished", step_index: 0, outcome: "ok" },
    ];
    const { getByTestId } = render(<Timeline events={events} />);
    const ax = getByTestId("api-exchange");
    expect(ax.textContent).toContain("PUT");
    expect(ax.textContent).toContain("http://x/api/roles/1");
    expect(ax.querySelector(".ax-status")?.textContent).toBe("500");
    expect(ax.querySelector(".ax-status")?.className).toContain("bad");
    // The response body opens automatically on a 5xx — the error is right there.
    expect(ax.textContent).toContain("write exception: immutable field");
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

  it("renders capture/video inline as a <video> with native controls (#215)", () => {
    const vid = "f".repeat(64);
    const { container, getByTestId } = render(
      <Artifacts
        artifacts={[{ id: vid, kind: "capture/video", url: `run/r/artifact/${vid}` }]}
      />,
    );
    const video = getByTestId("capture-video") as HTMLVideoElement;
    expect(video.tagName).toBe("VIDEO");
    expect(video.getAttribute("src")).toBe(`run/r/artifact/${vid}`);
    expect(video.hasAttribute("controls")).toBe(true);
    // Kept out of the image path — it isn't a screenshot.
    expect(container.querySelectorAll("img")).toHaveLength(0);
    // Labelled "Video", not the raw kind.
    expect(container.textContent).toContain("Video");
  });

  it("renders capture/network as a HAR request table with failing status flagged (#206)", async () => {
    const har = {
      log: {
        entries: [
          { request: { method: "GET", url: "http://x/" }, response: { status: 200 } },
          { request: { method: "POST", url: "http://x/api/charge" }, response: { status: 500 } },
        ],
      },
    };
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify(har), { status: 200 })),
    );
    render(<HarTable url="run/r/artifact/net" />);
    const table = await screen.findByTestId("har-table");
    const rows = table.querySelectorAll("tbody tr");
    expect(rows).toHaveLength(2);
    expect(rows[0].textContent).toContain("GET");
    expect(rows[1].className).toContain("har-bad");
    expect(rows[1].textContent).toContain("500");
  });

  it("survives a HAR blob with a non-array entries shape", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify({ log: {} }), { status: 200 })),
    );
    render(<HarTable url="run/r/artifact/empty" />);
    expect(await screen.findByText(/no requests recorded/i)).toBeTruthy();
  });
});

// ---- #210: in-page artifact inspection -----------------------------

describe("in-page inspection (#210)", () => {
  it("expands a HAR row to its redacted headers and bodies", async () => {
    const har = {
      log: {
        entries: [
          {
            request: {
              method: "POST",
              url: "http://x/api/charge",
              headers: [{ name: "authorization", value: "<redacted>" }],
              postData: { text: "<redacted>" },
            },
            response: {
              status: 500,
              headers: [{ name: "content-type", value: "application/json" }],
              content: { text: '{"error":"declined"}' },
            },
          },
        ],
      },
    };
    vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify(har), { status: 200 })));
    render(<HarTable url="run/r/artifact/net" />);
    const row = await screen.findByTestId("har-row");
    // Collapsed: the body is not shown yet, and it's keyboard-focusable.
    expect(screen.queryByText(/"error":"declined"/)).toBeNull();
    expect(row.getAttribute("role")).toBe("button");
    expect(row.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(row);
    expect(row.getAttribute("aria-expanded")).toBe("true");
    // Expanded: redacted header + response body are revealed from the blob.
    expect(screen.getByText("authorization")).toBeTruthy();
    expect(screen.getByText('{"error":"declined"}')).toBeTruthy();
    expect(screen.getAllByText("<redacted>").length).toBeGreaterThanOrEqual(1);
  });

  it("renders a captured DOM snapshot with source search; the render is collapsed by default", async () => {
    const html = "<html><body><button>Pay now</button><div>Payment complete</div></body></html>";
    vi.stubGlobal("fetch", vi.fn(async () => new Response(html, { status: 200 })));
    render(<DomViewer url="run/r/artifact/dom" />);
    const viewer = await screen.findByTestId("dom-viewer");
    // The heavy iframe is collapsed by default; search is always available.
    expect(viewer.querySelector("iframe")).toBeNull();
    fireEvent.change(viewer.querySelector("input")!, { target: { value: "Payment complete" } });
    expect(screen.getByTestId("dom-matches").textContent).toContain("1 match");
    // Reveal the rendered snapshot — fully sandboxed (untrusted content).
    fireEvent.click(screen.getByTestId("dom-render-toggle"));
    const frame = viewer.querySelector("iframe");
    expect(frame?.getAttribute("sandbox")).toBe("");
    expect(frame?.getAttribute("srcdoc")).toContain("Payment complete");
  });

  it("renders an image artifact as a thumbnail that expands to full size on click", () => {
    const art = { id: "e".repeat(64), kind: "capture/screenshot", url: "run/r/artifact/shot" };
    render(<ScreenshotArtifact artifact={art} />);
    const btn = screen.getByTestId("shot-toggle");
    // Collapsed (thumbnail) by default.
    expect(btn.getAttribute("aria-expanded")).toBe("false");
    expect(btn.className).toContain("shot-collapsed");
    fireEvent.click(btn);
    // Expanded (full size) after click.
    expect(btn.getAttribute("aria-expanded")).toBe("true");
    expect(btn.className).toContain("shot-expanded");
    expect(btn.querySelector("img")?.getAttribute("src")).toBe("run/r/artifact/shot");
  });

  it("groups a step's events into a node and keeps the verdict standalone", () => {
    const events: TraceEvent[] = [
      { seq: 1, ts: "t1", kind: "step_started", step_index: 0, uses: "ui/navigate", with: { url: "http://x/" } },
      { seq: 2, ts: "t2", kind: "step_observation", step_index: 0, output_name: "count", value: 1 },
      { seq: 3, ts: "t3", kind: "step_finished", step_index: 0, outcome: "ok" },
      { seq: 4, ts: "t4", kind: "check_finished", verdict: "pass" },
    ];
    const { container } = render(<Timeline events={events} />);
    const groups = container.querySelectorAll('[data-testid="step-group"]');
    expect(groups).toHaveLength(1);
    expect(groups[0].textContent).toContain("navigate");
    expect(groups[0].textContent).toContain("step ok");
    // The step's own started/finished raws stay reachable inside the group.
    const innerRaw = groups[0].querySelectorAll(".step-inner .ev-raw pre");
    expect([...innerRaw].some((p) => p.textContent?.includes("ui/navigate"))).toBe(true);
    // The verdict is a standalone row, never folded into a step group.
    const labels = [...container.querySelectorAll(".ev-label")]
      .filter((e) => !e.closest(".step-inner"))
      .map((e) => e.textContent);
    expect(labels).toContain("verdict: pass");
  });
});

// ---- #214: element-highlight overlay --------------------------------

describe("element-highlight (#214)", () => {
  it("notes an absent target from capture/target-rect", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        new Response(
          JSON.stringify([
            { selector: 'role=button[name="Sign in with SSO"]', expected: "visible", found: false },
          ]),
          { status: 200 },
        ),
      ),
    );
    render(
      <ScreenshotArtifact
        artifact={{ id: "a".repeat(64), kind: "capture/screenshot", url: "run/r/artifact/shot" }}
        rectsUrl="run/r/artifact/rect"
      />,
    );
    const note = await screen.findByTestId("target-note");
    expect(note.textContent).toContain("Sign in with SSO");
  });

  it("consumes capture/target-rect as an overlay, not a listed artifact row", () => {
    const { container } = render(
      <Artifacts
        artifacts={[
          { id: "s".repeat(64), kind: "capture/screenshot", url: "u/shot" },
          { id: "t".repeat(64), kind: "capture/target-rect", url: "u/rect" },
        ]}
      />,
    );
    expect(container.textContent).not.toContain("capture/target-rect");
    expect(container.querySelectorAll(".artifact")).toHaveLength(1);
  });

  it("lists capture/target-rect as its own row when there's no screenshot to overlay", () => {
    const { container } = render(
      <Artifacts artifacts={[{ id: "t".repeat(64), kind: "capture/target-rect", url: "u/rect" }]} />,
    );
    // Not lost: rendered as its own artifact row (friendly label).
    expect(container.querySelectorAll(".artifact")).toHaveLength(1);
    expect(container.textContent).toContain("Target highlight");
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
    // The broken layer carries its detail inline (the ✕ is now a lucide
    // SVG, so it's a fail-classed node rather than glyph text).
    const failNode = chain.querySelector(".span-fail");
    expect(failNode?.textContent).toContain("data");
    expect(failNode?.textContent).toContain("timeout");
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

// ---- #280: run status roll-up (donut + counts) ----------------------

import { StatusDonut, tallyChecks } from "../views/RunPage";

describe("run status roll-up (#280)", () => {
  const crit = (id: string, checks: { id: string; verdict: string | null }[]) => ({
    id,
    verdict: null,
    checks,
  });

  it("tallies check verdicts across criteria, bucketing pending and inconclusive", () => {
    const t = tallyChecks([
      crit("AC-1", [
        { id: "a", verdict: "pass" },
        { id: "b", verdict: "fail" },
      ]),
      crit("AC-2", [
        { id: "c", verdict: "inconclusive:timeout" },
        { id: "d", verdict: null },
      ]),
    ]);
    expect(t).toEqual({ pass: 1, fail: 1, inconclusive: 1, pending: 1, total: 4 });
  });

  it("renders a donut with the total in the middle, one arc per non-zero status", () => {
    const { getByTestId } = render(
      <StatusDonut tally={{ pass: 3, fail: 1, inconclusive: 0, pending: 0, total: 4 }} />,
    );
    const summary = getByTestId("status-summary");
    expect(getByTestId("count-pass").textContent).toContain("3");
    expect(getByTestId("count-fail").textContent).toContain("1");
    expect(summary.querySelector(".donut-center")?.textContent).toBe("4");
    // Only pass + fail are non-zero → two arcs drawn.
    expect(summary.querySelectorAll(".donut-seg")).toHaveLength(2);
  });
});
