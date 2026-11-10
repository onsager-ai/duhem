import { cleanup, render } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import type { ClaimScope, TraceEvent } from "../api";
import { ClaimScopeNotice } from "../components/ClaimScopeNotice";
import { foldRun } from "../fold";

afterEach(cleanup);

const scope: ClaimScope = {
  criteria: { authored: 1, selected: 1, executed: 0 },
  checks: { authored: 2, selected: 1, executed: 0 },
  filters: ["AC::first"],
  excluded: [{ criterion_id: "AC", check_id: "second", reason: "check_filter" }],
  manifest: { authored: 2, selected: 1, excluded: [{ verification: "beta.yml", reason: "directory_selection" }] },
};

it("keeps selected and recorded scope distinct, without counting retry outcomes twice", () => {
  const events: TraceEvent[] = [
    { seq: 0, ts: "t", kind: "claim_scope_declared", scope },
    { seq: 1, ts: "t", kind: "check_finished", criterion_id: "AC", check_id: "first", verdict: "inconclusive:timeout" },
    { seq: 2, ts: "t", kind: "check_finished", criterion_id: "AC", check_id: "first", verdict: "pass" },
  ];
  const before = foldRun("r", events.slice(0, 1));
  const after = foldRun("r", events);
  expect(before.claim_scope?.checks.executed).toBe(0);
  expect(after.claim_scope?.checks.executed).toBe(1);
  expect(scope.checks.executed).toBe(0);
  const view = render(<ClaimScopeNotice scope={after.claim_scope} />);
  expect(view.container.textContent).toContain("1 of 2 checks");
  expect(view.container.textContent).toContain("AC::second");
  expect(view.container.textContent).toContain("1 of 2 manifest leaves");
  expect(view.container.textContent).toContain("beta.yml");
});

it("does not infer scope for legacy traces", () => {
  expect(foldRun("r", []).claim_scope).toBeUndefined();
  const view = render(<ClaimScopeNotice />);
  expect(view.container.innerHTML).toBe("");
});
