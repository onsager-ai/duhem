// Pure derivations behind the run header's verdict band (#563). Every
// value here is presentation over the judge's recorded verdicts — the
// band never re-judges; it only names what the run detail already says.

import type { CheckDetail, LifecycleBlock, RunDetail, RunStatus, Verdict } from "./api";
import { compactValue, failureParts } from "./format";
import { lifecycleFailures } from "./lifecycle";
import { verdictFamily } from "./ui";

export type BandTone = "pass" | "fail" | "inconclusive" | "live" | "none";

/** The verdict as a word at display size, and the token that colours it. */
export function verdictWord(
  verdict: Verdict | null,
  status: RunStatus,
): { word: string; tone: BandTone } {
  if (status === "pending") return { word: "Pending", tone: "none" };
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

/** Count passing criteria for leaves or passing direct children for
 *  parents, then name failed setup and required cleanup: those decide the
 *  gate too, so "1 of 1 criterion passes" alone would contradict a
 *  failed run (#694). */
export function criteriaTally(run: Pick<RunDetail, "criteria" | "children" | "lifecycle">): string {
  const failures = lifecycleFailures(run);
  const suffix = [
    countPhrase(failures.setup.length, "setup failed", "setups failed"),
    countPhrase(failures.requiredCleanup.length, "required cleanup failed", "required cleanups failed"),
  ]
    .filter(Boolean)
    .map((part) => ` · ${part}`)
    .join("");
  return bodyTally(run) + suffix;
}

function countPhrase(count: number, one: string, many: string): string {
  if (count === 0) return "";
  return count === 1 ? one : `${count} ${many}`;
}

function bodyTally(run: Pick<RunDetail, "criteria" | "children">): string {
  const children = run.children ?? [];
  if (children.length > 0) {
    const passed = children.filter((child) => child.verdict === "pass").length;
    return `${passed} of ${children.length} ${children.length === 1 ? "run passes" : "runs pass"}`;
  }
  const total = run.criteria.length;
  if (total === 0) return "No criteria recorded";
  const passing = run.criteria.filter((c) => c.verdict === "pass").length;
  return `${passing} of ${total} ${total === 1 ? "criterion passes" : "criteria pass"}`;
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

/** A fixed, locale- and timezone-independent stamp to the minute —
 *  `2026-09-27 14:24 UTC` — so a shared screenshot reads the same for
 *  every viewer; `iso` is the full timestamp for a tooltip. Null for a
 *  missing or unparseable value. */
export function utcStamp(value: Date | string | null): { text: string; iso: string } | null {
  if (value === null) return null;
  const date = typeof value === "string" ? new Date(value) : value;
  if (Number.isNaN(date.getTime())) return null;
  const iso = date.toISOString();
  return { text: `${iso.slice(0, 10)} ${iso.slice(11, 16)} UTC`, iso };
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

/** The first failed setup or required cleanup block in recorded order —
 *  the first failure to name when no check failed. */
export function firstFailedLifecycle(run: Pick<RunDetail, "lifecycle">): LifecycleBlock | null {
  const failures = lifecycleFailures(run);
  const gating = new Set([...failures.setup, ...failures.requiredCleanup]);
  return (run.lifecycle ?? []).find((block) => gating.has(block)) ?? null;
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
  // A response without a timeline (a stubbed or foreign payload) names
  // no failure rather than throwing inside the fetch callback.
  const evt = (detail.timeline ?? []).find(
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
