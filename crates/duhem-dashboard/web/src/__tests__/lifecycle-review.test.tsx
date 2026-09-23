import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, useLocation } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CheckDetail, LifecycleBlock, LifecycleScopeSegment, RunDetail, TraceEvent } from "../api";
import appSource from "../App.tsx?raw";
import { LifecycleRailRow } from "../components/LifecycleRailRow";
import { DefinitionProvider } from "../views/definition-context";
import { formatEvent } from "../format";
import { LifecycleSections, Timeline } from "../views/CheckPage";
import { RunTree } from "../views/RunScaffold";

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

function block(seq: number, phase: "setup" | "teardown", scope: LifecycleScopeSegment[], outcome: "ok" | "error" = "ok"): LifecycleBlock {
  const fields = Object.fromEntries(scope.map(({ kind, id }) => [kind === "fixture" ? "fixture_name" : `${kind}_id`, id]));
  const evt = (offset: number, kind: string, extra: Record<string, unknown> = {}): TraceEvent => ({
    seq: seq + offset, ts: `2026-01-01T00:00:00.00${offset}Z`, kind,
    ...(phase === "teardown" ? { phase } : {}), ...fields, ...extra,
  });
  return {
    phase, scope, status: outcome === "error" ? "failed" : "passed",
    started_at: "2026-01-01T00:00:00.000Z", duration_ms: 2,
    steps: [{ index: 0, uses: "db/query", outcome, duration_ms: 1 }],
    timeline: [
      evt(0, "setup_started"),
      evt(1, "setup_step_started", { step_index: 0, uses: "db/query", layer: "db", with: { sql: "select 1" } }),
      evt(2, "setup_step_finished", { step_index: 0, outcome }),
      evt(3, "setup_finished", { aborted: false }),
    ],
  };
}

const criterion = { kind: "criterion", id: "AC-1" };
const check = { kind: "check", id: "AC-1.1" };
const fixture = { kind: "fixture", id: "database_session" };

const definition = `
setup: [{id: leaf-ready, uses: db/query}]
teardown: [{id: leaf-close, uses: db/query}]
fixtures:
  database_session:
    up: [{id: fixture-open, uses: db/query}]
    down: [{id: fixture-close, uses: db/query}]
criteria:
  - id: AC-1
    setup: [{id: criterion-ready, uses: db/query}]
    teardown: [{id: criterion-close, uses: db/query}]
    checks:
      - id: AC-1.1
        setup: [{uses: db/query}]
        teardown: [{id: check-close, uses: db/query}]
        steps: [{id: inspect, uses: db/query}]
`;

function Location() {
  const location = useLocation();
  return <output data-testid="location">{location.pathname}{location.search}</output>;
}

