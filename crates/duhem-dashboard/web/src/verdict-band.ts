// Pure derivations behind the run header's verdict band (#563). Every
// value here is presentation over the judge's recorded verdicts — the
// band never re-judges; it only names what the run detail already says.

import type { CheckDetail, RunDetail, RunStatus, RunsListEntry, Verdict } from "./api";
import { compactValue, failureParts } from "./format";
import { verdictFamily } from "./ui";

export type BandTone = "pass" | "fail" | "inconclusive" | "live" | "none";

/** The verdict as a word at display size, and the token that colours it. */
export function verdictWord(
  verdict: Verdict | null,
  status: RunStatus,
): { word: string; tone: BandTone } {
  if (status === "running") return { word: "Running", tone: "live" };
  const family = verdictFamily(verdict);
  if (family === "pass") return { word: "Passed", tone: "pass" };
  if (family === "fail") return { word: "Failed", tone: "fail" };
  if (family === "inconclusive") return { word: "Inconclusive", tone: "inconclusive" };
  // No recorded verdict: say why in the run's own lifecycle terms, and
  // keep it monochrome — there is no verdict colour to carry.
  if (status === "aborted") return { word: "Aborted", tone: "none" };
  if (status === "orphaned") return { word: "Orphaned", tone: "none" };
  return { word: "No verdict", tone: "none" };
}

/** "2 of 3 criteria pass" — the judge's per-criterion verdicts, counted. */
export function criteriaTally(run: Pick<RunDetail, "criteria">): string {
  const total = run.criteria.length;
  if (total === 0) return "No criteria recorded";
  const passing = run.criteria.filter((c) => c.verdict === "pass").length;
  return `${passing} of ${total} ${total === 1 ? "criterion passes" : "criteria pass"}`;
}

/** Locate a run's row in the polled runs list, descending into run-set
 *  children. `RunDetail` carries no duration; the list row does. */
export function findRunEntry(
  runs: RunsListEntry[] | null | undefined,
  runId: string,
): RunsListEntry | null {
  for (const entry of runs ?? []) {
    if (entry.run_id === runId) return entry;
    const child = findRunEntry(entry.children, runId);
    if (child) return child;
  }
  return null;
}

/** Wall-clock finish: start + recorded duration. Null while running or
 *  when either half is missing. */
export function finishedAt(
  startedAt: string | null,
  durationMs: number | null | undefined,
): Date | null {
  if (!startedAt || durationMs === null || durationMs === undefined) return null;
  const start = Date.parse(startedAt);
  if (Number.isNaN(start)) return null;
  return new Date(start + durationMs);
}

/** The first check the judge recorded as `fail`, in criteria → check
 *  order — the same order the Results tree lists them. */
export function firstFailedCheck(
  run: Pick<RunDetail, "criteria">,
): { criterionId: string; checkId: string } | null {
  for (const criterion of run.criteria) {
    const check = criterion.checks.find((c) => c.verdict === "fail");
    if (check) return { criterionId: criterion.id, checkId: check.id };
  }
  return null;
}

export interface FirstFailure {
  /** The recorded rule, when the assertion carried one. */
  expr?: string;
  /** Display values, each truncated to about 80 characters. */
  expected?: string;
  observed?: string;
  /** A non-comparison detail, shown verbatim (truncated) instead. */
  reason?: string;
  /** The untruncated detail, for a tooltip. */
  full: string;
}

/** The check's first failed assertion as an expected/observed pair,
 *  split by the same `failureParts` the check page renders, then
 *  truncated so the band stays a single legible line. */
export function firstFailedAssertion(detail: CheckDetail): FirstFailure | null {
  const evt = detail.timeline.find(
    (e) => e.kind === "assertion_evaluated" && e.state === "fail",
  );
  if (!evt) return null;
  const raw =
    (typeof evt.detail === "string" && evt.detail) ||
    (typeof evt.state === "string" ? evt.state : "no detail recorded");
  const parts = failureParts(raw);
  const expr = typeof evt.expr === "string" && evt.expr ? evt.expr : undefined;
  if (parts.expected !== undefined) {
    return {
      expr,
      expected: compactValue(parts.expected),
      observed: compactValue(parts.observed ?? ""),
      full: raw,
    };
  }
  return { expr, reason: compactValue(parts.reason ?? raw), full: raw };
}
