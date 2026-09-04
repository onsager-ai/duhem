import { describe, expect, it } from "vitest";

import type { CriterionDetail, TraceEvent } from "../api";
import { foldRun } from "../fold";
import {
  buildSuiteTree,
  countSuiteStatuses,
  filterSuiteTree,
  type SuiteStatus,
} from "../suite-tree";

const CRITERIA: CriterionDetail[] = [
  {
    id: "AC-1",
    verdict: "fail",
    checks: [
      { id: "AC-1.1", verdict: "fail" },
      { id: "AC-1.2", verdict: "pass" },
    ],
  },
  {
    id: "AC-2",
    verdict: "pass",
    checks: [{ id: "AC-2.1", verdict: "pass" }],
  },
  {
    id: "AC-3",
    verdict: "inconclusive:environment_error",
    checks: [{ id: "AC-3.1", verdict: "inconclusive:environment_error" }],
  },
  {
    id: "AC-4",
    verdict: "inconclusive:empty_aggregation",
    checks: [],
  },
];

describe("suite tree derivations", () => {
  it("groups recorded checks under criteria and preserves judge-propagated status", () => {
    const events: TraceEvent[] = [
      {
        seq: 1,
        ts: "2026-07-24T00:00:00.000Z",
        kind: "step_started",
        criterion_id: "AC-1",
        check_id: "AC-1.1",
      },
      {
        seq: 2,
        ts: "2026-07-24T00:00:00.010Z",
        kind: "check_finished",
        check_id: "AC-1.1",
        verdict: "fail",
      },
      {
        seq: 3,
        ts: "2026-07-24T00:00:00.020Z",
        kind: "criterion_finished",
        criterion_id: "AC-1",
        verdict: "fail",
      },
      {
        seq: 4,
        ts: "2026-07-24T00:00:00.030Z",
        kind: "criterion_finished",
        criterion_id: "AC-empty",
        verdict: "inconclusive:empty_aggregation",
      },
    ];

    const tree = buildSuiteTree(foldRun("R1", events).criteria);
    expect(tree.map((group) => group.criterion.id)).toEqual([
      "AC-1",
      "AC-empty",
    ]);
    expect(tree[0].status).toBe("fail");
    expect(tree[0].checks.map((node) => [node.check.id, node.status])).toEqual([
      ["AC-1.1", "fail"],
    ]);
    expect(tree[1]).toMatchObject({ status: "inconclusive", checks: [] });
  });

  it("counts check statuses using the run roll-up denominator", () => {
    expect(countSuiteStatuses(buildSuiteTree(CRITERIA))).toEqual({
      pass: 2,
      fail: 1,
      inconclusive: 1,
    });
  });

  it("counts the #488 step-less repro from check_finished ownership", () => {
    const events: TraceEvent[] = [
      ...[
        ["AC-1", "AC-1.1", "fail"],
        ["AC-2", "AC-2.1", "pass"],
        ["AC-3", "AC-3.1", "pass"],
      ].flatMap(([criterionId, checkId, verdict], index) => [
        {
          seq: index * 2,
          ts: `2026-07-24T00:00:0${index}.000Z`,
          kind: "check_finished",
          criterion_id: criterionId,
          check_id: checkId,
          verdict,
        },
        {
          seq: index * 2 + 1,
          ts: `2026-07-24T00:00:0${index}.010Z`,
          kind: "criterion_finished",
          criterion_id: criterionId,
          verdict,
        },
      ]),
    ];

    expect(countSuiteStatuses(buildSuiteTree(foldRun("R1", events).criteria))).toEqual({
      pass: 2,
      fail: 1,
      inconclusive: 0,
    });
  });

  it("filters by multiple statuses while retaining criterion context", () => {
    const filtered = filterSuiteTree(
      buildSuiteTree(CRITERIA),
      new Set<SuiteStatus>(["pass", "inconclusive"]),
    );

    expect(filtered.map((group) => group.criterion.id)).toEqual([
      "AC-1",
      "AC-2",
      "AC-3",
      "AC-4",
    ]);
    expect(filtered[0].contextOnly).toBe(true);
    expect(filtered[0].checks.map((node) => node.check.id)).toEqual([
      "AC-1.2",
    ]);
    expect(filtered[1].contextOnly).toBe(false);
    expect(filtered[3]).toMatchObject({ status: "inconclusive", checks: [] });
  });
});