describe("#558 review regressions", () => {
  it("labels lifecycle steps from their own authored block, including a check setup without id", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response(definition, { status: 200 })));
    const blocks = [
      block(1, "setup", []), block(10, "teardown", []),
      block(20, "setup", [criterion]), block(30, "teardown", [criterion]),
      block(40, "setup", [criterion, check]), block(50, "teardown", [criterion, check]),
      block(60, "setup", [check, fixture]), block(70, "teardown", [check, fixture]),
    ];
    render(<DefinitionProvider runId="r1" enabled><LifecycleSections blocks={blocks} /></DefinitionProvider>);
    await waitFor(() => expect(screen.getAllByTestId("step-group").map((row) => row.querySelector(".ev-label")?.textContent)).toEqual([
      "leaf-ready", "leaf-close", "criterion-ready", "criterion-close",
      "db/query #0", "check-close", "fixture-open", "fixture-close",
    ]));
  });

  it("uses teardown markers and treats a missing phase as setup", () => {
    expect(formatEvent({ seq: 1, ts: "2026-01-01T00:00:00Z", kind: "setup_started", phase: "teardown" }).label).toBe("teardown started");
    expect(formatEvent({ seq: 2, ts: "2026-01-01T00:00:01Z", kind: "setup_finished", phase: "teardown" }).label).toBe("teardown finished");
    expect(formatEvent({ seq: 3, ts: "2026-01-01T00:00:02Z", kind: "setup_started" }).label).toBe("setup started");
  });

  it("keeps fixture phase visible before the truncating name in the rail", () => {
    render(<MemoryRouter><LifecycleRailRow runId="r1" block={block(1, "setup", [check, fixture])} /></MemoryRouter>);
    const row = screen.getByTestId("lifecycle-rail-row");
    expect(row.querySelector("span.shrink-0")?.textContent).toBe("setup");
    expect(row.querySelector("span.truncate")?.textContent).toBe("fixture database_session");
  });

  it("keeps an error step's chip, summary, outcome and duration in the five-column row", () => {
    const failure = block(1, "teardown", [], "error");
    render(<Timeline events={failure.timeline} />);
    const row = screen.getByTestId("step-group").querySelector(".ev-row")!;
    expect(row.children).toHaveLength(5);
    expect(row.querySelector(".ev-detail [data-testid='step-layer']")).not.toBeNull();
    expect(row.querySelector(".ev-detail [data-testid='step-outcome']")).not.toBeNull();
    expect(row.querySelector("[data-testid='step-time']")).not.toBeNull();
  });

  it("clears lifecycle selection when a step row is clicked", async () => {
    const run: RunDetail = {
      run_id: "r1", verification: "example", started_at: null, inputs: {}, verdict: "pass",
      status: "finished", setup_aborted: false, has_definition: false,
      lifecycle: [block(20, "setup", [criterion, check])],
      criteria: [{ id: "AC-1", verdict: "pass", checks: [{ id: "AC-1.1", verdict: "pass" }] }],
    };
    const checkDetail: CheckDetail = {
      criterion_id: "AC-1", check_id: "AC-1.1", verdict: "pass", spans: [], artifacts: [],
      timeline: [0, 1].flatMap((index) => [
        { seq: index * 2 + 100, ts: "2026-01-01T00:00:01Z", kind: "step_started", criterion_id: "AC-1", check_id: "AC-1.1", step_index: index, uses: "db/query" },
        { seq: index * 2 + 101, ts: "2026-01-01T00:00:02Z", kind: "step_finished", step_index: index, outcome: "ok" },
      ]),
    };
    vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify(checkDetail), { status: 200 })));
    render(<MemoryRouter initialEntries={["/run/r1/check/AC-1%3A%3AAC-1.1?lifecycle=20"]}>
      <RunTree run={run} activePair="AC-1::AC-1.1" /><Location />
    </MemoryRouter>);
    fireEvent.click(await screen.findByRole("link", { name: "db/query #0" }));
    expect(screen.getByTestId("location").textContent).toBe("/run/r1/check/AC-1%3A%3AAC-1.1?step=0");
  });

  it("clears lifecycle selection when arrowing between steps", async () => {
    const run: RunDetail = {
      run_id: "r1", verification: "example", started_at: null, inputs: {}, verdict: "pass",
      status: "finished", setup_aborted: false, has_definition: false,
      lifecycle: [block(20, "setup", [criterion, check])],
      criteria: [{ id: "AC-1", verdict: "pass", checks: [{ id: "AC-1.1", verdict: "pass" }] }],
    };
    const detail: CheckDetail = {
      criterion_id: "AC-1", check_id: "AC-1.1", verdict: "pass", spans: [], artifacts: [],
      timeline: [0, 1].flatMap((index) => [
        { seq: index * 2 + 100, ts: "2026-01-01T00:00:01Z", kind: "step_started", criterion_id: "AC-1", check_id: "AC-1.1", step_index: index, uses: "db/query" },
        { seq: index * 2 + 101, ts: "2026-01-01T00:00:02Z", kind: "step_finished", step_index: index, outcome: "ok" },
      ]),
    };
    vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify(detail), { status: 200 })));
    render(<MemoryRouter initialEntries={["/run/r1/check/AC-1%3A%3AAC-1.1?lifecycle=20&step=0"]}>
      <RunTree run={run} activePair="AC-1::AC-1.1" /><Location />
    </MemoryRouter>);
    fireEvent.keyDown(await screen.findByRole("link", { name: "db/query #0" }), { key: "ArrowDown" });
    expect(screen.getByTestId("location").textContent).toBe("/run/r1/check/AC-1%3A%3AAC-1.1?step=1");
  });

  it("opens a lifecycle row without carrying an old step selection", () => {
    render(<MemoryRouter initialEntries={["/run/r1/check/AC-1%3A%3AAC-1.1?step=0"]}>
      <LifecycleRailRow runId="r1" checkPath="/run/r1/check/AC-1%3A%3AAC-1.1" block={block(20, "setup", [criterion, check])} />
      <Location />
    </MemoryRouter>);
    fireEvent.click(screen.getByTestId("lifecycle-rail-row"));
    expect(screen.getByTestId("location").textContent).toBe("/run/r1/check/AC-1%3A%3AAC-1.1?lifecycle=20");
  });

  it("keeps route components statically imported", () => {
    expect(appSource).not.toContain("lazy(");
    expect(appSource).not.toContain("<Suspense");
  });
});
