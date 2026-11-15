import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { RunDetail } from "../api";
import { carryFetched, foldRun } from "../fold";
import { RunSummary } from "../views/RunPage";

const run: RunDetail = {
  run_id: "root", verification: "native", started_at: null, inputs: {},
  verdict: "pass", status: "finished", setup_aborted: false, has_definition: false, criteria: [],
  native: [{
    run_id: "child", id: "rust-unit", layer: "unit", outcome: "fail", required: false,
    tests: 4, duration_ms: 1200, counts: { passed: 1, failed: 1, errored: 1, skipped: 1 },
    failing_tests: [{ test_id: "rust-unit:test", name: "rejects invalid input", file: "unit.rs", message: "expected true" }],
    evidence: [{ id: "sha", kind: "report", url: "/api/runs/child/artifact/sha" }],
  }],
  layers: [{ layer: "unit", tests: 4, passed: 1, failed: 1, errored: 1, skipped: 1, inconclusive: 0, duration_ms: 1200 }],
};

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

function renderSummary(detail: RunDetail) {
  return render(<MemoryRouter><RunSummary run={detail} /></MemoryRouter>);
}

const parent = { ...run, children: [{ run_id: "child", verification: "native", verdict: "pass", status: "finished", duration_ms: 1200, check_totals: { total: 0, pass: 0, fail: 0, inconclusive: 0, pending: 0 } }] } satisfies RunDetail;

describe("native run navigation", () => {
  it("shows counts, failing tests, diagnostic status and evidence on a passing gate", () => {
    renderSummary(run);
    const section = screen.getByRole("region", { name: "Native runs" });
    expect(within(section).getByText("Diagnostic")).toBeTruthy();
    expect(within(section).getByText(/4 tests · 1 passed · 1 failed · 1 errored · 1 skipped/)).toBeTruthy();
    expect(within(section).getByText("1.2s")).toBeTruthy();
    const tests = within(section).getByRole("list", { name: "Failing tests for rust-unit" });
    expect(within(tests).getByText("rejects invalid input · unit.rs")).toBeTruthy();
    expect(within(tests).getByRole("link", { name: "report" }).getAttribute("href")).toBe("api/runs/child/artifact/sha");
    const table = screen.getByRole("table", { name: "Layer totals" });
    expect(within(table).getByRole("rowheader", { name: "unit" })).toBeTruthy();
    expect(within(table).getByRole("columnheader", { name: "Errored" })).toBeTruthy();
  });

  it("does not add native sections to historical runs", () => {
    renderSummary({ ...run, native: undefined, layers: undefined });
    expect(screen.queryByRole("region", { name: "Native runs" })).toBeNull();
  });

  it("keeps native data across live event folds", () => {
    const folded = carryFetched(foldRun("root", []), run);
    expect(folded.native).toEqual(run.native);
    expect(folded.layers).toEqual(run.layers);
  });

  it("lists parent native failures without claiming no failures", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response(JSON.stringify({
      run_id: "root", verdict: "pass", failing: [], failing_native: [{
        run_id: "child", native_id: "rust-unit", required: false, outcome: "fail", failing_tests: [], evidence: run.native![0].evidence,
      }],
    }))));
    renderSummary(parent);
    const failures = await screen.findByRole("list", { name: "Native failures" });
    expect(within(failures).getByRole("link", { name: "rust-unit" }).getAttribute("href")).toBe("/run/child");
    expect(screen.queryByText("No failures recorded.")).toBeNull();
  });

  it("shows loading and error states for the parent failure envelope", async () => {
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new Error("offline")));
    renderSummary(parent);
    expect(screen.getByText("Loading failures…")).toBeTruthy();
    await waitFor(() => expect(screen.getByText("Failures could not be loaded.")).toBeTruthy());
    expect(screen.queryByText("No failures recorded.")).toBeNull();
  });
});
