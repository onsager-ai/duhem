// Pure report-tree derivations. Verdicts stay authoritative: check status
// comes from the recorded check verdict and criterion status comes from the
// judge's recorded criterion propagation (including zero-check criteria).

import type { CheckRef, CriterionDetail } from "./api";
import type { Family } from "./stats";
import { countVerdictFamilies } from "./stats";
import { verdictFamily } from "./ui";

export const SUITE_STATUSES = ["pass", "fail", "inconclusive"] as const;
export type SuiteStatus = (typeof SUITE_STATUSES)[number];

export interface SuiteCheckNode {
  check: CheckRef;
  status: Family;
}

export interface SuiteCriterionNode {
  criterion: CriterionDetail;
  status: Family;
  checks: SuiteCheckNode[];
  /** True when retained only to give matching descendants their criterion. */
  contextOnly?: boolean;
}

export type SuiteStatusCounts = Record<SuiteStatus, number>;

export function buildSuiteTree(
  criteria: CriterionDetail[],
): SuiteCriterionNode[] {
  return criteria.map((criterion) => ({
    criterion,
    status: verdictFamily(criterion.verdict),
    checks: criterion.checks.map((check) => ({
      check,
      status: verdictFamily(check.verdict),
    })),
  }));
}

// The chips are the check-result facets (the same denominator as the run's
// check roll-up). Criterion rows are hierarchy and keep their own badges;
// a zero-check criterion remains explicit without pretending it ran a check.
export function countSuiteStatuses(
  tree: SuiteCriterionNode[],
): SuiteStatusCounts {
  return countVerdictFamilies(
    tree.flatMap((group) =>
      group.checks.map((node) => node.check.verdict),
    ),
  );
}

// Tree filters retain a non-matching criterion as hierarchy context when one
// of its checks matches. The criterion itself is marked context-only and is
// not part of the matching chip count.
export function filterSuiteTree(
  tree: SuiteCriterionNode[],
  active: ReadonlySet<SuiteStatus>,
): SuiteCriterionNode[] {
  if (active.size === 0) return tree;
  return tree.flatMap((group) => {
    const checks = group.checks.filter(
      (check) => check.status !== null && active.has(check.status),
    );
    // A criterion with no checks has no check facet to match. Match its
    // recorded criterion status so the absence/cause node stays filterable.
    const criterionStatusMatches =
      group.status !== null && active.has(group.status);
    const emptyCriterionMatches =
      group.checks.length === 0 && criterionStatusMatches;
    if (!emptyCriterionMatches && checks.length === 0) return [];
    return [{ ...group, checks, contextOnly: !criterionStatusMatches }];
  });
}
