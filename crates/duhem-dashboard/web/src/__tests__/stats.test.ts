import { describe, expect, it } from "vitest";
import { computeStats, flatLeaves, verificationSummaries } from "../stats";
import type { RunsListEntry } from "../api";

function leaf(
  run_id: string,
  verification: string,
  verdict: string | null,
  started_at: string | null,
  live = false,
): RunsListEntry {
  return {
    run_id,
    verification,
    started_at,
    duration_ms: 1000,
    verdict,
    kind: "leaf",
    live,
  };
}

// A run-set (login, latest fails) + two standalone leaves, one live.
const entries: RunsListEntry[] = [
  {
    ...leaf("login", "login", "fail", "2026-06-10T10:00:00Z"),
    kind: "run-set",
    children: [
      leaf("r3", "login", "fail", "2026-06-10T10:00:00Z"),
      leaf("r2", "login", "pass", "2026-06-09T10:00:00Z"),
      leaf("r1", "login", "pass", "2026-06-08T10:00:00Z"),
    ],
  },
  leaf("api1", "api", "inconclusive:timeout", "2026-06-11T10:00:00Z"),
  leaf("live1", "checkout", null, "2026-06-12T10:00:00Z", true),
];

describe("flatLeaves", () => {
  it("expands run-sets to their children and keeps top-level leaves", () => {
    const leaves = flatLeaves(entries);
    expect(leaves.map((l) => l.run_id).sort()).toEqual([
      "api1",
      "live1",
      "r1",
      "r2",
      "r3",
    ]);
  });
});

describe("computeStats", () => {
  const s = computeStats(entries);

  it("tallies verdict families and live runs over the leaves", () => {
    expect(s.total).toBe(5);
    expect(s.pass).toBe(2);
    expect(s.fail).toBe(1);
    expect(s.inconclusive).toBe(1);
    expect(s.live).toBe(1);
  });

  it("computes pass rate over decided runs only (excludes the live run)", () => {
    // 2 pass / (2 pass + 1 fail + 1 inconclusive) = 0.5
    expect(s.passRate).toBe(0.5);
  });

  it("counts a verification as failing when its latest run failed", () => {
    // login's newest leaf (r3) failed; api is inconclusive; checkout is live.
    expect(s.verifications).toBe(3);
    expect(s.failingVerifications).toBe(1);
  });

  it("orders recent newest-first and trend oldest→newest", () => {
    expect(s.recent[0].run_id).toBe("live1");
    expect(s.trend[s.trend.length - 1]).toBe(null); // newest is the live run
  });
});

describe("verificationSummaries", () => {
  const summaries = verificationSummaries(entries);

  it("groups by verification, alphabetical, with latest + live", () => {
    expect(summaries.map((v) => v.name)).toEqual(["api", "checkout", "login"]);
    const login = summaries.find((v) => v.name === "login")!;
    expect(login.runs).toBe(3);
    expect(login.latest?.run_id).toBe("r3");
    expect(login.recent).toEqual(["pass", "pass", "fail"]); // oldest→newest
    const checkout = summaries.find((v) => v.name === "checkout")!;
    expect(checkout.live).toBe(true);
  });
});
